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

//! Windows：打开系统声音控制面板（`mmsys.cpl`，与旧实现 `pvplatform/system/_win.py` 一致）。
//!
//! 旧实现用 `ShellExecuteW` 是为了在无控制台进程里也不闪黑框；`control.exe` 本身是 GUI
//! 程序，`Command::spawn` 同样不会起控制台窗口，且不引入 unsafe 调用。

pub fn open() -> Result<(), String> {
  std::process::Command::new("control.exe")
    .arg("mmsys.cpl")
    .spawn()
    .map(|_| ())
    .map_err(|e| format!("打开声音控制面板失败：{e}"))
}
