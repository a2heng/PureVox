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

//! Linux 的线程亲和性（`sched_setaffinity`）。

/// 固定当前线程到 `core`（Linux 上优先级沿用默认策略，实时调度要 CAP_SYS_NICE，
/// 不在这里抢；亲和性已经能避免「两列的模型挤同一个核」）。
pub fn pin_current(core: usize) -> Result<(), String> {
  unsafe {
    let mut set: libc::cpu_set_t = std::mem::zeroed();
    libc::CPU_SET(core, &mut set);
    let rc = libc::sched_setaffinity(
      0,
      std::mem::size_of::<libc::cpu_set_t>(),
      &set as *const libc::cpu_set_t,
    );
    if rc != 0 {
      return Err(format!(
        "sched_setaffinity(cpu{core}) 失败：{}",
        std::io::Error::last_os_error()
      ));
    }
  }
  Ok(())
}
