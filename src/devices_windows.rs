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

use super::{DeviceInfo, DeviceList, LoopbackTarget};

/// Windows：回环目标就是 WASAPI 输出端点，`far_device` 可以填裸输出设备 ID。
pub(super) const LOOPBACK_ACCEPTS_DEVICE_ID: bool = true;

/// Windows 枚举本就干净，原样返回。
pub(super) fn simplify(devices: Vec<DeviceInfo>) -> Vec<DeviceInfo> {
  devices
}

/// 可回环的输出设备（`loopback:<wasapi 端点 ID>`）。
/// 入参是**已枚举好的列表**（由 `devices::loopback_targets` 统一给），本平台不再自己枚举。
pub(super) fn loopback_targets(list: &DeviceList) -> Vec<LoopbackTarget> {
  list
    .devices
    .iter()
    .filter(|d| d.direction == "output")
    .map(|d| LoopbackTarget {
      id: format!("loopback:{}", d.id),
      name: d.name.clone(),
    })
    .collect()
}
