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

//! 全局热键（仅 Windows）：`RegisterHotKey` + 轮询消息队列。
//!
//! 一组绑定同时注册：① 启停热键（设置里的「全局热键」）、② 每个音效一个触发键
//! （音效板处理行里逐个录制，同 legacy `uitk/main_window.py` 的 `pad:<i>` 语义）。
//! **一个线程管全部绑定**（`RegisterHotKey` 的 id 按序号分配），避免 N 个音效开 N 个线程。
//!
//! 键位一律用规范字符串表达，修饰键顺序固定 `Ctrl+Alt+Shift+Win`，如
//! `Ctrl+Alt+1` / `Alt+.` / `F8`；**空串 = 不监听**。字母/数字必须带至少一个
//! 修饰键，`F1`–`F24` 可单键（与旧实现 `uitk/hotkeys.py` 同一契约）。
//! 注册结果（成功 / 被占用失败）上报到调试接口 `ui`，便于无控制台定位。

use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering::Relaxed};
use std::thread::JoinHandle;

use crate::debug::SharedHub;

const MOD_ALT: u32 = 0x0001;
const MOD_CONTROL: u32 = 0x0002;
const MOD_SHIFT: u32 = 0x0004;
const MOD_WIN: u32 = 0x0008;

/// 修饰键名（顺序固定）与位。
const MODS: &[(&str, u32)] = &[
  ("ctrl", MOD_CONTROL),
  ("control", MOD_CONTROL),
  ("alt", MOD_ALT),
  ("shift", MOD_SHIFT),
  ("win", MOD_WIN),
  ("super", MOD_WIN),
  ("meta", MOD_WIN),
];

/// token → (vk, 显示名)。
fn token_of(t: &str) -> Option<(u32, &'static str)> {
  // 字母
  let bytes = t.as_bytes();
  if bytes.len() == 1 {
    let c = bytes[0];
    if c.is_ascii_lowercase() {
      return Some(((c - b'a') as u32 + 0x41, "")); // 显示名在下面统一大写
    }
    if c.is_ascii_digit() {
      return Some(((c - b'0') as u32 + 0x30, ""));
    }
    return match c {
      b'`' => Some((0xC0, "`")),
      b'-' => Some((0xBD, "-")),
      b'=' => Some((0xBB, "=")),
      b'[' => Some((0xDB, "[")),
      b']' => Some((0xDD, "]")),
      b'\\' => Some((0xDC, "\\")),
      b';' => Some((0xBA, ";")),
      b'\'' => Some((0xDE, "'")),
      b',' => Some((0xBC, ",")),
      b'.' => Some((0xBE, ".")),
      b'/' => Some((0xBF, "/")),
      _ => None,
    };
  }
  if let Some(n) = t.strip_prefix('f')
    && let Ok(i) = n.parse::<u32>()
    && (1..=24).contains(&i)
  {
    return Some((0x70 + i - 1, ""));
  }
  if let Some(n) = t.strip_prefix("num")
    && let Ok(i) = n.parse::<u32>()
    && i < 10
  {
    return Some((0x60 + i, ""));
  }
  match t {
    "backspace" => Some((0x08, "Backspace")),
    "tab" => Some((0x09, "Tab")),
    "enter" => Some((0x0D, "Enter")),
    "esc" => Some((0x1B, "Esc")),
    "space" => Some((0x20, "Space")),
    "pageup" => Some((0x21, "PageUp")),
    "pagedown" => Some((0x22, "PageDown")),
    "end" => Some((0x23, "End")),
    "home" => Some((0x24, "Home")),
    "left" => Some((0x25, "Left")),
    "up" => Some((0x26, "Up")),
    "right" => Some((0x27, "Right")),
    "down" => Some((0x28, "Down")),
    "insert" => Some((0x2D, "Insert")),
    "delete" => Some((0x2E, "Delete")),
    _ => None,
  }
}

/// 解析规范串 → (修饰位, vk)。空串 / 非法 / 无修饰且非功能键 → None。
pub fn parse_spec(spec: &str) -> Option<(u32, u32)> {
  let spec = spec.trim();
  if spec.is_empty() {
    return None;
  }
  let mut bits = 0u32;
  let mut token: Option<String> = None;
  for raw in spec.split('+') {
    let p = raw.trim();
    if p.is_empty() {
      continue;
    }
    let low = p.to_ascii_lowercase();
    if let Some((_, bit)) = MODS.iter().find(|(n, _)| *n == low) {
      bits |= *bit;
    } else {
      token = Some(low);
    }
  }
  let token = token?;
  let (vk, _) = token_of(&token)?;
  // 无修饰键时只允许功能键
  let is_fn = token.starts_with('f')
    && token[1..]
      .parse::<u32>()
      .map(|i| (1..=24).contains(&i))
      .unwrap_or(false);
  if bits == 0 && !is_fn {
    return None;
  }
  Some((bits, vk))
}

/// 规范化显示（校验后重排修饰键顺序）；非法返回空串。
pub fn normalize_spec(spec: &str) -> String {
  let Some((bits, _)) = parse_spec(spec) else {
    return String::new();
  };
  let mut token = String::new();
  for raw in spec.split('+') {
    let p = raw.trim().to_ascii_lowercase();
    if !MODS.iter().any(|(n, _)| *n == p) {
      token = p;
    }
  }
  let mut parts: Vec<String> = Vec::new();
  for (name, bit) in [
    ("Ctrl", MOD_CONTROL),
    ("Alt", MOD_ALT),
    ("Shift", MOD_SHIFT),
    ("Win", MOD_WIN),
  ] {
    if bits & bit != 0 {
      parts.push(name.to_string());
    }
  }
  // 主键显示名
  let disp = match token_of(&token) {
    Some((_, d)) if !d.is_empty() => d.to_string(),
    _ => {
      if token.len() == 1 && token.as_bytes()[0].is_ascii_alphabetic() {
        token.to_ascii_uppercase()
      } else {
        token.clone()
      }
    }
  };
  parts.push(disp);
  parts.join("+")
}

/// 一条热键绑定：规范键位串（空串忽略）+ 按下时的回调。
pub type Binding = (String, Arc<dyn Fn() + Send + Sync>);

/// 已解析、校验通过的绑定（交给平台实现注册）：(规范串, 修饰位, 虚拟键码, 回调)。
pub type Resolved = (String, u32, u32, Arc<dyn Fn() + Send + Sync>);

/// 一个正在监听的热键线程（管一组绑定，见 [`Binding`]）。
pub struct Hotkeys {
  stop: Arc<AtomicBool>,
  join: Option<JoinHandle<()>>,
}

impl Hotkeys {
  /// 注册并启动监听整组绑定；`bindings` 全为空串则不监听（返回空句柄）。
  /// 非法 / 被占用的键位跳过并写进调试接口 `ui`（**不静默**，逐条报错）。
  pub fn start(bindings: Vec<Binding>, hub: SharedHub) -> Self {
    let stop = Arc::new(AtomicBool::new(false));
    let mut ok: Vec<Resolved> = Vec::new();
    let mut seen: Vec<String> = Vec::new();
    for (spec, cb) in bindings {
      let spec = spec.trim().to_string();
      if spec.is_empty() {
        continue;
      }
      let Some((bits, vk)) = parse_spec(&spec) else {
        hub.push_ui("error", format!("热键键位无效：{spec}"));
        continue;
      };
      // 同一键位注册两次必然第二个失败（还被占用），提前拦下给个能看懂的提示
      if seen.contains(&spec) {
        hub.push_ui("error", format!("热键重复：{spec}"));
        continue;
      }
      seen.push(spec.clone());
      ok.push((spec, bits, vk, cb));
    }
    if ok.is_empty() {
      return Hotkeys { stop, join: None };
    }
    let join = imp::spawn(ok, hub, stop.clone());
    Hotkeys { stop, join }
  }

  pub fn stop(&mut self) {
    self.stop.store(true, Relaxed);
    if let Some(j) = self.join.take() {
      let _ = j.join();
    }
  }
}

// 平台实现分文件（CONTRIBUTING.md）：Windows 用 `RegisterHotKey`，其它平台明确不可用。
#[cfg(windows)]
#[path = "hotkey_windows.rs"]
mod imp;
#[cfg(not(windows))]
#[path = "hotkey_other.rs"]
mod imp;
