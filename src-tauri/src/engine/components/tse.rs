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

//! TSE 组件：把 `infer::tse::Tse` 包成 `Stage`（ARCHITECTURE.md §3.4、§7）。
//! 参考语音在加载时一次性编码；没有参考则直通（状态经 `Stage::status` 进调试接口）。

use std::path::Path;

use crate::engine::stage::{FrameContext, Stage, StageError};
use crate::infer::tse::Tse;

pub struct TseStage {
  engine: Tse,
  status: String,
}

impl TseStage {
  /// 参考语音固定为 `~/.purevox/tse_reference.wav`（界面不给路径设置，见 ARCHITECTURE.md §7）。
  pub fn load(model_file: &str) -> Result<Self, String> {
    let mut engine = Tse::load(model_file)?;
    let path = crate::config::default_tse_reference();
    let status = match load_reference(&mut engine, &path) {
      Ok(secs) => format!("参考已加载（{secs:.1} s）"),
      Err(e) => format!("未设置参考（直通）：{e}"),
    };
    Ok(TseStage { engine, status })
  }
}

fn load_reference(engine: &mut Tse, path: &Path) -> Result<f64, String> {
  let (samples, rate) = crate::wav::read_mono(path)?;
  if rate != crate::audio::SAMPLE_RATE {
    return Err(format!(
      "参考语音须为 48 kHz（当前 {rate} Hz）：{}",
      path.display()
    ));
  }
  let secs = samples.len() as f64 / rate as f64;
  engine.set_reference(&samples)?;
  Ok(secs)
}

impl Stage for TseStage {
  fn name(&self) -> &'static str {
    "tse"
  }

  fn status(&self) -> Option<String> {
    Some(self.status.clone())
  }

  fn process(&mut self, frame: &mut [f32], _ctx: &FrameContext) -> Result<(), StageError> {
    let out = self.engine.process(frame).map_err(StageError::Fatal)?;
    frame.copy_from_slice(out);
    Ok(())
  }

  /// 行/会话重启：模型缓存与流式 DSP（拼帧 / OLA 尾巴）一起归零。
  /// 参考 token 是加载时算好的，**不**随之丢弃。
  fn reset(&mut self) {
    self.engine.reset();
  }
}
