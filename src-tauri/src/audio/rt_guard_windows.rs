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

//! Windows 实现：MMCSS（Pro Audio）+ 执行保活（ExecutionRequired）。

use std::sync::atomic::{AtomicBool, Ordering::Relaxed};

use windows::core::PCWSTR;
use windows::Win32::Foundation::{CloseHandle, HANDLE};
use windows::core::PWSTR;
use windows::Win32::System::Power::{
  PowerClearRequest, PowerCreateRequest, PowerRequestExecutionRequired, PowerSetRequest,
  POWER_REQUEST_TYPE,
};
use windows::Win32::System::Threading::{
  AvRevertMmThreadCharacteristics, AvSetMmThreadCharacteristicsW, AvSetMmThreadPriority,
  AVRT_PRIORITY_CRITICAL, POWER_REQUEST_CONTEXT_FLAGS, REASON_CONTEXT, REASON_CONTEXT_0,
};

static FAILED: AtomicBool = AtomicBool::new(false);
static mut ERR: Option<&'static str> = None;

fn note_fail(msg: &'static str) {
  FAILED.store(true, Relaxed);
  // SAFETY: 只在失败路径写一次；读取方拿到的是 `&'static str`，不会有悬垂。
  unsafe {
    ERR = Some(msg);
  }
}

pub fn last_error() -> Option<&'static str> {
  if FAILED.load(Relaxed) {
    unsafe { ERR }
  } else {
    None
  }
}

/// MMCSS 句柄：Drop 时撤销登记（线程退出前必须还回去，否则系统会一直当你是音频任务）。
pub struct ThreadGuard(Option<HANDLE>);

unsafe impl Send for ThreadGuard {}

pub fn enter() -> ThreadGuard {
  // "Pro Audio" 是 MMCSS 里最高一档（比 "Audio" 更严格）；任务索引由系统回填
  let name: Vec<u16> = "Pro Audio\0".encode_utf16().collect();
  let mut idx: u32 = 0;
  match unsafe { AvSetMmThreadCharacteristicsW(PCWSTR(name.as_ptr()), &mut idx) } {
    Ok(h) => {
      // 拿不到 CRITICAL 也不致命（有些系统策略会拒绝），登记本身已经够用
      let _ = unsafe { AvSetMmThreadPriority(h, AVRT_PRIORITY_CRITICAL) };
      ThreadGuard(Some(h))
    }
    Err(_) => {
      note_fail("AvSetMmThreadCharacteristicsW(Pro Audio) 失败：线程没拿到实时音频优先级");
      ThreadGuard(None)
    }
  }
}

impl Drop for ThreadGuard {
  fn drop(&mut self) {
    if let Some(h) = self.0.take() {
      unsafe {
        let _ = AvRevertMmThreadCharacteristics(h);
      }
    }
  }
}

/// 执行保活句柄：Drop 时清请求并关句柄。
pub struct ExecGuard(Option<HANDLE>);

unsafe impl Send for ExecGuard {}

pub fn acquire_exec() -> ExecGuard {
  // REASON_CONTEXT：SimpleReasonString 是**给用户看的英文短语**（任务管理器里显示），
  // 用 static 宽字符常量，生命周期覆盖整个请求。
  const WHY: &[u16] = &[
    b'A' as u16, b'u' as u16, b'd' as u16, b'i' as u16, b'o' as u16, b' ' as u16,
    b's' as u16, b't' as u16, b'r' as u16, b'e' as u16, b'a' as u16, b'm' as u16,
    b'i' as u16, b'n' as u16, b'g' as u16, 0,
  ];
  let ctx = REASON_CONTEXT {
    Version: 0, // POWER_REQUEST_CONTEXT_VERSION
    Flags: POWER_REQUEST_CONTEXT_FLAGS(1), // POWER_REQUEST_CONTEXT_SIMPLE_STRING
    Reason: REASON_CONTEXT_0 {
      SimpleReasonString: PWSTR(WHY.as_ptr() as *mut u16),
    },
  };
  let h = match unsafe { PowerCreateRequest(&ctx) } {
    Ok(h) => h,
    Err(_) => {
      note_fail("PowerCreateRequest 失败：拿不到执行保活");
      return ExecGuard(None);
    }
  };
  if unsafe { PowerSetRequest(h, PowerRequestExecutionRequired) }.is_err() {
    note_fail("PowerSetRequest(ExecutionRequired) 失败：系统仍可能把进程按效率模式挂起");
    unsafe {
      let _ = CloseHandle(h);
    }
    return ExecGuard(None);
  }
  ExecGuard(Some(h))
}

impl Drop for ExecGuard {
  fn drop(&mut self) {
    if let Some(h) = self.0.take() {
      unsafe {
        let _ = PowerClearRequest(h, PowerRequestExecutionRequired as POWER_REQUEST_TYPE);
        let _ = CloseHandle(h);
      }
    }
  }
}
