// PureVox — AI 麦克风降噪工具
// Copyright (C) 2024-2026 a2heng <752848283@qq.com>
//
// PureVox is licensed under the GNU General Public License v3.0 or
// later (GPL-3.0-or-later).  See LICENSE for details.
//
// SPDX-License-Identifier: GPL-3.0-or-later

//! Windows 单实例：命名互斥体 + 注册消息。
//!
//! 命名互斥体负责「只有一个」；`RegisterWindowMessage` 负责把第二个进程的启动
//! 变成「显示已有窗口」——广播到所有顶层窗口，只有 PureVox 主窗口 WndProc
//! 认这条消息（见 `tray_windows.rs`）。
//!
//! 只有设了 `PUREVOX_ALLOW_MULTI=1` 的隔离测试才按 profile 区分名字；正常启动全局唯一。

use std::sync::atomic::{AtomicIsize, AtomicU32, Ordering};

use windows::Win32::Foundation::{CloseHandle, ERROR_ALREADY_EXISTS, GetLastError, LPARAM, WPARAM};
use windows::Win32::System::Threading::CreateMutexW;
use windows::Win32::UI::WindowsAndMessaging::{
  ASFW_ANY, AllowSetForegroundWindow, HWND_BROADCAST, PostMessageW, RegisterWindowMessageW,
};
use windows::core::PCWSTR;

/// 应用级 GUID：只用来隔离别的程序的互斥体，不做安全判断。
const APP_GUID: &str = "7d3b1e8a-5c4f-4a2b-9e6d-1f0c2a7b8d94";

static ACTIVATE_MSG: AtomicU32 = AtomicU32::new(0);
/// 互斥体句柄：故意活到进程结束（进程退出时由系统释放）。
static MUTEX: AtomicIsize = AtomicIsize::new(0);

fn wide(s: &str) -> Vec<u16> {
  s.encode_utf16().chain(std::iter::once(0)).collect()
}

/// 默认全局（key=0）；只有隔离测试放行并行时才按 profile 分开。
fn mutex_name() -> String {
  format!("Local\\PureVox-{APP_GUID}-{:016x}", super::instance_key())
}

fn activate_name() -> String {
  format!("PureVoxActivate-{:016x}", super::instance_key())
}

/// 占住单实例；返回 false = 已有同 profile 的实例，调用方直接退出。
pub fn claim() -> bool {
  let name = wide(&mutex_name());
  unsafe {
    let Ok(handle) = CreateMutexW(None, true, PCWSTR(name.as_ptr())) else {
      // 互斥体都建不出来时不要把自己锁死：宁可多开，也不静默起不来。
      return true;
    };
    if GetLastError() == ERROR_ALREADY_EXISTS {
      // 让已有实例有资格抢前台；然后广播「显示窗口」。
      let _ = AllowSetForegroundWindow(ASFW_ANY);
      let msg = activate_message();
      if msg != 0 {
        let _ = PostMessageW(Some(HWND_BROADCAST), msg, WPARAM(0), LPARAM(0));
      }
      let _ = CloseHandle(handle);
      false
    } else {
      // 句柄活到进程结束，由系统释放；不要 CloseHandle，否则互斥体立刻失效。
      MUTEX.store(handle.0 as isize, Ordering::Relaxed);
      true
    }
  }
}

/// 同一 profile 的激活消息 id（Windows 对同名注册返回同一个 id）。
pub fn activate_message() -> u32 {
  let cached = ACTIVATE_MSG.load(Ordering::Relaxed);
  if cached != 0 {
    return cached;
  }
  let name = wide(&activate_name());
  let msg = unsafe { RegisterWindowMessageW(PCWSTR(name.as_ptr())) };
  ACTIVATE_MSG.store(msg, Ordering::Relaxed);
  msg
}
