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

use rtrb::RingBuffer;

use super::capture::{CallbackStats, OPEN_TIMEOUT, PUBLISH_PERIOD, StallWatch, stalled_state};
use super::fanout::Fanout;
use super::{HOP, SAMPLE_RATE, WorkerHandle};
use crate::audio::tick::Tick;
use crate::debug::{Probe, SharedHub, StreamInfo, now_ms};
use crate::dsp::meter::{Meter, RateMeter, SPECTRUM_BIN_HZ};
use crate::dsp::resampler::Converter;
use crate::dsp::servo::Servo;

/// 48k 源缓冲目标水位（伺服设定点，也是预热量）。
///
/// 2026-11-10 从 40 → 20 → **10 ms**：逐列延迟探针实测「播放 48k 环」是全链最大的一段。
/// 10 ms = 一个 hop；ppm 级时钟差由 PI 伺服（±3%）连续吸收，不靠这个水位兜底。
/// 这是**能接受的最低值**：再低就要改 hop 网格。
pub(crate) const TARGET_MS: f64 = 10.0;
/// 48k 源缓冲封顶。
const CAP_MS: f64 = 300.0;
/// 设备侧缓冲下限：工作线程至少保持约此时长的数据在设备环中。
///
/// 实际目标水位取 `max(3 × 回调周期, 这个值)`：
/// - `IAudioClient3` 低延迟共享（实测 128 帧 / 2.67 ms）→ 10 ms 就够；
/// - 退回普通共享（480 帧 / 10 ms，且会把两次回调并成 22 ms 的大请求）→ 30 ms。
pub(crate) const DEVICE_RING_MS: usize = 10;
const POLL_PERIOD: Duration = Duration::from_millis(2);

/// 比例更新周期。
const RATIO_PERIOD: Duration = Duration::from_millis(50);

/// 一次打开的输出：流 + 设备环 + 格式信息。
struct Setup {
  stream: crate::audio::out::Stream,
  prod: rtrb::Producer<f32>,
  /// 设备环容量（帧）。
  dev_cap: usize,
  /// 设备环目标水位（帧）：工作线程只填到这里，多出来的容量是给回调抽水用的余量。
  dev_target: usize,
  dev_rate: u32,
  channels: u16,
  sample_text: String,
  name: String,
  conv: Converter,
}

fn ms_to_samples(ms: f64) -> usize {
  (ms * SAMPLE_RATE as f64 / 1000.0) as usize
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

fn run(
  hub: SharedHub,
  device_id: String,
  source: Arc<Fanout>,
  tag: String,
  stop: Arc<AtomicBool>,
  tx: mpsc::Sender<Result<(), String>>,
) {
  // 填设备环的线程同样要 MMCSS：回调是实时的，喂数据的不是实时的话低水位必欠载。
  let _rt = crate::audio::rt_guard::enter_audio_thread();
  let stats = Arc::new(CallbackStats::new());
  let primed = Arc::new(AtomicBool::new(false));

  // ---- 打开设备（WASAPI 直连：低延迟共享优先）----
  let setup = crate::audio::with_device_retry(|| -> Result<Setup, String> {
    let (_, raw_name) = crate::devices::find(&device_id, false)?;
    // 调试面板/流名字用枚举缓存里的系统名（Pulse 的 Description），不要 cpal 的裸名。
    let name = crate::devices::display_name(&device_id, false).unwrap_or(raw_name);
    let format = crate::audio::out::probe(&device_id)?;
    let conv = Converter::new(SAMPLE_RATE, format.rate, HOP, true)?;
    // 设备环目标水位：低延迟共享（2.67 ms 周期）10 ms 就够；退回普通共享（10 ms 周期、
    // 还会并成 22 ms 的大请求）要 30 ms。容量多留两个转换器块。
    let dev_target =
      (format.period_frames as usize * 3).max(format.rate as usize * DEVICE_RING_MS / 1000);
    let dev_cap = dev_target + conv.output_frames_max() * 2;
    let (prod, cons) = RingBuffer::<f32>::new(dev_cap);
    let stream = crate::audio::out::start(
      device_id.clone(),
      format.clone(),
      cons,
      stats.clone(),
      primed.clone(),
    )?;
    Ok(Setup {
      stream,
      prod,
      dev_cap,
      dev_target,
      dev_rate: format.rate,
      channels: format.channels,
      sample_text: format.sample_text(),
      name,
      conv,
    })
  });

  let Setup {
    stream,
    prod: mut dev_prod,
    dev_cap,
    dev_target,
    dev_rate,
    channels,
    sample_text,
    name,
    mut conv,
  } = match setup {
    Ok(v) => v,
    Err(e) => {
      let _ = tx.send(Err(e));
      return;
    }
  };
  let _ = tx.send(Ok(()));

  let stream_id = format!("output:{tag}");
  let mut src = source.subscribe(&stream_id);
  let token = crate::debug::threads::ThreadToken::new(format!("playback:{tag}"));

  // ---- 工作循环 ----
  let started_at = now_ms();
  let target = ms_to_samples(TARGET_MS);
  let cap = ms_to_samples(CAP_MS);
  // 预热量 = 目标水位 + 首次填满设备环要消耗的量；否则预热一结束，填设备环就把 48k 环抽到目标以下，
  // 立刻触发重同步，且伺服积分在低水位期间被带偏
  let prewarm_level = target + dev_target + HOP;
  let mut hop = vec![0.0f32; HOP];
  let mut meter = Meter::new();
  let mut in_rate = RateMeter::new();
  let mut out_rate = RateMeter::new();
  let mut undr_rate = RateMeter::new();
  let mut resync_rate = RateMeter::new();
  let mut drop_rate = RateMeter::new();
  let mut err_rate = RateMeter::new();
  let mut stall = StallWatch::new();
  let mut servo = Servo::new(TARGET_MS);
  let mut prewarm = true;
  // 一个 hop 的转换器输出暂存（按目标水位逐样本推设备环，见下面的填环循环）。
  let mut stage: Vec<f32> = Vec::with_capacity(conv.output_frames_max() * 2);
  let mut stage_pos = 0usize;
  let mut consumed: u64 = 0;
  let mut hops: u64 = 0;
  let mut dropped: u64 = 0;
  let mut resyncs: u64 = 0;
  let mut worker_error: Option<String> = None;
  let mut last_tick = Instant::now();
  let mut last_ratio = Instant::now();
  let mut last_publish = Instant::now() - PUBLISH_PERIOD;
  // 2 ms 轮询同样用高精度等待（Windows 默认 sleep 粒度 15.6 ms 会直接把设备环抽干）。
  let mut poll_tick = Tick::new(POLL_PERIOD);

  while !stop.load(Relaxed) {
    token.beat();
    // 封顶：丢最旧
    let fill = src.slots();
    if fill > cap {
      let n = fill - target;
      if let Ok(c) = src.read_chunk(n) {
        c.commit_all();
        dropped += n as u64;
      }
    }

    // 填设备环：逐**样本**补到目标水位。
    //
    // 不能按 hop 整块推：转换器一次出 480 帧，整块推的话水位只在 0/480 之间跳，
    // 回调（128 帧一次）必然抽到 0 → 欠载。这里用 `stage` 暂存转换器输出，
    // 按目标水位一次推多少算多少，粒度降到 1 帧。一轮里能喂多少喂多少，
    // 喂不进去（设备环到目标水位）就留到下一轮。
    loop {
      if worker_error.is_some() {
        break;
      }
      while stage_pos < stage.len()
        && dev_prod.slots() > 0
        && dev_cap - dev_prod.slots() < dev_target
      {
        let _ = dev_prod.push(stage[stage_pos]);
        stage_pos += 1;
      }
      if stage_pos < stage.len() {
        break; // 设备环到水位了
      }
      stage.clear();
      stage_pos = 0;
      if prewarm {
        if src.slots() < prewarm_level {
          break;
        }
        prewarm = false;
        servo.reset_level();
        primed.store(true, Relaxed);
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
        Ok(out) => stage.extend_from_slice(out),
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
        if let Err(e) = conv.set_relative_ratio(1.0 - servo.adj()) {
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
      let underruns = stats.xruns.load(Relaxed);
      let errors = stats.errors.load(Relaxed);
      let mut st = StreamInfo {
        id: stream_id.clone(),
        direction: "output",
        device_id: device_id.clone(),
        device_name: name.clone(),
        state,
        health: Probe::Pending, // 下面 health::annotate 会按 state + 计数/速率填上
        started_at,
        source: Some(source.name().to_string()),
        denoise: Probe::unavailable("输出流不适用（降噪在输入侧）"),
        sample_rate: dev_rate,
        channels: channels as u32,
        sample_format: sample_text.clone(),
        measured_input_rate: in_rate.push(now, consumed),
        output_rate: SAMPLE_RATE,
        measured_output_rate: out_rate.push(now, stats.frames.load(Relaxed)),
        resampler: conv.description().to_string(),
        resampler_delay_ms: conv.delay_frames() as f64 * 1000.0 / dev_rate as f64,
        asrc_adjust_ppm: Probe::ok(servo.adj() * 1e6),
        callbacks,
        callback_frames: stats.callback_frames(),
        frames_in: consumed,
        frames_processed: stats.frames.load(Relaxed),
        hops,
        pending_frames: 0,
        peak_dbfs: m.peak_dbfs,
        rms_dbfs: m.rms_dbfs,
        underruns: Probe::ok(underruns),
        overruns: dropped,
        resyncs: Probe::ok(resyncs),
        stream_errors: errors,
        underrun_per_s: undr_rate.push(now, underruns),
        resync_per_s: resync_rate.push(now, resyncs),
        overrun_per_s: drop_rate.push(now, dropped),
        stream_error_per_s: err_rate.push(now, errors),
        last_error,
        buffer_level_ms: src.slots() as f32 * 1000.0 / SAMPLE_RATE as f32,
        device_buffer_ms: Probe::ok(dev_level),
        latency_ms: Probe::unavailable("逐列延迟见 /debug/latency（单路流不含跨设备路径）"),
        inference_ms_avg: Probe::unavailable("未接入模型"),
        inference_ms_max: Probe::unavailable("未接入模型"),
        waveform: m.waveform,
        spectrum_db: m.spectrum_db,
        spectrum_bin_hz: SPECTRUM_BIN_HZ,
        spectrum_bands_db: m.spectrum_bands_db,
        spectrum_band_hz: crate::dsp::bands::BAND_HZ,
      };
      super::health::annotate(&mut st);
      hub.set_stream(st);
    }
    poll_tick.wait();
  }

  source.unsubscribe(&stream_id);
  drop(stream);
  hub.remove_stream(&stream_id);
}
