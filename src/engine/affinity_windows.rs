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

//! Windows 的线程亲和性（`SetThreadAffinityMask`）。

use windows::Win32::Foundation::GetLastError;
use windows::Win32::System::Threading::{
  GetCurrentThread, SetThreadAffinityMask, SetThreadPriority, THREAD_PRIORITY_ABOVE_NORMAL,
};

/// 固定当前线程到 `core` 并提到 ABOVE_NORMAL（失败返回人话原因）。
pub fn pin_current(core: usize) -> Result<(), String> {
  unsafe {
    // 逻辑核编号 ≥ 64 在 Windows 上要走 processor group，本机远没那么多，截到 63 即可。
    let mask = SetThreadAffinityMask(GetCurrentThread(), 1usize << core.min(63));
    if mask == 0 {
      let err = GetLastError();
      return Err(format!("SetThreadAffinityMask(cpu{core}) 失败：{err:?}"));
    }
    // 优先级只影响与普通线程的竞争，不改变实时音频回调（WASAPI 自己管）。
    if let Err(e) = SetThreadPriority(GetCurrentThread(), THREAD_PRIORITY_ABOVE_NORMAL) {
      return Err(format!("SetThreadPriority 失败：{e}"));
    }
  }
  Ok(())
}
