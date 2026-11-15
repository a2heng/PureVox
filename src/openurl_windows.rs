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

//! Windows：用系统默认浏览器打开链接（`cmd /C start`）；`reveal` 在资源管理器里定位文件。

use std::path::Path;

pub fn open(url: &str) -> Result<(), String> {
  std::process::Command::new("cmd")
    .args(["/C", "start", "", url])
    .spawn()
    .map(|_| ())
    .map_err(|e| format!("打开链接失败：{e}"))
}

/// 在资源管理器里打开 `path` 所在目录并选中它（`explorer /select,<path>`）。
/// 文件不存在时资源管理器仍会打开父目录，所以调用方不必先判断存在。
pub fn reveal(path: &Path) -> Result<(), String> {
  std::process::Command::new("explorer")
    .arg(format!("/select,{}", path.display()))
    .spawn()
    .map(|_| ())
    .map_err(|e| format!("打开文件夹失败：{e}"))
}
