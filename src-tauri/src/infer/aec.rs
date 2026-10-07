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

//! AEC 回声消除流式推理（`purevox_aec_202609_cpx` 契约，STFT 在模型图内）：
//!
//! ```text
//! 输入  mic_hop  [1,480]   近端（本机麦克风）hop
//!       far_hop  [1,480]   远端（扬声器回环 / 参考）hop，时间对齐后
//!       cache_in [1,D]     扁平流式缓存（首帧零起；D 由模型决定，如 215504）
//! 输出  enh_hop  [1,480]（滞后 1 hop）+ cache_out [1,D]
//! ```
//!
//! far 的对齐/延时由 [`crate::engine::aec::AecRow`] 负责；本文件只做无状态会话 + 每行缓存。

use ort::session::Session;
use ort::value::Tensor;

use crate::audio::HOP;

/// 现役 AEC 模型。
pub const MODEL_AEC: &str = "purevox_aec_202609_cpx_ep0375.onnx";

pub fn aec_models() -> &'static [(&'static str, &'static str)] {
  &[(MODEL_AEC, "AEC 202609（现役）")]
}

pub struct Aec {
  session: Session,
  dim: usize,
  cache: Vec<f32>,
  out: Vec<f32>,
}

impl Aec {
  pub fn load(model_file: &str) -> Result<Self, String> {
    let path = crate::infer::model_path(model_file)?;
    let session = crate::infer::build_session(&path)?;
    let dim = crate::infer::cache_dim(&session, "cache_in")?;
    Ok(Aec {
      session,
      dim,
      cache: vec![0.0; dim],
      out: vec![0.0; HOP],
    })
  }

  /// 处理一对对齐好的 hop（mic + far），返回增强后的 hop（借用内部缓冲）。
  pub fn process(&mut self, mic: &[f32], far: &[f32]) -> Result<&[f32], String> {
    debug_assert_eq!(mic.len(), HOP);
    debug_assert_eq!(far.len(), HOP);
    let mic = Tensor::from_array(([1i64, HOP as i64], mic.to_vec()))
      .map_err(|e| format!("构造 mic_hop 失败：{e}"))?;
    let far = Tensor::from_array(([1i64, HOP as i64], far.to_vec()))
      .map_err(|e| format!("构造 far_hop 失败：{e}"))?;
    let cache_in = std::mem::take(&mut self.cache);
    let cache = Tensor::from_array(([1i64, self.dim as i64], cache_in))
      .map_err(|e| format!("构造 cache_in 失败：{e}"))?;

    let outputs = match self.session.run(ort::inputs![
      "mic_hop" => mic, "far_hop" => far, "cache_in" => cache
    ]) {
      Ok(o) => o,
      Err(e) => {
        self.cache = vec![0.0; self.dim];
        return Err(format!("AEC 推理失败：{e}"));
      }
    };
    let (_, enh) = outputs["enh_hop"]
      .try_extract_tensor::<f32>()
      .map_err(|e| format!("读取 enh_hop 失败：{e}"))?;
    let (_, cache_out) = outputs["cache_out"]
      .try_extract_tensor::<f32>()
      .map_err(|e| format!("读取 cache_out 失败：{e}"))?;

    self.out.clear();
    self.out.extend_from_slice(&enh[..HOP]);
    self.cache.clear();
    self.cache.extend_from_slice(cache_out);
    Ok(&self.out)
  }
}
