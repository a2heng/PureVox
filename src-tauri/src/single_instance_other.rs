// PureVox — AI 麦克风降噪工具
// Copyright (C) 2024-2026 a2heng <752848283@qq.com>
//
// PureVox is licensed under the GNU General Public License v3.0 or
// later (GPL-3.0-or-later).  See LICENSE for details.
//
// SPDX-License-Identifier: GPL-3.0-or-later

//! 其它平台：没有可靠的单实例原语，明确放行（不假装成功）。

pub fn claim() -> bool {
  true
}

pub fn activate_message() -> u32 {
  0
}
