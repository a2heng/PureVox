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

//! 单应用音频采集：**仅 Windows**（WASAPI process loopback）。
//!
//! 其它平台明确报「不支持」，不静默降级到系统回环（那会采到整个系统的声音，
//! 用户以为选对了应用，实际错得很隐蔽）。

use super::capture::Capture;
use crate::debug::SharedHub;

pub fn spawn(_hub: SharedHub, _pid: u32, _tag: String) -> Result<Capture, String> {
  Err("单应用音频采集仅支持 Windows 10 2004+".into())
}

pub fn list_apps() -> Result<Vec<(u32, String)>, String> {
  Err("单应用音频采集仅支持 Windows 10 2004+".into())
}
