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

use std::collections::HashMap;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering::Relaxed};
use std::thread::JoinHandle;
use std::time::Duration;

use crate::debug::SharedHub;

const MOD_NOREPEAT: u32 = 0x4000;

/// 起始 id（后续按序号递增；一个线程管一组绑定）。
const ID_BASE: i32 = 0x5056;

/// 启动注册线程；注册成功返回其 JoinHandle，失败返回 None（结果写进调试接口）。
///
/// 消息队列里 `wParam` 就是 `RegisterHotKey` 的 id → 按 id 派发到对应回调。
/// 某一键注册失败只跳过那一条（写进 `ui`），不影响其余绑定。
pub(super) fn spawn(
  bindings: Vec<super::Resolved>,
  hub: SharedHub,
  stop: Arc<AtomicBool>,
) -> Option<JoinHandle<()>> {
  std::thread::Builder::new()
    .name("hotkey".into())
    .spawn(move || unsafe {
      use windows::Win32::UI::Input::KeyboardAndMouse::{
        HOT_KEY_MODIFIERS, RegisterHotKey, UnregisterHotKey,
      };
      use windows::Win32::UI::WindowsAndMessaging::{MSG, PM_REMOVE, PeekMessageW, WM_HOTKEY};
      let mut dispatch: HashMap<i32, Arc<dyn Fn() + Send + Sync>> = HashMap::new();
      let mut registered: Vec<i32> = Vec::new();
      for (i, (spec, bits, vk, cb)) in bindings.into_iter().enumerate() {
        let id = ID_BASE + i as i32;
        if RegisterHotKey(None, id, HOT_KEY_MODIFIERS(bits | MOD_NOREPEAT), vk).is_err() {
          hub.push_ui("error", format!("热键注册失败（可能被占用）：{spec}"));
          continue;
        }
        hub.push_ui("info", format!("热键已注册：{spec}"));
        registered.push(id);
        dispatch.insert(id, cb);
      }
      let mut msg = MSG::default();
      while !stop.load(Relaxed) {
        while PeekMessageW(&mut msg, None, 0, 0, PM_REMOVE).as_bool() {
          if msg.message == WM_HOTKEY
            && let Some(cb) = dispatch.get(&(msg.wParam.0 as i32))
          {
            cb();
          }
        }
        std::thread::sleep(Duration::from_millis(20));
      }
      for id in registered {
        let _ = UnregisterHotKey(None, id);
      }
    })
    .ok()
}
