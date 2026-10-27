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

//! 调试测量（在工作线程中运行，不在音频回调里）：电平、波形、平均频谱、速率。
//!
//! **两个频谱窗，各管一件事**：
//! - `spectrum_db`（481 bin）：960 点 = 2×HOP（AGENTS §3 的信号路径窗），每 hop 一帧、
//!   发布周期内功率平均 —— 兼容字段，线性画法用；
//! - `spectrum_bands_db`（64 段对数）：**4800 点（100 ms，10 Hz 一格）**，只用于显示，
//!   低频才分得开（见 `dsp::bands` 的说明）。它不参与音频处理，也不改变 10 ms hop 网格。

use std::collections::VecDeque;
use std::sync::Arc;
use std::time::{Duration, Instant};

use rustfft::num_complex::Complex;
use rustfft::{Fft, FftPlanner};

use crate::audio::{HOP, SAMPLE_RATE};
use crate::debug::Probe;
use crate::dsp::bands::BandAnalyzer;

const NFFT: usize = 2 * HOP;
const BINS: usize = NFFT / 2 + 1;
/// 波形保留最近 5 个 hop（50 ms）。
const WAVE_LEN: usize = 5 * HOP;
const DB_FLOOR: f32 = -160.0;

fn db(x: f64) -> f32 {
  if x <= 0.0 {
    DB_FLOOR
  } else {
    (20.0 * x.log10()) as f32
  }
  .max(DB_FLOOR)
}

pub struct MeterOutput {
  pub peak_dbfs: Probe<f32>,
  pub rms_dbfs: Probe<f32>,
  pub waveform: Vec<f32>,
  pub spectrum_db: Vec<f32>,
  /// 对数 61 段（ISO 1/6 倍频程，20 Hz ~ 20 kHz；界面直接画柱）
  pub spectrum_bands_db: Vec<f32>,
}

pub struct Meter {
  wave: VecDeque<f32>,
  prev_hop: Vec<f32>,
  fft: Arc<dyn Fft<f32>>,
  window: Vec<f32>,
  amp_scale: f32,
  scratch: Vec<Complex<f32>>,
  power_acc: Vec<f64>,
  frames: u32,
  peak: f32,
  sumsq: f64,
  samples: u64,
  band_analyzer: BandAnalyzer,
  bands_buf: Vec<f32>,
}

impl Meter {
  pub fn new() -> Self {
    let window: Vec<f32> = (0..NFFT)
      .map(|i| 0.5 - 0.5 * (2.0 * std::f32::consts::PI * i as f32 / NFFT as f32).cos())
      .collect();
    let wsum: f32 = window.iter().sum();
    Meter {
      wave: VecDeque::with_capacity(WAVE_LEN),
      prev_hop: vec![0.0; HOP],
      fft: FftPlanner::new().plan_fft_forward(NFFT),
      window,
      // 单边幅度谱归一化：满幅正弦 → 0 dBFS
      amp_scale: 2.0 / wsum,
      scratch: vec![Complex::default(); NFFT],
      power_acc: vec![0.0; BINS],
      frames: 0,
      peak: 0.0,
      sumsq: 0.0,
      samples: 0,
      band_analyzer: BandAnalyzer::new(),
      bands_buf: Vec::new(),
    }
  }

  pub fn on_hop(&mut self, hop: &[f32]) {
    for &x in hop {
      self.peak = self.peak.max(x.abs());
      self.sumsq += (x as f64) * (x as f64);
      if self.wave.len() == WAVE_LEN {
        self.wave.pop_front();
      }
      self.wave.push_back(x);
    }
    self.samples += hop.len() as u64;

    // 上一 hop + 本 hop 组成 960 点帧
    for (i, &x) in hop.iter().enumerate() {
      self.scratch[i] = Complex::new(self.prev_hop[i] * self.window[i], 0.0);
      self.scratch[HOP + i] = Complex::new(x * self.window[HOP + i], 0.0);
    }
    self.fft.process(&mut self.scratch);
    for (k, acc) in self.power_acc.iter_mut().enumerate() {
      let a = self.scratch[k].norm() * self.amp_scale;
      *acc += (a as f64) * (a as f64);
    }
    self.frames += 1;
    self.prev_hop.copy_from_slice(hop);
    self.band_analyzer.on_hop(hop, &mut self.bands_buf);
  }

  /// 取出本发布周期的统计并清零累加器。
  pub fn take(&mut self) -> MeterOutput {
    let (peak, rms) = if self.samples == 0 {
      (Probe::Pending, Probe::Pending)
    } else {
      (
        Probe::ok(db(self.peak as f64)),
        Probe::ok(db((self.sumsq / self.samples as f64).sqrt())),
      )
    };
    let spectrum: Vec<f32> = if self.frames == 0 {
      Vec::new()
    } else {
      self
        .power_acc
        .iter()
        .map(|p| db((p / self.frames as f64).sqrt()))
        .collect()
    };
    self.power_acc.iter_mut().for_each(|p| *p = 0.0);
    self.frames = 0;
    self.peak = 0.0;
    self.sumsq = 0.0;
    self.samples = 0;
    MeterOutput {
      peak_dbfs: peak,
      rms_dbfs: rms,
      waveform: self.wave.iter().copied().collect(),
      spectrum_db: spectrum,
      spectrum_bands_db: self.bands_buf.clone(),
    }
  }
}

/// 频谱每个 bin 的频率间隔（Hz）。
pub const SPECTRUM_BIN_HZ: f32 = SAMPLE_RATE as f32 / NFFT as f32;

/// 滑动窗口速率测量（帧/秒），窗口 3 s。
pub struct RateMeter {
  hist: VecDeque<(Instant, u64)>,
}

impl RateMeter {
  const WINDOW: Duration = Duration::from_secs(3);

  pub fn new() -> Self {
    RateMeter {
      hist: VecDeque::new(),
    }
  }

  pub fn push(&mut self, now: Instant, total: u64) -> Probe<f64> {
    // **计数器会不会变小**：会。网络断开时 `NetHub::client_disconnected` 会清空入站环
    // （`Playout::reset`），`rx_packets` / `rx_hops` 等累计值随之归零；本窗口里存着的
    // 起点比现在的 total 还大，`total - f0` 就会 **u64 下溢 panic**。
    //
    // 这个 panic 的位置极其恶劣：它首先发生在 tokio 线程（`net::status`），随后 2 Hz 的
    // 主线程轮询（`net_card` / `debug_snapshot`）再次触发 —— 主线程 panic 穿过 WebView2
    // 回调就是 `panic in a function that cannot unwind` → **整个 exe 秒退**
    // （2026-10-27 实测：手机点「重新连接」即复现）。
    //
    // 处理：计数器回退 = 计了一轮新的，**丢掉旧窗口重新起算**，返回「采集中」。
    if let Some(&(_, last)) = self.hist.back()
      && total < last
    {
      self.hist.clear();
    }
    self.hist.push_back((now, total));
    while self.hist.len() > 2 && now.duration_since(self.hist[0].0) > Self::WINDOW {
      self.hist.pop_front();
    }
    let (t0, f0) = self.hist[0];
    let dt = now.duration_since(t0).as_secs_f64();
    if dt < 0.5 {
      return Probe::Pending;
    }
    // 上面的重置保证 `total >= f0`；再兜一次底，速率尺子永远不能把进程带走。
    Probe::ok(total.saturating_sub(f0) as f64 / dt)
  }
}

#[cfg(test)]
mod rate_tests {
  use super::RateMeter;
  use crate::debug::Probe;
  use std::time::{Duration, Instant};

  /// 计数器被重置（断开客户端清入站）后不能下溢 panic：重新起窗口，先报采集中。
  #[test]
  fn counter_reset_does_not_underflow() {
    let mut m = RateMeter::new();
    let t0 = Instant::now();
    m.push(t0, 1000);
    m.push(t0 + Duration::from_secs(1), 1100);
    let before = m.push(t0 + Duration::from_secs(1), 1200);
    assert!(matches!(before, Probe::Ok { .. }), "{before:?}");
    // 断开 → 计数归零，再继续走
    assert!(matches!(
      m.push(t0 + Duration::from_secs(2), 0),
      Probe::Pending
    ));
    let after = m.push(t0 + Duration::from_secs(3), 100);
    assert!(matches!(after, Probe::Ok { .. }), "{after:?}");
    if let Probe::Ok { value } = after {
      assert!((value - 100.0).abs() < 1e-6, "新窗口速率：{value}");
    }
  }
}
