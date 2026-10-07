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

//! Stage 契约（DESIGN.md §3.4）：一帧 = 480 样本（10 ms @48 kHz），就地处理。
//!
//! 契约骨架：`ts` / `StageError::Unavailable` / `name` / `reset` / `release` 由列工作线程与
//! 行级调试接入后使用，现在先按契约固定形状。
#![allow(dead_code)]

/// 一帧的处理上下文。当前无必需字段，固定签名（需要跨组件旁路信息时再扩展）。
#[derive(Clone, Copy, Debug, Default)]
pub struct FrameContext {
  /// 帧起始时刻（秒），AEC / 可视化旁路用；普通处理可忽略。
  pub ts: f64,
}

#[derive(Clone, Debug)]
pub enum StageError {
  /// 本帧跳过（直通），不视为故障。
  Unavailable(String),
  /// 故障：该列标记不可用并显示原因。
  Fatal(String),
}

impl std::fmt::Display for StageError {
  fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
    match self {
      StageError::Unavailable(m) => write!(f, "{m}"),
      StageError::Fatal(m) => write!(f, "{m}"),
    }
  }
}

pub trait Stage: Send {
  fn name(&self) -> &'static str;

  /// 是否参与本帧（默认始终参与）。
  fn accepts(&self, _ctx: &FrameContext) -> bool {
    true
  }

  /// 就地处理一帧（480 样本）。除非契约允许，长度不变。
  fn process(&mut self, frame: &mut [f32], ctx: &FrameContext) -> Result<(), StageError>;

  /// 流式状态复位（行/会话重启）。
  fn reset(&mut self) {}

  /// 供调试接口展示的运行状态（模型/参考/对齐等）；默认无。
  fn status(&self) -> Option<String> {
    None
  }

  /// 释放重资源（ONNX 会话引用等）；调用后本组件不再可用。
  fn release(&mut self) {}
}
