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

//! 其它平台（非 Windows）：`RegisterHotKey` 是 Windows 专有，本平台明确不可用
//! （AGENTS.md：不静默失败）。规范串解析见 [`super`]（跨平台）。

use std::sync::Arc;
use std::sync::atomic::AtomicBool;
use std::thread::JoinHandle;

use crate::debug::SharedHub;

pub(super) fn spawn(
  _bits: u32,
  _vk: u32,
  spec: String,
  hub: SharedHub,
  _stop: Arc<AtomicBool>,
  _on_trigger: Arc<dyn Fn() + Send + Sync>,
) -> Option<JoinHandle<()>> {
  hub.push_ui("unavailable", format!("全局热键仅 Windows 支持：{spec}"));
  None
}
