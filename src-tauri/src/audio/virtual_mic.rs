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

//! 虚拟麦克风（Linux = PipeWire 虚拟声卡）—— 界面顶栏「驱动」页用它创建 / 移除。
//! **实现按平台分文件**：`virtual_mic_linux.rs`（真实实现）/ `virtual_mic_windows.rs`
//! （Windows 用 VB-CABLE，Rust 侧无实现）/ `virtual_mic_other.rs`（其它平台不可用）。
//!
//! Linux 架构（单一生产者 + 双出口，照搬 legacy
//! `legacy-v2026.09.30.1944/pvplatform/system/_posix.py`）：
//!   1. 单声道 null-sink `purevox_out`（`pw-cli create-node adapter`）—— 唯一写入口，
//!      PureVox 的降噪音频输出到它；
//!   2. 它的内置 monitor `purevox_out.monitor` 就是宽口径虚拟麦克风（Audacity /
//!      浏览器 / pavucontrol 等「列出全部源」的软件可选中）；
//!   3. `pactl load-module module-remap-source` 把 monitor 重映射成**非 monitor 真源**
//!      `purevox_mic`（media.class=Audio/Source），供 OBS 等「只列真源」的软件选中。
//!      用 `module-remap-source` 而非 `module-null-sink media.class=Audio/Source/Virtual`
//!      —— 后者实测会把 pipewire-pulse 协议搞坏（pactl 报协议错误、托盘清空）。
//! 移除 = `pactl unload-module`（定位 remap 模块）+ `pw-cli destroy`（sink 节点）。

use serde::Serialize;

use crate::debug::Probe;

/// 虚拟麦克风当前状态（进调试快照 `virtual_mic`）。
#[derive(Clone, Debug, Serialize)]
pub struct Status {
  /// 写入口 null-sink 是否存在
  pub sink: bool,
  /// 非 monitor 真源是否存在
  pub source: bool,
  /// 是否有 `pw-cli`（创建 sink 必需）
  pub pw_cli: bool,
  /// 是否有 `pactl`（创建真源需要；缺了只保留 monitor 出口）
  pub pactl: bool,
}

#[cfg(target_os = "linux")]
#[path = "virtual_mic_linux.rs"]
mod imp;
#[cfg(windows)]
#[path = "virtual_mic_windows.rs"]
mod imp;
#[cfg(not(any(target_os = "linux", windows)))]
#[path = "virtual_mic_other.rs"]
mod imp;

pub use imp::{create, remove, status};

/// 采集当前状态；不可用时返回原因（供 `Probe::unavailable`）。
pub fn status_probe() -> Probe<Status> {
  match status() {
    Ok(s) => Probe::ok(s),
    Err(e) => Probe::unavailable(e),
  }
}
