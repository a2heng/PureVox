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
use std::time::{Duration, Instant};

use cpal::traits::{DeviceTrait, StreamTrait};
use cpal::{FromSample, SampleFormat, SizedSample, StreamConfig};
use rtrb::{Producer, RingBuffer};
use serde::Serialize;

use super::fanout::Fanout;
use super::meter::{Meter, RateMeter, SPECTRUM_BIN_HZ};
use super::resampler::ToHops;
use super::{WorkerHandle, HOP, SAMPLE_RATE};
use crate::debug::{now_ms, Probe, SharedHub, StreamInfo};

pub(super) const PUBLISH_PERIOD: Duration = Duration::from_millis(200);
const POLL_PERIOD: Duration = Duration::from_millis(5);
pub(super) const OPEN_TIMEOUT: Duration = Duration::from_secs(5);
/// 超过此时长没有回调即判定流已停止（设备拔出、驱动异常等）。
pub(super) const STALL_LIMIT: Duration = Duration::from_secs(1);

/// 回调线程写、工作线程读的计数器（输入/输出流共用）。
pub(super) struct CallbackStats {
  pub callbacks: AtomicU64,
  pub frames: AtomicU64,
  last_block: AtomicU32,
  min_block: AtomicU32,
  max_block: AtomicU32,
  /// 输入：环满丢弃的样本数；输出：环空补静音的样本数（预热完成后才计）
  pub xruns: AtomicU64,
  pub errors: AtomicU64,
  /// 错误回调罕见且不在数据回调路径上，加锁可接受
  last_error: Mutex<Option<String>>,
}

impl CallbackStats {
  pub fn new() -> Self {
    CallbackStats {
      callbacks: AtomicU64::new(0),
      frames: AtomicU64::new(0),
      last_block: AtomicU32::new(0),
      min_block: AtomicU32::new(u32::MAX),
      max_block: AtomicU32::new(0),
      xruns: AtomicU64::new(0),
      errors: AtomicU64::new(0),
      last_error: Mutex::new(None),
    }
  }

  /// 在数据回调开头调用（仅原子操作）。
  #[inline]
  pub fn on_block(&self, frames: u32) {
    self.callbacks.fetch_add(1, Relaxed);
    self.frames.fetch_add(frames as u64, Relaxed);
    self.last_block.store(frames, Relaxed);
    self.min_block.fetch_min(frames, Relaxed);
    self.max_block.fetch_max(frames, Relaxed);
  }

  pub fn on_error(&self, e: String) {
    self.errors.fetch_add(1, Relaxed);
    *self.last_error.lock().unwrap() = Some(e);
  }

  pub fn last_error(&self) -> Option<String> {
    self.last_error.lock().unwrap().clone()
  }

  pub fn callback_frames(&self) -> Probe<CallbackFrames> {
    if self.callbacks.load(Relaxed) == 0 {
      return Probe::Pending;
    }
    Probe::ok(CallbackFrames {
      last: self.last_block.load(Relaxed),
      min: self.min_block.load(Relaxed),
      max: self.max_block.load(Relaxed),
    })
  }
}

#[derive(Clone, Debug, Serialize)]
pub struct CallbackFrames {
  pub last: u32,
  pub min: u32,
  pub max: u32,
}

/// 回调停顿判定：回调计数持续增长即健康。
pub(super) struct StallWatch {
  last_callbacks: u64,
  last_seen: Instant,
}

impl StallWatch {
  pub fn new() -> Self {
    StallWatch { last_callbacks: 0, last_seen: Instant::now() }
  }

  /// 返回已停顿的毫秒数。
  pub fn update(&mut self, now: Instant, callbacks: u64) -> u128 {
    if callbacks != self.last_callbacks {
      self.last_callbacks = callbacks;
      self.last_seen = now;
    }
    now.duration_since(self.last_seen).as_millis()
  }
}

pub(super) fn stalled_state(stalled_ms: u128, last_error: &Option<String>) -> Option<Probe<String>> {
  (stalled_ms >= STALL_LIMIT.as_millis()).then(|| {
    Probe::unavailable(format!(
      "设备回调已停止 {stalled_ms} ms{}",
      last_error.as_ref().map(|e| format!("（最近错误：{e}）")).unwrap_or_default()
    ))
  })
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
        stats.on_block((data.len() / ch) as u32);
        let mut dropped = 0u64;
        for f in data.chunks_exact(ch) {
          let mut s = 0.0f32;
          for &x in f {
            s += <f32 as FromSample<T>>::from_sample_(x);
          }
          if prod.push(s * inv).is_err() {
            dropped += 1;
          }
        }
        if dropped > 0 {
          stats.xruns.fetch_add(dropped, Relaxed);
        }
      },
      move |e| err_stats.on_error(e.to_string()),
      None,
    )
    .map_err(|e| format!("打开输入流失败：{e}"))
}

/// 打开输入设备并启动采集线程；阻塞到打开成功/失败（最多 5 s）。返回句柄与该源的扇出。
pub fn spawn(hub: SharedHub, device_id: String) -> Result<(WorkerHandle, Arc<Fanout>), String> {
  let stop = Arc::new(AtomicBool::new(false));
  let (tx, rx) = mpsc::channel::<Result<Arc<Fanout>, String>>();
  let stop2 = stop.clone();
  let short: String = device_id.chars().rev().take(8).collect::<String>().chars().rev().collect();
  let join = std::thread::Builder::new()
    .name(format!("capture-{short}"))
    .spawn(move || run(hub, device_id, stop2, tx))
    .map_err(|e| format!("创建采集线程失败：{e}"))?;
  match rx.recv_timeout(OPEN_TIMEOUT) {
    Ok(Ok(fanout)) => Ok((WorkerHandle::new(stop, join), fanout)),
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

fn run(
  hub: SharedHub,
  device_id: String,
  stop: Arc<AtomicBool>,
  tx: mpsc::Sender<Result<Arc<Fanout>, String>>,
) {
  let stats = Arc::new(CallbackStats::new());

  // ---- 打开设备 ----
  let setup = (|| -> Result<_, String> {
    let (dev, name) = crate::devices::find(&device_id, true)?;
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
    Ok(v) => v,
    Err(e) => {
      let _ = tx.send(Err(e));
      return;
    }
  };
  let fanout = Arc::new(Fanout::new(format!("输入：{name}")));
  let _ = tx.send(Ok(fanout.clone()));

  // ---- 工作循环 ----
  let stream_id = format!("input:{device_id}");
  let started_at = now_ms();
  let native_rate = cfg.sample_rate;
  let mut meter = Meter::new();
  let mut in_rate = RateMeter::new();
  let mut out_rate = RateMeter::new();
  let mut stall = StallWatch::new();
  let mut hops: u64 = 0;
  let mut resample_error: Option<String> = None;
  let mut last_publish = Instant::now() - PUBLISH_PERIOD;

  while !stop.load(Relaxed) {
    let n = cons.slots();
    let ring_level_ms = n as f32 * 1000.0 / native_rate as f32;
    if n > 0 {
      if let Ok(chunk) = cons.read_chunk(n) {
        let (a, b) = chunk.as_slices();
        for part in [a, b] {
          let r = to_hops.push(part, |hop| {
            meter.on_hop(hop);
            fanout.push_hop(hop);
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
      let last_error = stats.last_error();
      let callbacks = stats.callbacks.load(Relaxed);
      let stalled_ms = stall.update(now, callbacks);
      // 健康判定：回调持续到达即为运行中；单次流错误（如 WASAPI 不连续标志）只计数
      let state = match (&resample_error, stalled_state(stalled_ms, &last_error)) {
        (Some(e), _) => Probe::unavailable(e.clone()),
        (None, Some(s)) => s,
        (None, None) => Probe::ok("running".to_string()),
      };
      let frames_in = stats.frames.load(Relaxed);
      let frames_out = hops * HOP as u64;
      hub.set_stream(StreamInfo {
        id: stream_id.clone(),
        direction: "input",
        device_id: device_id.clone(),
        device_name: name.clone(),
        state,
        started_at,
        source: None,
        sample_rate: native_rate,
        channels: cfg.channels as u32,
        sample_format: fmt.to_string(),
        measured_input_rate: in_rate.push(now, frames_in),
        output_rate: SAMPLE_RATE,
        measured_output_rate: out_rate.push(now, frames_out),
        resampler: to_hops.description().to_string(),
        resampler_delay_ms: to_hops.delay_frames() as f64 * 1000.0 / SAMPLE_RATE as f64,
        asrc_adjust_ppm: Probe::unavailable("输入流无时钟伺服（重采样比例固定）"),
        callbacks,
        callback_frames: stats.callback_frames(),
        frames_in,
        frames_processed: frames_out,
        hops,
        pending_frames: to_hops.pending_frames() as u32,
        peak_dbfs: m.peak_dbfs,
        rms_dbfs: m.rms_dbfs,
        underruns: Probe::unavailable("输入流无欠载概念"),
        overruns: stats.xruns.load(Relaxed),
        resyncs: Probe::unavailable("输入流无重同步"),
        stream_errors: stats.errors.load(Relaxed),
        last_error,
        buffer_level_ms: ring_level_ms,
        device_buffer_ms: Probe::unavailable("输入流：环形缓冲即设备侧缓冲，见 buffer_level_ms"),
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

  fanout.close();
  drop(stream);
  hub.remove_stream(&stream_id);
}
