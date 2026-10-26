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

//! TSE 目标说话人提取流式推理（`purevox_tse_202609c` 契约，STFT 在模型图内）：
//!
//! ```text
//! 主模型  输入  mix_hop  [1,480]
//!              enr_tok  [1,2,1001,180]   目标说话人 token（参考编码器预计算）
//!              cache_in [1,D]            扁平流式缓存（首帧零起；D 由模型决定，如 513216）
//!         输出  enh_hop  [1,480]（滞后 1 hop）+ cache_out [1,D]
//!
//! 参考编码器（无训练参数）  输入 ref_spec [1,2,1001,481] → 输出 enr_tok [1,2,1001,180]
//! ```
//!
//! 参考语音：10 s 48 kHz 单声道 → 平铺/截断到 480000 → 双端零 pad 480 → 1001 帧 × 960
//! sqrt-Hann → rfft → ref_spec。无参考时 [`Tse::process`] 直通（不进模型、不动缓存）。
//!
//! **参考编码器不再带 ONNX**（2026-10-26）：`*_ref_encoder.onnx` 里没有任何训练参数，
//! 只有一层固定 DSP（逐 bin 的 DRC + 120 个原始 bin + 60 个 ERB 带平均），矩阵是固定的
//! 等权分带。既然 STFT 本来就在 Rust 里做，这一层直接在本文件实现，省掉一个模型文件、
//! 一个 ORT 会话和一次模型池租约；数值已与旧 ONNX 逐元素对比过（max |Δ| ≈ 1e-6）。

use ort::value::TensorRef;
use rustfft::FftPlanner;
use rustfft::num_complex::Complex;

use crate::audio::{HOP, SAMPLE_RATE};

/// 现役 TSE 主模型。
pub const MODEL_TSE: &str = "purevox_tse_202609c_ep0201.onnx";
/// 参考语音契约：10 s / 1001 帧 / 960 窗 / 481 bin。
const REF_SAMPLES: usize = SAMPLE_RATE as usize * 10;
const REF_FRAMES: usize = REF_SAMPLES / HOP + 1;
const WIN: usize = 2 * HOP;
const BINS: usize = WIN / 2 + 1;
/// `enr_tok` 每帧维度：120 个原始 bin + 60 个 ERB 带。
const TOK_DIM: usize = 180;
/// 原始 bin 与 ERB 带的分界（bin 0..120 原样保留）。
const RAW_BINS: usize = 120;
/// ERB 带边界（bin 索引，含末尾哨兵 481）：60 带恰好覆盖 bin 120..480。
/// 表来自旧 `*_ref_encoder.onnx` 的固定矩阵（每带等权平均），**是模型契约的一部分**。
const ERB_BAND_START: [usize; 61] = [
  120, 123, 126, 129, 132, 135, 138, 141, 145, 148, 152, 155, 159, 163, 166, 170, 174, 178, 183,
  187, 191, 196, 200, 205, 210, 215, 220, 225, 230, 236, 241, 247, 253, 258, 264, 271, 277, 283,
  290, 297, 304, 311, 318, 325, 333, 340, 348, 356, 365, 373, 382, 391, 400, 409, 418, 428, 438,
  448, 459, 469, 481,
];

pub fn tse_models() -> &'static [(&'static str, &'static str)] {
  &[(MODEL_TSE, "TSE 202609c（现役）")]
}

pub struct Tse {
  model: crate::infer::LeasedSession,
  dim: usize,
  cache: Vec<f32>,
  out: Vec<f32>,
  /// 参考 token（值 + 形状）；None = 未设置参考 → 直通
  enr_tok: Option<(Vec<f32>, [i64; 4])>,
}

impl Tse {
  pub fn load(model_file: &str) -> Result<Self, String> {
    let path = crate::infer::model_path(model_file)?;
    let model = crate::infer::checkout(&path)?;
    let dim = crate::infer::cache_dim(&model.lock(), "cache_in")?;
    Ok(Tse {
      model,
      dim,
      cache: vec![0.0; dim],
      out: vec![0.0; HOP],
      enr_tok: None,
    })
  }

  /// 用 48 kHz 单声道参考语音计算 `enr_tok`（一次性；会话内不热切换）。
  pub fn set_reference(&mut self, samples: &[f32]) -> Result<(), String> {
    if samples.len() < HOP {
      return Err("参考语音太短（不足 10 ms）".into());
    }
    self.enr_tok = Some(compute_enr_tok(samples));
    Ok(())
  }

  /// 处理一个 480 样本 hop。无参考时直通（借用内部缓冲）。
  ///
  /// **零分配**：`enr_tok`（1.44 MB）不再每跳 clone，cache 输出拷回预分配缓冲。
  pub fn process(&mut self, hop: &[f32]) -> Result<&[f32], String> {
    debug_assert_eq!(hop.len(), HOP);
    let Self {
      model,
      dim,
      cache,
      out,
      enr_tok,
    } = self;
    let Some((enr, shape)) = enr_tok.as_ref() else {
      out.copy_from_slice(hop);
      return Ok(out);
    };
    let dim = *dim;
    let mut guard = model.lock();
    let session = &mut *guard;

    let run = {
      let mix = TensorRef::from_array_view(([1i64, HOP as i64], hop))
        .map_err(|e| format!("构造 mix_hop 失败：{e}"))?;
      let tok = TensorRef::from_array_view((*shape, enr.as_slice()))
        .map_err(|e| format!("构造 enr_tok 失败：{e}"))?;
      let cache_in = TensorRef::from_array_view(([1i64, dim as i64], cache.as_slice()))
        .map_err(|e| format!("构造 cache_in 失败：{e}"))?;
      session.run(ort::inputs![
        "mix_hop" => mix, "enr_tok" => tok, "cache_in" => cache_in
      ])
    };
    let outputs = match run {
      Ok(o) => o,
      Err(e) => {
        cache.fill(0.0);
        return Err(format!("TSE 推理失败：{e}"));
      }
    };
    let (_, enh) = outputs["enh_hop"]
      .try_extract_tensor::<f32>()
      .map_err(|e| format!("读取 enh_hop 失败：{e}"))?;
    let (_, cache_out) = outputs["cache_out"]
      .try_extract_tensor::<f32>()
      .map_err(|e| format!("读取 cache_out 失败：{e}"))?;
    if enh.len() < HOP || cache_out.len() != dim {
      return Err(format!(
        "TSE 输出维度异常：enh_hop={}，cache_out={}（期望 {HOP} / {dim}）",
        enh.len(),
        cache_out.len()
      ));
    }
    out.copy_from_slice(&enh[..HOP]);
    cache.copy_from_slice(cache_out);
    Ok(out)
  }
}

/// 参考语音 → `enr_tok`（纯 Rust，等价于旧 `*_ref_encoder.onnx`）。
///
/// 逐帧：`sqrt-Hann` 窗 → rfft → 每个 bin 做 DRC
/// `g = ln(1 + 10·mag) / (mag + 1e-8)`（`mag = sqrt(re² + im² + 1e-8)`）→
/// 前 120 个 bin 原样进 token，120..480 按固定 ERB 带等权平均成 60 维。
fn compute_enr_tok(ref_samples: &[f32]) -> (Vec<f32>, [i64; 4]) {
  // 10 s 归一（不足平铺），双端零 pad 一个 hop
  let n = ref_samples.len();
  let mut x = vec![0.0f32; REF_SAMPLES + 2 * HOP];
  for i in 0..REF_SAMPLES {
    x[HOP + i] = ref_samples[i % n];
  }

  // sqrt-Hann（与训练一致：不含 1e-10 偏置）
  let win: Vec<f32> = (0..WIN)
    .map(|i| {
      let h = 0.5 - 0.5 * (2.0 * std::f64::consts::PI * i as f64 / WIN as f64).cos();
      h.max(0.0).sqrt() as f32
    })
    .collect();

  let mut planner = FftPlanner::<f32>::new();
  let fft = planner.plan_fft_forward(WIN);
  // [1,2,1001,180] 的连续布局：先通道 0 全部帧，再通道 1。
  let plane = REF_FRAMES * TOK_DIM;
  let mut tok = vec![0.0f32; 2 * plane];
  let mut buf = vec![Complex::new(0.0f32, 0.0f32); WIN];
  let mut re = vec![0.0f32; BINS];
  let mut im = vec![0.0f32; BINS];

  for k in 0..REF_FRAMES {
    for i in 0..WIN {
      buf[i] = Complex::new(x[k * HOP + i] * win[i], 0.0);
    }
    fft.process(&mut buf);
    for b in 0..BINS {
      let r = buf[b].re;
      let i = buf[b].im;
      let mag = (r * r + i * i + 1e-8).sqrt();
      let g = (1.0 + 10.0 * mag).ln() / (mag + 1e-8);
      re[b] = r * g;
      im[b] = i * g;
    }
    let base0 = k * TOK_DIM;
    let base1 = plane + k * TOK_DIM;
    tok[base0..base0 + RAW_BINS].copy_from_slice(&re[..RAW_BINS]);
    tok[base1..base1 + RAW_BINS].copy_from_slice(&im[..RAW_BINS]);
    for band in 0..(ERB_BAND_START.len() - 1) {
      let lo = ERB_BAND_START[band];
      let hi = ERB_BAND_START[band + 1];
      let w = 1.0 / (hi - lo) as f32;
      let mut sr = 0.0f32;
      let mut si = 0.0f32;
      for b in lo..hi {
        sr += re[b] * w;
        si += im[b] * w;
      }
      tok[base0 + RAW_BINS + band] = sr;
      tok[base1 + RAW_BINS + band] = si;
    }
  }

  (tok, [1, 2, REF_FRAMES as i64, TOK_DIM as i64])
}

#[cfg(test)]
mod tests {
  use super::*;

  /// 参考编码器的 Rust 实现必须与旧的 `*_ref_encoder.onnx` 数值等价。
  /// 期望值由旧 ONNX 对同一段确定性信号（0.1 s 双音平铺到 10 s）算得：
  /// sum / sumsq / 若干采样点（见 2026-10-26 的核对记录）。
  #[test]
  fn enr_tok_matches_ref_encoder_onnx() {
    let n = 4_800usize;
    let refv: Vec<f32> = (0..n)
      .map(|i| {
        let t = i as f64 / SAMPLE_RATE as f64;
        (0.3 * (2.0 * std::f64::consts::PI * 440.0 * t).sin()
          + 0.1 * (2.0 * std::f64::consts::PI * 3_500.0 * t).sin()) as f32
      })
      .collect();
    let (tok, shape) = compute_enr_tok(&refv);
    assert_eq!(shape, [1, 2, REF_FRAMES as i64, TOK_DIM as i64]);
    assert_eq!(tok.len(), 2 * REF_FRAMES * TOK_DIM);

    let sum: f64 = tok.iter().map(|&x| x as f64).sum();
    let sumsq: f64 = tok.iter().map(|&x| (x as f64) * (x as f64)).sum();
    assert!((sum - 38_598.620_608_881_574).abs() < 0.05, "sum={sum}");
    assert!(
      (sumsq - 332_570.926_743_560_65).abs() < 5.0,
      "sumsq={sumsq}"
    );
    for (idx, expect) in [
      (0usize, 3.982_298_9f32),
      (119, 0.910_522),
      (120, -0.300_055_4),
      (180, 1.341_437_7),
      (180_000, -3.982_298_9),
    ] {
      assert!(
        (tok[idx] - expect).abs() < 1e-3,
        "tok[{idx}]={} 期望 {expect}",
        tok[idx]
      );
    }
  }
}
