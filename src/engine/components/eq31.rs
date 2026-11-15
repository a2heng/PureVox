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

//! 31 段标准均衡（ISO 1/3 倍频程，20 Hz ~ 20 kHz），替换旧的 3 段 EQ。
//!
//! **滤波算法用社区维护的 [`biquad`] crate**（MIT/Apache-2.0：RBJ Audio EQ Cookbook
//! 系数 + Direct Form 1），不自己写 biquad：
//! - 每段一个 `PeakingEQ`，Q = 4.318（正好 1/3 倍频程），±24 dB；
//! - 低切 = `HighPass`（Q = 0.7071，12 dB/oct），高切 = `LowPass`；
//! - 运行中改曲线走 [`HotEq`]（无锁原子单元格）：`DirectForm1::update_coefficients`
//!   **不重置滤波器状态**，所以拖动时不会有咔哒声，也不重建会话。
//!
//! 曲线绘制用的频响也由 **同一套系数** 计算（`response_db`），界面不另写公式。
//!
//! [`biquad`]: https://crates.io/crates/biquad

use std::sync::Arc;

use biquad::{Biquad, Coefficients, DirectForm1, Hertz, Q_BUTTERWORTH_F32, Type};

use crate::audio::SAMPLE_RATE;
use crate::engine::hot::{EQ_BANDS, EqSettings, HotEq};
use crate::engine::stage::{FrameContext, Stage, StageError};

/// ISO 266 / IEC 61672 的 31 个 1/3 倍频程中心频率（Hz）。
pub const CENTER_HZ: [f32; EQ_BANDS] = [
  20.0, 25.0, 31.5, 40.0, 50.0, 63.0, 80.0, 100.0, 125.0, 160.0, 200.0, 250.0, 315.0, 400.0, 500.0,
  630.0, 800.0, 1_000.0, 1_250.0, 1_600.0, 2_000.0, 2_500.0, 3_150.0, 4_000.0, 5_000.0, 6_300.0,
  8_000.0, 10_000.0, 12_500.0, 16_000.0, 20_000.0,
];

/// 1/3 倍频程的 Q（`sqrt(2^(1/3))/(2^(1/3)-1)` ≈ 4.318）。
const THIRD_OCTAVE_Q: f32 = 4.318_473_4;
fn fs_hz() -> Hertz<f32> {
  // 48 kHz 是引擎常量，不会失败
  Hertz::<f32>::from_hz(SAMPLE_RATE as f32)
    .unwrap_or_else(|_| Hertz::from_hz(48_000.0_f32).unwrap())
}

fn identity() -> Coefficients<f32> {
  Coefficients {
    b0: 1.0,
    b1: 0.0,
    b2: 0.0,
    a1: 0.0,
    a2: 0.0,
  }
}

fn hz(f: f32) -> Hertz<f32> {
  Hertz::from_hz(f.max(1.0)).unwrap_or_else(|_| Hertz::from_hz(1.0_f32).unwrap())
}

/// 31 段 EQ 的 Stage：每 hop 检查一次热参数版本，变了只重算系数、不清状态。
pub struct Eq31Stage {
  hot: Arc<HotEq>,
  version: u64,
  bands: [DirectForm1<f32>; EQ_BANDS],
  active: [bool; EQ_BANDS],
  low: DirectForm1<f32>,
  high: DirectForm1<f32>,
  low_on: bool,
  high_on: bool,
  settings: EqSettings,
}

impl Eq31Stage {
  pub fn new(hot: Arc<HotEq>) -> Self {
    let settings = hot.load();
    let mut s = Self {
      version: 0,
      hot,
      bands: [DirectForm1::new(identity()); EQ_BANDS],
      active: [false; EQ_BANDS],
      low: DirectForm1::new(identity()),
      high: DirectForm1::new(identity()),
      low_on: false,
      high_on: false,
      settings,
    };
    s.rebuild(true);
    s
  }

  fn rebuild(&mut self, force: bool) {
    let v = self.hot.version();
    if !force && v == self.version {
      return;
    }
    self.version = v;
    let s = self.hot.load();
    let fs = fs_hz();
    for (i, &freq) in CENTER_HZ.iter().enumerate() {
      let g = s.bands[i];
      let on = g.abs() >= 0.01;
      self.active[i] = on;
      if on
        && let Ok(c) =
          Coefficients::<f32>::from_params(Type::PeakingEQ(g), fs, hz(freq), THIRD_OCTAVE_Q)
      {
        self.bands[i].update_coefficients(c);
      }
    }
    // 低切：20 Hz ~ 500 Hz，12 dB/oct；关闭时直通（不跑滤波器）
    self.low_on = s.low_cut_on;
    if self.low_on
      && let Ok(c) =
        Coefficients::<f32>::from_params(Type::HighPass, fs, hz(s.low_cut_hz), Q_BUTTERWORTH_F32)
    {
      self.low.update_coefficients(c);
    } else {
      self.low_on = false;
    }
    // 高切：Nyquist（24 kHz）处 from_params 会拒绝，钳到 23 kHz
    self.high_on = s.high_cut_on;
    if self.high_on
      && let Ok(c) = Coefficients::<f32>::from_params(
        Type::LowPass,
        fs,
        hz(s.high_cut_hz.min(23_000.0)),
        Q_BUTTERWORTH_F32,
      )
    {
      self.high.update_coefficients(c);
    } else {
      self.high_on = false;
    }
    self.settings = s;
  }
}

impl Stage for Eq31Stage {
  fn name(&self) -> &'static str {
    "eq31"
  }

  fn process(&mut self, frame: &mut [f32], _ctx: &FrameContext) -> Result<(), StageError> {
    self.rebuild(false);
    for x in frame.iter_mut() {
      let mut v = *x;
      if self.low_on {
        v = self.low.run(v);
      }
      for i in 0..EQ_BANDS {
        if self.active[i] {
          v = self.bands[i].run(v);
        }
      }
      if self.high_on {
        v = self.high.run(v);
      }
      *x = v;
    }
    Ok(())
  }

  fn reset(&mut self) {
    self.low.reset_state();
    self.high.reset_state();
    for b in &mut self.bands {
      b.reset_state();
    }
  }

  fn status(&self) -> Option<String> {
    let active = self
      .settings
      .bands
      .iter()
      .filter(|g| g.abs() >= 0.01)
      .count();
    let cut = match (self.low_on, self.high_on) {
      (true, true) => format!(
        "低切 {:.0} Hz + 高切 {:.0} Hz",
        self.settings.low_cut_hz, self.settings.high_cut_hz
      ),
      (true, false) => format!("低切 {:.0} Hz", self.settings.low_cut_hz),
      (false, true) => format!("高切 {:.0} Hz", self.settings.high_cut_hz),
      (false, false) => "无切点".to_string(),
    };
    Some(format!("31 段（{active} 段非平坦）· {cut}"))
  }
}

/// 计算 31 段 EQ 的**真实幅频响应**（dB），给界面画曲线用。
///
/// 与音频路径使用**同一套 biquad 系数**：每段 PeakingEQ（Q ≈ 4.318）+ 低切 HighPass /
/// 高切 LowPass，在每个频点求 |H(e^{jω})| 并把 dB 相加。**不是**把各段增益做插值：
/// 曲线是实际响应，圆点是频段设置；相邻滤波器的裙边会让两者略有差异，拖动一段时
/// 曲线也不会在两侧出现插值过冲。
pub fn response_db(settings: &EqSettings, points_hz: &[f32]) -> Vec<f32> {
  let fs = SAMPLE_RATE as f64;
  let fs_hz = fs_hz();
  let mut filters: Vec<Coefficients<f32>> = Vec::with_capacity(EQ_BANDS + 2);
  for (i, &freq) in CENTER_HZ.iter().enumerate() {
    let g = settings.bands.get(i).copied().unwrap_or(0.0);
    // 与 Stage 一致：|g| < 0.01 的段直接旁路，不参与响应。
    if g.abs() < 0.01 {
      continue;
    }
    if let Ok(c) = Coefficients::from_params(Type::PeakingEQ(g), fs_hz, hz(freq), THIRD_OCTAVE_Q) {
      filters.push(c);
    }
  }
  if settings.low_cut_on
    && let Ok(c) = Coefficients::from_params(
      Type::HighPass,
      fs_hz,
      hz(settings.low_cut_hz.clamp(20.0, 500.0)),
      Q_BUTTERWORTH_F32,
    )
  {
    filters.push(c);
  }
  if settings.high_cut_on
    && let Ok(c) = Coefficients::from_params(
      Type::LowPass,
      fs_hz,
      hz(settings.high_cut_hz.clamp(1_000.0, 23_000.0)),
      Q_BUTTERWORTH_F32,
    )
  {
    filters.push(c);
  }
  points_hz
    .iter()
    .map(|&f| {
      let db: f32 = filters.iter().map(|c| magnitude_db(c, f, fs)).sum();
      db.clamp(-60.0, 24.0)
    })
    .collect()
}

/// 单个 biquad 在频率 `f` 处的幅度响应（dB）。
fn magnitude_db(c: &Coefficients<f32>, f: f32, fs: f64) -> f32 {
  let w = std::f64::consts::TAU * (f.max(1.0) as f64) / fs;
  let (sin1, cos1) = w.sin_cos();
  let (sin2, cos2) = (2.0 * w).sin_cos();
  let (b0, b1, b2) = (c.b0 as f64, c.b1 as f64, c.b2 as f64);
  let (a1, a2) = (c.a1 as f64, c.a2 as f64);
  let num_re = b0 + b1 * cos1 + b2 * cos2;
  let num_im = -(b1 * sin1 + b2 * sin2);
  let den_re = 1.0 + a1 * cos1 + a2 * cos2;
  let den_im = -(a1 * sin1 + a2 * sin2);
  let mag2 = (num_re * num_re + num_im * num_im) / (den_re * den_re + den_im * den_im).max(1e-30);
  (10.0 * mag2.log10()) as f32
}

#[cfg(test)]
mod tests {
  use super::*;

  fn stage(settings: EqSettings) -> Eq31Stage {
    Eq31Stage::new(Arc::new(HotEq::new(&settings)))
  }

  #[test]
  fn flat_eq_is_transparent() {
    let mut s = stage(EqSettings::default());
    let mut frame = [0.25f32; 480];
    s.process(&mut frame, &FrameContext::default()).unwrap();
    assert!(
      frame.iter().all(|x| (*x - 0.25).abs() < 1e-6),
      "平坦必须直通"
    );
  }

  #[test]
  fn a_boost_stays_finite_and_changes_the_signal() {
    let mut st = EqSettings::default();
    st.bands[17] = 12.0; // 1 kHz
    let mut s = stage(st);
    let mut frame = [0.0f32; 480];
    for (i, x) in frame.iter_mut().enumerate() {
      *x = (2.0 * std::f32::consts::PI * 1000.0 * i as f32 / SAMPLE_RATE as f32).sin() * 0.2;
    }
    let src = frame;
    s.process(&mut frame, &FrameContext::default()).unwrap();
    assert!(frame.iter().all(|x| x.is_finite()));
    // 1 kHz 处 +12 dB ≈ 4 倍，稳态后明显更大
    let rms = |v: &[f32]| (v.iter().map(|x| x * x).sum::<f32>() / v.len() as f32).sqrt();
    assert!(rms(&frame) > rms(&src) * 2.0, "1 kHz 频段应被抬高");
  }

  #[test]
  fn response_matches_a_band_boost_at_its_center() {
    let mut s = EqSettings::default();
    s.bands[17] = 6.0; // 1 kHz
    let pts = [1000.0f32];
    let db = response_db(&s, &pts);
    assert!(
      (db[0] - 6.0).abs() < 0.6,
      "1 kHz 处的曲线应≈6 dB：{}",
      db[0]
    );
  }

  #[test]
  fn low_and_high_cut_show_in_the_response() {
    let s = EqSettings {
      low_cut_on: true,
      low_cut_hz: 200.0,
      high_cut_on: true,
      high_cut_hz: 8000.0,
      ..EqSettings::default()
    };
    let pts = [50.0f32, 1000.0, 18000.0];
    let db = response_db(&s, &pts);
    assert!(db[0] < -10.0, "50 Hz 应被低切压掉：{}", db[0]);
    assert!(db[1].abs() < 1.0, "中频应基本不变：{}", db[1]);
    assert!(db[2] < -10.0, "18 kHz 应被高切压掉：{}", db[2]);
  }
}
