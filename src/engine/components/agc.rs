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

//! 自动增益（AGC）：用 [`dagc`]（MIT/Apache-2.0，论文实现）处理 10 ms 帧。
//!
//! 参数（全是热参数，运行中改不重建会话）：
//! - `target_db`：目标输出 RMS（dBFS）；
//! - `speed`：跟随速度（dagc 的 distortion factor，越小越稳、越大越快）；
//! - `floor_db`：低于这个电平就**冻结增益**（不放大人声间隙里的底噪）。
//!
//! 不自己写控制律：增益估计 / 平滑 / 应用全部由 `dagc::MonoAgc` 完成。

use std::sync::Arc;

use dagc::MonoAgc;

use crate::engine::hot::HotGain;
use crate::engine::stage::{FrameContext, Stage, StageError};

pub struct AgcStage {
  target: Arc<HotGain>,
  speed: Arc<HotGain>,
  floor: Arc<HotGain>,
  agc: MonoAgc,
  last: (f32, f32, f32),
  gain_db: f32,
}

impl AgcStage {
  pub fn new(
    target: Arc<HotGain>,
    speed: Arc<HotGain>,
    floor: Arc<HotGain>,
  ) -> Result<Self, String> {
    let agc = Self::build(&target, &speed)?;
    Ok(Self {
      last: (target.db(), speed.db(), floor.db()),
      target,
      speed,
      floor,
      agc,
      gain_db: 0.0,
    })
  }

  fn build(target: &HotGain, speed: &HotGain) -> Result<MonoAgc, String> {
    // **注意 dagc 的 `desired_output_rms` 实际是「均方值」**：它的控制律是
    // `y = x² / desired`、增益收敛到 y≈1，所以这里要传 10^(dB/10)（功率），
    // 不是 10^(dB/20)（幅度）。传幅度会让目标响 20 dB（用户实测：设 -30 实际 -10）。
    let power = 10f32.powf(target.db() / 10.0).clamp(1e-8, 1.0);
    // distortion factor：太大（>0.01）会有明显失真，太小则跟得慢
    MonoAgc::new(power, speed.db().clamp(1e-5, 0.02)).map_err(|e| format!("AGC 初始化失败：{e}"))
  }

  fn refresh(&mut self) -> Result<(), StageError> {
    let now = (self.target.db(), self.speed.db(), self.floor.db());
    if now.0 != self.last.0 || now.1 != self.last.1 {
      self.agc = Self::build(&self.target, &self.speed).map_err(StageError::Fatal)?;
      self.last.0 = now.0;
      self.last.1 = now.1;
    }
    self.last.2 = now.2;
    Ok(())
  }
}

impl Stage for AgcStage {
  fn name(&self) -> &'static str {
    "agc"
  }

  fn process(&mut self, frame: &mut [f32], _ctx: &FrameContext) -> Result<(), StageError> {
    self.refresh()?;
    // 低于底噪（人声间隙）冻结增益，避免把房间噪声放大
    let floor_db = self.floor.db();
    let rms = if frame.is_empty() {
      0.0
    } else {
      (frame.iter().map(|x| (*x as f64) * (*x as f64)).sum::<f64>() / frame.len() as f64).sqrt()
        as f32
    };
    let level_db = if rms <= 1e-9 {
      -160.0
    } else {
      20.0 * rms.log10()
    };
    self.agc.freeze_gain(level_db < floor_db);
    self.agc.process(frame);
    let g = self.agc.gain().max(1e-9);
    self.gain_db = 20.0 * g.log10();
    Ok(())
  }

  fn reset(&mut self) {
    if let Ok(agc) = Self::build(&self.target, &self.speed) {
      self.agc = agc;
    }
    self.gain_db = 0.0;
  }

  fn status(&self) -> Option<String> {
    Some(format!(
      "增益 {:+.1} dB（目标 {:.0} dBFS{}）",
      self.gain_db,
      self.target.db(),
      if self.agc.is_gain_frozen() {
        "，底噪以下已冻结"
      } else {
        ""
      }
    ))
  }
}

#[cfg(test)]
mod tests {
  use super::*;

  fn stage() -> AgcStage {
    AgcStage::new(
      Arc::new(HotGain::new(-20.0)),
      Arc::new(HotGain::new(0.001)),
      Arc::new(HotGain::new(-55.0)),
    )
    .expect("AGC 应能建起来")
  }

  #[test]
  fn quiet_signal_gets_boosted() {
    let mut s = stage();
    let mut frame = [0.01f32; 480]; // -40 dBFS
    let src = frame;
    for _ in 0..40 {
      frame = src;
      s.process(&mut frame, &FrameContext::default()).unwrap();
    }
    let rms = |v: &[f32]| (v.iter().map(|x| x * x).sum::<f32>() / v.len() as f32).sqrt();
    assert!(rms(&frame) > rms(&src) * 1.5, "小信号应被抬高");
    assert!(frame.iter().all(|x| x.is_finite()));
  }

  #[test]
  fn converges_towards_the_target_rms() {
    let mut s = stage();
    // -40 dBFS 的正弦，目标 -20 dBFS：收敛后 RMS 应落在 -23..-17 dBFS
    let mut frame = [0.0f32; 480];
    let mut out = [0.0f32; 480];
    for _ in 0..600 {
      for (i, x) in frame.iter_mut().enumerate() {
        *x = 0.01 * (2.0 * std::f32::consts::PI * 440.0 * i as f32 / 48_000.0).sin();
      }
      out = frame;
      s.process(&mut out, &FrameContext::default()).unwrap();
    }
    let rms = (out.iter().map(|x| x * x).sum::<f32>() / out.len() as f32).sqrt();
    let db = 20.0 * rms.log10();
    assert!(
      (-23.0..-17.0).contains(&db),
      "收敛后的电平应≈-20 dBFS，实际 {db:.1}"
    );
  }

  #[test]
  fn silence_does_not_blow_up() {
    let mut s = stage();
    let mut frame = [0.0f32; 480];
    s.process(&mut frame, &FrameContext::default()).unwrap();
    assert!(frame.iter().all(|x| x.is_finite()));
    assert!(s.agc.is_gain_frozen(), "静音应在底噪以下冻结增益");
  }
}
