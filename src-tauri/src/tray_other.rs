// PureVox — AI 麦克风降噪工具
// Copyright (C) 2024-2026 a2heng <752848283@qq.com>
//
// PureVox is licensed under the GNU General Public License v3.0 or
// later (GPL-3.0-or-later).  See LICENSE for details.
//
// SPDX-License-Identifier: GPL-3.0-or-later

//! 非 Windows 的托盘桩：明确不可用，关闭窗口就正常退出。

use crate::debug::SharedHub;

pub fn spawn(_hwnd: isize, _hub: SharedHub) {}

/// 没有托盘可收，返回 false 让窗口正常关闭。
pub fn hide_main() -> bool {
  false
}

pub fn quitting() -> bool {
  false
}

/// 没有托盘实现：保持同一套调用点（Windows 按引擎状态换明暗图标）。
pub fn set_running(_on: bool) {}
