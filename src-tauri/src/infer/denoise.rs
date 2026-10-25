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

use ort::value::TensorRef;

use crate::audio::HOP;

pub struct Denoise {
  /// 从模型池借来的会话（Drop 归还；同一模型多行/多次重建不重复加载）
  model: crate::infer::LeasedSession,
  dim: usize,
  cache: Vec<f32>,
  out: Vec<f32>,
}

impl Denoise {
  pub fn load(path: &Path) -> Result<Self, String> {
    let model = crate::infer::checkout(path)?;
    let dim = crate::infer::cache_dim(&model.lock(), "cache_in")?;
    Ok(Denoise {
      model,
      dim,
      cache: vec![0.0; dim],
      out: vec![0.0; HOP],
    })
  }

  /// 处理一个 480 样本 hop，返回增强后的 hop（借用内部缓冲）。
  ///
  /// **零分配**：输入张量借用 `hop` / `cache`（`TensorRef::from_array_view`），
  /// cache 输出拷回预分配缓冲；不再每跳 `to_vec()` + 重新分配 36506 个 f32。
  pub fn process(&mut self, hop: &[f32]) -> Result<&[f32], String> {
    debug_assert_eq!(hop.len(), HOP);
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
      let mix = TensorRef::from_array_view(([1i64, HOP as i64], hop))
        .map_err(|e| format!("构造 mix_hop 失败：{e}"))?;
      let cache_in = TensorRef::from_array_view(([1i64, dim as i64], cache.as_slice()))
        .map_err(|e| format!("构造 cache_in 失败：{e}"))?;
      session.run(ort::inputs!["mix_hop" => mix, "cache_in" => cache_in])
    };
    let outputs = match run {
      Ok(o) => o,
      Err(e) => {
        cache.fill(0.0);
        return Err(format!("推理失败：{e}"));
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
        "模型输出维度异常：enh_hop={}，cache_out={}（期望 {HOP} / {dim}）",
        enh.len(),
        cache_out.len()
      ));
    }
    out.copy_from_slice(&enh[..HOP]);
    cache.copy_from_slice(cache_out);
    Ok(out)
  }
}
