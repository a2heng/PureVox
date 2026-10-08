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

//! Windows 的设备面：WASAPI/MME 枚举本就干净（不精简）；可回环目标是各 WASAPI 输出端点。

use super::{DeviceInfo, LoopbackTarget};

/// Windows 枚举本就干净，原样返回。
pub(super) fn simplify(devices: Vec<DeviceInfo>) -> Vec<DeviceInfo> {
  devices
}

/// 可回环的输出设备（`loopback:<wasapi 端点 ID>`）。
pub(super) fn loopback_targets() -> Vec<LoopbackTarget> {
  let mut v = Vec::new();
  for d in super::enumerate().devices {
    if d.direction == "output" {
      v.push(LoopbackTarget {
        id: format!("loopback:{}", d.id),
        name: d.name,
      });
    }
  }
  v
}
