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

//! Windows 定时器精度（网络出站节拍用）。
//!
//! 默认系统定时器粒度是 **15.625 ms**，`tokio::time::sleep_until(now+10ms)` 实际
//! 15.6 ms 才醒一次，于是 10 ms 的包会成串发出（两包一起到）。客户端为这种突发
//! 只能加大 jitter buffer，直接抬高播放延迟。
//!
//! `timeBeginPeriod(1)` 把本进程的等待粒度提到 1 ms（Windows 10 2004+ 是进程级），
//! 服务起时 raise、停时 drop；不常驻、不影响没开网络时的行为。

#[link(name = "winmm")]
unsafe extern "system" {
  fn timeBeginPeriod(period: u32) -> u32;
  fn timeEndPeriod(period: u32) -> u32;
}

pub struct TimerResolution {
  active: bool,
}

impl TimerResolution {
  pub fn raise() -> Self {
    // 失败不致命：只是退回系统默认粒度，网络与音频仍能工作
    let r = unsafe { timeBeginPeriod(1) };
    TimerResolution { active: r == 0 }
  }
}

impl Drop for TimerResolution {
  fn drop(&mut self) {
    if self.active {
      unsafe {
        timeEndPeriod(1);
      }
    }
  }
}
