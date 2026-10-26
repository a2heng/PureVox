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

//! 主窗口几何记忆：启动时恢复上次的位置与大小，运行中定时把变化写回设置。
//!
//! 平台差异只体现在「怎么读写几何」这一层：
//! - Windows（[`winstate_windows`]）：`GetWindowPlacement` / `SetWindowPlacement`
//!   —— 拿得到**还原外框**和 `showCmd`，最大化时也不会把尺寸记成整屏；
//! - Linux / 其它（[`winstate_linux`] / [`winstate_other`]）：Tauri 的
//!   `outer_position` + `inner_size`，最大化时只翻标记、保留上一次的还原尺寸。
//!
//! 三条纪律：
//! - **存物理像素**（见 [`crate::config::WindowGeom`]），跨缩放比不会漂；
//! - **恢复时必须夹进当前显示器**：上次在副屏（或已拔掉的显示器）上的坐标不能把窗口
//!   丢到看不见的地方（Windows 不会帮你救回来，用户只会觉得「启动就没了」）；
//! - **最大化时不覆盖尺寸**：最大化状态下量到的尺寸是整屏，照抄就永久毁了用户的还原尺寸。

#[cfg(windows)]
#[path = "winstate_windows.rs"]
mod platform;
#[cfg(target_os = "linux")]
#[path = "winstate_linux.rs"]
mod platform;
#[cfg(not(any(windows, target_os = "linux")))]
#[path = "winstate_other.rs"]
mod platform;

use std::time::Duration;

use tauri::{Manager, WebviewWindow};

use crate::config::{self, WindowGeom};

/// 轮询周期：拖动/缩放会在几百毫秒内产生几十个事件，没必要逐事件写盘；
/// 1.5 s 一次足够，进程被强杀时最多丢最后 1.5 s 的移动。
const SAVE_EVERY_MS: u64 = 1500;

/// 恢复上次保存的几何（在 `setup` 里、窗口显示之前调用）。
pub fn restore(win: &WebviewWindow) {
  let Some(g) = config::load_settings().window else {
    return;
  };
  let g = clamp_into_monitors(win, g);
  platform::apply(win, g);
  if g.maximized {
    let _ = win.maximize();
  }
}

/// 后台把窗口几何写回设置（变化时才写，避免每 1.5 s 一次磁盘写）。
pub fn spawn_saver(app: tauri::AppHandle) {
  std::thread::Builder::new()
    .name("win-geom".into())
    .spawn(move || {
      let mut saved = config::load_settings().window;
      loop {
        std::thread::sleep(Duration::from_millis(SAVE_EVERY_MS));
        let Some(win) = app.get_webview_window("main") else {
          continue;
        };
        let Some(next) = platform::read(&win) else {
          continue;
        };
        // 最大化时读到的尺寸是整屏（Linux/其它平台）：保留上次的还原尺寸，只翻标记。
        // Windows 给的是 rcNormalPosition，本来就已经是还原尺寸，这里同样不会丢信息。
        let next = if next.maximized {
          match saved {
            Some(mut prev) => {
              prev.maximized = true;
              prev
            }
            None => next,
          }
        } else {
          next
        };
        if Some(next) == saved {
          continue;
        }
        match config::save_window_geom(next) {
          Ok(()) => saved = Some(next),
          Err(e) => crate::debug::events::log("error", "window", format!("保存窗口位置失败：{e}")),
        }
      }
    })
    .map_err(|e| crate::debug::events::log("error", "window", format!("窗口几何线程起不来：{e}")))
    .ok();
}

/// 把保存的几何夹进当前某块显示器的范围。
///
/// 判据：**窗口左上角**必须落在某块屏幕里（留 64px 让它至少露出一角）。
/// 都不落 → 用主显示器居中，尺寸不超过该屏。
fn clamp_into_monitors(win: &WebviewWindow, g: WindowGeom) -> WindowGeom {
  let monitors = win.available_monitors().unwrap_or_default();
  let mut w = g.w.max(320);
  let mut h = g.h.max(240);
  let visible = monitors.iter().any(|m| {
    let p = m.position();
    let s = m.size();
    g.x >= p.x
      && g.y >= p.y
      && g.x + 64 <= p.x + s.width as i32
      && g.y + 64 <= p.y + s.height as i32
  });
  if visible {
    return WindowGeom { w, h, ..g };
  }
  let primary = win
    .primary_monitor()
    .ok()
    .flatten()
    .or_else(|| monitors.first().cloned());
  let (x, y) = match primary {
    Some(m) => {
      let p = m.position();
      let s = m.size();
      w = w.min(s.width);
      h = h.min(s.height);
      (
        p.x + (s.width as i32 - w as i32) / 2,
        p.y + (s.height as i32 - h as i32) / 2,
      )
    }
    None => (g.x, g.y),
  };
  WindowGeom { x, y, w, h, ..g }
}
