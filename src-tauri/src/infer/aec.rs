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

//! AEC 回声消除流式推理（谱接口，2026-10-29 起；傅里叶在图外）：
//!
//! ```text
//! 输入  mic_spec [1,2,481]  近端（本机麦克风）分析谱
//!       far_spec [1,2,481]  远端（扬声器回环 / 参考）分析谱，时间对齐后
//!       cache_in [1,D]      扁平流式缓存（首帧零起；D 由模型决定，如 91,920）
//! 输出  mic_spec_out [1,2,481] + cache_out [1,D]
//! ```
//!
//! 远端的对齐/延时由 [`crate::engine::aec::AecRow`] 负责（它给的是两个 **480 样本 hop**）；
//! 本文件把两路各自拼成 960 分析帧、跑模型、再把近端那路合成回波形（滞后 1 hop）。

use ort::value::TensorRef;

use crate::audio::HOP;
use crate::infer::spec::{BINS, SPEC_LEN, SpecStream};

/// 现役 AEC 模型（单流：far/mic 各 5ch 在第 0 层 early fusion，无显式延迟线；
/// 对齐靠 GRU 隐状态，前端感受野 ≥ 数据最大延迟）。
pub const MODEL_AEC: &str = "purevox_aec.onnx";

/// 可选 AEC 模型 (文件名, 界面名)，供界面下拉。
/// 界面名只是**显示用**，不重复版本号：真正的标识在 ONNX metadata `model`。
pub fn aec_models() -> &'static [(&'static str, &'static str)] {
  &[(MODEL_AEC, "AEC（现役）")]
}

pub struct Aec {
  model: crate::infer::LeasedSession,
  dim: usize,
  cache: Vec<f32>,
  /// 近端 / 远端各一路分析与合成状态（合成只用近端那路）
  mic: SpecStream,
  far: SpecStream,
  spec_out: [f32; SPEC_LEN],
}

impl Aec {
  pub fn load(model_file: &str) -> Result<Self, String> {
    let path = crate::infer::model_path(model_file)?;
    let model = crate::infer::checkout(&path)?;
    let dim = crate::infer::cache_dim(&model.lock(), "cache_in")?;
    crate::infer::check_spec(&model.lock(), "mic_spec")?;
    crate::infer::check_spec(&model.lock(), "far_spec")?;
    Ok(Aec {
      model,
      dim,
      cache: vec![0.0; dim],
      mic: SpecStream::new(),
      far: SpecStream::new(),
      spec_out: [0.0; SPEC_LEN],
    })
  }

  /// 会话/行重启（同样给延时校准用）：缓存 + 两路 DSP 状态归零。
  pub fn reset(&mut self) {
    self.cache.fill(0.0);
    self.mic.reset();
    self.far.reset();
  }

  /// 处理一对对齐好的 hop（mic + far），返回增强后的 hop（借用内部缓冲，滞后 1 hop）。
  ///
  /// **零分配**：mic/far/cache 都是借用张量，输出拷回预分配缓冲。
  pub fn process(&mut self, mic: &[f32], far: &[f32]) -> Result<&[f32], String> {
    debug_assert_eq!(mic.len(), HOP);
    debug_assert_eq!(far.len(), HOP);
    let Self {
      model,
      dim,
      cache,
      mic: mic_stft,
      far: far_stft,
      spec_out,
    } = self;
    let dim = *dim;
    let mut guard = model.lock();
    let session = &mut *guard;

    let run = {
      let mic_spec = mic_stft.analyze(mic);
      let far_spec = far_stft.analyze(far);
      let mic_t = TensorRef::from_array_view(([1i64, 2, BINS as i64], mic_spec))
        .map_err(|e| format!("构造 mic_spec 失败：{e}"))?;
      let far_t = TensorRef::from_array_view(([1i64, 2, BINS as i64], far_spec))
        .map_err(|e| format!("构造 far_spec 失败：{e}"))?;
      let cache_in = TensorRef::from_array_view(([1i64, dim as i64], cache.as_slice()))
        .map_err(|e| format!("构造 cache_in 失败：{e}"))?;
      session.run(ort::inputs![
        "mic_spec" => mic_t, "far_spec" => far_t, "cache_in" => cache_in
      ])
    };
    let outputs = match run {
      Ok(o) => o,
      Err(e) => {
        cache.fill(0.0);
        mic_stft.reset();
        far_stft.reset();
        return Err(format!("AEC 推理失败：{e}"));
      }
    };
    {
      let (_, spec) = outputs["mic_spec_out"]
        .try_extract_tensor::<f32>()
        .map_err(|e| format!("读取 mic_spec_out 失败：{e}"))?;
      let (_, cache_out) = outputs["cache_out"]
        .try_extract_tensor::<f32>()
        .map_err(|e| format!("读取 cache_out 失败：{e}"))?;
      if spec.len() != SPEC_LEN || cache_out.len() != dim {
        return Err(format!(
          "AEC 输出维度异常：mic_spec_out={}，cache_out={}（期望 {SPEC_LEN} / {dim}）",
          spec.len(),
          cache_out.len()
        ));
      }
      spec_out.copy_from_slice(&spec[..SPEC_LEN]);
      cache.copy_from_slice(cache_out);
    }
    Ok(mic_stft.synthesize(&spec_out[..]))
  }
}
