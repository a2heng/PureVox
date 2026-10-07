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
use ort::value::{Tensor, ValueType};

use crate::audio::HOP;

pub struct Denoise {
  session: Session,
  model: String,
  dim: usize,
  cache: Vec<f32>,
  out: Vec<f32>,
  /// 最近一次推理耗时（ms），供调试接口显示
  last_ms: f64,
}

impl Denoise {
  pub fn load(path: &Path) -> Result<Self, String> {
    // 实时音频场景：单线程推理且**关闭自旋等待**。ort/onnxruntime 默认按核数建线程池并
    // 忙等（allow_spinning），会占满 CPU 并和音频回调线程抢核，实测把 1 ms 的推理拖到 100 ms。
    let session = Session::builder()
      .map_err(|e| format!("创建 onnxruntime 会话失败：{e}"))?
      .with_intra_threads(1)
      .map_err(|e| format!("设置推理线程数失败：{e}"))?
      .with_inter_threads(1)
      .map_err(|e| format!("设置并行线程数失败：{e}"))?
      .with_intra_op_spinning(false)
      .map_err(|e| format!("关闭自旋等待失败：{e}"))?
      .commit_from_file(path)
      .map_err(|e| format!("加载模型 {} 失败：{e}", path.display()))?;
    let dim = session
      .inputs()
      .iter()
      .find(|i| i.name() == "cache_in")
      .and_then(|i| match i.dtype() {
        ValueType::Tensor { shape, .. } => Some(shape.num_elements()),
        _ => None,
      })
      .ok_or_else(|| "模型缺少 cache_in 输入或不是张量".to_string())?;
    let model = path.file_name().map(|s| s.to_string_lossy().into_owned()).unwrap_or_default();
    Ok(Denoise { session, model, dim, cache: vec![0.0; dim], out: vec![0.0; HOP], last_ms: 0.0 })
  }

  pub fn model_name(&self) -> &str {
    &self.model
  }

  pub fn last_ms(&self) -> f64 {
    self.last_ms
  }

  /// 处理一个 480 样本 hop，返回增强后的 hop（借用内部缓冲）。
  pub fn process(&mut self, hop: &[f32]) -> Result<&[f32], String> {
    debug_assert_eq!(hop.len(), HOP);
    let __t = std::time::Instant::now();
    let mix = Tensor::from_array(([1i64, HOP as i64], hop.to_vec()))
      .map_err(|e| format!("构造 mix_hop 失败：{e}"))?;
    let cache_in = std::mem::take(&mut self.cache);
    let cache = Tensor::from_array(([1i64, self.dim as i64], cache_in))
      .map_err(|e| format!("构造 cache_in 失败：{e}"))?;

    let outputs = match self.session.run(ort::inputs!["mix_hop" => mix, "cache_in" => cache]) {
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
    self.last_ms = __t.elapsed().as_secs_f64() * 1000.0;
    Ok(&self.out)
  }
}
