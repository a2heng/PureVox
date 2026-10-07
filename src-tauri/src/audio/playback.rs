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

//! 单路输出（播放正确性只在本文件一处实现）。
//!
//! 设备回调是唯一主时钟。数据流：
//! 源扇出（48 kHz hop）→ 48k 订阅环 → 工作线程：[`Converter`] 重采样到设备采样率 → 设备环 → 回调。
//!
//! 工作线程的缓冲策略（回调里不做任何策略，只取样或补静音）：
//! - 预热：48k 环攒够 `TARGET_MS` 才开始输出；
//! - 时钟伺服：PI 控制器按 48k 环水位（EMA 平滑）微调重采样比例，限幅 ±3%，
//!   消化源时钟与设备时钟的速率差；
//! - 欠载：源断流 → 设备环耗尽后回调补静音，工作线程回到预热（重同步）；
//! - 封顶：48k 环超过 `CAP_MS` 时丢弃最旧样本回到目标水位。

use std::sync::atomic::{AtomicBool, Ordering::Relaxed};
use std::sync::{Arc, mpsc};
use std::time::{Duration, Instant};

use cpal::traits::{DeviceTrait, StreamTrait};
use cpal::{FromSample, SampleFormat, SizedSample, StreamConfig};
use rtrb::{Consumer, RingBuffer};

use super::capture::{CallbackStats, OPEN_TIMEOUT, PUBLISH_PERIOD, StallWatch, stalled_state};
use super::fanout::Fanout;
use super::{HOP, SAMPLE_RATE, WorkerHandle};
use crate::debug::{Probe, SharedHub, StreamInfo, now_ms};
use crate::dsp::meter::{Meter, RateMeter, SPECTRUM_BIN_HZ};
use crate::dsp::resampler::Converter;

/// 48k 源缓冲目标水位（伺服设定点，也是预热量）。
const TARGET_MS: f64 = 40.0;
/// 48k 源缓冲封顶。
const CAP_MS: f64 = 300.0;
/// 设备侧缓冲：工作线程尽量保持约此时长的数据在设备环中。
const DEVICE_RING_MS: usize = 30;
const POLL_PERIOD: Duration = Duration::from_millis(2);

/// 伺服参数：闭环约为二阶，时间常数约 5 s、阻尼约 0.7（误差单位 ms）。
const KP: f64 = 2.0e-4;
const KI: f64 = 2.0e-5;
const ADJ_LIMIT: f64 = 0.03;
/// 水位 EMA 时间常数。
const EMA_TAU_S: f64 = 1.0;
/// 比例更新周期。
const RATIO_PERIOD: Duration = Duration::from_millis(50);

fn ms_to_samples(ms: f64) -> usize {
  (ms * SAMPLE_RATE as f64 / 1000.0) as usize
}

fn build_stream<T>(
  dev: &cpal::Device,
  cfg: &StreamConfig,
  mut cons: Consumer<f32>,
  stats: Arc<CallbackStats>,
  primed: Arc<AtomicBool>,
) -> Result<cpal::Stream, String>
where
  T: SizedSample + FromSample<f32>,
{
  let ch = cfg.channels.max(1) as usize;
  let err_stats = stats.clone();
  dev
    .build_output_stream::<T, _, _>(
      *cfg,
      move |data: &mut [T], _info| {
        stats.on_block((data.len() / ch) as u32);
        let mut silent = 0u64;
        for f in data.chunks_exact_mut(ch) {
          let s = match cons.pop() {
            Ok(v) => v,
            Err(_) => {
              silent += 1;
              0.0
            }
          };
          let v = <T as FromSample<f32>>::from_sample_(s);
          for x in f.iter_mut() {
            *x = v;
          }
        }
        if silent > 0 && primed.load(Relaxed) {
          stats.xruns.fetch_add(silent, Relaxed);
        }
      },
      move |e| err_stats.on_error(e.to_string()),
      None,
    )
    .map_err(|e| format!("打开输出流失败：{e}"))
}

/// 打开输出设备并开始播放 `source`；阻塞到打开成功/失败（最多 5 s）。
/// `tag`：本流在调试接口里的唯一标识（同一设备可有多个流）。
pub fn spawn(
  hub: SharedHub,
  device_id: String,
  source: Arc<Fanout>,
  tag: String,
) -> Result<WorkerHandle, String> {
  let stop = Arc::new(AtomicBool::new(false));
  let (tx, rx) = mpsc::channel::<Result<(), String>>();
  let stop2 = stop.clone();
  let short: String = device_id
    .chars()
    .rev()
    .take(8)
    .collect::<String>()
    .chars()
    .rev()
    .collect();
  let join = std::thread::Builder::new()
    .name(format!("playback-{short}"))
    .spawn(move || run(hub, device_id, source, tag, stop2, tx))
    .map_err(|e| format!("创建播放线程失败：{e}"))?;
  match rx.recv_timeout(OPEN_TIMEOUT) {
    Ok(Ok(())) => Ok(WorkerHandle::new(stop, join)),
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

/// PI 时钟伺服：输入 48k 环水位（ms），输出比例修正量 adj（正 = 消耗更快）。
struct Servo {
  ema_ms: f64,
  integral: f64,
  adj: f64,
}

impl Servo {
  fn new() -> Self {
    Servo {
      ema_ms: TARGET_MS,
      integral: 0.0,
      adj: 0.0,
    }
  }

  /// 预热完成 / 重同步后，从设定点重新起算水位（保留积分项 = 已学到的速率差）。
  fn reset_level(&mut self) {
    self.ema_ms = TARGET_MS;
  }

  fn update(&mut self, level_ms: f64, dt_s: f64) {
    let a = (dt_s / EMA_TAU_S).min(1.0);
    self.ema_ms += a * (level_ms - self.ema_ms);
    let e = self.ema_ms - TARGET_MS;
    self.integral += e * dt_s;
    // 抗饱和：积分项单独不超过限幅
    let i_lim = ADJ_LIMIT / KI;
    self.integral = self.integral.clamp(-i_lim, i_lim);
    self.adj = (KP * e + KI * self.integral).clamp(-ADJ_LIMIT, ADJ_LIMIT);
  }
}

fn run(
  hub: SharedHub,
  device_id: String,
  source: Arc<Fanout>,
  tag: String,
  stop: Arc<AtomicBool>,
  tx: mpsc::Sender<Result<(), String>>,
) {
  let stats = Arc::new(CallbackStats::new());
  let primed = Arc::new(AtomicBool::new(false));

  // ---- 打开设备 ----
  let setup = (|| -> Result<_, String> {
    let (dev, name) = crate::devices::find(&device_id, false)?;
    let supported = dev
      .default_output_config()
      .map_err(|e| format!("读取默认格式失败：{e}"))?;
    let cfg = supported.config();
    let fmt = supported.sample_format();
    let conv = Converter::new(SAMPLE_RATE, cfg.sample_rate, HOP, true)?;
    let dev_cap = cfg.sample_rate as usize * DEVICE_RING_MS / 1000 + conv.output_frames_max();
    let (prod, cons) = RingBuffer::<f32>::new(dev_cap);
    let stream = match fmt {
      SampleFormat::F32 => build_stream::<f32>(&dev, &cfg, cons, stats.clone(), primed.clone()),
      SampleFormat::I16 => build_stream::<i16>(&dev, &cfg, cons, stats.clone(), primed.clone()),
      SampleFormat::I32 => build_stream::<i32>(&dev, &cfg, cons, stats.clone(), primed.clone()),
      SampleFormat::U16 => build_stream::<u16>(&dev, &cfg, cons, stats.clone(), primed.clone()),
      other => Err(format!("不支持的采样格式 {other}")),
    }?;
    stream.play().map_err(|e| format!("启动输出流失败：{e}"))?;
    Ok((stream, prod, dev_cap, cfg, fmt, name, conv))
  })();

  let (stream, mut dev_prod, dev_cap, cfg, fmt, name, mut conv) = match setup {
    Ok(v) => v,
    Err(e) => {
      let _ = tx.send(Err(e));
      return;
    }
  };
  let _ = tx.send(Ok(()));

  let stream_id = format!("output:{tag}");
  let mut src = source.subscribe(&stream_id);

  // ---- 工作循环 ----
  let started_at = now_ms();
  let dev_rate = cfg.sample_rate;
  let target = ms_to_samples(TARGET_MS);
  let cap = ms_to_samples(CAP_MS);
  // 预热量 = 目标水位 + 首次填满设备环要消耗的量；否则预热一结束，填设备环就把 48k 环抽到目标以下，
  // 立刻触发重同步，且伺服积分在低水位期间被带偏
  let prewarm_level = target + ms_to_samples(DEVICE_RING_MS as f64) + HOP;
  let mut hop = vec![0.0f32; HOP];
  let mut meter = Meter::new();
  let mut in_rate = RateMeter::new();
  let mut out_rate = RateMeter::new();
  let mut stall = StallWatch::new();
  let mut servo = Servo::new();
  let mut prewarm = true;
  let mut consumed: u64 = 0;
  let mut hops: u64 = 0;
  let mut dropped: u64 = 0;
  let mut resyncs: u64 = 0;
  let mut worker_error: Option<String> = None;
  let mut last_tick = Instant::now();
  let mut last_ratio = Instant::now();
  let mut last_publish = Instant::now() - PUBLISH_PERIOD;

  while !stop.load(Relaxed) {
    // 封顶：丢最旧
    let fill = src.slots();
    if fill > cap {
      let n = fill - target;
      if let Ok(c) = src.read_chunk(n) {
        c.commit_all();
        dropped += n as u64;
      }
    }

    // 填设备环
    while worker_error.is_none() && dev_prod.slots() >= conv.output_frames_max() {
      if prewarm {
        if src.slots() >= prewarm_level {
          prewarm = false;
          servo.reset_level();
          primed.store(true, Relaxed);
        } else {
          break;
        }
      }
      if src.slots() < HOP {
        // 源断流：设备环耗尽后回调补静音，回到预热
        prewarm = true;
        resyncs += 1;
        break;
      }
      if let Ok(c) = src.read_chunk(HOP) {
        let (a, b) = c.as_slices();
        hop[..a.len()].copy_from_slice(a);
        hop[a.len()..].copy_from_slice(b);
        c.commit_all();
      }
      consumed += HOP as u64;
      hops += 1;
      meter.on_hop(&hop);
      match conv.process(&hop) {
        Ok(out) => {
          for &x in out {
            let _ = dev_prod.push(x);
          }
        }
        Err(e) => worker_error = Some(e),
      }
    }

    // 伺服：每轮更新水位，周期性下发比例
    let now = Instant::now();
    let dt = now.duration_since(last_tick).as_secs_f64();
    last_tick = now;
    if !prewarm {
      servo.update(src.slots() as f64 * 1000.0 / SAMPLE_RATE as f64, dt);
      if now.duration_since(last_ratio) >= RATIO_PERIOD {
        last_ratio = now;
        // adj>0 = 水位高于目标 = 要消耗更快。比例是「输出/输入」，FixedAsync::Input 下每读 1 hop(480)
        // 产出 ~480*ratio 个设备样本，故 48k 环的消耗速率 = 100/ratio hop/s：
        // ratio 越大 → 读 hop 越慢 → 48k 环越满。要排水位就得 ratio<1 → 用 (1.0 - adj)。
        if let Err(e) = conv.set_relative_ratio(1.0 - servo.adj) {
          worker_error = Some(e);
        }
      }
    }

    if now.duration_since(last_publish) >= PUBLISH_PERIOD {
      last_publish = now;
      let m = meter.take();
      let last_error = stats.last_error();
      let callbacks = stats.callbacks.load(Relaxed);
      let stalled_ms = stall.update(now, callbacks);
      let state = if let Some(e) = &worker_error {
        Probe::unavailable(e.clone())
      } else if let Some(s) = stalled_state(stalled_ms, &last_error) {
        s
      } else if source.is_closed() {
        Probe::unavailable(format!("信号源已停止（{}）", source.name()))
      } else if prewarm {
        Probe::ok(format!(
          "预热中（等待 48k 缓冲达到 {:.0} ms）",
          prewarm_level as f64 * 1000.0 / SAMPLE_RATE as f64
        ))
      } else {
        Probe::ok("running".to_string())
      };
      let dev_level = (dev_cap - dev_prod.slots()) as f32 * 1000.0 / dev_rate as f32;
      hub.set_stream(StreamInfo {
        id: stream_id.clone(),
        direction: "output",
        device_id: device_id.clone(),
        device_name: name.clone(),
        state,
        started_at,
        source: Some(source.name().to_string()),
        denoise: Probe::unavailable("输出流不适用（降噪在输入侧）"),
        sample_rate: dev_rate,
        channels: cfg.channels as u32,
        sample_format: fmt.to_string(),
        measured_input_rate: in_rate.push(now, consumed),
        output_rate: SAMPLE_RATE,
        measured_output_rate: out_rate.push(now, stats.frames.load(Relaxed)),
        resampler: conv.description().to_string(),
        resampler_delay_ms: conv.delay_frames() as f64 * 1000.0 / dev_rate as f64,
        asrc_adjust_ppm: Probe::ok(servo.adj * 1e6),
        callbacks,
        callback_frames: stats.callback_frames(),
        frames_in: consumed,
        frames_processed: stats.frames.load(Relaxed),
        hops,
        pending_frames: 0,
        peak_dbfs: m.peak_dbfs,
        rms_dbfs: m.rms_dbfs,
        underruns: Probe::ok(stats.xruns.load(Relaxed)),
        overruns: dropped,
        resyncs: Probe::ok(resyncs),
        stream_errors: stats.errors.load(Relaxed),
        last_error,
        buffer_level_ms: src.slots() as f32 * 1000.0 / SAMPLE_RATE as f32,
        device_buffer_ms: Probe::ok(dev_level),
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

  source.unsubscribe(&stream_id);
  drop(stream);
  hub.remove_stream(&stream_id);
}
