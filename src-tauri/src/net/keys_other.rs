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

//! 其它平台（非 Windows）：按键注入未实现（Linux 需 `uinput` / XTEST，macOS 需 CGEvent），
//! 给明确原因，**不静默失败**。说明见 [`super`]。

use crate::net::keymap::Scan;

pub fn backend() -> Result<&'static str, String> {
  Err("当前平台未实现按键注入（Windows 用 SendInput；Linux 需 uinput 或 XTEST）".to_string())
}

pub fn inject_key(scan: Scan, down: bool) -> Result<(), String> {
  let _ = (scan, down);
  backend().map(|_| ())
}

pub fn inject_text(s: &str) -> Result<usize, String> {
  let units: Vec<u16> = s.encode_utf16().collect();
  if units.is_empty() {
    return Ok(0);
  }
  let n = units.len();
  backend().map(|_| n)
}
