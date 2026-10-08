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

//! 构建版本号（**版本号 = 日期**）。
//!
//! 打包/发版时由 `tools/automation/version.{sh,ps1}` 从 tag（`v<yyyy.MM.dd.HHmm>`）
//! 或当前 UTC 时间推导，注入 `PUREVOX_BUILD_VERSION` 环境变量（并用 `--config`
//! 覆盖 Tauri 包版本，见 `src-tauri/.build-version.json`）。开发态没注入时回退
//! Cargo 包版本，保证任何构建都有版本号可显示。窗口标题与调试面板「版本」同源。

/// 构建版本：CI 注入的 `PUREVOX_BUILD_VERSION`（tag / 日期），否则 Cargo 包版本。
pub const VERSION: &str = match option_env!("PUREVOX_BUILD_VERSION") {
  Some(v) => v,
  None => env!("CARGO_PKG_VERSION"),
};
