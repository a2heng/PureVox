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

//! Windows：退出电源节流（EcoQoS）。
//!
//! `SetProcessInformation(ProcessPowerThrottling)` 里的 `StateMask = 0` 表示
//! **不接受**这两项节流：
//!
//! - `PROCESS_POWER_THROTTLING_EXECUTION_SPEED` —— 降频执行（效率模式）；
//! - `PROCESS_POWER_THROTTLING_IGNORE_TIMER_RESOLUTION` —— 忽略我们设的定时器分辨率。
//!
//! 应用在托盘里（`start_hidden`）时最容易被判成后台进程而吃上这些。播放音频是它的主场景，
//! 所以显式声明「满速执行」—— 这是媒体类应用的标准做法，不改任何全局设置。

use windows::Win32::System::Threading::{
  GetCurrentProcess, PROCESS_POWER_THROTTLING_EXECUTION_SPEED,
  PROCESS_POWER_THROTTLING_IGNORE_TIMER_RESOLUTION, PROCESS_POWER_THROTTLING_STATE,
  ProcessPowerThrottling, SetProcessInformation,
};

pub fn opt_out_of_throttling() -> Result<(), String> {
  let state = PROCESS_POWER_THROTTLING_STATE {
    Version: 1, // PROCESS_POWER_THROTTLING_CURRENT_VERSION
    ControlMask: PROCESS_POWER_THROTTLING_EXECUTION_SPEED
      | PROCESS_POWER_THROTTLING_IGNORE_TIMER_RESOLUTION,
    StateMask: 0, // 0 = 这两项都不启用
  };
  // SAFETY: 传入的是本进程伪句柄 + 指向本栈结构体的有效指针，长度就是该结构体大小。
  let r = unsafe {
    SetProcessInformation(
      GetCurrentProcess(),
      ProcessPowerThrottling,
      &state as *const _ as *const core::ffi::c_void,
      std::mem::size_of::<PROCESS_POWER_THROTTLING_STATE>() as u32,
    )
  };
  r.map_err(|e| format!("SetProcessInformation(ProcessPowerThrottling) 失败：{e}"))
}
