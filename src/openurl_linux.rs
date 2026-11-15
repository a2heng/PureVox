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

//! Linux：用系统默认浏览器打开链接（`xdg-open`）；`reveal` 打开文件所在目录。

use std::path::Path;

pub fn open(url: &str) -> Result<(), String> {
  std::process::Command::new("xdg-open")
    .arg(url)
    .spawn()
    .map(|_| ())
    .map_err(|e| format!("打开链接失败：{e}"))
}

/// 用文件管理器打开 `path` 所在目录（`xdg-open <dir>`；桌面环境没有「定位并选中」的
/// 通用命令，打开父目录已经满足「找到参考语音」的用途）。
pub fn reveal(path: &Path) -> Result<(), String> {
  let dir = path.parent().unwrap_or(path);
  std::process::Command::new("xdg-open")
    .arg(dir)
    .spawn()
    .map(|_| ())
    .map_err(|e| format!("打开文件夹失败：{e}"))
}
