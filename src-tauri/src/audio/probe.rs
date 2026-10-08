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

//! 校准探针的平台播放。实现按平台分文件（AGENTS.md §4）。
//!
//! [`NATIVE`] = 本平台能否把探针**直送指定的输出 sink**（Linux → `probe_linux.rs` 用
//! `pacat`；Windows 不行，回环探针走 cpal 输出端点）。`play` 只在 `NATIVE` 时有意义。

#[cfg(target_os = "linux")]
#[path = "probe_linux.rs"]
mod imp;
#[cfg(windows)]
#[path = "probe_windows.rs"]
mod imp;
#[cfg(not(any(target_os = "linux", windows)))]
#[path = "probe_other.rs"]
mod imp;

pub use imp::{NATIVE, play};
