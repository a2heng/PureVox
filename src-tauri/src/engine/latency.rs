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

//! **逐列延迟探测**（DEBUG.md §1.4）：一列从「输入 hop」到「输出 hop」的实测信号延时，
//! 加上这一列每个缓冲的实测水位，合成一条可归因的延迟预算。
//!
//! 为什么要有这一块：延迟是这条链上最容易悄悄退化、又最难归因的东西
//! （ARCHITECTURE.md §8「每一列的延迟必须自己说了算」）。水位（`buffer_level_ms`）只是
//! 「缓冲里还有多少样本」，**不等于**信号实际被推迟了多少：模型有算法延迟、重采样器有
//! 群延迟、OLA 有一个 hop 的输出滞后。只有把两端波形拿来对齐，才知道真实的处理链延时。
//!
//! 测量方式（**唯一一条**）：
//! - 每个列线程每 tick 把「第一路有数据的输入 hop」与「第一路输出行取到的 hop」**原样**
//!   拷进本模块的 ring（`rtrb`，音频线程只 push，不分配、不加锁等待）；
//! - 分析线程每 500 ms 取走两种波形，按 4:1 抽取到 12 kHz（0.083 ms 分辨率），
//!   做**归一化互相关**（FFT）找峰值 → 处理链延时（单位 ms）；
//! - 峰值不够高 / 不够突出（周期信号、信号太弱、处理后波形差异太大）就**明说测不到**，
//!   不编数字（DEBUG.md §1.4）。
//!
//! 预算各段（输入设备回调、采集环、行输入环、处理链、混音源环、播放 48k 环、设备环、
//! 输出设备回调）都是**实测**水位/回调周期；缺哪段就写明缺哪段，`total_ms` 只求已知段之和
//! 并标 `complete:false`。`dominant` 直接指出最大的一段 —— 这就是「延迟原因」的答案。
//!
//! 三条读同一份（DEBUG.md §1.4）：快照 `latency` / `GET /debug/latency` /
//! `~/.purevox/latency_last.json`。界面只渲染，不在前端重算。

use std::collections::VecDeque;
use std::sync::atomic::{AtomicBool, Ordering::Relaxed};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use rtrb::{Consumer, Producer, RingBuffer};
use rustfft::FftPlanner;
use rustfft::num_complex::Complex;
use serde::Serialize;

use crate::audio::{HOP, SAMPLE_RATE};
use crate::debug::{Health, Probe, SharedHub, now_ms};

/// 报告结构版本（DEBUG.md §1.4：只增不改名）。
pub const LATENCY_SCHEMA_VERSION: u32 = 1;

/// 分析周期：500 ms 足够跟上 200 ms 一发的列概要，又不至于每秒做十几次 FFT。
const ANALYZE_PERIOD: Duration = Duration::from_millis(500);
/// 落盘周期：覆盖写（DEBUG.md §1.4），不追求实时。
const DUMP_PERIOD: Duration = Duration::from_secs(10);

/// 做相关的输出窗口（ms）。太长会被处理后的差异稀释，太短统计不稳。
const WIN_MS: usize = 300;
/// 搜索的最大延时（ms）。TSE / 大缓存之外再大就该看别处了。
const MAX_LAG_MS: usize = 500;
/// 相关用的采样率（抽取后）：0.083 ms 分辨率，延迟诊断足够。
const PROBE_RATE: usize = 12_000;
/// 每毫秒多少个抽取后的样本（= 12）。
const PROBE_PER_MS: usize = PROBE_RATE / 1000;
/// 抽取比：48 kHz → 12 kHz（HOP=480 被 4 整除，hop 边界不产生余数）。
const DECIM: usize = SAMPLE_RATE as usize / PROBE_RATE;
/// 输出窗口相对「现在」往后挪的一截（ms）：这样 0 ms 延时落在搜索窗中间，
/// 不会因为贴着边界而判不出突出度（真实延时的下限就是 0）。
const SLACK_MS: usize = 60;

/// 输出窗口不短于这么多秒、信号不弱于这个 RMS（dBFS）才做相关：底噪/静音不做。
const MIN_RMS_DBFS: f32 = -55.0;
/// 归一化相关系数的最低可信峰。
const MIN_CONFIDENCE: f32 = 0.30;
/// 峰值必须比「离开主峰 3 ms 以外的次高值」高出这么多，才认为不模糊。
const MIN_MARGIN: f32 = 0.10;

/// 分段的来源：实测信号 / 缓冲水位 / 设备回调周期。
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum StageSource {
  /// 两端波形互相关实测（处理链）
  Measured,
  /// 环形缓冲水位（ms）
  Buffer,
  /// 设备回调周期（设备侧缓冲的实测估计）
  Device,
  /// 重采样器群延迟
  Resampler,
}

/// 延迟预算里的一段（单位 ms；`None` = 这段测不到，原因在 `note` 里）。
#[derive(Clone, Debug, Serialize)]
pub struct LatencyStage {
  pub name: String,
  pub ms: Option<f32>,
  pub source: StageSource,
  pub note: String,
}

impl LatencyStage {
  fn new(
    name: impl Into<String>,
    ms: Option<f32>,
    source: StageSource,
    note: impl Into<String>,
  ) -> Self {
    LatencyStage {
      name: name.into(),
      ms,
      source,
      note: note.into(),
    }
  }
}

/// 一行在延迟预算里的落点（列线程按发布周期更新，分析线程读）。
#[derive(Clone, Debug, Default)]
pub struct RowTapInfo {
  /// 这一行对应的调试流 id（`input:<设备>` / `input:c0r1m` …）；没有设备流时为 None
  pub stream_id: Option<String>,
  /// 这一行自己的环水位（ms）
  pub buffer_ms: Option<f32>,
  /// 人话标签（进 note）
  pub label: String,
}

/// 一列的延迟报告（快照 `columns[].latency` 与 `/debug/latency` 的 `columns[]` 同源）。
#[derive(Clone, Debug, Serialize)]
pub struct ColumnLatency {
  pub column: usize,
  /// `pending` / `running` / `disabled`
  pub state: String,
  /// 处理链（输入 hop → 输出 hop）的实测延时；null 表示没测到
  pub measured_ms: Option<f32>,
  /// 实测方法：`waveform`（12 kHz 波形）或 `envelope`（10 ms 能量包络；降噪/TSE 改了相位时用）
  pub measured_method: Option<&'static str>,
  /// 测量分辨率（ms）
  pub measured_resolution_ms: Option<f32>,
  /// 互相关峰值（0~1）：越高越可信
  pub confidence: Option<f32>,
  /// 多个测量的波动（ms）：> 3 ms 说明水位在爬/信号在变，数字只能当参考
  pub jitter_ms: Option<f32>,
  /// 已知各段之和；`complete=false` 时是下限
  pub total_ms: Option<f32>,
  pub complete: bool,
  /// 最大的一段（「延迟原因」）
  pub dominant: Option<String>,
  pub stages: Vec<LatencyStage>,
  /// 人话概要（含数字）
  pub detail: String,
  pub health: Probe<Health>,
}

impl ColumnLatency {
  fn pending(column: usize) -> Self {
    ColumnLatency {
      column,
      state: "pending".into(),
      measured_ms: None,
      measured_method: None,
      measured_resolution_ms: None,
      confidence: None,
      jitter_ms: None,
      total_ms: None,
      complete: false,
      dominant: None,
      stages: Vec::new(),
      detail: "采集中：需要 1 秒以上的有效信号（说话/音乐），静音与周期测试音测不出延时".into(),
      health: Probe::Pending,
    }
  }

  fn disabled(column: usize) -> Self {
    ColumnLatency {
      column,
      state: "disabled".into(),
      measured_ms: None,
      measured_method: None,
      measured_resolution_ms: None,
      confidence: None,
      jitter_ms: None,
      total_ms: None,
      complete: false,
      dominant: None,
      stages: Vec::new(),
      detail: "这一列已停用（没有输出可测）".into(),
      health: Probe::ok(Health::ok("已停用")),
    }
  }
}

/// 三处共用的结构化报告。
#[derive(Clone, Debug, Serialize)]
pub struct LatencyReport {
  pub schema_version: u32,
  pub created_ms: u64,
  /// `windows` / `linux` / `macos` / `other`
  pub platform: &'static str,
  /// 所有列都测全了（没有缺项）
  pub complete: bool,
  pub columns: Vec<ColumnLatency>,
  pub verdict: Probe<Health>,
  /// 测不到时下一步做什么
  pub next_step: Option<String>,
  /// 本次落盘位置（写盘失败时为 None，原因进 verdict）
  pub dump: Option<String>,
}

// ---------------------------------------------------------------------------
// 采集端（列线程）与分析端共享的句柄
// ---------------------------------------------------------------------------

/// 一条采集通道：音频线程 push 整 hop，分析线程在另一头取走。
struct Chan {
  prod: Mutex<Producer<f32>>,
}

impl Chan {
  fn push(&self, hop: Option<&[f32]>) {
    let Ok(mut p) = self.prod.lock() else {
      return;
    };
    if p.slots() < HOP {
      // 分析线程没跟上（正常不会：2 s 环 vs 500 ms 消费）。**整 hop 一起跳过**，
      // 两条通道因此仍保持同样的时间基准（只差一段音频，不会错位）。
      return;
    }
    match hop {
      Some(h) => {
        for &x in &h[..HOP] {
          let _ = p.push(x);
        }
      }
      None => {
        for _ in 0..HOP {
          let _ = p.push(0.0);
        }
      }
    }
  }
}

/// 一行采集端的即时信息（只在发布周期写一次，量很小）。
#[derive(Clone, Debug, Default)]
struct Rows {
  enabled: bool,
  input: Option<RowTapInfo>,
  output: Option<RowTapInfo>,
}

/// 一列的延迟探针（`Session` 持有，列线程写、分析线程读）。
pub struct LatencyTap {
  column: usize,
  input: Chan,
  output: Chan,
  /// 分析线程启动时取走；之后列线程不再碰它
  input_cons: Mutex<Option<Consumer<f32>>>,
  output_cons: Mutex<Option<Consumer<f32>>>,
  rows: Mutex<Rows>,
  result: Mutex<Option<ColumnLatency>>,
}

impl LatencyTap {
  pub fn new(column: usize) -> Self {
    // 2 s @48k：分析窗口最多用 860 ms，留足余量（分析线程 500 ms 消费一次）
    let cap = SAMPLE_RATE as usize * 2;
    let (ip, ic) = RingBuffer::<f32>::new(cap);
    let (op, oc) = RingBuffer::<f32>::new(cap);
    LatencyTap {
      column,
      input: Chan {
        prod: Mutex::new(ip),
      },
      output: Chan {
        prod: Mutex::new(op),
      },
      input_cons: Mutex::new(Some(ic)),
      output_cons: Mutex::new(Some(oc)),
      rows: Mutex::new(Rows::default()),
      result: Mutex::new(None),
    }
  }

  /// 列线程每 tick 调一次：`None` = 这一 tick 这一侧没有 hop（写等长静音，保时间基准）。
  pub fn push(&self, input: Option<&[f32]>, output: Option<&[f32]>) {
    self.input.push(input);
    self.output.push(output);
  }

  /// 发布周期更新「这一列现在用的是哪一行」；行增删/换设备后下一次分析就跟上。
  /// 用 `try_lock`：分析线程正在读时直接跳过一次（下一次发布 200 ms 后还有），
  /// 列线程绝不在这里等锁。
  pub fn set_rows(&self, enabled: bool, input: Option<RowTapInfo>, output: Option<RowTapInfo>) {
    if let Ok(mut r) = self.rows.try_lock() {
      *r = Rows {
        enabled,
        input,
        output,
      };
    }
  }

  /// 列线程发布概要时读最近一份结果（分析线程 500 ms 更新一次）。
  pub fn result(&self) -> ColumnLatency {
    self
      .result
      .lock()
      .ok()
      .and_then(|g| g.clone())
      .unwrap_or_else(|| ColumnLatency::pending(self.column))
  }

  fn take_consumers(&self) -> (Option<Consumer<f32>>, Option<Consumer<f32>>) {
    (
      self.input_cons.lock().ok().and_then(|mut g| g.take()),
      self.output_cons.lock().ok().and_then(|mut g| g.take()),
    )
  }
}

// ---------------------------------------------------------------------------
// 分析线程
// ---------------------------------------------------------------------------

/// 分析线程句柄（随 `Session` 生命周期）。
pub struct LatencyAnalyzer {
  stop: Arc<AtomicBool>,
  join: Option<std::thread::JoinHandle<()>>,
}

impl LatencyAnalyzer {
  /// 起分析线程。`taps` 的索引 = 列号。
  pub fn spawn(hub: SharedHub, taps: Vec<(usize, Arc<LatencyTap>)>) -> Self {
    let stop = Arc::new(AtomicBool::new(false));
    let stop2 = stop.clone();
    let join = std::thread::Builder::new()
      .name("latency-analyzer".into())
      .spawn(move || run(hub, taps, stop2))
      .ok();
    LatencyAnalyzer { stop, join }
  }

  pub fn stop(&mut self) {
    self.stop.store(true, Relaxed);
    if let Some(j) = self.join.take() {
      let _ = j.join();
    }
  }
}

impl Drop for LatencyAnalyzer {
  fn drop(&mut self) {
    self.stop();
  }
}

/// 一路分析状态：取走的两条 ring + 各自的历史（已抽取到 12 kHz）。
struct ChanState {
  tap: Arc<LatencyTap>,
  input: Consumer<f32>,
  output: Consumer<f32>,
  in_hist: VecDeque<f32>,
  out_hist: VecDeque<f32>,
  /// 最近几次有效测量的中位数平滑
  history: VecDeque<f32>,
  last_input_ms: u64,
}

const HIST_SAMPLES: usize = (WIN_MS + MAX_LAG_MS + 2 * SLACK_MS) * PROBE_PER_MS;
const OUT_SAMPLES: usize = WIN_MS * PROBE_PER_MS;
/// 输出历史里除了相关窗，还要留出 `SLACK_MS` 的位移。
const OUT_HIST: usize = OUT_SAMPLES + SLACK_MS * PROBE_PER_MS;
/// `k` 里对应 0 ms 的那个位置：输入窗比输出窗早 `MAX_LAG_MS + SLACK_MS`
/// （再多留一个 `SLACK_MS` 让 0 ms 不贴搜索边界）。
const ZERO_K: usize = (MAX_LAG_MS + SLACK_MS) * PROBE_PER_MS;
/// 搜索上限：允许到 0 ms 再往前 `SLACK_MS`（这些点算出来是负延时，会被拒）。
const K_MAX: usize = ZERO_K + SLACK_MS * PROBE_PER_MS;
/// 相关用的 FFT 长度（≥ HIST + OUT 的下一个 2 的幂）。
const FFT_N: usize = 16_384;

/// 一次互相关的结果。
struct Peak {
  /// 输入 → 输出的延时（ms）
  ms: f32,
  /// 归一化相关系数峰值
  confidence: f32,
  /// 主峰与次峰的差（越大越不模糊）
  margin: f32,
  /// `waveform`（12 kHz 波形，0.083 ms）或 `envelope`（10 ms 能量包络）
  method: &'static str,
  /// 测量分辨率（ms）
  resolution_ms: f32,
}

struct Analyzer {
  fft_fwd: Arc<dyn rustfft::Fft<f32>>,
  fft_inv: Arc<dyn rustfft::Fft<f32>>,
  a: Vec<Complex<f32>>,
  b: Vec<Complex<f32>>,
  in_dec: Vec<f32>,
  out_dec: Vec<f32>,
  prefix: Vec<f64>,
  corr: Vec<f32>,
}

impl Analyzer {
  fn new() -> Self {
    let mut planner = FftPlanner::new();
    let fft_fwd = planner.plan_fft_forward(FFT_N);
    let fft_inv = planner.plan_fft_inverse(FFT_N);
    Analyzer {
      fft_fwd,
      fft_inv,
      a: vec![Complex::new(0.0, 0.0); FFT_N],
      b: vec![Complex::new(0.0, 0.0); FFT_N],
      in_dec: vec![0.0; HIST_SAMPLES],
      out_dec: vec![0.0; OUT_SAMPLES],
      prefix: vec![0.0; HIST_SAMPLES + 1],
      corr: vec![0.0; K_MAX + 1],
    }
  }

  fn measure(&mut self, in_hist: &VecDeque<f32>, out_hist: &VecDeque<f32>) -> Result<Peak, String> {
    if in_hist.len() < HIST_SAMPLES || out_hist.len() < OUT_HIST {
      return Err("历史还不够（至少 1 秒有效信号）".into());
    }
    // 两端都按 tick 对齐写入。输入取到「现在」，输出窗整体后移 `SLACK_MS`：
    // 于是 0 ms 延时的峰落在搜索窗中间，而不是贴着边界。
    for (i, v) in in_hist
      .iter()
      .skip(in_hist.len() - HIST_SAMPLES)
      .enumerate()
    {
      self.in_dec[i] = *v;
    }
    let out_end = out_hist.len() - SLACK_MS * PROBE_PER_MS;
    for (i, v) in out_hist
      .iter()
      .skip(out_end - OUT_SAMPLES)
      .take(OUT_SAMPLES)
      .enumerate()
    {
      self.out_dec[i] = *v;
    }
    let e_out: f64 = self.out_dec.iter().map(|x| (*x as f64) * (*x as f64)).sum();
    let rms = (e_out / OUT_SAMPLES as f64).sqrt() as f32;
    if rms <= 0.0 || 20.0 * rms.log10() < MIN_RMS_DBFS {
      return Err(format!(
        "输出信号太弱（{:.0} dBFS，低于 {MIN_RMS_DBFS:.0} dBFS）",
        20.0 * rms.max(1e-9).log10()
      ));
    }
    self.prefix[0] = 0.0;
    for i in 0..HIST_SAMPLES {
      let x = self.in_dec[i] as f64;
      self.prefix[i + 1] = self.prefix[i] + x * x;
    }
    let n = FFT_N;
    for i in 0..n {
      self.a[i] = Complex::new(
        if i < HIST_SAMPLES {
          self.in_dec[i]
        } else {
          0.0
        },
        0.0,
      );
      self.b[i] = Complex::new(
        if i < OUT_SAMPLES {
          self.out_dec[i]
        } else {
          0.0
        },
        0.0,
      );
    }
    self.fft_fwd.process(&mut self.a);
    self.fft_fwd.process(&mut self.b);
    for i in 0..n {
      // IFFT(FFT(A) · conj(FFT(B)))[k] = Σ_n A[k+n]·B[n]
      self.a[i] = self.a[i] * self.b[i].conj();
    }
    self.fft_inv.process(&mut self.a);
    let scale = 1.0 / n as f32;
    for k in 0..=K_MAX {
      let num = self.a[k].re * scale;
      let e_in = self.prefix[k + OUT_SAMPLES] - self.prefix[k];
      let den = (e_in * e_out).sqrt() as f32;
      self.corr[k] = if den > 1e-12 { num / den } else { 0.0 };
    }

    // 边界各留 24 个点（2 ms）：主峰贴边时无法判突出度，而且真实延时不会在 0 ms。
    let guard = 24;
    let mut best_k = 0usize;
    let mut best = f32::NEG_INFINITY;
    for k in guard..=K_MAX.saturating_sub(guard) {
      if self.corr[k] > best {
        best = self.corr[k];
        best_k = k;
      }
    }
    if !best.is_finite() || best < MIN_CONFIDENCE {
      // 波形对不上（降噪/TSE 会改相位）→ 退到 10 ms 能量包络再试一次。
      // 包络只要求「响度起伏」一致，对相位变化不敏感。
      return self.measure_envelope(
        in_hist,
        out_hist,
        &format!("波形相关 {:.2}（低于 {MIN_CONFIDENCE:.2}）", best.max(0.0)),
      );
    }
    let guard_samples = 3 * PROBE_PER_MS; // 3 ms
    let mut second = f32::NEG_INFINITY;
    for k in guard..=K_MAX.saturating_sub(guard) {
      if k.abs_diff(best_k) > guard_samples {
        second = second.max(self.corr[k]);
      }
    }
    let margin = if second.is_finite() {
      best - second
    } else {
      best
    };
    if margin < MIN_MARGIN {
      return Err(format!(
        "峰值不突出（主 {best:.2} / 次 {:.2}；周期信号/测试音会这样），无法判定延时",
        second.max(0.0)
      ));
    }
    // k 是「输入窗里匹配输出窗的偏移」。0 ms 对应 `ZERO_K`，所以
    // 延时 = (ZERO_K − k) / 12 kHz。
    let ms = (ZERO_K as f32 - best_k as f32) * 1000.0 / PROBE_RATE as f32;
    if !(0.0..=MAX_LAG_MS as f32).contains(&ms) {
      return Err(format!(
        "测到的延时 {ms:.0} ms 超出 0~{MAX_LAG_MS} ms 搜索范围（有更大的固定延迟）"
      ));
    }
    Ok(Peak {
      ms,
      confidence: best,
      margin,
      method: "waveform",
      resolution_ms: 1000.0 / PROBE_RATE as f32,
    })
  }

  /// 10 ms 能量包络互相关（Pearson）：波形被模型改相位时的退路。
  ///
  /// 分辨率 10 ms —— 对「处理链 vs 缓冲」的归因足够（缓冲都是 10 ms 的整数倍），
  /// 而且对降噪 / TSE 的相位重建不敏感。周期信号（测试音）的包络是常数，直接拒绝。
  fn measure_envelope(
    &mut self,
    in_hist: &VecDeque<f32>,
    out_hist: &VecDeque<f32>,
    why: &str,
  ) -> Result<Peak, String> {
    if in_hist.len() < HIST_SAMPLES || out_hist.len() < OUT_HIST {
      return Err("历史还不够（至少 1 秒有效信号）".into());
    }
    let in_dec: Vec<f32> = in_hist
      .iter()
      .skip(in_hist.len() - HIST_SAMPLES)
      .copied()
      .collect();
    let out_end = out_hist.len() - SLACK_MS * PROBE_PER_MS;
    let out_dec: Vec<f32> = out_hist
      .iter()
      .skip(out_end - OUT_SAMPLES)
      .take(OUT_SAMPLES)
      .copied()
      .collect();
    let hop_samples = PROBE_PER_MS * 10;
    let in_env = hop_envelope(&in_dec, hop_samples);
    let out_env = hop_envelope(&out_dec, hop_samples);
    let out_hops = out_env.len();
    let zero_hops = (MAX_LAG_MS + SLACK_MS) / 10; // 0 ms 对应的 hop 偏移
    let max_k = (MAX_LAG_MS + 2 * SLACK_MS) / 10;
    let guard = 1usize;
    let mut best = f32::NEG_INFINITY;
    let mut best_k = 0usize;
    for k in guard..=max_k.saturating_sub(guard) {
      if k + out_hops > in_env.len() {
        break;
      }
      let c = pearson(&in_env[k..k + out_hops], &out_env);
      if c > best {
        best = c;
        best_k = k;
      }
    }
    if !best.is_finite() || best < 0.50 {
      return Err(format!(
        "{why}；能量包络也对不上（相关 {:.2}，低于 0.50；信号太弱或处理改变太大）",
        best.max(0.0)
      ));
    }
    let mut second = f32::NEG_INFINITY;
    for k in guard..=max_k.saturating_sub(guard) {
      if k + out_hops > in_env.len() {
        break;
      }
      if k.abs_diff(best_k) > 2 {
        second = second.max(pearson(&in_env[k..k + out_hops], &out_env));
      }
    }
    let margin = if second.is_finite() {
      best - second
    } else {
      best
    };
    if margin < 0.15 {
      return Err(format!(
        "{why}；包络峰值不突出（主 {best:.2} / 次 {:.2}；周期/重复信号会这样）",
        second.max(0.0)
      ));
    }
    let ms = (zero_hops as f32 - best_k as f32) * 10.0;
    if !(0.0..=MAX_LAG_MS as f32).contains(&ms) {
      return Err(format!("包络测到 {ms:.0} ms，超出 0~{MAX_LAG_MS} ms 范围"));
    }
    Ok(Peak {
      ms,
      confidence: best,
      margin,
      method: "envelope",
      resolution_ms: 10.0,
    })
  }
}

/// 每 `hop` 个抽取样本求一次 RMS（包络）。
fn hop_envelope(dec: &[f32], hop: usize) -> Vec<f32> {
  dec
    .chunks_exact(hop)
    .map(|c| {
      let e = c.iter().map(|x| (*x as f64) * (*x as f64)).sum::<f64>() / hop as f64;
      (e.sqrt() as f32).max(1e-6).ln()
    })
    .collect()
}

/// Pearson 相关系数（两段等长包络）。
fn pearson(a: &[f32], b: &[f32]) -> f32 {
  let n = a.len().min(b.len()) as f32;
  if n < 2.0 {
    return 0.0;
  }
  let ma = a.iter().take(n as usize).sum::<f32>() / n;
  let mb = b.iter().take(n as usize).sum::<f32>() / n;
  let mut num = 0.0f32;
  let mut da = 0.0f32;
  let mut db = 0.0f32;
  for i in 0..n as usize {
    let x = a[i] - ma;
    let y = b[i] - mb;
    num += x * y;
    da += x * x;
    db += y * y;
  }
  if da <= 1e-12 || db <= 1e-12 {
    0.0
  } else {
    num / (da.sqrt() * db.sqrt())
  }
}

fn drain_hops(cons: &mut Consumer<f32>, hist: &mut VecDeque<f32>, max_len: usize) -> usize {
  let mut added = 0usize;
  while cons.slots() >= HOP {
    let Ok(chunk) = cons.read_chunk(HOP) else {
      break;
    };
    let (a, b) = chunk.as_slices();
    // 4:1 抽取（HOP=480，4 整除；每个 hop 边界都干净）
    let mut acc = 0.0f32;
    let mut count = 0usize;
    for &x in a.iter().chain(b.iter()) {
      acc += x;
      count += 1;
      if count == DECIM {
        hist.push_back(acc / DECIM as f32);
        added += 1;
        acc = 0.0;
        count = 0;
      }
    }
    chunk.commit_all();
  }
  while hist.len() > max_len {
    hist.pop_front();
  }
  added
}

fn run(hub: SharedHub, taps: Vec<(usize, Arc<LatencyTap>)>, stop: Arc<AtomicBool>) {
  let token = crate::debug::threads::ThreadToken::new("latency-analyzer");
  let mut chans: Vec<ChanState> = Vec::new();
  for (_idx, tap) in taps {
    let (ic, oc) = tap.take_consumers();
    let (Some(ic), Some(oc)) = (ic, oc) else {
      continue;
    };
    chans.push(ChanState {
      tap,
      input: ic,
      output: oc,
      in_hist: VecDeque::with_capacity(HIST_SAMPLES + 512),
      out_hist: VecDeque::with_capacity(OUT_HIST + 512),
      history: VecDeque::new(),
      last_input_ms: now_ms(),
    });
  }
  let mut analyzer = Analyzer::new();
  let dump_path = crate::config::config_dir()
    .ok()
    .map(|d| d.join("latency_last.json"));
  let mut last_dump = Instant::now() - DUMP_PERIOD;

  while !stop.load(Relaxed) {
    token.beat();
    // 先睡再算：会话刚起步时给采集侧留出填满窗口的时间。
    std::thread::sleep(ANALYZE_PERIOD);
    if stop.load(Relaxed) {
      break;
    }
    let now = now_ms();
    let mut columns: Vec<ColumnLatency> = Vec::with_capacity(chans.len());
    for ch in chans.iter_mut() {
      let rows = ch.tap.rows.lock().map(|r| r.clone()).unwrap_or_default();
      let in_added = drain_hops(&mut ch.input, &mut ch.in_hist, HIST_SAMPLES);
      drain_hops(&mut ch.output, &mut ch.out_hist, OUT_HIST);
      if in_added > 0 || ch.in_hist.len() < HIST_SAMPLES {
        ch.last_input_ms = now;
      }
      let stalled = now.saturating_sub(ch.last_input_ms) > 1500;

      let mut measured: Result<Peak, String> = if !rows.enabled {
        Err("这一列已停用（没有输出可测）".into())
      } else if stalled {
        Err("列线程没有新数据（已停止？）".into())
      } else {
        analyzer.measure(&ch.in_hist, &ch.out_hist)
      };

      let mut measured_ms = None;
      let mut confidence = None;
      let mut jitter = None;
      if let Ok(p) = &measured {
        ch.history.push_back(p.ms);
        while ch.history.len() > 5 {
          ch.history.pop_front();
        }
        let mut v: Vec<f32> = ch.history.iter().copied().collect();
        v.sort_by(|a, b| a.partial_cmp(b).unwrap_or(std::cmp::Ordering::Equal));
        let median = v[v.len() / 2];
        let spread = v[v.len() - 1] - v[0];
        measured_ms = Some(median);
        confidence = Some(p.confidence);
        jitter = Some(spread);
        if spread > 3.0 {
          measured = Err(format!(
            "处理链延时在抖动（{spread:.1} ms，中位数 {median:.1} ms），数字只能当参考"
          ));
        }
      }

      let latency = assemble(
        &hub,
        ch.tap.column,
        &rows,
        measured_ms,
        confidence,
        jitter,
        &measured,
      );
      if let Ok(mut g) = ch.tap.result.lock() {
        *g = Some(latency.clone());
      }
      columns.push(latency);
    }

    let complete =
      !columns.is_empty() && columns.iter().all(|c| c.complete || c.state == "disabled");
    let mut next_step = None;
    for c in &columns {
      if c.measured_ms.is_none() && c.state != "disabled" {
        next_step = Some(
          "处理链需要**非周期**信号：对着麦克风说话、或播放一段音乐/噪声；\
           测试音（1 kHz）与静音无法用于互相关。"
            .to_string(),
        );
        break;
      }
    }
    let verdict = report_verdict(&columns);
    let report = LatencyReport {
      schema_version: LATENCY_SCHEMA_VERSION,
      created_ms: now,
      platform: platform(),
      complete,
      columns,
      verdict,
      next_step,
      dump: dump_path.as_ref().map(|p| p.display().to_string()),
    };
    if last_dump.elapsed() >= DUMP_PERIOD {
      last_dump = Instant::now();
      if let Some(path) = dump_path.as_ref() {
        write_dump(&report, path);
      }
    }
    hub.set_latency_report(Probe::ok(report));
  }
}

fn platform() -> &'static str {
  if cfg!(windows) {
    "windows"
  } else if cfg!(target_os = "linux") {
    "linux"
  } else if cfg!(target_os = "macos") {
    "macos"
  } else {
    "other"
  }
}

fn report_verdict(columns: &[ColumnLatency]) -> Probe<Health> {
  if columns.is_empty() {
    return Probe::unavailable("没有可测的列（引擎未运行？）");
  }
  let active: Vec<&ColumnLatency> = columns.iter().filter(|c| c.state != "disabled").collect();
  if active.is_empty() {
    return Probe::ok(Health::ok("全部列已停用"));
  }
  let missing = active.iter().filter(|c| c.measured_ms.is_none()).count();
  let worst = active
    .iter()
    .filter_map(|c| c.total_ms.map(|t| (c.column, t)))
    .max_by(|a, b| a.1.partial_cmp(&b.1).unwrap_or(std::cmp::Ordering::Equal));
  let text = match worst {
    Some((col, total)) => format!(
      "{} 列在测，最慢是第 {} 列 ≈ {total:.0} ms{}",
      active.len(),
      col + 1,
      if missing > 0 {
        format!("（{missing} 列处理链未测到，合计是下限）")
      } else {
        String::new()
      }
    ),
    None => format!("{} 列在测，缓冲/设备分段还不可用", active.len()),
  };
  if missing > 0 {
    Probe::ok(Health::warn(text))
  } else {
    Probe::ok(Health::ok(text))
  }
}

/// 覆盖写 `~/.purevox/latency_last.json`（DEBUG.md §1.4 的落盘出口）。
fn write_dump(report: &LatencyReport, path: &std::path::Path) {
  let Ok(json) = serde_json::to_string_pretty(report) else {
    return;
  };
  let _ = std::fs::write(path, json);
}

// ---------------------------------------------------------------------------
// 预算组装
// ---------------------------------------------------------------------------

#[allow(clippy::too_many_arguments)]
fn assemble(
  hub: &SharedHub,
  column: usize,
  rows: &Rows,
  measured_ms: Option<f32>,
  confidence: Option<f32>,
  jitter: Option<f32>,
  measured: &Result<Peak, String>,
) -> ColumnLatency {
  if !rows.enabled {
    return ColumnLatency::disabled(column);
  }
  let mut stages: Vec<LatencyStage> = Vec::new();

  // ---- 输入侧 ----
  match rows.input.as_ref() {
    Some(inp) => {
      let st = inp.stream_id.as_deref().and_then(|id| hub.stream(id));
      if let Some(st) = st.as_ref() {
        stages.push(device_callback_stage(st, "输入设备缓冲（回调周期）"));
        stages.push(LatencyStage::new(
          "采集环形缓冲",
          Some(st.buffer_level_ms),
          StageSource::Buffer,
          "回调 → 采集工作线程（硬顶 60 ms，目标 10 ms）",
        ));
        stages.push(LatencyStage::new(
          "采集重采样",
          Some(st.resampler_delay_ms as f32),
          StageSource::Resampler,
          st.resampler.clone(),
        ));
      } else {
        let why = if inp.stream_id.is_none() {
          "这一路没有设备流（测试音/网络/临时采集），设备侧缓冲不可从这里读取"
        } else {
          "设备流还没发布（刚打开/刚重启）"
        };
        stages.push(LatencyStage::new(
          "输入设备缓冲（回调周期）",
          None,
          StageSource::Device,
          why,
        ));
      }
      stages.push(LatencyStage::new(
        "行输入环",
        inp.buffer_ms,
        StageSource::Buffer,
        format!("{}：目标 2 hop，超 4 hop 丢最旧", inp.label),
      ));
    }
    None => stages.push(LatencyStage::new(
      "输入侧",
      None,
      StageSource::Buffer,
      "这一列没有可用的输入行",
    )),
  }

  // ---- 处理链（实测）----
  let chain_note = match measured {
    Ok(p) => {
      let what = if p.method == "envelope" {
        "能量包络互相关（波形被模型改相位，退到 10 ms 包络）"
      } else {
        "波形互相关（12 kHz）"
      };
      format!(
        "输入 hop → 输出 hop 的{what}：峰值 {:.2}，领先次峰 {:.2}，分辨率 {:.2} ms",
        p.confidence, p.margin, p.resolution_ms
      )
    }
    Err(e) => e.clone(),
  };
  stages.push(LatencyStage::new(
    "处理链（实测）",
    measured_ms,
    StageSource::Measured,
    chain_note,
  ));

  // ---- 输出侧 ----
  match rows.output.as_ref() {
    Some(out) => {
      stages.push(LatencyStage::new(
        "输出混音源环",
        out.buffer_ms,
        StageSource::Buffer,
        format!("{}：目标 30 ms，硬顶 100 ms（每源各自伺服）", out.label),
      ));
      let st = out.stream_id.as_deref().and_then(|id| hub.stream(id));
      if let Some(st) = st.as_ref() {
        stages.push(LatencyStage::new(
          "播放 48k 环",
          Some(st.buffer_level_ms),
          StageSource::Buffer,
          "时钟伺服目标 40 ms",
        ));
        if let Probe::Ok { value } = &st.device_buffer_ms {
          stages.push(LatencyStage::new(
            "播放设备环",
            Some(*value),
            StageSource::Buffer,
            "工作线程 → 设备回调（目标 30 ms）",
          ));
        } else {
          stages.push(LatencyStage::new(
            "播放设备环",
            None,
            StageSource::Buffer,
            "设备侧水位还没发布",
          ));
        }
        stages.push(LatencyStage::new(
          "播放重采样",
          Some(st.resampler_delay_ms as f32),
          StageSource::Resampler,
          st.resampler.clone(),
        ));
        stages.push(device_callback_stage(st, "输出设备缓冲（回调周期）"));
      } else {
        let why = if out.stream_id.is_none() {
          "这一路没有设备流（网络扬声器），输出设备侧不可从这里读取"
        } else {
          "设备流还没发布（刚打开/刚重启）"
        };
        stages.push(LatencyStage::new(
          "输出设备缓冲（回调周期）",
          None,
          StageSource::Device,
          why,
        ));
      }
    }
    None => stages.push(LatencyStage::new(
      "输出侧",
      None,
      StageSource::Buffer,
      "这一列没有可用的输出行",
    )),
  }

  let known: f32 = stages.iter().filter_map(|s| s.ms).sum();
  let complete = stages.iter().all(|s| s.ms.is_some());
  let dominant = stages
    .iter()
    .filter_map(|s| s.ms.map(|ms| (s.name.clone(), ms)))
    .max_by(|a, b| a.1.partial_cmp(&b.1).unwrap_or(std::cmp::Ordering::Equal))
    .map(|(name, ms)| format!("{name}（{ms:.1} ms）"));

  let detail = match (dominant.as_ref(), complete) {
    (Some(d), true) => format!("合计 ≈ {known:.0} ms（各段都测到）｜ 主要：{d}"),
    (Some(d), false) => {
      let missing: Vec<&str> = stages
        .iter()
        .filter(|s| s.ms.is_none())
        .map(|s| s.name.as_str())
        .collect();
      format!(
        "已知合计 ≈ {known:.0} ms（缺 {}）｜ 主要：{d}",
        missing.join("、")
      )
    }
    (None, _) => "分段还不可用（引擎刚起步/设备未打开）".to_string(),
  };
  let health = if !complete {
    Probe::ok(Health::warn(detail.clone()))
  } else if measured_ms.is_none() {
    Probe::ok(Health::warn(format!(
      "处理链未测到：{}",
      match measured {
        Err(e) => e.as_str(),
        Ok(_) => "无数据",
      }
    )))
  } else if jitter.unwrap_or(0.0) > 3.0 {
    Probe::ok(Health::warn(detail.clone()))
  } else {
    Probe::ok(Health::ok(detail.clone()))
  };

  let (measured_method, measured_resolution_ms) = match measured {
    Ok(p) => (Some(p.method), Some(p.resolution_ms)),
    Err(_) => (None, None),
  };
  ColumnLatency {
    column,
    state: "running".into(),
    measured_ms,
    measured_method,
    measured_resolution_ms,
    confidence,
    jitter_ms: jitter,
    total_ms: if stages.iter().any(|s| s.ms.is_some()) {
      Some(known)
    } else {
      None
    },
    complete,
    dominant,
    stages,
    detail,
    health,
  }
}

fn device_callback_stage(st: &crate::debug::StreamInfo, name: &str) -> LatencyStage {
  let note = match &st.callback_frames {
    Probe::Ok { value } => {
      let ms = value.last as f32 * 1000.0 / st.sample_rate.max(1) as f32;
      let mode = if cfg!(windows) {
        "WASAPI 共享模式"
      } else {
        "设备原生"
      };
      return LatencyStage::new(
        name,
        Some(ms),
        StageSource::Device,
        format!(
          "{mode}：每回调 {} 帧 @ {} Hz（设备侧一个块的缓冲；最近间隔 {:.1} ms，最大 {:.1} ms）",
          value.last, st.sample_rate, value.last_gap_ms, value.max_gap_ms
        ),
      );
    }
    _ => "设备回调还没发生".to_string(),
  };
  LatencyStage::new(name, None, StageSource::Device, note)
}

#[cfg(test)]
mod tests {
  use super::*;

  /// 造一段宽带信号（伪随机），延时 D 后送进分析器：测出来的必须是 D。
  ///
  /// 这条钉住三件事：① 相关公式的符号/窗对齐（`delay = ZERO_K − k`）；② 4:1 抽取后的
  /// 0.083 ms 分辨率；③ 相关峰足够突出时不会被误判成「周期信号」。
  #[test]
  fn measures_known_delay() {
    let mut analyzer = Analyzer::new();
    let mut seed = 0x1234_5678u32;
    let mut rand = || {
      seed ^= seed << 13;
      seed ^= seed >> 17;
      seed ^= seed << 5;
      (seed as f32 / u32::MAX as f32) * 2.0 - 1.0
    };
    let x: Vec<f32> = (0..HIST_SAMPLES).map(|_| rand() * 0.25).collect();
    let start = HIST_SAMPLES - SLACK_MS * PROBE_PER_MS - OUT_SAMPLES;
    for delay_ms in [0usize, 10, 37, 120, 480] {
      let d = delay_ms * PROBE_PER_MS;
      let mut out_hist: VecDeque<f32> = VecDeque::with_capacity(OUT_HIST);
      for j in 0..OUT_SAMPLES {
        let idx = start as isize + j as isize - d as isize;
        out_hist.push_back(if idx >= 0 { x[idx as usize] } else { 0.0 });
      }
      // 末尾是「被 SLACK_MS 丢掉」的那一截（输出窗相对现在后移）
      for _ in 0..(OUT_HIST - OUT_SAMPLES) {
        out_hist.push_back(0.0);
      }
      let in_hist: VecDeque<f32> = x.iter().copied().collect();
      let p = analyzer
        .measure(&in_hist, &out_hist)
        .expect("宽带信号应当可测");
      assert!(
        (p.ms - delay_ms as f32).abs() <= 1.2,
        "延时 {delay_ms} ms，实测 {:.1} ms（相关 {:.2}）",
        p.ms,
        p.confidence
      );
    }
  }

  /// 静音/纯周期信号必须**明确报错**，不能给出一个看似精确的数字（DEBUG.md §1.4）。
  #[test]
  fn refuses_silence_and_periodic_signal() {
    let mut analyzer = Analyzer::new();
    let silence: VecDeque<f32> = std::iter::repeat_n(0.0, HIST_SAMPLES).collect();
    assert!(analyzer.measure(&silence, &silence).is_err());
    // 周期 = 24 个抽取样本（3 kHz）：「延时」每 24 个点就有一个同样高的峰
    let tone: Vec<f32> = (0..HIST_SAMPLES)
      .map(|i| (i as f32 * std::f32::consts::TAU / 24.0).sin() * 0.2)
      .collect();
    let start = HIST_SAMPLES - SLACK_MS * PROBE_PER_MS - OUT_SAMPLES;
    let mut out_hist: VecDeque<f32> = tone[start..start + OUT_SAMPLES].iter().copied().collect();
    for _ in 0..(OUT_HIST - OUT_SAMPLES) {
      out_hist.push_back(0.0);
    }
    let in_hist: VecDeque<f32> = tone.into_iter().collect();
    let e = analyzer.measure(&in_hist, &out_hist);
    assert!(e.is_err(), "周期信号不该给出确定延时");
  }

  /// 降噪/TSE 会改相位：波形相关失效时必须退到 **10 ms 能量包络**，
  /// 而且包络测出来的延时仍要落在 ±10 ms 内。
  #[test]
  fn envelope_fallback_for_phase_changing_processing() {
    let mut analyzer = Analyzer::new();
    let mut seed = 0x9e37_79b9u32;
    let mut rand = || {
      seed ^= seed << 13;
      seed ^= seed >> 17;
      seed ^= seed << 5;
      (seed as f32 / u32::MAX as f32) * 2.0 - 1.0
    };
    // 包络：92 个 hop 的慢变起伏（10 ms 一格），有明显峰谷
    let hops = HIST_SAMPLES / (PROBE_PER_MS * 10);
    let env: Vec<f32> = (0..hops)
      .map(|i| {
        0.05
          + 0.20
            * (0.7 * (i as f32 * 0.31).sin() + 0.5 * (i as f32 * 0.77).cos() + 1.5).max(0.0)
            * (i as f32 * 1.9).sin().abs()
      })
      .collect();
    let per_hop = PROBE_PER_MS * 10;
    let mut a = Vec::with_capacity(HIST_SAMPLES);
    for i in 0..hops {
      for _ in 0..per_hop {
        a.push(env[i] * rand());
      }
    }
    let d_hops = 12; // 120 ms
    let start = hops - SLACK_MS / 10 - OUT_SAMPLES / per_hop;
    let mut out_hist: VecDeque<f32> = VecDeque::with_capacity(OUT_HIST);
    // 输出用**另一段独立噪声**乘同一条包络（波形不相关、包络相关），延时 120 ms
    for j in 0..(OUT_SAMPLES / per_hop) {
      let e = if start + j >= d_hops {
        env[start + j - d_hops]
      } else {
        0.0
      };
      for _ in 0..per_hop {
        out_hist.push_back(e * rand());
      }
    }
    for _ in 0..(OUT_HIST - OUT_SAMPLES) {
      out_hist.push_back(0.0);
    }
    let in_hist: VecDeque<f32> = a.into_iter().collect();
    let p = analyzer.measure(&in_hist, &out_hist).expect("包络应当可测");
    assert_eq!(p.method, "envelope", "波形不相关时必须退到包络");
    assert!(
      (p.ms - 120.0).abs() <= 11.0,
      "包络延时 120 ms，实测 {:.0} ms（相关 {:.2}）",
      p.ms,
      p.confidence
    );
  }

  /// 分段求和的语义：缺项时是下限，且 dominant 指向最大的一段。
  #[test]
  fn budget_reports_dominant_and_incomplete() {
    let hub = crate::debug::DebugHub::new();
    let rows = Rows {
      enabled: true,
      input: Some(RowTapInfo {
        stream_id: None,
        buffer_ms: Some(20.0),
        label: "测试音".into(),
      }),
      output: Some(RowTapInfo {
        stream_id: None,
        buffer_ms: Some(30.0),
        label: "网络扬声器".into(),
      }),
    };
    let c = assemble(
      &hub,
      0,
      &rows,
      Some(12.0),
      Some(0.9),
      Some(0.5),
      &Ok(Peak {
        ms: 12.0,
        confidence: 0.9,
        margin: 0.5,
        method: "waveform",
        resolution_ms: 1000.0 / PROBE_RATE as f32,
      }),
    );
    assert_eq!(c.measured_ms, Some(12.0));
    assert!(!c.complete, "设备流缺失时不许声称测全");
    assert_eq!(c.total_ms, Some(62.0), "20 + 12 + 30");
    assert!(
      c.dominant.as_deref().unwrap_or("").contains("输出混音源环"),
      "主要来源应当是 30 ms 的那段，实测 {:?}",
      c.dominant
    );
  }
}
