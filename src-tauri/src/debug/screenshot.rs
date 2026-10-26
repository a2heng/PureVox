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

//! 内置 UI 截屏（开发/报障工具，不是音频链路的一部分）。
//!
//! 目的：迁移期界面迭代需要“看得见”的反馈；系统级截图在 Wayland/GNOME 50 上
//! 被 `Shell.Screenshot` 禁用，`wlr-screencopy` 也不支持。这里直接调用 WebKitGTK 的
//! `webkit_web_view_get_snapshot`，截的是**应用内容本身**（不含窗口边框），跨显示服务器稳定。
//!
//! 平台实现按文件分（AGENTS §4）：
//! - `screenshot_linux.rs`：WebKitGTK `webkit_web_view_get_snapshot`（内容本身）；
//! - `screenshot_windows.rs`：`PrintWindow(PW_RENDERFULLCONTENT)` 截客户区（WebView2 内容）；
//! - `screenshot_other.rs`：明确不可用（给出原因）。
//!
//! Windows 上窗口必须**放得下**才有意义（200% 缩放下 1560 逻辑 px = 3120 物理 px，
//! 比屏幕还宽 → 只能截到左边一块），所以启动时 `main.rs` 会把窗口夹进工作区。
//! 入口：
//! - Tauri 命令 `capture_ui`（调试面板「截屏」按钮）；
//! - 调试 HTTP `GET /debug/ui.png`（给我们/脚本直接抓图，见 AGENTS §1.2）。
//!
//! 默认落到 `~/.purevox/screenshots/ui.png`（每次覆盖，方便脚本固定路径读取）。

use std::path::{Path, PathBuf};
use std::sync::OnceLock;

#[cfg(target_os = "linux")]
#[path = "screenshot_linux.rs"]
mod imp;
#[cfg(windows)]
#[path = "screenshot_windows.rs"]
mod imp;
#[cfg(not(any(target_os = "linux", windows)))]
#[path = "screenshot_other.rs"]
mod imp;

pub(crate) static APP: OnceLock<tauri::AppHandle> = OnceLock::new();

/// 在 `setup` 里登记 AppHandle（截屏必须回到主线程拿 WebView）。
pub fn init(app: tauri::AppHandle) {
  let _ = APP.set(app);
}

/// 截屏的固定落点（`~/.purevox/screenshots/ui.png`）。
pub fn default_path() -> Result<PathBuf, String> {
  let dir = crate::config::config_dir()?.join("screenshots");
  std::fs::create_dir_all(&dir).map_err(|e| format!("创建截图目录失败：{e}"))?;
  Ok(dir.join("ui.png"))
}

pub fn capture_to(path: &Path) -> Result<(), String> {
  imp::capture_to(path)
}

/// 返回截图文件的字节（HTTP `/debug/ui.png` 用；同一份实现）。
pub fn capture() -> Result<PathBuf, String> {
  let path = default_path()?;
  capture_to(&path)?;
  Ok(path)
}

pub fn capture_png() -> Result<Vec<u8>, String> {
  let path = default_path()?;
  capture_to(&path)?;
  std::fs::read(&path).map_err(|e| format!("读取截图失败：{e}"))
}
