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

//! 非 Windows 的等待：`nanosleep` 本身就是微秒级精度，直接 `thread::sleep`。
//!
//! （Linux / macOS 的调度粒度远小于 10 ms，不需要 Windows 那套可等待定时器。）

use std::time::Duration;

pub struct Waiter;

impl Waiter {
  pub fn new() -> Self {
    Self
  }

  pub fn is_high_res(&self) -> bool {
    true
  }

  pub fn wait(&mut self, d: Duration) {
    std::thread::sleep(d);
  }
}
