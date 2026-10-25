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

//! AEC 回声消除流式推理（`purevox_aec_202610` 契约，STFT 在模型图内）：
//!
//! ```text
//! 输入  mic_hop  [1,480]   近端（本机麦克风）hop
//!       far_hop  [1,480]   远端（扬声器回环 / 参考）hop，时间对齐后
//!       cache_in [1,D]     扁平流式缓存（首帧零起；D 由模型决定，如 91920）
//! 输出  enh_hop  [1,480]（滞后 1 hop）+ cache_out [1,D]
//! ```
//!
//! far 的对齐/延时由 [`crate::engine::aec::AecRow`] 负责；本文件只做无状态会话 + 每行缓存。

use ort::value::TensorRef;

use crate::audio::HOP;

/// 现役 AEC 模型（202610 单流：far/mic 各 5ch 在第 0 层 early fusion，无显式延迟线；
/// 对齐靠 GRU 隐状态，前端感受野 ≥ 数据最大延迟）。
pub const MODEL_AEC: &str = "purevox_aec_202610_ep0372.onnx";

pub fn aec_models() -> &'static [(&'static str, &'static str)] {
  &[(MODEL_AEC, "AEC 202610（现役）")]
}

pub struct Aec {
  model: crate::infer::LeasedSession,
  dim: usize,
  cache: Vec<f32>,
  out: Vec<f32>,
}

impl Aec {
  pub fn load(model_file: &str) -> Result<Self, String> {
    let path = crate::infer::model_path(model_file)?;
    let model = crate::infer::checkout(&path)?;
    let dim = crate::infer::cache_dim(&model.lock(), "cache_in")?;
    Ok(Aec {
      model,
      dim,
      cache: vec![0.0; dim],
      out: vec![0.0; HOP],
    })
  }

  /// 处理一对对齐好的 hop（mic + far），返回增强后的 hop（借用内部缓冲）。
  ///
  /// **零分配**：mic/far/cache 都是借用张量，cache 输出拷回预分配缓冲。
  pub fn process(&mut self, mic: &[f32], far: &[f32]) -> Result<&[f32], String> {
    debug_assert_eq!(mic.len(), HOP);
    debug_assert_eq!(far.len(), HOP);
    let Self {
      model,
      dim,
      cache,
      out,
    } = self;
    let dim = *dim;
    let mut guard = model.lock();
    let session = &mut *guard;

    let run = {
      let mic_t = TensorRef::from_array_view(([1i64, HOP as i64], mic))
        .map_err(|e| format!("构造 mic_hop 失败：{e}"))?;
      let far_t = TensorRef::from_array_view(([1i64, HOP as i64], far))
        .map_err(|e| format!("构造 far_hop 失败：{e}"))?;
      let cache_in = TensorRef::from_array_view(([1i64, dim as i64], cache.as_slice()))
        .map_err(|e| format!("构造 cache_in 失败：{e}"))?;
      session.run(ort::inputs![
        "mic_hop" => mic_t, "far_hop" => far_t, "cache_in" => cache_in
      ])
    };
    let outputs = match run {
      Ok(o) => o,
      Err(e) => {
        cache.fill(0.0);
        return Err(format!("AEC 推理失败：{e}"));
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
        "AEC 输出维度异常：enh_hop={}，cache_out={}（期望 {HOP} / {dim}）",
        enh.len(),
        cache_out.len()
      ));
    }
    out.copy_from_slice(&enh[..HOP]);
    cache.copy_from_slice(cache_out);
    Ok(out)
  }
}
