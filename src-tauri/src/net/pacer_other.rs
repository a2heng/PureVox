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

//! 非 Windows：内核定时器粒度本来就是 1 ms 级（Linux hrtimer / macOS），无需调整。
//! 保留同名空实现，让 `net/mod.rs` 的调用点跨平台一份。

pub struct TimerResolution;

impl TimerResolution {
  pub fn raise() -> Self {
    TimerResolution
  }
}
