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

//! Windows：虚拟麦克风**没有** Rust 实现 —— Windows 用 VB-CABLE 虚拟声卡，
//! 走界面顶栏「驱动」页（`ui/drivers_windows.js`：驱动下载 / 视频教程 / 有无检测）。
//! 这里只给明确的不可用原因（不静默）。

const MSG: &str = "虚拟麦克风只实现了 Linux（PipeWire）；Windows 请在「驱动」页安装 VB-CABLE";

pub fn status() -> Result<super::Status, String> {
  Err(MSG.into())
}

pub fn create() -> Result<super::Status, String> {
  Err(MSG.into())
}

pub fn remove() -> Result<super::Status, String> {
  Err(MSG.into())
}
