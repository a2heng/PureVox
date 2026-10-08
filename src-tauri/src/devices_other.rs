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

//! 其它平台（非 Windows / 非 Linux）：不精简；无回环目标（AEC 回环不可用）。

use super::{DeviceInfo, LoopbackTarget};

pub(super) fn simplify(devices: Vec<DeviceInfo>) -> Vec<DeviceInfo> {
  devices
}

pub(super) fn loopback_targets() -> Vec<LoopbackTarget> {
  Vec::new()
}
