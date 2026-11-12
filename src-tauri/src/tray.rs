// PureVox — AI 麦克风降噪工具
// Copyright (C) 2024-2026 a2heng <752848283@qq.com>
//
// PureVox is licensed under the GNU General Public License v3.0 or
// later (GPL-3.0-or-later).  See LICENSE for details.
//
// SPDX-License-Identifier: GPL-3.0-or-later

//! 系统托盘（按平台拆实现，见 `CONTRIBUTING.md`）。
//!
//! 职责只有三件：显示 / 隐藏主窗口、启停引擎、退出；关闭和最小化都收进托盘。
//! Windows 用 `tray-icon`（Tauri 同源）+ 对主窗口做一次 WndProc 子类化拦截最小化；
//! 其它平台先给明确不可用的桩，不假装成功。

#[cfg(windows)]
#[path = "tray_windows.rs"]
mod imp;

#[cfg(target_os = "linux")]
#[path = "tray_linux.rs"]
mod imp;

#[cfg(not(any(windows, target_os = "linux")))]
#[path = "tray_other.rs"]
mod imp;

pub use imp::{hide_main, quitting, spawn};
