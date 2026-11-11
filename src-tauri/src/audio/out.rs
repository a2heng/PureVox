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

//! 输出后端的选择层（**按平台分文件**）：`playback` 只认这一层的接口。
//!
//! - Windows：`wasapi_out`（直连 WASAPI，`IAudioClient3` 低延迟共享优先）；
//! - 其它：`cpal_out`（cpal 默认输出流，行为与改造前一致）。
//!
//! 两个后端都只提供三样东西：[`Format`]（设备格式 + 回调周期）、[`probe`]（打开前问一次）、
//! [`start`]（建流并开始回调）。

/// 采样格式（共享混音格式实际只有这几种）。
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Sample {
  F32,
  I16,
  I32,
  /// cpal 后端（非 Windows）才可能返回；WASAPI 混音格式不含无符号 16 bit。
  #[allow(dead_code)]
  U16,
}

/// 输出端点格式 + 周期能力（打开设备前先问一次，用来建环和选转换器）。
#[derive(Clone, Debug)]
pub struct Format {
  pub rate: u32,
  pub channels: u16,
  pub bits: u16,
  pub sample: Sample,
  /// 实际会用的回调周期（帧）。`IAudioClient3` 支持时是端点 `min`，否则是引擎默认。
  pub period_frames: u32,
  /// 是否用上了低延迟共享模式（`IAudioClient3`）。
  pub low_latency: bool,
}

impl Format {
  pub fn period_ms(&self) -> f64 {
    self.period_frames as f64 * 1000.0 / self.rate.max(1) as f64
  }

  pub fn sample_text(&self) -> String {
    let kind = match self.sample {
      Sample::F32 => "f32",
      Sample::I16 => "i16",
      Sample::I32 => "i32",
      Sample::U16 => "u16",
    };
    format!(
      "{kind} {}-bit {} ch @ {} Hz",
      self.bits, self.channels, self.rate
    )
  }
}

impl std::fmt::Display for Format {
  fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
    write!(
      f,
      "{}, 回调周期 {:.2} ms（{} 帧{}）",
      self.sample_text(),
      self.period_ms(),
      self.period_frames,
      if self.low_latency {
        "，低延迟共享"
      } else {
        ""
      }
    )
  }
}

#[cfg(not(windows))]
pub use super::cpal_out::{Stream, probe, start};
#[cfg(windows)]
pub use super::wasapi_out::{Stream, probe, start};
