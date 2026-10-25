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
//! 参考编码器（无参数）  输入 ref_spec [1,2,1001,481] → 输出 enr_tok [1,2,1001,180]
//! ```
//!
//! 参考语音：10 s 48 kHz 单声道 → 平铺/截断到 480000 → 双端零 pad 480 → 1001 帧 × 960
//! sqrt-Hann → rfft → ref_spec。无参考时 [`Tse::process`] 直通（不进模型、不动缓存）。

use ort::value::{Tensor, TensorRef};
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

pub fn tse_models() -> &'static [(&'static str, &'static str)] {
  &[(MODEL_TSE, "TSE 202609c（现役）")]
}

/// 主模型文件名 → 同目录参考编码器文件名（剥离 `_epNNN` 段）。
pub fn ref_encoder_file(model_file: &str) -> String {
  let stem = model_file.strip_suffix(".onnx").unwrap_or(model_file);
  let stem = match stem.rfind("_ep") {
    Some(i) if i + 3 < stem.len() && stem[i + 3..].bytes().all(|b| b.is_ascii_digit()) => {
      &stem[..i]
    }
    _ => stem,
  };
  format!("{stem}_ref_encoder.onnx")
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
  pub fn set_reference(&mut self, model_file: &str, samples: &[f32]) -> Result<(), String> {
    if samples.len() < HOP {
      return Err("参考语音太短（不足 10 ms）".into());
    }
    self.enr_tok = Some(compute_enr_tok(model_file, samples)?);
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

/// 参考语音 → `enr_tok`（走 `*_ref_encoder.onnx`，同样复用模型池）。
fn compute_enr_tok(model_file: &str, ref_samples: &[f32]) -> Result<(Vec<f32>, [i64; 4]), String> {
  let path = crate::infer::model_path(&ref_encoder_file(model_file))?;
  let model = crate::infer::checkout(&path)?;
  let mut session = model.lock();

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
  let mut spec = vec![0.0f32; 2 * REF_FRAMES * BINS];
  let mut buf = vec![Complex::new(0.0f32, 0.0f32); WIN];
  for k in 0..REF_FRAMES {
    for i in 0..WIN {
      buf[i] = Complex::new(x[k * HOP + i] * win[i], 0.0);
    }
    fft.process(&mut buf);
    for b in 0..BINS {
      spec[k * BINS + b] = buf[b].re;
      spec[REF_FRAMES * BINS + k * BINS + b] = buf[b].im;
    }
  }

  let input = Tensor::from_array(([1i64, 2, REF_FRAMES as i64, BINS as i64], spec))
    .map_err(|e| format!("构造 ref_spec 失败：{e}"))?;
  let out_name = session
    .outputs()
    .first()
    .map(|o| o.name().to_string())
    .ok_or_else(|| "参考编码器没有输出".to_string())?;
  let outputs = session
    .run(ort::inputs!["ref_spec" => input])
    .map_err(|e| format!("参考编码器推理失败：{e}"))?;
  let (shape, tok) = outputs[out_name.as_str()]
    .try_extract_tensor::<f32>()
    .map_err(|e| format!("读取 enr_tok 失败：{e}"))?;
  let dims: Vec<i64> = shape.iter().copied().collect();
  if dims.len() != 4 {
    return Err(format!("参考编码器输出维度异常：{dims:?}"));
  }
  let mut arr = [0i64; 4];
  arr.copy_from_slice(&dims);
  Ok((tok.to_vec(), arr))
}
