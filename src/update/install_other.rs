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

//! 其它平台（macOS 等）：打包方式与自动替换都还没有对应实现，**明确不可用**
//! （AGENTS.md：不静默失败）。仍可下载，看界面给的 release 页面手动装。

use super::{Phase, Status};

/// 没有可认领的产物后缀 → `check` 那边直接落 `unsupported` + 原因，不会白下载。
pub const ASSET_SUFFIXES: &[&str] = &[];

pub fn asset_suffixes() -> &'static [&'static str] {
  ASSET_SUFFIXES
}

/// 安装方式短名（进 `/debug` 的 `update.install_kind`）：本平台没有实现，只报系统名。
pub fn kind_name() -> &'static str {
  std::env::consts::OS
}

pub fn install(_st: &Status) -> Result<String, String> {
  super::with(|s| {
    s.phase = Phase::Unsupported;
    s.reason = format!(
      "{} 还没有自动更新实现，请手动下载安装",
      std::env::consts::OS
    );
  });
  Err(format!("{} 暂不支持自动更新", std::env::consts::OS))
}
