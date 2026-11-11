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

//! 引擎唯一的重采样实现（固定输入块，单声道）。
//!
//! - **整数倍降采样 / 同采样率**（192k/96k/48k → 48k）：多相滤波器，只在**输出率**
//!   上卷积；可微调分数相位，采集侧时钟伺服仍能吸收 ±ppm 漂移。这是热点路径。
//! - **升采样**（44.1k → 48k、48k → 96k）：同一套多相表，64 抽头。实测 44.1k→48k
//!   镜像 −78 dB、20 kHz −1.2 dB，比 rubato 128 抽头快 **6.3×**。
//! - **非整数降采样**（48k → 44.1k 播放侧）：rubato `Async` sinc。
//! - [`Converter`]：任意采样率 → 任意采样率，可在运行时微调比例（输出侧 ASRC 时钟伺服用）。
//! - [`ToHops`]：输入侧，原生采样率 → 48 kHz，并在 48k 输出侧切 10 ms hop。

use rubato::audioadapter_buffers::direct::InterleavedSlice;
use rubato::{
  Adjustable, Async, FixedAsync, Resampler, SincInterpolationParameters, SincInterpolationType,
  WindowFunction,
};

use crate::audio::{HOP, SAMPLE_RATE};

/// 多相滤波器的相位表分辨率：1/128 样本（相位**截断**，不做插值）。
/// 截断误差 < 1/128 样本（平均 1/256），对 20 kHz 内容约 −34 dB 的延迟抖动旁带 —— 这是本路径的
/// 固有上限；要更高只需把 `POLY_PHASES` 调大（只影响表大小，不影响每样本运算量）。
const POLY_PHASES: usize = 128;
const _: () = assert!(POLY_PHASES.is_power_of_two());

/// 小数位置 / 步进的定点精度（Q32）。芯内循环只做整数移位，不再每样本 `floor` 一次
/// `f64`；Q32 的量化误差 2⁻³² 样本，远小于 128 相位表的粒度。
const FBITS: u32 = 32;
const FONE: u64 = 1 << FBITS;
/// 相位索引 = 小数位右移 (32 - log2(128))。
const PHASE_SHIFT: u32 = FBITS - POLY_PHASES.trailing_zeros();

/// 多相重采样器（整数倍降采样 / 同率分数延迟 / 升采样），可变分数相位。
///
/// 为什么比 rubato sinc 快：rubato 的 sinc 在**输入采样率**上做长卷积；多相只在
/// **输出采样率**上算 `taps` 个 MAC。192k→48k 时直接少 4 倍工作量。
///
/// 热路径纪律（每样本一次，48000 次/秒/通道，**不要**往回改）：
/// - 位置用 Q32 整数累加，相位/整数部分都是移位，不碰浮点除法；
/// - 抽头数是编译期常量（12 / 32 / 64 三档各自展开），内层 4 路累加器并行，
///   单条依赖链 12 个 add 的延迟就够拖垮整个循环。
struct Polyphase {
  in_rate: u32,
  out_rate: u32,
  taps: usize,
  /// `phases * taps` 的系数表，按相位连续存放
  table: Vec<f32>,
  /// 输入样本（只保留窗口覆盖到的部分）
  buf: Vec<f32>,
  /// 下一个输出中心在 `buf` 里的 Q32 位置
  pos: u64,
  /// 每个输出前进多少输入样本（Q32，= in/out / rel）
  step: u64,
  out_buf: Vec<f32>,
}

impl Polyphase {
  /// `taps`：每输出的抽头数（12 = 同率分数延迟，32 = 整数倍降采样，64 = 非整数升采样）。
  /// 抽头越多过渡带越窄：44.1k→48k 用 64 抽头时 18.8 kHz 以上开始滚降，
  /// 镜像（22.05–24 kHz）仍被压到 −70 dB 以下。**别按趋势直接加抽头**：
  /// 每加一档都是线性的每样本成本，先用 `resampler_bench` 量。
  fn new(in_rate: u32, out_rate: u32, chunk: usize, taps: usize) -> Result<Self, String> {
    let m = in_rate as f64 / out_rate as f64;
    // 降采样用 0.94 给输出 Nyquist 留过渡带；升采样同样是 0.47（实测 64 抽头时
    // 20 kHz −1.2 dB、镜像 −78 dB，通带/镜像都比 rubato 的 128 抽头 sinc 不差）。
    let fc = if m > 1.0 { 0.5 / m * 0.94 } else { 0.5 * 0.94 };
    let mut table = vec![0.0f32; POLY_PHASES * taps];
    for p in 0..POLY_PHASES {
      let frac = p as f64 / POLY_PHASES as f64;
      let mut sum = 0.0f64;
      for j in 0..taps {
        let t = (j as f64 - (taps as f64 - 1.0) * 0.5) - frac;
        // 理想低通：h(t) = 2fc * sinc(2fc t)，再乘 Blackman 窗
        let x = 2.0 * fc * t;
        let sinc = if x.abs() < 1e-9 {
          1.0
        } else {
          (std::f64::consts::PI * x).sin() / (std::f64::consts::PI * x)
        };
        let w = if t.abs() >= taps as f64 * 0.5 {
          0.0
        } else {
          let u = (t / (taps as f64 * 0.5) + 1.0) * 0.5; // 0..1
          0.42 - 0.5 * (2.0 * std::f64::consts::PI * u).cos()
            + 0.08 * (4.0 * std::f64::consts::PI * u).cos()
        };
        let v = 2.0 * fc * sinc * w;
        table[p * taps + j] = v as f32;
        sum += v;
      }
      // 每个相位单独归一化：DC 增益必须恰好 1
      if sum.abs() > 1e-12 {
        let inv = (1.0 / sum) as f32;
        for j in 0..taps {
          table[p * taps + j] *= inv;
        }
      }
    }
    let out_max = ((chunk as f64 / m) * MAX_RELATIVE).ceil() as usize + 8;
    Ok(Polyphase {
      in_rate,
      out_rate,
      taps,
      table,
      buf: Vec::with_capacity(chunk * 2 + taps),
      pos: (taps / 2) as u64 * FONE,
      step: (m * FONE as f64).round() as u64,
      out_buf: vec![0.0; out_max],
    })
  }

  fn delay_frames(&self) -> usize {
    let m = self.in_rate as f64 / self.out_rate as f64;
    (self.taps as f64 * 0.5 / m).round() as usize
  }

  fn output_frames_max(&self) -> usize {
    self.out_buf.len()
  }

  fn process(&mut self, input: &[f32]) -> &[f32] {
    // 抽头数是常量：每档各自展开（4 路一组，循环次数编译期已知）。
    match self.taps {
      12 => self.run::<12>(input),
      32 => self.run::<32>(input),
      64 => self.run::<64>(input),
      n => unreachable!("多相抽头数只有 12/32/64，得到 {n}"),
    }
  }

  /// 芯内循环：`T` 是抽头数，循环次数与展开都是编译期常量。
  fn run<const T: usize>(&mut self, input: &[f32]) -> &[f32] {
    self.buf.extend_from_slice(input);
    let half = T / 2;
    let half_q = half as u64 * FONE;
    let mut n = 0usize;
    // pos 恒在 buf 里；判断条件用整数比较，省掉每样本一次 f64 比较。
    while self.pos + half_q < (self.buf.len() as u64) * FONE {
      let b = (self.pos >> FBITS) as usize;
      let phase = ((self.pos >> PHASE_SHIFT) & (POLY_PHASES as u64 - 1)) as usize;
      // b ≥ half 时窗口完整；预热阶段 pos 从 half 开始，所以这里恒成立。
      let start = b + 1 - half;
      let coeff = &self.table[phase * T..phase * T + T];
      let src = &self.buf[start..start + T];
      let acc = dot::<T>(coeff, src);
      if n < self.out_buf.len() {
        self.out_buf[n] = acc;
        n += 1;
      }
      self.pos += self.step;
    }
    // 丢掉窗口再也用不到的输入
    let keep_from = ((self.pos >> FBITS) as usize).saturating_sub(half - 1);
    if keep_from > 0 && keep_from <= self.buf.len() {
      self.buf.drain(..keep_from);
      self.pos -= keep_from as u64 * FONE;
    }
    &self.out_buf[..n]
  }

  fn set_relative_ratio(&mut self, rel: f64) {
    let rel = rel.clamp(1.0 / MAX_RELATIVE, MAX_RELATIVE);
    let m = self.in_rate as f64 / self.out_rate as f64;
    let step = m / rel;
    self.step = (step * FONE as f64).round() as u64;
  }
}

/// `T` 抽头点积：4 路累加器打破单条 f32 加法依赖链（12 抽头单链是 12×4 = 48 拍延迟，
/// 4 路后 ≈ 12 拍；LLVM 也能把这 4 路打成 4×f32 SIMD）。
#[inline(always)]
fn dot<const T: usize>(coeff: &[f32], src: &[f32]) -> f32 {
  let c = &coeff[..T];
  let x = &src[..T];
  let mut a = [0.0f32; 4];
  let mut i = 0;
  while i + 4 <= T {
    a[0] += c[i] * x[i];
    a[1] += c[i + 1] * x[i + 1];
    a[2] += c[i + 2] * x[i + 2];
    a[3] += c[i + 3] * x[i + 3];
    i += 4;
  }
  let mut s = (a[0] + a[1]) + (a[2] + a[3]);
  while i < T {
    s += c[i] * x[i];
    i += 1;
  }
  s
}

enum Kind {
  Passthrough,
  Rubato(Box<Async<f32>>),
  Poly(Polyphase),
}

pub struct Converter {
  kind: Kind,
  chunk: usize,
  out_buf: Vec<f32>,
  description: String,
  delay_frames: usize,
}

impl Converter {
  /// `chunk`：每次 [`process`](Self::process) 的输入帧数。
  /// `adjustable`：需要运行时微调比例时为 true（此时即使采样率相同也走重采样）。
  pub fn new(in_rate: u32, out_rate: u32, chunk: usize, adjustable: bool) -> Result<Self, String> {
    if in_rate == 0 || out_rate == 0 {
      return Err(format!("非法采样率 {in_rate} → {out_rate}"));
    }
    if in_rate == out_rate && !adjustable {
      return Ok(Converter {
        kind: Kind::Passthrough,
        chunk,
        out_buf: Vec::new(),
        description: format!("直通（{in_rate} Hz）"),
        delay_frames: 0,
      });
    }
    // 非整数比例：只要不走 rubato 就统一用多相表（每输出 taps 个 MAC，与比例无关）。
    // - 整数倍降采样 / 同采样率：12 / 32 抽头，够用且最便宜；
    // - 升采样（44.1k→48k 等）：64 抽头。过渡带 ≈ 5.5/taps，18.8 kHz 以上滚降，
    //   镜像落在 22–24 kHz 且被压到 −70 dB 以下（44.1k 输入本身没有 22.05k 以上内容）。
    let taps = if in_rate == out_rate {
      Some(12)
    } else if in_rate.is_multiple_of(out_rate) && in_rate / out_rate <= 8 {
      Some(32)
    } else if in_rate < out_rate {
      Some(64)
    } else {
      None
    };
    if let Some(taps) = taps {
      let p = Polyphase::new(in_rate, out_rate, chunk, taps)?;
      let desc = if in_rate == out_rate {
        format!("多相分数延迟 {in_rate} Hz（{} 抽头）", p.taps)
      } else {
        format!(
          "多相 {in_rate} → {out_rate} Hz（{} 抽头 × {} 相位）",
          p.taps, POLY_PHASES
        )
      };
      let delay = p.delay_frames();
      return Ok(Converter {
        kind: Kind::Poly(p),
        chunk,
        out_buf: Vec::new(),
        description: desc,
        delay_frames: delay,
      });
    }
    let ratio = out_rate as f64 / in_rate as f64;
    let params = SincInterpolationParameters::new(128, WindowFunction::Blackman2)
      .oversampling_factor(256)
      .interpolation(SincInterpolationType::Quadratic);
    let rs = Async::<f32>::new_sinc(ratio, MAX_RELATIVE, &params, chunk, 1, FixedAsync::Input)
      .map_err(|e| format!("创建重采样器失败：{e}"))?;
    let out_buf = vec![0.0; rs.output_frames_max()];
    let delay_frames = rs.output_delay();
    Ok(Converter {
      kind: Kind::Rubato(Box::new(rs)),
      chunk,
      out_buf,
      description: format!("rubato sinc {in_rate} → {out_rate} Hz（块 {chunk}）"),
      delay_frames,
    })
  }

  pub fn description(&self) -> &str {
    &self.description
  }

  pub fn is_passthrough(&self) -> bool {
    matches!(self.kind, Kind::Passthrough)
  }

  pub fn delay_frames(&self) -> usize {
    self.delay_frames
  }

  pub fn output_frames_max(&self) -> usize {
    match &self.kind {
      Kind::Passthrough => self.chunk,
      Kind::Rubato(rs) => rs.output_frames_max(),
      Kind::Poly(p) => p.output_frames_max(),
    }
  }

  /// 微调比例（相对标称比例）：>1 输出变多，<1 输出变少。
  pub fn set_relative_ratio(&mut self, rel: f64) -> Result<(), String> {
    match &mut self.kind {
      // 直通没有比例可调；调用方（采集伺服）可以继续按放行速率工作。
      Kind::Passthrough => Ok(()),
      Kind::Rubato(rs) => rs
        .set_resample_ratio_relative(rel, true)
        .map_err(|e| format!("调比例失败：{e}")),
      Kind::Poly(p) => {
        p.set_relative_ratio(rel);
        Ok(())
      }
    }
  }

  /// 处理恰好 `chunk` 帧输入，返回本次输出。
  pub fn process<'a>(&'a mut self, input: &'a [f32]) -> Result<&'a [f32], String> {
    debug_assert_eq!(input.len(), self.chunk);
    match &mut self.kind {
      Kind::Passthrough => Ok(input),
      Kind::Rubato(rs) => {
        let inp = InterleavedSlice::new(input, 1, self.chunk).map_err(|e| e.to_string())?;
        let cap = self.out_buf.len();
        let mut out =
          InterleavedSlice::new_mut(&mut self.out_buf, 1, cap).map_err(|e| e.to_string())?;
        let (_, n_out) = rs
          .process_into_buffer(&inp, &mut out, None)
          .map_err(|e| format!("重采样失败：{e}"))?;
        Ok(&self.out_buf[..n_out])
      }
      Kind::Poly(p) => Ok(p.process(input)),
    }
  }
}

/// 比例可调范围：允许相对标称比例 ±10%（伺服本身另行限幅到 ±3%）。
const MAX_RELATIVE: f64 = 1.1;

/// 输入侧：原生采样率 → 48 kHz，并切 10 ms hop。原生即 48 kHz 时直通。
pub struct ToHops {
  conv: Converter,
  in_buf: Vec<f32>,
  pending: Vec<f32>,
}

impl ToHops {
  pub fn new(native_rate: u32) -> Result<Self, String> {
    // 原生 10 ms 块；非 100 整除的采样率（如 22050）取整，输出侧 hop 网格不受影响。
    // **一律走可调比例**（即使 48k→48k）：采集侧要用环水位伺服吸收设备时钟差
    // （实测 Realtek 输入 48067 Hz，+1400 ppm；固定比例只能靠 60 ms 硬顶丢最旧）。
    let chunk = ((native_rate as usize) / 100).max(1);
    let conv = Converter::new(native_rate, SAMPLE_RATE, chunk, true)?;
    Ok(ToHops {
      conv,
      in_buf: Vec::with_capacity(chunk * 8),
      pending: Vec::with_capacity(HOP * 4),
    })
  }

  pub fn description(&self) -> &str {
    self.conv.description()
  }

  pub fn delay_frames(&self) -> usize {
    self.conv.delay_frames()
  }

  /// 采集侧时钟伺服：微调比例，保持设备环水位（相对标称比例，±3% 由调用方限幅）。
  pub fn set_relative_ratio(&mut self, rel: f64) -> Result<(), String> {
    self.conv.set_relative_ratio(rel)
  }

  /// 已重采样但不足一个 hop 的剩余帧数（恒 < HOP）。
  pub fn pending_frames(&self) -> usize {
    self.pending.len()
  }

  /// 喂入原生采样率单声道样本；每凑满一个 48 kHz hop 回调一次。
  pub fn push(&mut self, input: &[f32], mut on_hop: impl FnMut(&[f32])) -> Result<(), String> {
    if self.conv.is_passthrough() {
      self.pending.extend_from_slice(input);
    } else {
      self.in_buf.extend_from_slice(input);
      let chunk = self.conv.chunk;
      let mut consumed = 0;
      while self.in_buf.len() - consumed >= chunk {
        let out = self
          .conv
          .process(&self.in_buf[consumed..consumed + chunk])?;
        self.pending.extend_from_slice(out);
        consumed += chunk;
      }
      self.in_buf.drain(..consumed);
    }
    let mut start = 0;
    while self.pending.len() - start >= HOP {
      on_hop(&self.pending[start..start + HOP]);
      start += HOP;
    }
    self.pending.drain(..start);
    Ok(())
  }
}

#[cfg(test)]
mod tests {
  use super::*;

  fn run_sine(in_rate: u32, out_rate: u32, hz: f64, rel: f64) -> Vec<f32> {
    let chunk = (in_rate as usize / 100).max(1);
    let mut c = Converter::new(in_rate, out_rate, chunk, true).expect("converter");
    c.set_relative_ratio(rel).unwrap();
    let n = in_rate as usize / 2; // 500 ms
    let step = 2.0 * std::f64::consts::PI * hz / in_rate as f64;
    let input: Vec<f32> = (0..n).map(|i| (i as f64 * step).sin() as f32).collect();
    let mut out = Vec::new();
    for block in input.chunks(chunk) {
      // 让最后一块也走完整 chunk 的接口
      let mut b = block.to_vec();
      if b.len() < chunk {
        b.resize(chunk, 0.0);
      }
      let y = c.process(&b).unwrap();
      out.extend_from_slice(y);
    }
    out
  }

  fn rms(x: &[f32]) -> f32 {
    (x.iter().map(|v| (*v as f64) * (*v as f64)).sum::<f64>() / x.len() as f64).sqrt() as f32
  }

  /// 192k → 48k：输出长度约 1/4，1 kHz 正弦的 RMS 应接近 1/√2（不因滤波掉电平）。
  #[test]
  fn polyphase_192k_to_48k_keeps_sine() {
    let out = run_sine(192_000, 48_000, 1000.0, 1.0);
    let expect = 48_000usize / 2;
    assert!(
      (out.len() as isize - expect as isize).abs() < 64,
      "输出长度 {} 应接近 {expect}",
      out.len()
    );
    // 跳过滤波器预热
    let steady = &out[200..out.len() - 200];
    let r = rms(steady);
    assert!((r - 0.707).abs() < 0.08, "RMS {r} 应接近 0.707");
  }

  /// 降采样的抗混叠：30 kHz（高于 48k 的 24k Nyquist）必须被压掉，
  /// 不能折返成 18 kHz。
  #[test]
  fn polyphase_192k_to_48k_rejects_above_nyquist() {
    let out = run_sine(192_000, 48_000, 30_000.0, 1.0);
    let steady = &out[200..out.len() - 200];
    let r = rms(steady);
    assert!(r < 0.08, "30 kHz 应被抗混叠滤波压掉，实测 RMS {r}");
  }

  /// 升采样（44.1k → 48k）也走多相：1 kHz 正弦电平不变。
  #[test]
  fn polyphase_44k1_to_48k_keeps_sine() {
    let out = run_sine(44_100, 48_000, 1000.0, 1.0);
    let steady = &out[200..out.len() - 200];
    let r = rms(steady);
    assert!((r - 0.707).abs() < 0.08, "44.1k→48k RMS {r} 应接近 0.707");
  }

  /// 非整数**降**采样（48k → 44.1k，播放侧）仍走 rubato sinc，不能被多相路径改坏。
  #[test]
  fn rubato_48k_to_44k1_keeps_sine() {
    let out = run_sine(48_000, 44_100, 1000.0, 1.0);
    let steady = &out[200..out.len() - 200];
    let r = rms(steady);
    assert!((r - 0.707).abs() < 0.08, "48k→44.1k RMS {r} 应接近 0.707");
  }

  /// 窗函数 DFT 幅度（Hann 窗，单边归一化；只用于质量测量，不进信号路径）。
  fn tone_amp(x: &[f32], fs: f64, hz: f64) -> f64 {
    let n = x.len() as f64;
    let w = |i: usize| 0.5 - 0.5 * (2.0 * std::f64::consts::PI * i as f64 / n).cos();
    let (mut re, mut im, mut ws) = (0.0, 0.0, 0.0);
    for (i, &v) in x.iter().enumerate() {
      let a = w(i);
      let ph = 2.0 * std::f64::consts::PI * hz * i as f64 / fs;
      re += a * v as f64 * ph.cos();
      im += a * v as f64 * ph.sin();
      ws += a;
    }
    (re * re + im * im).sqrt() * 2.0 / ws
  }

  /// 升采样的镜像频率（44.1k 的镜像是 44.1k − f，按 48k 折回 Nyquist 以内）。
  fn image_hz(in_rate: u32, out_rate: u32, hz: f64) -> f64 {
    let fs = out_rate as f64;
    let mut a = (in_rate as f64 - hz) % fs;
    if a > fs / 2.0 {
      a = fs - a;
    }
    a
  }

  /// 44.1k → 48k 多相质量回归：通带电平、镜像抑制、每 10 ms 块耗时（含 rubato 对照）。
  /// 手动跑：`cargo test --release -- --ignored --nocapture measure_44k1`
  #[test]
  #[ignore = "手动质量/耗时测量（约 1 s）"]
  fn measure_44k1_quality() {
    use std::time::Instant;

    /// 跑 2 s 正弦，返回稳态输出。
    fn run(conv: &mut Converter, hz: f64) -> Vec<f32> {
      let chunk = 441usize;
      let n = 44_100 * 2;
      let step = 2.0 * std::f64::consts::PI * hz / 44_100.0;
      let input: Vec<f32> = (0..n).map(|i| (i as f64 * step).sin() as f32).collect();
      let mut out = Vec::new();
      for b in input.chunks(chunk) {
        let mut b = b.to_vec();
        b.resize(chunk, 0.0);
        out.extend_from_slice(conv.process(&b).unwrap());
      }
      out
    }

    let rubato = || {
      let params = SincInterpolationParameters::new(128, WindowFunction::Blackman2)
        .oversampling_factor(256)
        .interpolation(SincInterpolationType::Quadratic);
      Async::<f32>::new_sinc(
        48_000.0 / 44_100.0,
        MAX_RELATIVE,
        &params,
        441,
        1,
        FixedAsync::Input,
      )
      .unwrap()
    };
    // rubato 对照单独走一遍（Converter 现在不选它）
    let mut rub = rubato();
    let mut rub_out = |hz: f64| -> Vec<f32> {
      let chunk = 441usize;
      let n = 44_100 * 2;
      let step = 2.0 * std::f64::consts::PI * hz / 44_100.0;
      let input: Vec<f32> = (0..n).map(|i| (i as f64 * step).sin() as f32).collect();
      let cap = rub.output_frames_max();
      let mut out = vec![0.0f32; cap * 256];
      let mut n_out = 0usize;
      for b in input.chunks(chunk) {
        let mut b = b.to_vec();
        b.resize(chunk, 0.0);
        let inp = InterleavedSlice::new(&b, 1, chunk).unwrap();
        let mut o = InterleavedSlice::new_mut(&mut out[n_out..], 1, cap).unwrap();
        let (_, k) = rub.process_into_buffer(&inp, &mut o, None).unwrap();
        n_out += k;
      }
      out.truncate(n_out);
      out
    };

    let freqs = [1_000.0, 10_000.0, 15_000.0, 18_000.0, 20_000.0, 21_000.0];
    let mut poly = Converter::new(44_100, 48_000, 441, true).unwrap();
    println!(
      "\n{:<14} {:>6} {:>6} {:>6} {:>6} {:>6} {:>6} {:>9}",
      "实现", "1k", "10k", "15k", "18k", "20k", "21k", "最差镜像"
    );
    for (name, is_poly) in [("多相 64 抽头", true), ("rubato 128", false)] {
      let mut worst_img = -999.0f64;
      print!("{name:<14}");
      for &hz in &freqs {
        let out = if is_poly {
          run(&mut poly, hz)
        } else {
          rub_out(hz)
        };
        let steady = &out[4800..out.len() - 4800];
        let sig = tone_amp(steady, 48_000.0, hz);
        let img = tone_amp(steady, 48_000.0, image_hz(44_100, 48_000, hz));
        worst_img = worst_img.max(20.0 * (img / sig).log10());
        print!(" {:>6.1}", 20.0 * sig.log10());
      }
      println!(" {worst_img:>9.1}");
      if is_poly {
        assert!(
          worst_img < -70.0,
          "多相镜像抑制 {worst_img:.1} dB 应 < −70 dB"
        );
      }
    }

    // 每 10 ms 块耗时（单通道）
    let mut p = Converter::new(44_100, 48_000, 441, true).unwrap();
    let block = vec![0.1f32; 441];
    let iters = 100_000;
    let t = Instant::now();
    for _ in 0..iters {
      std::hint::black_box(p.process(&block).unwrap());
    }
    let ns = t.elapsed().as_nanos() as f64 / iters as f64;
    println!(
      "多相 64 抽头耗时 {ns:.0} ns/10ms 块（{:.3}% 单核）",
      ns / 1e7 * 100.0
    );
  }

  /// 同采样率：只做分数延迟，DC 必须原样通过（1.0 → 1.0）。
  #[test]
  fn polyphase_same_rate_dc_gain_unity() {
    let chunk = 480;
    let mut c = Converter::new(48_000, 48_000, chunk, true).unwrap();
    let input = vec![1.0f32; 4800];
    let mut out = Vec::new();
    for b in input.chunks(chunk) {
      out.extend_from_slice(c.process(b).unwrap());
    }
    let steady = &out[100..out.len() - 100];
    for v in steady {
      assert!((*v - 1.0).abs() < 1e-3, "DC 应为 1.0，得到 {v}");
    }
  }

  /// 比例微调：+1% 应多出约 1% 的输出样本（采集侧时钟伺服依赖它）。
  #[test]
  fn polyphase_ratio_adjusts_output_count() {
    let chunk = 480;
    let mut a = Converter::new(48_000, 48_000, chunk, true).unwrap();
    let mut b = Converter::new(48_000, 48_000, chunk, true).unwrap();
    b.set_relative_ratio(1.01).unwrap();
    let input = vec![0.1f32; chunk * 100];
    let mut na = 0;
    let mut nb = 0;
    for block in input.chunks(chunk) {
      na += a.process(block).unwrap().len();
      nb += b.process(block).unwrap().len();
    }
    assert!(nb > na, "rel=1.01 应输出更多：{nb} vs {na}");
    let ratio = nb as f64 / na as f64;
    assert!((ratio - 1.01).abs() < 0.01, "输出比例 {ratio} 应接近 1.01");
  }
}
