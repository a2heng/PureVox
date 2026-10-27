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

//! 对数频谱分段（可视化插件与调试面板的频谱共用这一份映射）。
//!
//! **分析窗 = 4800 点（100 ms，10 Hz 一格）**，不是信号路径上的 960 点：960 点只有
//! 50 Hz 分辨率，20~200 Hz 的对数段会挤在同一两个 bin 里，怎么画都像一条线。
//! 100 ms 只用于显示，不进信号路径、不改变 10 ms hop 网格。
//!
//! 按 **ISO 266 / IEC 61260 的 1/6 倍频程**取 **61 段**（20 Hz ~ 20 kHz，10 个倍频程 ×
//! 6 段 + 1）：界面按对数轴摆柱子。段值取该段 bin 范围里的**平均功率**（ISO 频带电平
//! 的口径）；段比一个 bin 还窄时在相邻 bin 之间插值。
//!
//! 只在 Rust 侧算：插件、`/debug/audio`、`/debug/viz` 拿到的都是这 64 段 dB。

use std::sync::Arc;

use rustfft::num_complex::Complex;
use rustfft::{Fft, FftPlanner};

use crate::audio::HOP;
use crate::audio::SAMPLE_RATE;

/// 段数：ISO 1/6 倍频程，20 Hz ~ 20 kHz → 10 倍频程 × 6 + 1 = **61 段**。
pub const VIZ_BANDS: usize = 61;
/// 每倍频程段数（1/6 倍频程）。
pub const BANDS_PER_OCTAVE: f32 = 6.0;
/// 第一个频带的中心频率（Hz）。ISO 266 基准 = 20 Hz。
pub const BAND_LO_HZ: f32 = 20.0;
/// 最后一个频带的中心频率（Hz）= 20 × 2^10。
pub const BAND_HI_HZ: f32 = 20_480.0;
/// 段覆盖范围（Hz），进快照给界面标注用。
pub const BAND_HZ: [f32; 2] = [BAND_LO_HZ, BAND_HI_HZ];

/// 分析窗 = 10 个 hop = 100 ms（10 Hz 一格）。
pub const ANALYSIS_HOPS: usize = 10;
pub const ANALYSIS_NFFT: usize = HOP * ANALYSIS_HOPS;
/// 分析谱的 bin 宽度（Hz）。
pub const ANALYSIS_BIN_HZ: f32 = SAMPLE_RATE as f32 / ANALYSIS_NFFT as f32;

/// 每段：bin 范围（左闭右开）+ 中心（小数 bin，低频窄段插值用）。
#[derive(Clone, Debug)]
pub struct LogBands {
  bands: Vec<(usize, usize, f32)>,
}

impl LogBands {
  pub fn new(bin_hz: f32, bins: usize) -> Self {
    let n = bins.max(2);
    let mut bands = Vec::with_capacity(VIZ_BANDS);
    for i in 0..VIZ_BANDS {
      // ISO 1/6 倍频程：中心 f_i = 20 × 2^(i/6)，频带上下边 = 中心 × 2^(±1/12)
      let lo_hz = BAND_LO_HZ * 2f32.powf((i as f32 - 0.5) / BANDS_PER_OCTAVE);
      let hi_hz = BAND_LO_HZ * 2f32.powf((i as f32 + 0.5) / BANDS_PER_OCTAVE);
      let lo = ((lo_hz / bin_hz).round() as usize).clamp(0, n - 2);
      let hi = (hi_hz / bin_hz).round() as usize;
      let hi = hi.max(lo + 1).min(n);
      let center = ((lo_hz * hi_hz).sqrt() / bin_hz).clamp(0.0, (n - 2) as f32);
      bands.push((lo, hi, center));
    }
    Self { bands }
  }

  pub fn for_analysis() -> Self {
    Self::new(ANALYSIS_BIN_HZ, ANALYSIS_NFFT / 2 + 1)
  }

  /// 每段的中心频率（Hz），界面画对数轴 / 悬停读数用。
  #[allow(dead_code)]
  pub fn center_hz() -> [f32; VIZ_BANDS] {
    std::array::from_fn(|i| BAND_LO_HZ * 2f32.powf(i as f32 / BANDS_PER_OCTAVE))
  }

  #[allow(dead_code)]
  pub fn power_db(&self, bins_db: &[f32]) -> Vec<f32> {
    let mut out = Vec::with_capacity(VIZ_BANDS);
    self.power_db_into(bins_db, &mut out);
    out
  }

  pub fn power_db_into(&self, bins_db: &[f32], out: &mut Vec<f32>) {
    out.clear();
    let n = bins_db.len();
    if n < 2 {
      out.resize(VIZ_BANDS, db_floor());
      return;
    }
    for &(lo, hi, center) in &self.bands {
      let lo = lo.min(n - 1);
      let hi = hi.max(lo + 1).min(n);
      if hi - lo > 1 {
        // ISO 频带电平：段内功率平均（保持平直噪声画出来是平的）
        let mut p = 0.0f64;
        for &db in &bins_db[lo..hi] {
          p += 10f32.powf(db / 10.0) as f64;
        }
        p /= (hi - lo) as f64;
        out.push(if p <= 0.0 {
          db_floor()
        } else {
          (10.0 * p.log10()) as f32
        });
      } else {
        let k = (center.floor() as usize).min(n - 2);
        let t = (center - k as f32).clamp(0.0, 1.0);
        let p0 = 10f32.powf(bins_db[k] / 10.0);
        let p1 = 10f32.powf(bins_db[k + 1] / 10.0);
        let p = p0 + (p1 - p0) * t;
        out.push(if p <= 0.0 {
          db_floor()
        } else {
          10.0 * p.log10()
        });
      }
    }
  }
}

fn db_floor() -> f32 {
  -160.0
}

/// 可视化频谱分析器：4800 点（100 ms）汉宁窗 → 64 段对数频谱。
pub struct BandAnalyzer {
  ring: Vec<f32>,
  pos: usize,
  window: Vec<f32>,
  fft: Arc<dyn Fft<f32>>,
  scratch: Vec<Complex<f32>>,
  amp_scale: f32,
  bins_db: Vec<f32>,
  bands: LogBands,
}

impl Default for BandAnalyzer {
  fn default() -> Self {
    Self::new()
  }
}

impl BandAnalyzer {
  pub fn new() -> Self {
    let window: Vec<f32> = (0..ANALYSIS_NFFT)
      .map(|i| 0.5 - 0.5 * (2.0 * std::f32::consts::PI * i as f32 / ANALYSIS_NFFT as f32).cos())
      .collect();
    let wsum: f32 = window.iter().sum();
    Self {
      ring: vec![0.0; ANALYSIS_NFFT],
      pos: 0,
      window,
      fft: FftPlanner::new().plan_fft_forward(ANALYSIS_NFFT),
      scratch: vec![Complex::default(); ANALYSIS_NFFT],
      amp_scale: 2.0 / wsum,
      bins_db: vec![db_floor(); ANALYSIS_NFFT / 2 + 1],
      bands: LogBands::for_analysis(),
    }
  }

  /// 喂一个 hop，把 64 段 dBFS 写进 `out`。
  pub fn on_hop(&mut self, hop: &[f32], out: &mut Vec<f32>) {
    for &x in hop {
      self.ring[self.pos] = x;
      self.pos = (self.pos + 1) % ANALYSIS_NFFT;
    }
    for i in 0..ANALYSIS_NFFT {
      let x = self.ring[(self.pos + i) % ANALYSIS_NFFT] * self.window[i];
      self.scratch[i] = Complex::new(x, 0.0);
    }
    self.fft.process(&mut self.scratch);
    for (k, slot) in self.bins_db.iter_mut().enumerate() {
      let a = self.scratch[k].norm() * self.amp_scale;
      *slot = if a <= 0.0 {
        db_floor()
      } else {
        (20.0 * a.log10()).max(db_floor())
      };
    }
    self.bands.power_db_into(&self.bins_db, out);
  }
}

#[cfg(test)]
mod tests {
  use super::*;

  #[test]
  fn flat_spectrum_stays_flat() {
    let b = LogBands::for_analysis();
    let flat = vec![-60.0f32; ANALYSIS_NFFT / 2 + 1];
    let out = b.power_db(&flat);
    assert_eq!(out.len(), VIZ_BANDS);
    for &v in &out {
      assert!((v + 60.0).abs() < 0.01, "段值应当仍是 -60：{v}");
    }
  }

  #[test]
  fn a_tone_lands_near_its_frequency() {
    let b = LogBands::for_analysis();
    let mut bins = vec![-160.0f32; ANALYSIS_NFFT / 2 + 1];
    bins[(1000.0 / ANALYSIS_BIN_HZ).round() as usize] = -6.0;
    let out = b.power_db(&bins);
    let (i, _) = out
      .iter()
      .enumerate()
      .max_by(|a, b| a.1.partial_cmp(b.1).unwrap())
      .unwrap();
    let centers = LogBands::center_hz();
    assert!(
      (700.0..1500.0).contains(&centers[i]),
      "1 kHz 应落在 0.7~1.5 kHz 的段：实际第 {i} 段 ≈ {:.0} Hz",
      centers[i]
    );
  }

  #[test]
  fn low_tones_are_resolved() {
    let mut a = BandAnalyzer::new();
    let mut out_a = Vec::new();
    let mut b = BandAnalyzer::new();
    let mut out_b = Vec::new();
    for h in 0..40 {
      let mut fa = [0.0f32; HOP];
      let mut fb = [0.0f32; HOP];
      for i in 0..HOP {
        let t = (h * HOP + i) as f32 / SAMPLE_RATE as f32;
        fa[i] = (2.0 * std::f32::consts::PI * 60.0 * t).sin();
        fb[i] = (2.0 * std::f32::consts::PI * 80.0 * t).sin();
      }
      a.on_hop(&fa, &mut out_a);
      b.on_hop(&fb, &mut out_b);
    }
    let peak = |v: &[f32]| {
      v.iter()
        .enumerate()
        .max_by(|x, y| x.1.partial_cmp(y.1).unwrap())
        .map(|(i, _)| i)
        .unwrap()
    };
    assert_ne!(peak(&out_a), peak(&out_b), "60/80 Hz 应落在不同段");
  }
}
