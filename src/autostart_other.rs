// PureVox — AI 麦克风降噪工具
// Copyright (C) 2024-2026 a2heng <752848283@qq.com>
//
// PureVox is licensed under the GNU General Public License v3.0 or
// later (GPL-3.0-or-later).  See LICENSE for details.
// SPDX-License-Identifier: GPL-3.0-or-later

//! 开机自启（其它平台）：明确不可用，不伪造成功。

pub fn set(_on: bool) -> Result<(), String> {
  Err(format!(
    "{} 上没有实现开机自启（目前只有 Windows / Linux）",
    std::env::consts::OS
  ))
}

pub fn is_enabled() -> bool {
  false
}
