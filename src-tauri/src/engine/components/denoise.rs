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

//! 降噪组件：把 `infer::denoise::Denoise` 包成 `Stage`（DESIGN.md §3.4）。
//! 一帧 480 样本进，480 出；流式缓存由引擎实例持有。

use crate::engine::stage::{FrameContext, Stage, StageError};
use crate::infer;
use crate::infer::denoise::Denoise;

pub struct DenoiseStage {
  engine: Denoise,
}

impl DenoiseStage {
  pub fn load(model_file: &str) -> Result<Self, String> {
    let path = infer::model_path(model_file)?;
    Ok(DenoiseStage { engine: Denoise::load(&path)? })
  }
}

impl Stage for DenoiseStage {
  fn name(&self) -> &'static str {
    "denoise"
  }

  fn process(&mut self, frame: &mut [f32], _ctx: &FrameContext) -> Result<(), StageError> {
    let out = self.engine.process(frame).map_err(StageError::Fatal)?;
    frame.copy_from_slice(out);
    Ok(())
  }
}
