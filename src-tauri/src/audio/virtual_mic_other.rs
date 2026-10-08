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

//! 其它平台（非 Windows / 非 Linux）：虚拟麦克风不可用，给明确原因（不静默）。

const MSG: &str = "虚拟麦克风只实现了 Linux（PipeWire）";

pub fn status() -> Result<super::Status, String> {
  Err(MSG.into())
}

pub fn create() -> Result<super::Status, String> {
  Err(MSG.into())
}

pub fn remove() -> Result<super::Status, String> {
  Err(MSG.into())
}
