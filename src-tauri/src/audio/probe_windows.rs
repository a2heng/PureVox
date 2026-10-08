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

//! Windows：回环探针走 cpal 输出端点（`loopback:<wasapi 端点 ID>` 本就是 cpal 能打开的），
//! 不需要平台直送。

pub const NATIVE: bool = false;

pub fn play(_pcm: &[f32], _target: Option<&str>) -> Result<(), String> {
  Err("Windows 回环探针走 cpal 输出路径".into())
}
