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

//! 远程输入注入（DESIGN.md §4.1）：手机输入法打字 + 手机实体键当全尺寸键盘。
//!
//! **两件事分开**（协议里的 `text` 与 `key`）：
//! - [`inject_text`]：已组合好的字符串，逐个 UTF-16 码元用 `KEYEVENTF_UNICODE` 注入。
//!   中文、emoji、任何输入法组合结果都能打，且不经过电脑端输入法。
//! - [`inject_key`]：按 [`crate::net::keymap`] 的扫描码映射注入，支持修饰键、方向键、
//!   小键盘、扩展键。
//!
//! **平台**：注入后端按平台分。Windows 用 `SendInput`；其余平台未实现，返回
//! `Unavailable` 原因（Linux 需 `uinput` / XTEST，macOS 需 CGEvent），**不静默失败**。
//! 纯函数（组装事件）跨平台可测；真正调用系统 API 只在 Windows 编译。

use super::keymap::Scan;

/// 注入后端描述（进调试接口，便于判断「为什么没反应」）。
pub fn backend() -> Result<&'static str, String> {
  backend_name()
}

fn backend_name() -> Result<&'static str, String> {
  #[cfg(windows)]
  {
    Ok("windows/SendInput")
  }
  #[cfg(not(windows))]
  {
    Err("当前平台未实现按键注入（Windows 用 SendInput；Linux 需 uinput 或 XTEST）".to_string())
  }
}

/// 注入一次按键（`down = false` 是松开）。
pub fn inject_key(scan: Scan, down: bool) -> Result<(), String> {
  #[cfg(windows)]
  {
    let inputs = key_inputs(scan, down);
    send(&inputs)
  }
  #[cfg(not(windows))]
  {
    let _ = (scan, down);
    backend()
  }
}

/// 注入一段文本，返回注入的 UTF-16 码元数。
///
/// **不做过滤**：手机端组合好什么就是什么（空串 → 0 个码元，直接返回，不调系统 API）。
pub fn inject_text(s: &str) -> Result<usize, String> {
  let units: Vec<u16> = s.encode_utf16().collect();
  if units.is_empty() {
    return Ok(0);
  }
  #[cfg(windows)]
  {
    // 每批 64 个码元 = 128 条 INPUT，避免一次性构造过长数组
    const BATCH: usize = 64;
    for chunk in units.chunks(BATCH) {
      send(&text_inputs(chunk))?;
    }
    Ok(units.len())
  }
  #[cfg(not(windows))]
  {
    let n = units.len();
    let _ = units;
    backend().map(|_| n)
  }
}

// ---------------------------------------------------------------- 平台后端

#[cfg(windows)]
mod backend_impl {
  use super::*;
  use windows::Win32::UI::Input::KeyboardAndMouse::{
    INPUT, INPUT_0, INPUT_KEYBOARD, KEYBD_EVENT_FLAGS, KEYBDINPUT, KEYEVENTF_EXTENDEDKEY,
    KEYEVENTF_KEYUP, KEYEVENTF_SCANCODE, KEYEVENTF_UNICODE, SendInput, VIRTUAL_KEY,
  };

  fn kb(wvk: u16, wscan: u16, flags: KEYBD_EVENT_FLAGS) -> INPUT {
    INPUT {
      r#type: INPUT_KEYBOARD,
      Anonymous: INPUT_0 {
        ki: KEYBDINPUT {
          wVk: VIRTUAL_KEY(wvk),
          wScan: wscan,
          dwFlags: flags,
          time: 0,
          dwExtraInfo: 0,
        },
      },
    }
  }

  /// 一个扫描码事件（纯函数：可单测，不调系统 API）。
  pub(super) fn key_inputs(scan: Scan, down: bool) -> Vec<INPUT> {
    let mut flags = KEYEVENTF_SCANCODE;
    if scan.extended {
      flags |= KEYEVENTF_EXTENDEDKEY;
    }
    if !down {
      flags |= KEYEVENTF_KEYUP;
    }
    // 扫描码注入时 wVk 必须是 0，否则被当成虚拟键码
    vec![kb(0, scan.code as u16, flags)]
  }

  /// 一段 UTF-16 码元的 down + up 事件（纯函数：可单测）。
  pub(super) fn text_inputs(units: &[u16]) -> Vec<INPUT> {
    let mut v = Vec::with_capacity(units.len() * 2);
    for u in units {
      v.push(kb(0, *u, KEYEVENTF_UNICODE));
      v.push(kb(0, *u, KEYEVENTF_UNICODE | KEYEVENTF_KEYUP));
    }
    v
  }

  /// 一次 `SendInput`。返回被系统接受的条数；与请求不符即失败（UIPI 会拦下高权限窗口）。
  pub(super) fn send(inputs: &[INPUT]) -> Result<(), String> {
    if inputs.is_empty() {
      return Ok(());
    }
    let size = std::mem::size_of::<INPUT>() as i32;
    // SAFETY: inputs 是本函数构造的、长度正确的 INPUT 数组；size 是真实结构大小。
    let n = unsafe { SendInput(inputs, size) };
    if n as usize != inputs.len() {
      return Err(format!(
        "注入被系统拒绝：提交 {} 条，实际 {} 条（目标窗口可能有更高权限）",
        inputs.len(),
        n
      ));
    }
    Ok(())
  }
}

#[cfg(windows)]
use backend_impl::{key_inputs, send, text_inputs};

#[cfg(test)]
mod tests {
  use super::*;
  use crate::net::keymap::{self, KEYCODE_A, KEYCODE_DPAD_UP};

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

  /// 扫码映射 → 事件形状（Windows）：扫描码注入时虚拟键码必须为 0。
  #[cfg(windows)]
  #[test]
  fn key_event_shape() {
    let a = key_inputs(keymap::scan(KEYCODE_A).unwrap(), true);
    assert_eq!(a.len(), 1);
    // SAFETY: INPUT 的 union 字段读取（只读已构造好的本地数组）
    let (scan, vk, flags) = unsafe {
      (
        a[0].Anonymous.ki.wScan,
        a[0].Anonymous.ki.wVk.0,
        a[0].Anonymous.ki.dwFlags.0,
      )
    };
    assert_eq!(scan, 0x1E);
    assert_eq!(vk, 0, "扫描码注入时 wVk 必须是 0");

    let up = key_inputs(keymap::scan(KEYCODE_A).unwrap(), false);
    // SAFETY: 同上
    let up_flags = unsafe { up[0].Anonymous.ki.dwFlags.0 };
    assert_ne!(up_flags, flags, "松开必须多 KEYEVENTF_KEYUP");

    // 方向键必须带扩展标记
    let up_key = key_inputs(keymap::scan(KEYCODE_DPAD_UP).unwrap(), true);
    // SAFETY: 同上
    let up_key_flags = unsafe { up_key[0].Anonymous.ki.dwFlags.0 };
    assert_ne!(up_key_flags & 1, 0, "方向键应带 EXTENDEDKEY");
  }

  /// 文本事件：每个码元一 down 一 up，且用 UNICODE 标志。
  #[cfg(windows)]
  #[test]
  fn text_event_pairs() {
    let units: Vec<u16> = "你好a".encode_utf16().collect();
    let ev = text_inputs(&units);
    assert_eq!(ev.len(), units.len() * 2);
    for (i, u) in units.iter().enumerate() {
      // SAFETY: 只读本地构造好的 INPUT 数组的 union 字段
      let (down, up) = unsafe {
        (
          (
            ev[i * 2].Anonymous.ki.wScan,
            ev[i * 2].Anonymous.ki.dwFlags.0,
          ),
          (
            ev[i * 2 + 1].Anonymous.ki.wScan,
            ev[i * 2 + 1].Anonymous.ki.dwFlags.0,
          ),
        )
      };
      assert_eq!(down.0, *u);
      assert_eq!(down.1 & 4, 4, "down 应带 UNICODE");
      assert_eq!(down.1 & 2, 0, "down 不应带 KEYUP");
      assert_eq!(up.0, *u);
      assert_eq!(up.1 & 2, 2, "up 应带 KEYUP");
    }
    // 代理对（emoji）按 UTF-16 码元注入：长度应是码元数而非字符数
    let emoji: Vec<u16> = "🎤".encode_utf16().collect();
    assert_eq!(emoji.len(), 2);
    assert_eq!(text_inputs(&emoji).len(), 4);
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
