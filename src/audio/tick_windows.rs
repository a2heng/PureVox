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

//! Windows 侧的高精度等待：`CREATE_WAITABLE_TIMER_HIGH_RESOLUTION`（Win10 1803+）。
//!
//! 不用 `timeBeginPeriod(1)`：那是**进程级**副作用（整机时钟分辨率被改），为了一个
//! 测试音的节拍不值得；高精度可等待定时器是**每个定时器**自己的精度，行为可预期。
//! 老系统（没有这个标志）退回普通可等待定时器（精度 ~15.6 ms），再不行退回 `sleep`。

use std::time::Duration;

use windows::Win32::Foundation::{CloseHandle, HANDLE};
use windows::Win32::System::Threading::{
  CREATE_WAITABLE_TIMER_HIGH_RESOLUTION, CreateWaitableTimerExW, INFINITE, SetWaitableTimer,
  TIMER_ALL_ACCESS, WaitForSingleObject,
};

/// 可等待定时器句柄（Drop 时关掉）。
pub struct Waiter {
  handle: Option<HANDLE>,
  /// 是不是拿到了高精度那一档（诊断用）。
  high_res: bool,
}

// 句柄只在本线程用（`Tick` 不是 `Sync` 的需求），但 Send 让它能进线程闭包。
unsafe impl Send for Waiter {}

impl Waiter {
  pub fn new() -> Self {
    // 先试高精度；失败再试普通；都失败就退回 sleep（仍然比原来的 sleep 循环好：
    // 至少不会在「落后」时连发两格）。
    let hi = unsafe {
      CreateWaitableTimerExW(
        None,
        None,
        CREATE_WAITABLE_TIMER_HIGH_RESOLUTION,
        TIMER_ALL_ACCESS.0,
      )
    };
    if let Ok(h) = hi {
      return Self {
        handle: Some(h),
        high_res: true,
      };
    }
    let plain = unsafe { CreateWaitableTimerExW(None, None, 0, TIMER_ALL_ACCESS.0) };
    match plain {
      Ok(h) => Self {
        handle: Some(h),
        high_res: false,
      },
      Err(_) => Self {
        handle: None,
        high_res: false,
      },
    }
  }

  pub fn is_high_res(&self) -> bool {
    self.high_res
  }

  /// 睡至少 `d`（相对时长，负的 100 ns 单位）。
  pub fn wait(&mut self, d: Duration) {
    let Some(h) = self.handle else {
      std::thread::sleep(d);
      return;
    };
    // 相对到期时间：负数 = 相对；单位 100 ns。至少 1 个单位，别传 0（= 立刻触发）。
    let units = (d.as_nanos() / 100).max(1) as i64;
    let due = -units;
    let set = unsafe { SetWaitableTimer(h, &due, 0, None, None, false) };
    if set.is_err() {
      std::thread::sleep(d);
      return;
    }
    unsafe {
      WaitForSingleObject(h, INFINITE);
    }
  }
}

impl Drop for Waiter {
  fn drop(&mut self) {
    if let Some(h) = self.handle.take() {
      unsafe {
        let _ = CloseHandle(h);
      }
    }
  }
}
