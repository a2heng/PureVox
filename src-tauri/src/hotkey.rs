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

//! 全局热键（仅 Windows）：`RegisterHotKey` + 轮询消息队列，切换「启动 / 停止」。
//!
//! 键位一律用规范字符串表达，修饰键顺序固定 `Ctrl+Alt+Shift+Win`，如
//! `Ctrl+Alt+1` / `Alt+.` / `F8`；**空串 = 不监听**。字母/数字必须带至少一个
//! 修饰键，`F1`–`F24` 可单键（与旧实现 `uitk/hotkeys.py` 同一契约）。
//! 注册结果（成功 / 被占用失败）上报到调试接口 `ui`，便于无控制台定位。

use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering::Relaxed};
use std::thread::JoinHandle;
use std::time::Duration;

use crate::debug::SharedHub;

const MOD_ALT: u32 = 0x0001;
const MOD_CONTROL: u32 = 0x0002;
const MOD_SHIFT: u32 = 0x0004;
const MOD_WIN: u32 = 0x0008;
const MOD_NOREPEAT: u32 = 0x4000;

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

/// 一个正在监听的热键线程。
pub struct Hotkeys {
  stop: Arc<AtomicBool>,
  join: Option<JoinHandle<()>>,
}

impl Hotkeys {
  /// 注册并启动监听；`spec` 为空则不监听（返回空句柄）。注册结果写进调试接口。
  pub fn start(spec: &str, hub: SharedHub, on_trigger: Arc<dyn Fn() + Send + Sync>) -> Self {
    let spec = spec.trim().to_string();
    let stop = Arc::new(AtomicBool::new(false));
    if spec.is_empty() {
      return Hotkeys { stop, join: None };
    }
    let parsed = parse_spec(&spec);
    let Some((bits, vk)) = parsed else {
      hub.push_ui("error", format!("热键键位无效：{spec}"));
      return Hotkeys { stop, join: None };
    };
    let stop2 = stop.clone();
    let hub2 = hub.clone();
    let spec2 = spec.clone();
    let join = std::thread::Builder::new()
      .name("hotkey".into())
      .spawn(move || unsafe {
        use windows::Win32::UI::Input::KeyboardAndMouse::{
          HOT_KEY_MODIFIERS, RegisterHotKey, UnregisterHotKey,
        };
        use windows::Win32::UI::WindowsAndMessaging::{MSG, PM_REMOVE, PeekMessageW, WM_HOTKEY};
        const ID: i32 = 0x5056;
        if RegisterHotKey(None, ID, HOT_KEY_MODIFIERS(bits | MOD_NOREPEAT), vk).is_err() {
          hub2.push_ui("error", format!("热键注册失败（可能被占用）：{spec2}"));
          return;
        }
        hub2.push_ui("info", format!("热键已注册：{spec2}"));
        let mut msg = MSG::default();
        while !stop2.load(Relaxed) {
          while PeekMessageW(&mut msg, None, 0, 0, PM_REMOVE).as_bool() {
            if msg.message == WM_HOTKEY && msg.wParam.0 as i32 == ID {
              on_trigger();
            }
          }
          std::thread::sleep(Duration::from_millis(20));
        }
        let _ = UnregisterHotKey(None, ID);
      })
      .ok();
    Hotkeys { stop, join }
  }

  pub fn stop(&mut self) {
    self.stop.store(true, Relaxed);
    if let Some(j) = self.join.take() {
      let _ = j.join();
    }
  }
}
