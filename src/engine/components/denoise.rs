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

//! 降噪组件：把 `infer::denoise::Denoise` 包成 `Stage`（ARCHITECTURE.md §3.4）。
//! 一帧 480 样本进，480 出；流式缓存由引擎实例持有。
//!
//! **降噪深度**是热参数：单元格在行建立时登记（`engine/hot.rs`），`process` 每 hop 读一次，
//! 界面拖动滑块**不重建会话**（Linux 上重建 = 几百毫秒 + 排他重开设备）。

use std::sync::Arc;

use crate::engine::hot::HotGain;
use crate::engine::stage::{FrameContext, Stage, StageError};
use crate::infer;
use crate::infer::denoise::Denoise;

pub struct DenoiseStage {
  engine: Denoise,
  /// 降噪深度（掩码最深衰减 dB）：与界面滑块的原子单元格共享
  depth: Arc<HotGain>,
}

impl DenoiseStage {
  pub fn load(model_file: &str, depth: Arc<HotGain>) -> Result<Self, String> {
    let path = infer::model_path(model_file)?;
    Ok(DenoiseStage {
      engine: Denoise::load(&path)?,
      depth,
    })
  }
}

impl Stage for DenoiseStage {
  fn name(&self) -> &'static str {
    "denoise"
  }

  fn process(&mut self, frame: &mut [f32], _ctx: &FrameContext) -> Result<(), StageError> {
    let depth = self.depth.db();
    let out = self
      .engine
      .process(frame, depth)
      .map_err(StageError::Fatal)?;
    frame.copy_from_slice(out);
    Ok(())
  }

  /// 行/会话重启：模型缓存与流式 DSP（拼帧 / OLA 尾巴）一起归零。
  fn reset(&mut self) {
    self.engine.reset();
  }

  /// 频谱合成固定滞后 1 hop（见 `infer/spec.rs`）。
  fn latency_samples(&self) -> usize {
    crate::audio::HOP
  }
}
