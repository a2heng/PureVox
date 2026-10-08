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

//! Windows 全局热键实现（`RegisterHotKey` + 轮询消息队列）。解析/规范串见 [`super`]。

use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering::Relaxed};
use std::thread::JoinHandle;
use std::time::Duration;

use crate::debug::SharedHub;

const MOD_NOREPEAT: u32 = 0x4000;

/// 启动注册线程；注册成功返回其 JoinHandle，失败返回 None（结果写进调试接口）。
pub(super) fn spawn(
  bits: u32,
  vk: u32,
  spec: String,
  hub: SharedHub,
  stop: Arc<AtomicBool>,
  on_trigger: Arc<dyn Fn() + Send + Sync>,
) -> Option<JoinHandle<()>> {
  std::thread::Builder::new()
    .name("hotkey".into())
    .spawn(move || unsafe {
      use windows::Win32::UI::Input::KeyboardAndMouse::{
        HOT_KEY_MODIFIERS, RegisterHotKey, UnregisterHotKey,
      };
      use windows::Win32::UI::WindowsAndMessaging::{MSG, PM_REMOVE, PeekMessageW, WM_HOTKEY};
      const ID: i32 = 0x5056;
      if RegisterHotKey(None, ID, HOT_KEY_MODIFIERS(bits | MOD_NOREPEAT), vk).is_err() {
        hub.push_ui("error", format!("热键注册失败（可能被占用）：{spec}"));
        return;
      }
      hub.push_ui("info", format!("热键已注册：{spec}"));
      let mut msg = MSG::default();
      while !stop.load(Relaxed) {
        while PeekMessageW(&mut msg, None, 0, 0, PM_REMOVE).as_bool() {
          if msg.message == WM_HOTKEY && msg.wParam.0 as i32 == ID {
            on_trigger();
          }
        }
        std::thread::sleep(Duration::from_millis(20));
      }
      let _ = UnregisterHotKey(None, ID);
    })
    .ok()
}
