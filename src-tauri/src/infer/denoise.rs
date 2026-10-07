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

//! 降噪模型流式推理（`purevox_denoise_202609` 契约，STFT 在模型图内）：
//!
//! ```text
//! 输入  mix_hop  [1,480]   波形 hop（10 ms @48 kHz）
//!       cache_in [1,D]     扁平流式缓存（首帧零起；D 由模型决定，如 36506）
//! 输出  enh_hop  [1,480]   增强波形 hop（滞后 1 hop = 10 ms，模型内 tail 语义）
//!       cache_out [1,D]
//! ```
//!
//! 从零缓存起步即与训练流式契约一致，无需静音预热。缓存维度从模型输入读，不写死。

use std::path::Path;

use ort::session::Session;
use ort::value::Tensor;

use crate::audio::HOP;

pub struct Denoise {
  session: Session,
  dim: usize,
  cache: Vec<f32>,
  out: Vec<f32>,
}

impl Denoise {
  pub fn load(path: &Path) -> Result<Self, String> {
    let session = super::build_session(path)?;
    let dim = super::cache_dim(&session, "cache_in")?;
    Ok(Denoise {
      session,
      dim,
      cache: vec![0.0; dim],
      out: vec![0.0; HOP],
    })
  }

  /// 处理一个 480 样本 hop，返回增强后的 hop（借用内部缓冲）。
  pub fn process(&mut self, hop: &[f32]) -> Result<&[f32], String> {
    debug_assert_eq!(hop.len(), HOP);
    let mix = Tensor::from_array(([1i64, HOP as i64], hop.to_vec()))
      .map_err(|e| format!("构造 mix_hop 失败：{e}"))?;
    let cache_in = std::mem::take(&mut self.cache);
    let cache = Tensor::from_array(([1i64, self.dim as i64], cache_in))
      .map_err(|e| format!("构造 cache_in 失败：{e}"))?;

    let outputs = match self
      .session
      .run(ort::inputs!["mix_hop" => mix, "cache_in" => cache])
    {
      Ok(o) => o,
      Err(e) => {
        self.cache = vec![0.0; self.dim];
        return Err(format!("推理失败：{e}"));
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
