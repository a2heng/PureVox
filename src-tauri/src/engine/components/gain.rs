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

//! 增益组件（DESIGN.md §4 处理行）：就地乘一个固定增益（dB）。不做限幅/AGC。

use crate::engine::stage::{FrameContext, Stage, StageError};

pub struct GainStage {
  gain: f32,
  db: f64,
}

impl GainStage {
  pub fn load(db: f64) -> Self {
    GainStage { gain: 10f32.powf((db / 20.0) as f32), db }
  }
}

impl Stage for GainStage {
  fn name(&self) -> &'static str {
    "gain"
  }

  fn status(&self) -> Option<String> {
    Some(format!("增益 {:+.1} dB", self.db))
  }

  fn process(&mut self, frame: &mut [f32], _ctx: &FrameContext) -> Result<(), StageError> {
    for x in frame.iter_mut() {
      *x *= self.gain;
    }
    Ok(())
  }
}
