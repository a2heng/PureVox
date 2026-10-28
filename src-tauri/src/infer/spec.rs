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

//! 谱进谱出模型的应用侧 DSP（2026-10-29 换代，三个模型共用，**DSP 只此一处**）。
//!
//! 模型换代把傅里叶拿出图外（省 7.4 MB 常量 + 1.85M MAC/hop）：图内只有网络，
//! 输入/输出都是 `(1,2,481)` 复数谱。应用侧负责：
//!
//! ```text
//! 波形 hop(480) + 上一 hop → 960 点**周期 Hann** 分析帧 → rfft → spec (1,2,481)
//! spec → 模型 → spec_out → irfft → ×同一 Hann → OLA(hop 480) **逐样本除 Σw²** → 输出
//! ```
//!
//! - 帧 t 以 `t·480` 为中心、±480（= 上一 hop ‖ 本 hop），所以**输出固定滞后 1 hop(10 ms)**：
//!   本 hop 进来时输出的是上一个 hop（它的两帧都已到齐）。
//! - Hann+Hann 在 50% 交叠下 `Σw² = 0.75 + 0.25·cos(2πn/480) ∈ [0.5, 1]`，**必须逐样本除**，
//!   否则每 10 ms 一次 6 dB 起伏（≈100 Hz 嗡声）。这条口径与训练侧 `torch.istft` 逐位等价
//!   （训练侧 `Fourier` 注释、模型 metadata `stft_ola=div_sum_sq` 同源）。
//! - 谱的扁平布局 = `[实部 0..480 ‖ 虚部 0..480]`，与 ONNX 输入 `(1,2,481)` 的连续内存一致。
//!
//! 谱缓冲区是 `(1,2,481)` 的 f32，**不入 cache**；每行每路一份 `SpecStream`（AEC 两路）。

use std::sync::Arc;

use rustfft::num_complex::Complex;
use rustfft::{Fft, FftPlanner};

/// hop = 480 样本（10 ms @48 kHz）。**自己定义**（不引 `crate::audio`）是为了让
/// `examples/` 能 `#[path]` 复用这一份 DSP，不抄第二份；与 `audio::HOP` 的一致性由
/// `infer/mod.rs` 的编译期断言锁住。
pub const HOP: usize = 480;
/// 分析/综合窗长（2×hop）。48 kHz 下 20 ms。
pub const NFFT: usize = 2 * HOP;
/// 单边谱 bin 数（含 DC 与 Nyquist）。
pub const BINS: usize = NFFT / 2 + 1;
/// `(1,2,481)` 摊平后的 f32 个数（实部平面 + 虚部平面）。
pub const SPEC_LEN: usize = 2 * BINS;

/// 一路「波形 hop ↔ 复数谱」的流式转换状态。
pub struct SpecStream {
  win: [f32; NFFT],
  ola: [f32; HOP],
  /// 上一个 hop（拼分析帧用）
  prev: [f32; HOP],
  /// irfft 的后半窗（下一个 hop 才凑齐，先押着）
  tail: [f32; HOP],
  spec: [f32; SPEC_LEN],
  syn: [f32; NFFT],
  out: [f32; HOP],
  work: [Complex<f32>; NFFT],
  fft: Arc<dyn Fft<f32>>,
  ifft: Arc<dyn Fft<f32>>,
}

impl SpecStream {
  // 断言常量没被改歪（窗长必须恰好 2 hop、bin 数必须是 481）
  pub fn new() -> Self {
    const _: () = assert!(NFFT == 2 * HOP && BINS == NFFT / 2 + 1);
    let mut planner = FftPlanner::<f32>::new();
    let fft = planner.plan_fft_forward(NFFT);
    let ifft = planner.plan_fft_inverse(NFFT);
    // 周期 Hann（`torch.hann_window` 口径：分母 NFFT，不是 NFFT-1）
    let win = std::array::from_fn(|i| {
      let x = 0.5 - 0.5 * (std::f64::consts::TAU * i as f64 / NFFT as f64).cos();
      x as f32
    });
    let ola = std::array::from_fn(|i| {
      (0.75 + 0.25 * (std::f64::consts::TAU * i as f64 / HOP as f64).cos()) as f32
    });
    Self {
      win,
      ola,
      prev: [0.0; HOP],
      tail: [0.0; HOP],
      spec: [0.0; SPEC_LEN],
      syn: [0.0; NFFT],
      out: [0.0; HOP],
      work: [Complex::new(0.0, 0.0); NFFT],
      fft,
      ifft,
    }
  }

  /// 丢掉全部流式状态（会话/行重启；`cache` 也要一起清，见各引擎的 `reset`）。
  /// `#[allow(dead_code)]`：`examples/` 复用本文件时用不到 reset。
  #[allow(dead_code)]
  pub fn reset(&mut self) {
    self.prev = [0.0; HOP];
    self.tail = [0.0; HOP];
    self.spec = [0.0; SPEC_LEN];
    self.syn = [0.0; NFFT];
    self.out = [0.0; HOP];
    self.work = [Complex::new(0.0, 0.0); NFFT];
  }

  /// 分析一个 hop，返回 `(1,2,481)` 复数谱（借用内部缓冲，下一次 [analyze] 前有效）。
  pub fn analyze(&mut self, hop: &[f32]) -> &[f32] {
    debug_assert_eq!(hop.len(), HOP);
    for i in 0..NFFT {
      let x = if i < HOP { self.prev[i] } else { hop[i - HOP] };
      self.work[i] = Complex::new(x * self.win[i], 0.0);
    }
    self.fft.process(&mut self.work);
    for k in 0..BINS {
      self.spec[k] = self.work[k].re;
      self.spec[BINS + k] = self.work[k].im;
    }
    self.prev.copy_from_slice(hop);
    &self.spec
  }

  /// 合成：吃模型输出的 `(1,2,481)`，返回**上一个 hop** 的波形（滞后 1 hop，借用内部缓冲）。
  pub fn synthesize(&mut self, spec_out: &[f32]) -> &[f32] {
    debug_assert_eq!(spec_out.len(), SPEC_LEN);
    // 谱对实信号是共轭对称的：0 / Nyquist 取实部，其余按 k 与 N-k 共轭补齐
    self.work[0] = Complex::new(spec_out[0], 0.0);
    for k in 1..BINS - 1 {
      let c = Complex::new(spec_out[k], spec_out[BINS + k]);
      self.work[k] = c;
      self.work[NFFT - k] = c.conj();
    }
    self.work[BINS - 1] = Complex::new(spec_out[BINS - 1], 0.0);
    self.ifft.process(&mut self.work);
    // rustfft 的逆变换不除 N（torch.fft.irfft 会除）⇒ 这里补 1/N
    let k = 1.0 / NFFT as f32;
    for i in 0..NFFT {
      self.syn[i] = self.work[i].re * k * self.win[i];
    }
    for i in 0..HOP {
      self.out[i] = (self.tail[i] + self.syn[i]) / self.ola[i];
    }
    self.tail.copy_from_slice(&self.syn[HOP..]);
    &self.out
  }
}

impl Default for SpecStream {
  fn default() -> Self {
    Self::new()
  }
}

#[cfg(test)]
mod tests {
  use super::*;

  /// 恒等谱（模型什么都不做）必须**逐样本**还原输入（只是滞后 1 hop）。
  /// 这条同时锁住：窗口径、OLA 除 Σw²、逆变换的 1/N、共轭补齐。
  #[test]
  fn identity_roundtrip_is_exact() {
    let mut s = SpecStream::new();
    let mut rng = 12345u64;
    let n_hops = 64;
    let mut input = Vec::with_capacity(n_hops * HOP);
    for _ in 0..n_hops * HOP {
      // xorshift：确定性噪声，不引第三方 rand
      rng ^= rng << 13;
      rng ^= rng >> 7;
      rng ^= rng << 17;
      input.push(((rng >> 40) as f32 / 8_388_608.0) - 1.0);
    }
    let mut out = Vec::new();
    for t in 0..n_hops {
      let spec = s.analyze(&input[t * HOP..(t + 1) * HOP]).to_vec();
      let y = s.synthesize(&spec).to_vec();
      out.extend_from_slice(&y);
    }
    // 输出滞后 1 hop：out[hop k] 对应 input[hop k-1]
    let mut worst = 0.0f32;
    for k in 1..n_hops {
      for i in 0..HOP {
        worst = worst.max((out[k * HOP + i] - input[(k - 1) * HOP + i]).abs());
      }
    }
    assert!(worst < 2e-5, "恒等重建误差 {worst:.3e} 太大");
  }

  /// 前两 hop 只有半窗（上一 hop 是零），所以输出的第一个 hop 不等于输入 —— 这是契约本身
  /// （离线整段处理时训练侧同样丢掉第 0 个 hop）。这里只确认它不是 NaN/爆掉。
  #[test]
  fn first_hop_is_finite() {
    let mut s = SpecStream::new();
    let hop = vec![0.3f32; HOP];
    let spec = s.analyze(&hop).to_vec();
    let y = s.synthesize(&spec).to_vec();
    assert!(y.iter().all(|v| v.is_finite()));
  }
}
