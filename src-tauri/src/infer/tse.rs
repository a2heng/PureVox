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
//! 主图  输入  <谱输入> [1,2,481]     分析谱（名字随代变：`spec_in` / `mix_spec`；从模型读）
//!            <参考条件>                注册编码器一次算出、**每 hop 原样喂**（名字/形状随代变）
//!            cache_in [1,D] / [D]     扁平流式缓存（首帧零起；D 与 rank 都从模型读）
//!       输出  spec_out [1,2,481] + cache_out（滞后 1 hop）
//!
//! 注册编码器（`purevox_tse_ref_encoder.onnx`，**每句只跑一次**）
//!        单输入 `ref_spec`/`enr_spec` [1,2,Te,481] → 主图要的那些参考条件
//! ```
//!
//! **谱输入与参考条件都不写死名字**（2026-10-31）：现役 202609c 线是 `mix_spec` +
//! `enr_tok (1,2,1001,180)`；202610 线是 `spec_in` + `keys (1,K,Tk)` + `gb (1,G)`（FiLM）；
//! 202611 线是 `spec_in` + `enr_comp (1,2,Te2,166)`。应用侧按形状找谱输入，其余输入一律当
//! 参考条件（注册编码器必须**同名输出**、形状逐个对上）。
//!
//! 两个模型文件必须**同源**（键投影 / 参考通路的权重都在里面），应用侧整对替换。
//! 缺注册编码器或未设置参考 → [`Tse::process`] 直通（不进模型、不动缓存）。

use ort::value::TensorRef;
use rustfft::FftPlanner;
use rustfft::num_complex::Complex;

use crate::audio::HOP;
use crate::infer::spec::{BINS, NFFT, SPEC_LEN, SpecStream};

/// 现役 TSE 主模型。
pub const MODEL_TSE: &str = "purevox_tse.onnx";
/// 注册编码器（与主模型成对）。
pub const MODEL_TSE_REF: &str = "purevox_tse_ref_encoder.onnx";

/// 可选 TSE 模型 (文件名, 界面名)，供界面下拉。
/// 界面名只是**显示用**，不重复版本号：真正的标识在 ONNX metadata `model`
/// （主图与注册编码器必须同源）。
pub fn tse_models() -> &'static [(&'static str, &'static str)] {
  &[(MODEL_TSE, "TSE（现役）")]
}

pub struct Tse {
  model: crate::infer::LeasedSession,
  /// 主图的谱输入名（`spec_in` / `mix_spec`，随代变；从模型读，不写死）
  spec_name: String,
  dim: usize,
  /// 喂 `cache_in` 用的形状（`[1,D]` 或 `[D]`，照模型输入来）
  cache_shape: Vec<i64>,
  cache: Vec<f32>,
  stft: SpecStream,
  spec_out: [f32; SPEC_LEN],
  /// 主图除 `spec_in` / `cache_in` 之外的输入（参考条件）：名字 + 期望形状，顺序同模型
  ref_names: Vec<String>,
  ref_shapes: Vec<Vec<i64>>,
  /// 注册编码器算出的参考条件（值 + 形状）；None = 未设置参考 → 直通
  ref_inputs: Option<Vec<RefInput>>,
  /// 直通缓冲（无参考时用：不推进进度、不加延迟）
  out: [f32; HOP],
}

/// 一个参考条件的张量（注册编码器输出，每 hop 原样重喂）。
struct RefInput {
  name: String,
  shape: Vec<i64>,
  data: Vec<f32>,
}

impl Tse {
  pub fn load(model_file: &str) -> Result<Self, String> {
    let path = crate::infer::model_path(model_file)?;
    let model = crate::infer::checkout(&path)?;
    let (spec_name, dim, cache_shape, ref_names, ref_shapes) = {
      let guard = model.lock();
      let spec_name = crate::infer::find_spec_input(&guard)?;
      let dim = crate::infer::cache_dim(&guard, "cache_in")?;
      let cache_shape: Vec<i64> = crate::infer::input_shape(&guard, "cache_in")?
        .iter()
        .map(|&d| d as i64)
        .collect();
      if cache_shape.is_empty() || cache_shape.iter().map(|&d| d as usize).product::<usize>() != dim
      {
        return Err(format!(
          "TSE 的 cache_in 形状异常：{cache_shape:?}（元素数应为 {dim}）"
        ));
      }
      let mut names = Vec::new();
      let mut shapes = Vec::new();
      for i in guard.inputs() {
        let name = i.name().to_string();
        if name == spec_name || name == "cache_in" {
          continue;
        }
        shapes.push(
          crate::infer::input_shape(&guard, &name)?
            .iter()
            .map(|&d| d as i64)
            .collect::<Vec<i64>>(),
        );
        names.push(name);
      }
      if names.is_empty() {
        return Err(
          "TSE 主图没有参考条件输入（应为 keys/gb 或 enr_comp 之类；傅里叶在应用侧）".into(),
        );
      }
      (spec_name, dim, cache_shape, names, shapes)
    };
    Ok(Tse {
      model,
      spec_name,
      dim,
      cache_shape,
      cache: vec![0.0; dim],
      stft: SpecStream::new(),
      spec_out: [0.0; SPEC_LEN],
      ref_names,
      ref_shapes,
      ref_inputs: None,
      out: [0.0; HOP],
    })
  }

  pub fn reset(&mut self) {
    self.cache.fill(0.0);
    self.stft.reset();
  }

  /// 用 48 kHz 单声道参考语音算参考条件（一次性；会话内不热切换）。
  ///
  /// 注册长度**跟着模型走**：注册编码器的单输入形状 `(1,2,Te,481)` 里的 Te 就是它要的帧数
  /// （202610 线 1001 帧 ≈ 10 s，202611 线 201 帧 = 2 s）。参考语音不足 Te 帧就平铺、
  /// 超出就截断（见 [`build_ref_spec`]），所以录 10 s 也能喂 2 s 的模型。
  pub fn set_reference(&mut self, samples: &[f32]) -> Result<(), String> {
    if samples.len() < HOP {
      return Err("参考语音太短（不足 10 ms）".into());
    }
    let path = crate::infer::model_path(MODEL_TSE_REF)?;
    let model = crate::infer::checkout(&path)?;
    // 成对校验（2026-10-30）：主图与注册编码器必须同一代。形状能挡住「尺寸不一致」，
    // 但挡不住「同形状、不同权重」——再用 ONNX metadata `model` 比一次。换模型尺寸
    // 或换代时配错文件会在加载期就报出来，而不是悄悄降质。
    let tag = |s: &crate::infer::LeasedSession| {
      let g = s.lock();
      g.metadata().ok().and_then(|m| m.custom("model"))
    };
    if let (Some(a), Some(b)) = (tag(&self.model), tag(&model))
      && a != b
    {
      return Err(format!(
        "TSE 主图与注册编码器不同源：{a} vs {b}（两个文件必须成对发布）"
      ));
    }
    // 注册编码器只有一个输入（`ref_spec` 或 `enr_spec`），名字与帧数都从模型读。
    let (ref_in, frames, bins) = {
      let guard = model.lock();
      let inputs = guard.inputs();
      if inputs.len() != 1 {
        return Err(format!(
          "注册编码器应只有 1 个输入（参考谱），模型给了 {} 个：{:?}",
          inputs.len(),
          inputs.iter().map(|i| i.name()).collect::<Vec<_>>()
        ));
      }
      let name = inputs[0].name().to_string();
      let shape = crate::infer::input_shape(&guard, &name)?;
      if shape.len() != 4 || shape[0] != 1 || shape[1] != 2 {
        return Err(format!(
          "注册编码器的 {name} 应为 (1,2,Te,481)，模型给的是 {shape:?}"
        ));
      }
      if shape[3] != BINS {
        return Err(format!(
          "注册编码器的 bin 数 {} ≠ {BINS}（应用侧 STFT 固定 960/481）",
          shape[3]
        ));
      }
      (name, shape[2], shape[3])
    };
    let spec = build_ref_spec(samples, frames, bins);

    let mut guard = model.lock();
    let session = &mut *guard;
    let outputs = {
      let t = TensorRef::from_array_view(([1i64, 2, frames as i64, bins as i64], spec.as_slice()))
        .map_err(|e| format!("构造 {ref_in} 失败：{e}"))?;
      session
        .run(ort::inputs![ref_in.as_str() => t])
        .map_err(|e| format!("注册编码器推理失败：{e}"))?
    };
    // 主图要的每个参考条件，注册编码器都要**同名输出**、而且形状一模一样。
    // 按形状比，不只比元素个数：形状变了但元素数恰好相同（如 [1,64,143] vs [1,143,64]）
    // 时，只比个数会静默把错误布局喂进主图。
    let mut refs = Vec::with_capacity(self.ref_names.len());
    for (name, want) in self.ref_names.iter().zip(self.ref_shapes.iter()) {
      let Some(out) = outputs.get(name.as_str()) else {
        return Err(format!(
          "注册编码器没有输出 {name}（主图要它）；现有输出：{:?}（请成对替换两个文件）",
          outputs.iter().map(|(k, _)| k).collect::<Vec<_>>()
        ));
      };
      let (shape, data) = out
        .try_extract_tensor::<f32>()
        .map_err(|e| format!("读取 {name} 失败：{e}"))?;
      let shape: Vec<i64> = shape.to_vec();
      if shape != *want {
        return Err(format!(
          "注册编码器与主模型不配套：输出 {name} {shape:?}，主图要 {want:?}\
           （请成对替换两个文件）"
        ));
      }
      refs.push(RefInput {
        name: name.clone(),
        shape,
        data: data.to_vec(),
      });
    }
    self.ref_inputs = Some(refs);
    Ok(())
  }

  /// 处理一个 480 样本 hop。无参考时直通（借用内部缓冲）。**滞后 1 hop**。
  pub fn process(&mut self, hop: &[f32]) -> Result<&[f32], String> {
    debug_assert_eq!(hop.len(), HOP);
    let Self {
      model,
      spec_name,
      dim,
      cache_shape,
      cache,
      stft,
      spec_out,
      ref_inputs,
      out,
      ..
    } = self;
    let Some(refs) = ref_inputs.as_ref() else {
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
        .map_err(|e| format!("构造 {spec_name} 失败：{e}"))?;
      let mut inputs: Vec<(&str, ort::session::SessionInputValue<'_>)> =
        Vec::with_capacity(refs.len() + 2);
      inputs.push((spec_name.as_str(), spec_in.into()));
      for r in refs {
        let t = TensorRef::from_array_view((r.shape.as_slice(), r.data.as_slice()))
          .map_err(|e| format!("构造 {} 失败：{e}", r.name))?;
        inputs.push((r.name.as_str(), t.into()));
      }
      let cache_in = TensorRef::from_array_view((cache_shape.as_slice(), cache.as_slice()))
        .map_err(|e| format!("构造 cache_in 失败：{e}"))?;
      inputs.push(("cache_in", cache_in.into()));
      session.run(inputs)
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
    // **逐位全零输入可能出 NaN**（2026-10-31 实测 202611a：谱全 0 时图内归一 0/0；现役 202609c /
    // 202610 线没这毛病，但保不齐下一代又踩）。数字静音是真会出现的（设备静音 / 欠载补零），NaN 一旦进 OLA
    // 就是持续爆音，所以这里丢**这一 hop** 的输出（照常走 OLA 补零，不破坏时序）、清掉缓存
    // 让模型从零状态重新预热，并留一条 warn 事件（不静默）。根治在训练侧（归一分母加 eps）。
    if !spec_out.iter().all(|v| v.is_finite()) || !cache.iter().all(|v| v.is_finite()) {
      cache.fill(0.0);
      spec_out.fill(0.0);
      crate::debug::events::log(
        "warn",
        "infer",
        "TSE 输出含 NaN/Inf（逐位全零输入）→ 该 hop 补零并清缓存".to_string(),
      );
    }
    Ok(stft.synthesize(&spec_out[..]))
  }
}

/// 参考语音 → `ref_spec (1,2,Te,481)`（与模型导出口径一致：`center=True` + 两端零 pad 半窗）。
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
