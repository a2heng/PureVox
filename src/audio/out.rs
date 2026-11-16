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

/// 采样格式（共享混音格式 / cpal 默认格式的完整集合）。
///
/// WASAPI 共享混音只会是 PCM（8/16/24/32）或 IEEE float（32/64）；
/// 24-bit 有两种落盘方式：3 字节紧凑（`I24Packed`）和 32 位容器里高 24 位有效
/// （`I24In32`）。两者分开，写缓冲时才不会把低位当有效数据。
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Sample {
  U8,
  I8,
  I16,
  U16,
  /// 3 字节紧凑 24-bit（WASAPI 直出时可能出现）。
  I24Packed,
  /// 32 位容器、有效 24 位（WASAPI 混音 / cpal 的 I24；数值在高 24 位）。
  I24In32,
  U24,
  I32,
  U32,
  I64,
  U64,
  F32,
  F64,
}

impl Sample {
  /// 容器位宽（显示 / 建 `WAVEFORMATEX` 用；I24In32 是 32）。
  pub fn container_bits(self) -> u16 {
    match self {
      Sample::U8 | Sample::I8 => 8,
      Sample::I16 | Sample::U16 => 16,
      Sample::I24Packed | Sample::U24 => 24,
      Sample::I24In32 | Sample::I32 | Sample::U32 | Sample::F32 => 32,
      Sample::I64 | Sample::U64 | Sample::F64 => 64,
    }
  }

  pub fn name(self) -> &'static str {
    match self {
      Sample::U8 => "u8",
      Sample::I8 => "i8",
      Sample::I16 => "i16",
      Sample::U16 => "u16",
      Sample::I24Packed => "i24",
      Sample::I24In32 => "i24(32)",
      Sample::U24 => "u24(32)",
      Sample::I32 => "i32",
      Sample::U32 => "u32",
      Sample::I64 => "i64",
      Sample::U64 => "u64",
      Sample::F32 => "f32",
      Sample::F64 => "f64",
    }
  }
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
  /// 混音格式不认识时用 f32 + `AUTOCONVERTPCM`，由 Windows 音频引擎转换。
  pub engine_convert: bool,
}

impl Format {
  pub fn period_ms(&self) -> f64 {
    self.period_frames as f64 * 1000.0 / self.rate.max(1) as f64
  }

  pub fn sample_text(&self) -> String {
    format!(
      "{} {}-bit {} ch @ {} Hz{}",
      self.sample.name(),
      self.bits,
      self.channels,
      self.rate,
      if self.engine_convert {
        "（引擎转换）"
      } else {
        ""
      }
    )
  }
}

impl std::fmt::Display for Format {
  fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
    write!(
      f,
      "{}, 回调周期 {:.2} ms（{} 帧{}{}）",
      self.sample_text(),
      self.period_ms(),
      self.period_frames,
      if self.low_latency {
        "，低延迟共享"
      } else {
        ""
      },
      if self.engine_convert {
        "，引擎转换"
      } else {
        ""
      }
    )
  }
}

#[cfg(all(not(windows), not(target_os = "linux")))]
pub use super::cpal_out::{Stream, probe, start};
#[cfg(target_os = "linux")]
pub use super::linux_out::{Stream, probe, start};
#[cfg(windows)]
pub use super::wasapi_out::{Stream, probe, start};
