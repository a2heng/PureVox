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

//! 处理链（DESIGN.md §3.2）：按顺序对同一帧就地处理；只此一处实现链式调用。
//!
//! `reset` / `release` 由列工作线程的行/会话重启路径调用（后续步骤）。
#![allow(dead_code)]

use super::stage::{FrameContext, Stage, StageError};

pub struct Pipeline {
  stages: Vec<Box<dyn Stage>>,
}

impl Pipeline {
  pub fn new(stages: Vec<Box<dyn Stage>>) -> Self {
    Pipeline { stages }
  }

  pub fn process(&mut self, frame: &mut [f32], ctx: &FrameContext) -> Result<(), StageError> {
    for s in &mut self.stages {
      if !s.accepts(ctx) {
        continue;
      }
      match s.process(frame, ctx) {
        Ok(()) => {}
        // Unavailable = 本帧跳过（该阶段未改动帧，等同直通）
        Err(StageError::Unavailable(_)) => {}
        Err(e) => return Err(e),
      }
    }
    Ok(())
  }

  pub fn reset(&mut self) {
    for s in &mut self.stages {
      s.reset();
    }
  }

  pub fn release(&mut self) {
    for s in &mut self.stages {
      s.release();
    }
  }
}
