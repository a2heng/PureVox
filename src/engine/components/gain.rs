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

//! 增益组件（ARCHITECTURE.md §4 处理行）：就地乘一个增益（dB）。不做限幅/AGC。
//!
//! **热参数**：增益单元格是 [`HotGain`]（`engine/hot.rs`），[`Stage::process`] 每 hop 读一次
//! —— 界面拖动增益**不重建会话**，下一 hop 就是新值。

use std::sync::Arc;

use crate::engine::hot::HotGain;
use crate::engine::stage::{FrameContext, Stage, StageError};

pub struct GainStage {
  gain: Arc<HotGain>,
}

impl GainStage {
  /// 用会话里登记过的热参数单元格建（同一份单元格也会发给界面改值用）。
  pub fn new(gain: Arc<HotGain>) -> Self {
    GainStage { gain }
  }
}

impl Stage for GainStage {
  fn name(&self) -> &'static str {
    "gain"
  }

  fn status(&self) -> Option<String> {
    Some(format!("增益 {:+.1} dB", self.gain.db()))
  }

  fn process(&mut self, frame: &mut [f32], _ctx: &FrameContext) -> Result<(), StageError> {
    let k = self.gain.linear();
    for x in frame.iter_mut() {
      *x *= k;
    }
    Ok(())
  }
}
