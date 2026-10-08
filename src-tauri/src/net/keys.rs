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

//! 手机远程输入 → 电脑键鼠注入（DESIGN.md §4.1）。
//!
//! **两件事分开**（协议里的 `text` 与 `key`）：
//! - [`inject_text`]：已组合好的字符串，逐个 UTF-16 码元用 `KEYEVENTF_UNICODE` 注入。
//!   中文、emoji、任何输入法组合结果都能打，且不经过电脑端输入法。
//! - [`inject_key`]：按 [`crate::net::keymap`] 的扫描码映射注入，支持修饰键、方向键、
//!   小键盘、扩展键。
//!
//! **平台实现分文件**（AGENTS.md §4）：`keys_windows.rs` 用 `SendInput`；
//! `keys_other.rs` 未实现，返回明确原因（Linux 需 `uinput` / XTEST，macOS 需 CGEvent），
//! **不静默失败**。

#[cfg(windows)]
#[path = "keys_windows.rs"]
mod imp;
#[cfg(not(windows))]
#[path = "keys_other.rs"]
mod imp;

pub use imp::{backend, inject_key, inject_text};

#[cfg(test)]
mod tests {
  use super::*;
  use crate::net::keymap;

  /// 后端探测：Windows 上必须可用，否则功能就是死的（要能报出来）。
  #[test]
  fn backend_available_on_windows() {
    if cfg!(windows) {
      assert!(backend().is_ok());
    } else {
      assert!(backend().is_err(), "非 Windows 应明确报不可用");
    }
  }

  /// 空串不注入（避免无谓的系统调用）。
  #[test]
  fn empty_text_is_noop() {
    assert_eq!(inject_text("").unwrap_or(0), 0);
  }

  /// 码元计数按 UTF-16（与 Android `String.length` 一致），中英混排才对得上。
  #[test]
  fn counts_utf16_units() {
    // 3 个 ASCII（各 1 码元）+ 2 个汉字（各 1 码元）= 5
    assert_eq!("abc你好".encode_utf16().count(), 5);
    // emoji 是代理对 = 2 码元（所以计数必须按码元而非字符）
    assert_eq!("🎤".encode_utf16().count(), 2);
  }

  /// 未映射的键不会被注入（返回 None 由上层计数忽略）。
  #[test]
  fn unmapped_keys_are_ignored() {
    assert!(keymap::scan(KEYCODE_VOLUME_UP_FOR_TEST).is_none());
    assert!(keymap::scan(-7).is_none());
  }

  const KEYCODE_VOLUME_UP_FOR_TEST: i32 = 24;
}
