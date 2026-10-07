// PureVox — AI 麦克风降噪工具
// Copyright (C) 2024-2026 a2heng <752848283@qq.com>
//
// PureVox is licensed under the GNU General Public License v3.0 or
// later (GPL-3.0-or-later).  See LICENSE for details.
//
// This program is free software: you can redistribute it and/or modify
// it under the terms of the GNU General Public License as published by
// the Free Software Foundation, either version 3 of the License, or
// (at your option) any later version.
//
// The built-in AI models are NOT covered by the GPL; they are the
// property of a2heng and may only be used with PureVox under
// authorization.  See MODEL-LICENSE.md for details.
//
// SPDX-License-Identifier: GPL-3.0-or-later

//! 单路输入采集。
//!
//! 每路一个工作线程，独占 cpal 流（在本线程创建、本线程销毁）：
//! - 回调线程（实时）：只做下混单声道 + 写 rtrb 无锁环 + 原子计数，不分配、不加锁。
//! - 工作线程：读环 → [`ToHops`] 重采样并切 hop → [`Meter`] 测量 → 每 200 ms 发布到 DebugHub。

use std::sync::atomic::{AtomicBool, AtomicU32, AtomicU64, Ordering::Relaxed};
use std::sync::{mpsc, Arc, Mutex};
use std::thread::JoinHandle;
use std::time::{Duration, Instant};

use cpal::traits::{DeviceTrait, HostTrait, StreamTrait};
use cpal::{FromSample, SampleFormat, SizedSample, StreamConfig};
use rtrb::{Producer, RingBuffer};
use serde::Serialize;

use super::meter::{Meter, RateMeter, SPECTRUM_BIN_HZ};
use super::resampler::ToHops;
use super::{HOP, SAMPLE_RATE};
use crate::debug::{now_ms, Probe, SharedHub, StreamInfo};

const PUBLISH_PERIOD: Duration = Duration::from_millis(200);
const POLL_PERIOD: Duration = Duration::from_millis(5);
const OPEN_TIMEOUT: Duration = Duration::from_secs(5);
/// 超过此时长没有回调即判定流已停止（设备拔出、驱动异常等）。
const STALL_LIMIT: Duration = Duration::from_secs(1);

/// 回调线程写、工作线程读的计数器。
struct CallbackStats {
  callbacks: AtomicU64,
  frames: AtomicU64,
  last_block: AtomicU32,
  min_block: AtomicU32,
  max_block: AtomicU32,
  dropped: AtomicU64,
  errors: AtomicU64,
  /// 错误回调罕见且不在数据回调路径上，加锁可接受
  last_error: Mutex<Option<String>>,
}

impl CallbackStats {
  fn new() -> Self {
    CallbackStats {
      callbacks: AtomicU64::new(0),
      frames: AtomicU64::new(0),
      last_block: AtomicU32::new(0),
      min_block: AtomicU32::new(u32::MAX),
      max_block: AtomicU32::new(0),
      dropped: AtomicU64::new(0),
      errors: AtomicU64::new(0),
      last_error: Mutex::new(None),
    }
  }
}

#[derive(Clone, Debug, Serialize)]
pub struct CallbackFrames {
  pub last: u32,
  pub min: u32,
  pub max: u32,
}

pub struct CaptureHandle {
  stop: Arc<AtomicBool>,
  join: Option<JoinHandle<()>>,
}

impl CaptureHandle {
  pub fn stop(mut self) {
    self.stop.store(true, Relaxed);
    if let Some(j) = self.join.take() {
      let _ = j.join();
    }
  }
}

fn find_input(device_id: &str) -> Result<(cpal::Device, String), String> {
  for host_id in cpal::available_hosts() {
    let Ok(host) = cpal::host_from_id(host_id) else { continue };
    let Ok(devs) = host.input_devices() else { continue };
    for d in devs {
      if d.id().map(|i| i.to_string()).ok().as_deref() == Some(device_id) {
        let name = d.description().map(|x| x.name().to_string()).unwrap_or_else(|_| d.to_string());
        return Ok((d, name));
      }
    }
  }
  Err(format!("找不到输入设备 {device_id}（可能已拔出，请刷新设备列表）"))
}

fn build_stream<T>(
  dev: &cpal::Device,
  cfg: &StreamConfig,
  mut prod: Producer<f32>,
  stats: Arc<CallbackStats>,
) -> Result<cpal::Stream, String>
where
  T: SizedSample,
  f32: FromSample<T>,
{
  let ch = cfg.channels.max(1) as usize;
  let inv = 1.0 / ch as f32;
  let err_stats = stats.clone();
  dev
    .build_input_stream::<T, _, _>(
      cfg.clone(),
      move |data: &[T], _info| {
        let frames = (data.len() / ch) as u32;
        stats.callbacks.fetch_add(1, Relaxed);
        stats.frames.fetch_add(frames as u64, Relaxed);
        stats.last_block.store(frames, Relaxed);
        stats.min_block.fetch_min(frames, Relaxed);
        stats.max_block.fetch_max(frames, Relaxed);
        let mut dropped = 0u64;
        for f in data.chunks_exact(ch) {
          let mut s = 0.0f32;
          for &x in f {
            s += x.to_sample::<f32>();
          }
          if prod.push(s * inv).is_err() {
            dropped += 1;
          }
        }
        if dropped > 0 {
          stats.dropped.fetch_add(dropped, Relaxed);
        }
      },
      move |e| {
        err_stats.errors.fetch_add(1, Relaxed);
        *err_stats.last_error.lock().unwrap() = Some(e.to_string());
      },
      None,
    )
    .map_err(|e| format!("打开输入流失败：{e}"))
}

/// 打开设备并启动采集线程；阻塞到打开成功/失败（最多 5 s）。
pub fn spawn(hub: SharedHub, device_id: String) -> Result<CaptureHandle, String> {
  let stop = Arc::new(AtomicBool::new(false));
  let (tx, rx) = mpsc::channel::<Result<(), String>>();
  let stop2 = stop.clone();
  let short: String = device_id.chars().rev().take(8).collect::<String>().chars().rev().collect();
  let join = std::thread::Builder::new()
    .name(format!("capture-{short}"))
    .spawn(move || run(hub, device_id, stop2, tx))
    .map_err(|e| format!("创建采集线程失败：{e}"))?;
  match rx.recv_timeout(OPEN_TIMEOUT) {
    Ok(Ok(())) => Ok(CaptureHandle { stop, join: Some(join) }),
    Ok(Err(e)) => {
      let _ = join.join();
      Err(e)
    }
    Err(_) => {
      stop.store(true, Relaxed);
      Err("打开设备超时（5 s）".into())
    }
  }
}

fn run(hub: SharedHub, device_id: String, stop: Arc<AtomicBool>, tx: mpsc::Sender<Result<(), String>>) {
  let stats = Arc::new(CallbackStats::new());

  // ---- 打开设备 ----
  let setup = (|| -> Result<_, String> {
    let (dev, name) = find_input(&device_id)?;
    let supported = dev.default_input_config().map_err(|e| format!("读取默认格式失败：{e}"))?;
    let cfg = supported.config();
    let fmt = supported.sample_format();
    // 环形缓冲 500 ms（原生采样率单声道）
    let (prod, cons) = RingBuffer::<f32>::new((cfg.sample_rate as usize / 2).max(HOP * 4));
    let stream = match fmt {
      SampleFormat::F32 => build_stream::<f32>(&dev, &cfg, prod, stats.clone()),
      SampleFormat::I16 => build_stream::<i16>(&dev, &cfg, prod, stats.clone()),
      SampleFormat::I32 => build_stream::<i32>(&dev, &cfg, prod, stats.clone()),
      SampleFormat::U16 => build_stream::<u16>(&dev, &cfg, prod, stats.clone()),
      other => Err(format!("不支持的采样格式 {other}")),
    }?;
    let to_hops = ToHops::new(cfg.sample_rate)?;
    stream.play().map_err(|e| format!("启动输入流失败：{e}"))?;
    Ok((stream, cons, cfg, fmt, name, to_hops))
  })();

  let (stream, mut cons, cfg, fmt, name, mut to_hops) = match setup {
    Ok(v) => {
      let _ = tx.send(Ok(()));
      v
    }
    Err(e) => {
      let _ = tx.send(Err(e));
      return;
    }
  };

  // ---- 工作循环 ----
  let started_at = now_ms();
  let native_rate = cfg.sample_rate;
  let mut meter = Meter::new();
  let mut in_rate = RateMeter::new();
  let mut out_rate = RateMeter::new();
  let mut hops: u64 = 0;
  let mut resample_error: Option<String> = None;
  let mut last_publish = Instant::now() - PUBLISH_PERIOD;
  // 健康判定：回调持续到达即为运行中；单次流错误（如 WASAPI 不连续标志）只计数
  let mut last_callbacks = 0u64;
  let mut last_callback_seen = Instant::now();

  while !stop.load(Relaxed) {
    let n = cons.slots();
    let ring_level_ms = n as f32 * 1000.0 / native_rate as f32;
    if n > 0 {
      if let Ok(chunk) = cons.read_chunk(n) {
        let (a, b) = chunk.as_slices();
        for part in [a, b] {
          let r = to_hops.push(part, |hop| {
            meter.on_hop(hop);
            hops += 1;
          });
          if let Err(e) = r {
            resample_error = Some(e);
          }
        }
        chunk.commit_all();
      }
    }

    let now = Instant::now();
    if now.duration_since(last_publish) >= PUBLISH_PERIOD {
      last_publish = now;
      let m = meter.take();
      let errors = stats.errors.load(Relaxed);
      let last_error = stats.last_error.lock().unwrap().clone();
      let callbacks = stats.callbacks.load(Relaxed);
      if callbacks != last_callbacks {
        last_callbacks = callbacks;
        last_callback_seen = now;
      }
      let stalled_ms = now.duration_since(last_callback_seen).as_millis();
      let state = if let Some(e) = &resample_error {
        Probe::unavailable(e.clone())
      } else if stalled_ms >= STALL_LIMIT.as_millis() {
        Probe::unavailable(format!(
          "设备回调已停止 {stalled_ms} ms{}",
          last_error.as_ref().map(|e| format!("（最近错误：{e}）")).unwrap_or_default()
        ))
      } else {
        Probe::ok("running".to_string())
      };
      let callback_frames = if callbacks == 0 {
        Probe::Pending
      } else {
        Probe::ok(CallbackFrames {
          last: stats.last_block.load(Relaxed),
          min: stats.min_block.load(Relaxed),
          max: stats.max_block.load(Relaxed),
        })
      };
      let frames_in = stats.frames.load(Relaxed);
      let frames_out = hops * HOP as u64;
      hub.set_stream(StreamInfo {
        id: device_id.clone(),
        direction: "input",
        device_id: device_id.clone(),
        device_name: name.clone(),
        state,
        started_at,
        sample_rate: native_rate,
        channels: cfg.channels as u32,
        sample_format: fmt.to_string(),
        measured_input_rate: in_rate.push(now, frames_in),
        output_rate: SAMPLE_RATE,
        measured_output_rate: out_rate.push(now, frames_out),
        resampler: to_hops.description().to_string(),
        resampler_delay_ms: to_hops.delay_frames() as f64 * 1000.0 / SAMPLE_RATE as f64,
        callbacks,
        callback_frames,
        frames_in,
        frames_processed: frames_out,
        hops,
        pending_frames: to_hops.pending_frames() as u32,
        peak_dbfs: m.peak_dbfs,
        rms_dbfs: m.rms_dbfs,
        underruns: Probe::unavailable("输入流无欠载概念"),
        overruns: stats.dropped.load(Relaxed),
        stream_errors: errors,
        last_error,
        buffer_level_ms: ring_level_ms,
        latency_ms: Probe::unavailable("端到端延迟尚未测量"),
        inference_ms_avg: Probe::unavailable("未接入模型"),
        inference_ms_max: Probe::unavailable("未接入模型"),
        waveform: m.waveform,
        spectrum_db: m.spectrum_db,
        spectrum_bin_hz: SPECTRUM_BIN_HZ,
      });
    }
    std::thread::sleep(POLL_PERIOD);
  }

  drop(stream);
  hub.remove_stream(&device_id);
}
