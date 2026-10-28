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

//! TSE 目标说话人提取流式推理（谱接口，2026-10-29 起；傅里叶在图外）：
//!
//! ```text
//! 主图  输入  spec_in  [1,2,481]   分析谱
//!            keys     [1,K,Tk]     目标说话人 token（注册编码器算出；K/Tk 从模型读，如 64×143）
//!            gb       [1,2C]       全局声纹条件（FiLM 的 γ/β 已预算好，如 64）
//!            cache_in [1,D]        扁平流式缓存（首帧零起；D 由模型决定，如 16,072）
//!       输出  spec_out [1,2,481] + cache_out [1,D]（滞后 1 hop）
//!
//! 注册编码器（`purevox_tse_ref_encoder.onnx`，**每句只跑一次**）
//!        ref_spec [1,2,Te,481]（10 s / 1001 帧）→ keys + gb
//! ```
//!
//! 两个模型文件必须**同源**（键投影 / FiLM 的权重都在里面）：epoch 末成对导出，
//! 应用侧整对替换。缺注册编码器或未设置参考 → [`Tse::process`] 直通（不进模型、不动缓存）。

use ort::value::TensorRef;
use rustfft::FftPlanner;
use rustfft::num_complex::Complex;

use crate::audio::HOP;
use crate::infer::spec::{BINS, NFFT, SPEC_LEN, SpecStream};

/// 现役 TSE 主模型。
pub const MODEL_TSE: &str = "purevox_tse.onnx";
/// 注册编码器（与主模型成对）。
pub const MODEL_TSE_REF: &str = "purevox_tse_ref_encoder.onnx";

pub fn tse_models() -> &'static [(&'static str, &'static str)] {
  &[(MODEL_TSE, "TSE 202610a（现役）")]
}

pub struct Tse {
  model: crate::infer::LeasedSession,
  dim: usize,
  cache: Vec<f32>,
  stft: SpecStream,
  spec_out: [f32; SPEC_LEN],
  /// 主图对 keys / gb 的形状要求（从模型读：换一代模型不用改代码）
  keys_shape: Vec<i64>,
  gb_shape: Vec<i64>,
  /// 注册 token（值 + 形状）；None = 未设置参考 → 直通
  keys: Option<(Vec<f32>, [i64; 3])>,
  gb: Option<Vec<f32>>,
  /// 直通缓冲（无参考时用：不推进进度、不加延迟）
  out: [f32; HOP],
}

impl Tse {
  pub fn load(model_file: &str) -> Result<Self, String> {
    let path = crate::infer::model_path(model_file)?;
    let model = crate::infer::checkout(&path)?;
    let dim = crate::infer::cache_dim(&model.lock(), "cache_in")?;
    crate::infer::check_spec(&model.lock(), "spec_in")?;
    let keys_shape = crate::infer::input_shape(&model.lock(), "keys")?;
    let gb_shape = crate::infer::input_shape(&model.lock(), "gb")?;
    if keys_shape.len() != 3 {
      return Err(format!(
        "TSE 的 keys 输入应为 (1,K,Tk)，模型给的是 {keys_shape:?}"
      ));
    }
    Ok(Tse {
      model,
      dim,
      cache: vec![0.0; dim],
      stft: SpecStream::new(),
      spec_out: [0.0; SPEC_LEN],
      keys_shape: keys_shape.iter().map(|&d| d as i64).collect(),
      gb_shape: gb_shape.iter().map(|&d| d as i64).collect(),
      keys: None,
      gb: None,
      out: [0.0; HOP],
    })
  }

  pub fn reset(&mut self) {
    self.cache.fill(0.0);
    self.stft.reset();
  }

  /// 用 48 kHz 单声道参考语音算 keys/gb（一次性；会话内不热切换）。
  ///
  /// 参考固定 **10 s**（不足平铺、超出截断）→ 双端零 pad 一个 hop → 1001 帧周期 Hann →
  /// rfft → `ref_spec (1,2,1001,481)` → 注册编码器。帧数与 bin 数**从模型输入读**，
  /// 所以换一代注册长度也不用改代码。
  pub fn set_reference(&mut self, samples: &[f32]) -> Result<(), String> {
    if samples.len() < HOP {
      return Err("参考语音太短（不足 10 ms）".into());
    }
    let path = crate::infer::model_path(MODEL_TSE_REF)?;
    let model = crate::infer::checkout(&path)?;
    let (frames, bins) = {
      let guard = model.lock();
      let shape = crate::infer::input_shape(&guard, "ref_spec")?;
      if shape.len() != 4 || shape[0] != 1 || shape[1] != 2 {
        return Err(format!(
          "注册编码器的 ref_spec 应为 (1,2,Te,481)，模型给的是 {shape:?}"
        ));
      }
      if shape[3] != BINS {
        return Err(format!(
          "注册编码器的 bin 数 {} ≠ {BINS}（应用侧 STFT 固定 960/481）",
          shape[3]
        ));
      }
      (shape[2], shape[3])
    };
    let spec = build_ref_spec(samples, frames, bins);

    let mut guard = model.lock();
    let session = &mut *guard;
    let outputs = {
      let t = TensorRef::from_array_view(([1i64, 2, frames as i64, bins as i64], spec.as_slice()))
        .map_err(|e| format!("构造 ref_spec 失败：{e}"))?;
      session
        .run(ort::inputs!["ref_spec" => t])
        .map_err(|e| format!("注册编码器推理失败：{e}"))?
    };
    let (_, keys) = outputs["keys"]
      .try_extract_tensor::<f32>()
      .map_err(|e| format!("读取 keys 失败：{e}"))?;
    let (_, gb) = outputs["gb"]
      .try_extract_tensor::<f32>()
      .map_err(|e| format!("读取 gb 失败：{e}"))?;
    let want_keys: usize = self.keys_shape.iter().product::<i64>() as usize;
    let want_gb: usize = self.gb_shape.iter().product::<i64>() as usize;
    if keys.len() != want_keys || gb.len() != want_gb {
      return Err(format!(
        "注册编码器与主模型不配套：keys {} / gb {}，主图要 {want_keys} / {want_gb}（请成对替换两个文件）",
        keys.len(),
        gb.len()
      ));
    }
    self.keys = Some((
      keys.to_vec(),
      [self.keys_shape[0], self.keys_shape[1], self.keys_shape[2]],
    ));
    self.gb = Some(gb.to_vec());
    Ok(())
  }

  /// 处理一个 480 样本 hop。无参考时直通（借用内部缓冲）。**滞后 1 hop**。
  pub fn process(&mut self, hop: &[f32]) -> Result<&[f32], String> {
    debug_assert_eq!(hop.len(), HOP);
    let Self {
      model,
      dim,
      cache,
      stft,
      spec_out,
      keys,
      gb,
      out,
      ..
    } = self;
    let (Some((keys, keys_shape)), Some(gb)) = (keys.as_ref(), gb.as_ref()) else {
      // 无参考：直通（不进模型、不动缓存、不推进 DSP ⇒ 零额外延迟）。
      // 参考是加载时一次性算好的（不会中途变），所以不需要"预热" STFT。
      out.copy_from_slice(hop);
      return Ok(&out[..]);
    };
    let dim = *dim;
    let mut guard = model.lock();
    let session = &mut *guard;

    let run = {
      let spec = stft.analyze(hop);
      let spec_in = TensorRef::from_array_view(([1i64, 2, BINS as i64], spec))
        .map_err(|e| format!("构造 spec_in 失败：{e}"))?;
      let k = TensorRef::from_array_view((*keys_shape, keys.as_slice()))
        .map_err(|e| format!("构造 keys 失败：{e}"))?;
      let g = TensorRef::from_array_view(([1i64, gb.len() as i64], gb.as_slice()))
        .map_err(|e| format!("构造 gb 失败：{e}"))?;
      let cache_in = TensorRef::from_array_view(([1i64, dim as i64], cache.as_slice()))
        .map_err(|e| format!("构造 cache_in 失败：{e}"))?;
      session.run(ort::inputs![
        "spec_in" => spec_in, "keys" => k, "gb" => g, "cache_in" => cache_in
      ])
    };
    let outputs = match run {
      Ok(o) => o,
      Err(e) => {
        cache.fill(0.0);
        stft.reset();
        return Err(format!("TSE 推理失败：{e}"));
      }
    };
    {
      let (_, spec) = outputs["spec_out"]
        .try_extract_tensor::<f32>()
        .map_err(|e| format!("读取 spec_out 失败：{e}"))?;
      let (_, cache_out) = outputs["cache_out"]
        .try_extract_tensor::<f32>()
        .map_err(|e| format!("读取 cache_out 失败：{e}"))?;
      if spec.len() != SPEC_LEN || cache_out.len() != dim {
        return Err(format!(
          "TSE 输出维度异常：spec_out={}，cache_out={}（期望 {SPEC_LEN} / {dim}）",
          spec.len(),
          cache_out.len()
        ));
      }
      spec_out.copy_from_slice(&spec[..SPEC_LEN]);
      cache.copy_from_slice(cache_out);
    }
    Ok(stft.synthesize(&spec_out[..]))
  }
}

/// 参考语音 → `ref_spec (1,2,Te,481)`（与训练侧 `torch.stft(center=True, pad_mode='constant')` 同口径）。
///
/// 参考归一为 `(Te-1)·hop` 个样本（不足平铺、超出截断），双端各零 pad 一个 hop，
/// 第 k 帧 = `x[k·hop .. k·hop+960] · Hann` → rfft。
fn build_ref_spec(samples: &[f32], frames: usize, bins: usize) -> Vec<f32> {
  debug_assert_eq!(bins, BINS);
  let n = samples.len();
  let body = frames.saturating_sub(1) * HOP; // = Te·hop - hop，配合双端 pad 正好 Te 帧
  let mut x = vec![0.0f32; body + NFFT];
  for i in 0..body {
    x[HOP + i] = samples[i % n];
  }
  let win: Vec<f32> = (0..NFFT)
    .map(|i| (0.5 - 0.5 * (std::f64::consts::TAU * i as f64 / NFFT as f64).cos()) as f32)
    .collect();
  let mut planner = FftPlanner::<f32>::new();
  let fft = planner.plan_fft_forward(NFFT);
  let plane = frames * bins;
  let mut spec = vec![0.0f32; 2 * plane];
  let mut buf = vec![Complex::new(0.0f32, 0.0f32); NFFT];
  for k in 0..frames {
    for i in 0..NFFT {
      buf[i] = Complex::new(x[k * HOP + i] * win[i], 0.0);
    }
    fft.process(&mut buf);
    let base = k * bins;
    for b in 0..bins {
      spec[base + b] = buf[b].re;
      spec[plane + base + b] = buf[b].im;
    }
  }
  spec
}

#[cfg(test)]
mod tests {
  use super::*;

  /// 参考谱的整流正确性：440 Hz 纯音 → 只在对应 bin 附近有能量，
  /// 且幅度 ≈ 窗增益（Σw/2 = hop/2）+ 相位按帧推进。这条不依赖任何权重文件。
  #[test]
  fn ref_spec_peaks_at_the_right_bin() {
    let n = 48_000; // 1 s，平铺到 10 s
    let tone: Vec<f32> = (0..n)
      .map(|i| (std::f64::consts::TAU * 440.0 * i as f64 / 48_000.0).sin() as f32)
      .collect();
    let frames = 1001;
    let spec = build_ref_spec(&tone, frames, BINS);
    assert_eq!(spec.len(), 2 * frames * BINS);
    let plane = frames * BINS;
    // 第 100 帧（远离两端 pad）
    let k = 100;
    let bin = (440.0 * NFFT as f64 / 48_000.0).round() as usize; // ≈ 9
    let mag = |b: usize| {
      let re = spec[k * BINS + b] as f64;
      let im = spec[plane + k * BINS + b] as f64;
      (re * re + im * im).sqrt()
    };
    let peak = mag(bin);
    for b in 0..BINS {
      if (b as i64 - bin as i64).abs() > 2 {
        assert!(mag(b) < peak * 0.02, "bin {b} 泄漏过大");
      }
    }
    // 满幅正弦经 20 ms Hann 的峰值 ≈ A·Σw/2 = A·hop/2 = 240
    assert!((peak - 240.0).abs() < 12.0, "峰值 {peak:.1}");
  }
}
