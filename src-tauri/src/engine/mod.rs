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

//! L2 引擎：列与行、信号装配、插件链（DESIGN.md §3）。
//!
//! 当前阶段只落了骨架：`Stage` 契约、`Pipeline`、`registry`。列工作线程见后续步骤。

pub mod aec;
pub mod calib;
pub mod components;
pub mod pipeline;
pub mod registry;
pub mod session;
pub mod stage;
