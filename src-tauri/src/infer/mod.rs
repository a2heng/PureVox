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

//! ONNX 推理（onnxruntime，`ort`）。模型契约见 `denoise.rs`。
//!
//! 模型是流式的：每 10 ms 一个 `*_hop`（480 样本）+ 一个大的一维 `cache_in`，
//! 返回处理后的 hop 与新的 `cache_out`；引擎只负责搬运波形与缓存，STFT 在模型图内。

pub mod aec;
pub mod denoise;
pub mod tse;

use std::path::{Path, PathBuf};

use ort::session::Session;

/// 统一 ONNX 会话：单线程推理 + 关闭自旋等待。
///
/// ort/onnxruntime 默认按核数建线程池并忙等（allow_spinning），会占满 CPU 并和音频回调抢核，
/// 实测把 1 ms 的推理拖到 100 ms。降噪 / AEC / TSE / 参考编码器都走这一条路径。
pub(crate) fn build_session(path: &Path) -> Result<Session, String> {
  Session::builder()
    .map_err(|e| format!("创建 onnxruntime 会话失败：{e}"))?
    .with_intra_threads(1)
    .map_err(|e| format!("设置推理线程数失败：{e}"))?
    .with_inter_threads(1)
    .map_err(|e| format!("设置并行线程数失败：{e}"))?
    .with_intra_op_spinning(false)
    .map_err(|e| format!("关闭自旋等待失败：{e}"))?
    .commit_from_file(path)
    .map_err(|e| format!("加载模型 {} 失败：{e}", path.display()))
}

/// 从会话输入里读某个一维缓存的元素个数（不写死维度）。
pub(crate) fn cache_dim(session: &Session, name: &str) -> Result<usize, String> {
  use ort::value::ValueType;
  session
    .inputs()
    .iter()
    .find(|i| i.name() == name)
    .and_then(|i| match i.dtype() {
      ValueType::Tensor { shape, .. } => Some(shape.num_elements()),
      _ => None,
    })
    .ok_or_else(|| format!("模型缺少 {name} 输入或不是张量"))
}

/// 现役降噪模型（models/ 下的文件名）。
pub const MODEL_DENOISE: &str = "purevox_denoise_202609c_ep0012.onnx";

/// 可选降噪模型 (文件名, 界面名)，供界面下拉（DESIGN.md §7）。
pub fn denoise_models() -> &'static [(&'static str, &'static str)] {
  &[
    (MODEL_DENOISE, "降噪 202609c（现役）"),
    ("purevox_denoise_202609b_ep0046.onnx", "降噪 202609b"),
    ("purevox_denoise_202609a_ep0278.onnx", "降噪 202609a"),
    ("purevox_denoise_202606_ep0014_op17.onnx", "降噪 202606"),
  ]
}

/// 解析模型文件路径。开发时模型在仓库 `models/`，打包后应在可执行文件旁/资源目录。
pub fn model_path(file: &str) -> Result<PathBuf, String> {
  let mut tried = Vec::new();
  let mut candidates: Vec<PathBuf> = Vec::new();
  if let Ok(dir) = std::env::var("PUREVOX_MODEL_DIR") {
    candidates.push(PathBuf::from(dir).join(file));
  }
  if let Ok(cwd) = std::env::current_dir() {
    candidates.push(cwd.join("models").join(file));
    candidates.push(cwd.join("..").join("models").join(file));
  }
  if let Ok(exe) = std::env::current_exe() {
    let dir = exe.parent().unwrap_or(Path::new("."));
    candidates.push(dir.join("models").join(file));
    // target/debug/purevox.exe → 仓库根/models
    if let Some(repo) = dir.ancestors().nth(3) {
      candidates.push(repo.join("models").join(file));
    }
    candidates.push(dir.join("resources").join("models").join(file));
  }
  for c in candidates {
    if c.is_file() {
      return Ok(c);
    }
    tried.push(c.display().to_string());
  }
  Err(format!(
    "找不到模型 {file}；可用 PUREVOX_MODEL_DIR 指定模型目录。已尝试：{}",
    tried.join("；")
  ))
}
