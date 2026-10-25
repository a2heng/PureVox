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

//! 单路输入采集（cpal 输入设备 + WASAPI 回环共用同一工作循环）。
//!
//! 数据源（cpal 回调 / WASAPI 轮询）只做下混单声道 + 写 rtrb 无锁环 + 原子计数；
//! 工作线程 [`worker`] 读环 → [`ToHops`] 重采样并切 hop → [`Meter`] 测量 → 每 200 ms
//! 发布到 DebugHub，并把 hop 推入本源的扇出（列工作线程作为该列的一个输入行订阅）。
//! 处理链不在这里：见 `engine::session`（DESIGN.md §3.3）。

use std::sync::atomic::{AtomicBool, AtomicU32, AtomicU64, Ordering::Relaxed};
use std::sync::{Arc, Mutex, mpsc};
use std::time::{Duration, Instant};

use cpal::traits::{DeviceTrait, StreamTrait};
use cpal::{FromSample, SampleFormat, SizedSample, StreamConfig};
use rtrb::{Producer, RingBuffer};
use serde::Serialize;

use super::fanout::Fanout;
use super::{HOP, SAMPLE_RATE, WorkerHandle};
use crate::debug::{Probe, SharedHub, StreamInfo, now_ms};
use crate::dsp::meter::{Meter, RateMeter, SPECTRUM_BIN_HZ};
use crate::dsp::resampler::ToHops;

/// 一路采集：工作线程句柄 + 本源的 48k hop 扇出。
pub struct Capture {
  pub handle: WorkerHandle,
  pub fanout: Arc<Fanout>,
}

pub(super) const PUBLISH_PERIOD: Duration = Duration::from_millis(200);
const POLL_PERIOD: Duration = Duration::from_millis(5);
/// 设备环**目标水位**（ms）：延迟封顶（[`DEV_MAX_MS`]）触发时保留这么多，其余丢掉。
/// 取一格（10 ms）：保留垫层只为吸收本线程 5 ms 的轮询抖动，不参与「攒延迟」。
const DEV_TARGET_MS: usize = 10;
/// 设备环**硬顶**（ms）：超过就丢最旧到 [`DEV_TARGET_MS`]。
///
/// **这一条是本路源的延迟硬保证**，也是 v18 缺失的那一条。超硬顶的样本是
/// **已经过时的音频**（流刚打开时设备预填的一大块、会话重建期间的积压、调度停顿），
/// 正确动作是**丢掉、跳到现在**；不是「按 2× 慢慢放出去」—— 那等于把一次突发变成
/// 「之后十几秒都在听旧声音」。实测（2026-10-18，单列 D80 麦克风）：v18 把它当待播数据，
/// 设备环长期停在 **289 ms**（唯一出口是 300 ms 那道闸，而放行速率恰好等于源速率，
/// 水位是随机游走、只进不出）；硬顶改成 60 ms 后同一条链回到 10 ms 量级。
///
/// 60 ms 的依据：必须**高于正常回调块**，否则每个块都要丢一次 —— cpal 实测块
/// 5~21 ms（本机 PipeWire 241 帧 = 5 ms）、`parec` 块 = [`super::loopback::PARE_LATENCY_MS`]
/// （30 ms）。块比硬顶还大的源要压的是**源自己的缓冲**，不是抬高这个值。
const DEV_MAX_MS: usize = 60;
/// 单拍放行上限（格）：恒等于实时 —— 突发源的整块因此被摊成平滑 hop 流，
/// 不会把下游 10 ms 级的有界环一次冲爆（`ROW_CAP` 丢最旧，听感是规律地卡）。
const RELEASE_SLOTS: usize = 1;
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
  /// 本流建立的时刻：回调间隔以它为基准（`Instant` 进不了原子量，故存相对纳秒）
  base: Instant,
  /// 上一次回调相对 `base` 的纳秒数；0 = 还没有过回调
  last_cb_ns: AtomicU64,
  /// 相邻两次回调的间隔（微秒）：最近 / 最大。设备按不按时喂块，比块大小更直接
  last_gap_us: AtomicU32,
  max_gap_us: AtomicU32,
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
      base: Instant::now(),
      last_cb_ns: AtomicU64::new(0),
      last_gap_us: AtomicU32::new(0),
      max_gap_us: AtomicU32::new(0),
    }
  }

  /// 在数据回调开头调用（仅原子操作 + 读一次时钟，不加锁）。
  #[inline]
  pub fn on_block(&self, frames: u32) {
    self.callbacks.fetch_add(1, Relaxed);
    self.frames.fetch_add(frames as u64, Relaxed);
    self.last_block.store(frames, Relaxed);
    self.min_block.fetch_min(frames, Relaxed);
    self.max_block.fetch_max(frames, Relaxed);
    // 回调间隔：首个回调不记（`base` 到首块的距离是启动开销，不是设备节拍），
    // 之后每两次之间的间隔都算 —— 抽一下能立刻看见「某次回调晚了 120 ms」。
    let t = self.base.elapsed().as_nanos() as u64;
    let prev = self.last_cb_ns.swap(t, Relaxed);
    if prev != 0 && t > prev {
      let gap_us = ((t - prev) / 1000).min(u32::MAX as u64) as u32;
      self.last_gap_us.store(gap_us, Relaxed);
      self.max_gap_us.fetch_max(gap_us, Relaxed);
    }
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
      last_gap_ms: self.last_gap_us.load(Relaxed) as f32 / 1000.0,
      max_gap_ms: self.max_gap_us.load(Relaxed) as f32 / 1000.0,
    })
  }
}

#[derive(Clone, Debug, Serialize)]
pub struct CallbackFrames {
  /// 块大小（帧）：最近 / 最小 / 最大
  pub last: u32,
  pub min: u32,
  pub max: u32,
  /// 相邻两次回调的间隔（ms）：最近 / 最大（首个回调不计，见 `on_block`）
  pub last_gap_ms: f32,
  pub max_gap_ms: f32,
}

/// 回调停顿判定：回调计数持续增长即健康。
pub(super) struct StallWatch {
  last_callbacks: u64,
  last_seen: Instant,
}

impl StallWatch {
  pub fn new() -> Self {
    StallWatch {
      last_callbacks: 0,
      last_seen: Instant::now(),
    }
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

pub(super) fn stalled_state(
  stalled_ms: u128,
  last_error: &Option<String>,
) -> Option<Probe<String>> {
  (stalled_ms >= STALL_LIMIT.as_millis()).then(|| {
    Probe::unavailable(format!(
      "设备回调已停止 {stalled_ms} ms{}",
      last_error
        .as_ref()
        .map(|e| format!("（最近错误：{e}）"))
        .unwrap_or_default()
    ))
  })
}

/// 采集源描述（cpal 输入设备 / WASAPI 回环共用）。
pub(super) struct SourceInfo {
  pub name: String,
  pub device_id: String,
  pub native_rate: u32,
  pub channels: u16,
  pub sample_format: String,
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
      *cfg,
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

/// 打开输入设备并启动采集线程；阻塞到打开成功/失败（最多 5 s）。
/// `tag`：本流在调试接口里的唯一标识（同一设备可有多个流，如出现在多列中）。
pub fn spawn(hub: SharedHub, device_id: String, tag: String) -> Result<Capture, String> {
  // Linux 的 Pulse 设备（`pulse:<source>`）走 parec（OBS 同源思路：用系统混音器的设备，
  // 而不是把 ALSA 插件 PCM 全列出来）；其余平台/ID 继续用 cpal。
  #[cfg(target_os = "linux")]
  if let Some(source) = device_id.strip_prefix("pulse:") {
    return super::loopback::spawn_source(hub, source.to_string(), tag);
  }
  let stop = Arc::new(AtomicBool::new(false));
  let (tx, rx) = mpsc::channel::<Result<Arc<Fanout>, String>>();
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
    .name(format!("capture-{short}"))
    .spawn(move || run(hub, device_id, tag, stop2, tx))
    .map_err(|e| format!("创建采集线程失败：{e}"))?;
  match rx.recv_timeout(OPEN_TIMEOUT) {
    Ok(Ok(fanout)) => Ok(Capture {
      handle: WorkerHandle::new(stop, join),
      fanout,
    }),
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
  tag: String,
  stop: Arc<AtomicBool>,
  tx: mpsc::Sender<Result<Arc<Fanout>, String>>,
) {
  let stats = Arc::new(CallbackStats::new());

  // ---- 打开设备 ----
  let setup = crate::audio::with_device_retry(|| -> Result<_, String> {
    let (dev, raw_name) = crate::devices::find(&device_id, true)?;
    let name = crate::devices::display_name(&device_id, true).unwrap_or(raw_name);
    let supported = dev
      .default_input_config()
      .map_err(|e| format!("读取默认格式失败：{e}"))?;
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
    stream.play().map_err(|e| format!("启动输入流失败：{e}"))?;
    Ok((stream, cons, cfg, fmt, name))
  });

  let (stream, cons, cfg, fmt, name) = match setup {
    Ok(v) => v,
    Err(e) => {
      let _ = tx.send(Err(e));
      return;
    }
  };
  let info = SourceInfo {
    name,
    device_id,
    native_rate: cfg.sample_rate,
    channels: cfg.channels,
    sample_format: fmt.to_string(),
  };
  worker(
    hub,
    tag,
    info,
    WorkerSrc {
      cons,
      stats,
      keep: stream,
      stop,
    },
    tx,
  );
}

/// 数据源侧交给工作线程的采集件：环消费端、回调统计、保活句柄、停止开关。
/// 打包成一个类型，让 [`worker`] 的入参保持在「线程契约」的粒度上。
pub(super) struct WorkerSrc<K> {
  pub cons: rtrb::Consumer<f32>,
  pub stats: Arc<CallbackStats>,
  pub keep: K,
  pub stop: Arc<AtomicBool>,
}

/// 延迟封顶要丢掉的样本数（纯函数）：环里超过 [`DEV_MAX_MS`] 的部分丢最旧到
/// [`DEV_TARGET_MS`]，返回丢弃样本数（0 = 没超）。
fn clamp_excess(n: usize, slot_frames: usize) -> usize {
  let max = DEV_MAX_MS / 10 * slot_frames;
  if n <= max {
    0
  } else {
    n - DEV_TARGET_MS / 10 * slot_frames
  }
}

/// 实时预算放行（纯函数）：把「距上次结算经过的时间」折算成样本累加进 `budget`
/// （上限 [`RELEASE_SLOTS`] 格），返回本拍放行样本数并从 `budget` 里扣掉。
///
/// **只看本路自己的时钟** —— 不看下游水位、不看别的列。放行速率恒等于源标称速率，
/// 于是「一大块到达」的源被摊平，而水位不会因为某个消费者慢/某一列重建而被垫高。
fn release_realtime(
  n: usize,
  budget: &mut f64,
  dt: f64,
  native_rate: f64,
  slot_frames: usize,
) -> usize {
  let cap = (RELEASE_SLOTS * slot_frames) as f64;
  *budget = (*budget + dt * native_rate).min(cap);
  let take = n.min(*budget as usize);
  *budget -= take as f64;
  take
}

/// 工作循环：读环 → 重采样切 hop → 计量 → 发布 → 推扇出。
/// `src.keep` 在循环期间保活（cpal 流 / WASAPI 轮询线程）；`src.stats` 由数据源侧更新。
pub(super) fn worker<K>(
  hub: SharedHub,
  tag: String,
  info: SourceInfo,
  src: WorkerSrc<K>,
  tx: mpsc::Sender<Result<Arc<Fanout>, String>>,
) {
  let WorkerSrc {
    mut cons,
    stats,
    keep,
    stop,
  } = src;
  let _keep = keep;
  let mut to_hops = match ToHops::new(info.native_rate) {
    Ok(t) => t,
    Err(e) => {
      let _ = tx.send(Err(e));
      return;
    }
  };
  let fanout = Arc::new(Fanout::new(format!("输入：{}", info.name)));
  let _ = tx.send(Ok(fanout.clone()));

  let stream_id = format!("input:{tag}");
  let started_at = now_ms();
  let token = crate::debug::threads::ThreadToken::new(format!("capture:{tag}"));
  let native_rate = info.native_rate;
  let mut meter = Meter::new();
  let mut in_rate = RateMeter::new();
  let mut out_rate = RateMeter::new();
  let mut drop_rate = RateMeter::new();
  let mut err_rate = RateMeter::new();
  let mut stall = StallWatch::new();
  let mut hops: u64 = 0;
  let mut resample_error: Option<String> = None;
  let mut last_publish = Instant::now() - PUBLISH_PERIOD;
  // 一格 = 10 ms 的原生样本量；放行预算与「上次结算时刻」是**这一路自己的时钟**
  let slot_frames = (native_rate as usize / 100).max(1);
  let mut budget: f64 = 0.0;
  let mut last_budget = Instant::now();
  // 延迟封顶丢掉的样本数：进 overruns（+ 速率），面板上看得见（丢了不报只剩一句「声音卡」）
  let mut clamped: u64 = 0;

  while !stop.load(Relaxed) {
    token.beat();
    let now = Instant::now();
    // ---- 延迟封顶：设备环超硬顶就丢最旧 ----
    // 超硬顶的是**已经过时的音频**（流刚打开时设备预填的一大块、会话重建期间的积压、
    // 调度停顿），正确动作是丢掉、跳到现在；按 2× 放出去等于把一次突发变成
    // 「之后十几秒都在听旧声音」（v18 实测：单列麦克风监听长期停在 289 ms）。
    let n = cons.slots();
    let excess = clamp_excess(n, slot_frames);
    if excess > 0
      && let Ok(c) = cons.read_chunk(excess)
    {
      c.commit_all();
      clamped += excess as u64;
    }
    let n = n - excess;
    let ring_level_ms = n as f32 * 1000.0 / native_rate as f32;
    // ---- 放行节拍：只看本路自己的时钟 ----
    // 源可能「一大块到达」：Linux `parec` 一次一整块（现为 30 ms，见
    // `loopback::PARE_LATENCY_MS`），部分 ALSA / WASAPI 设备一次十几毫秒。
    // 整块顺着往下推会把下游 10 ms 级的有界环一次灌满、块与块的接缝上又立刻饿死
    // （行输入环超 `ROW_CAP` 丢最旧，实测丢掉 ~80%，听感是规律的咔咔声）。
    // 所以按**本路的实时速率**放行：整块留在设备环里当垫层、被摊成平滑 hop 流。
    // 那块垫层带来的延迟由**源的块大小**决定，压不掉 —— 要更低就压源侧的缓冲。
    //
    // **不要退回「拿下游水位当放行开关」**（v18）：放行速率恰好等于源速率时水位是
    // 随机游走，一旦被垫高就再也降不回来（设备预填的那一块会永久留在环里）。
    let dt = now.duration_since(last_budget).as_secs_f64();
    last_budget = now;
    let take = release_realtime(n, &mut budget, dt, native_rate as f64, slot_frames);
    if take > 0
      && let Ok(chunk) = cons.read_chunk(take)
    {
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

    if now.duration_since(last_publish) >= PUBLISH_PERIOD {
      last_publish = now;
      let m = meter.take();
      let last_error = stats.last_error();
      let callbacks = stats.callbacks.load(Relaxed);
      let stalled_ms = stall.update(now, callbacks);
      let state = match (&resample_error, stalled_state(stalled_ms, &last_error)) {
        (Some(e), _) => Probe::unavailable(e.clone()),
        (None, Some(s)) => s,
        (None, None) => Probe::ok("running".to_string()),
      };
      let frames_in = stats.frames.load(Relaxed);
      let frames_out = hops * HOP as u64;
      let xruns = stats.xruns.load(Relaxed);
      // 输入流的「丢弃」= ALSA 欠载（xrun）+ 本工作线程的延迟封顶丢最旧。
      // 两者都是「样本没走到下游」，合在一个字段里报（+ 速率），面板一眼看出还在不在丢。
      let dropped = xruns + clamped;
      let errors = stats.errors.load(Relaxed);
      let (inf_avg, inf_max) = (
        Probe::unavailable("处理链在列里（见引擎概要）"),
        Probe::unavailable("处理链在列里（见引擎概要）"),
      );
      let mut st = StreamInfo {
        id: stream_id.clone(),
        direction: "input",
        device_id: info.device_id.clone(),
        device_name: info.name.clone(),
        state,
        health: Probe::Pending, // 下面 health::annotate 会按 state + 计数/速率填上
        started_at,
        source: None,
        denoise: Probe::unavailable("降噪在处理链（列）里"),
        sample_rate: native_rate,
        channels: info.channels as u32,
        sample_format: info.sample_format.clone(),
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
        overruns: dropped,
        resyncs: Probe::unavailable("输入流无重同步"),
        stream_errors: errors,
        underrun_per_s: Probe::unavailable("输入流无欠载概念"),
        resync_per_s: Probe::unavailable("输入流无重同步"),
        overrun_per_s: drop_rate.push(now, dropped),
        stream_error_per_s: err_rate.push(now, errors),
        last_error,
        buffer_level_ms: ring_level_ms,
        device_buffer_ms: Probe::unavailable("输入流：环形缓冲即设备侧缓冲，见 buffer_level_ms"),
        latency_ms: Probe::unavailable("端到端延迟尚未测量"),
        inference_ms_avg: inf_avg,
        inference_ms_max: inf_max,
        waveform: m.waveform,
        spectrum_db: m.spectrum_db,
        spectrum_bin_hz: SPECTRUM_BIN_HZ,
      };
      super::health::annotate(&mut st);
      hub.set_stream(st);
    }
    std::thread::sleep(POLL_PERIOD);
  }

  fanout.close();
  hub.remove_stream(&stream_id);
}

/// 放行节拍的模拟：`arrivals` 是每个 tick 到达的原生样本数。
/// 返回（放行后的环水位峰值、单拍最大放行量、累计丢弃、结束时环里的样本数）。
///
/// 顺序与 [`worker`] 里一致：**先延迟封顶丢最旧，再按实时预算放行**。
#[cfg(test)]
fn simulate(arrivals: &[usize], slot_frames: usize, tick_s: f64) -> (usize, usize, usize, usize) {
  let mut budget = 0.0f64;
  let mut ring = 0usize;
  let mut peak = 0usize;
  let mut max_take = 0usize;
  let mut dropped = 0usize;
  for &a in arrivals {
    ring += a;
    let excess = clamp_excess(ring, slot_frames);
    ring -= excess;
    dropped += excess;
    let take = release_realtime(
      ring,
      &mut budget,
      tick_s,
      slot_frames as f64 * 100.0,
      slot_frames,
    );
    ring -= take;
    peak = peak.max(ring);
    max_take = max_take.max(take);
  }
  (peak, max_take, dropped, ring)
}

#[cfg(test)]
mod tests {
  use super::*;

  /// 实时源（每 tick 正好 10 ms 的数据）**一点都不许累积**。
  ///
  /// 这条是 v18 那个 bug 的回归测试：v18 拿「下游水位」当放行开关，而放行速率恰好
  /// 等于源速率，于是设备环里的水位**只进不出**（实测单列麦克风监听长期 289 ms）。
  #[test]
  fn realtime_source_never_accumulates() {
    let slot = 480;
    let (peak, max_take, dropped, left) = simulate(&vec![slot; 1000], slot, 0.010);
    assert_eq!(peak, 0, "实时源不该在设备环里留下水位");
    assert_eq!(left, 0, "结束时环里应该空的");
    assert_eq!(max_take, slot, "单拍放行恒为一格（= 10 ms）");
    assert_eq!(dropped, 0, "实时源不该丢样本");
  }

  /// 设备比标称快（每 tick 多 1 个样本）：水位被**硬顶钉住**，不会一路涨上去。
  /// 守恒：多出来的样本只有两条出路 —— 丢掉，或者还在环里（有硬顶）。
  #[test]
  fn faster_device_is_clamped_not_accumulated() {
    let slot = 480;
    let ticks = 10_000;
    let (peak, max_take, dropped, left) = simulate(&vec![slot + 1; ticks], slot, 0.010);
    assert!(
      peak <= DEV_MAX_MS / 10 * slot,
      "水位必须钉在硬顶以内，实测峰值 {peak}（硬顶 {}）",
      DEV_MAX_MS / 10 * slot
    );
    assert_eq!(max_take, slot, "单拍仍只放一格（不许退化成突发放行）");
    assert_eq!(dropped + left, ticks, "多出来的样本要么丢掉、要么在环里");
  }

  /// 突发源（一整块到达）：**摊成平滑 hop 流**、不冲爆下游，也不丢。
  /// `parec` 是 30 ms 一块 = 1440 样本；块比硬顶小的时候一个都不许丢。
  #[test]
  fn burst_source_is_paced_not_flooded() {
    let slot = 480;
    let mut arrivals = Vec::new();
    for i in 0..300 {
      arrivals.push(if i % 3 == 0 { 3 * slot } else { 0 }); // 每 3 拍来一个 30 ms 的块
    }
    let (peak, max_take, dropped, left) = simulate(&arrivals, slot, 0.010);
    assert_eq!(dropped, 0, "块（30 ms）小于硬顶（60 ms）时不许丢");
    assert_eq!(left, 0, "推取同速，结束时应放完");
    assert_eq!(max_take, slot, "整块必须被摊平：单拍只放一格");
    assert!(
      peak <= 3 * slot,
      "设备环里最多留一个块（30 ms），实测 {peak}"
    );
  }

  /// 长停顿 / 会话重建期间的积压：**丢掉**（跳到现在），不许慢慢放出去。
  /// 「慢慢放」= 之后十几秒都在听旧声音 —— v18 正是这么把 300 ms 变成稳态的。
  #[test]
  fn backlog_is_dropped_not_played_out() {
    let slot = 480;
    let mut arrivals = vec![20 * slot]; // 一次到达 200 ms
    arrivals.extend(vec![slot; 500]);
    let (_, max_take, dropped, left) = simulate(&arrivals, slot, 0.010);
    assert_eq!(dropped, 20 * slot - slot, "超硬顶的部分全丢，只留目标一格");
    assert_eq!(left, 0, "丢完就跳到现在，不留在环里当待播数据");
    assert_eq!(max_take, slot, "收尾仍是实时放行");
  }
}
