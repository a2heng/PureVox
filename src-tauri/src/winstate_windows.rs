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

//! Windows：窗口几何走 Win32 的 `GetWindowPlacement` / `SetWindowPlacement`。
//!
//! **为什么不用 Tauri 的 `inner_size` / `outer_position`**：那两个量在窗口最大化时
//! 给的是整屏尺寸，照抄就把用户的「还原尺寸」永久写成整屏（点还原也回不去）；
//! `WINDOWPLACEMENT.rcNormalPosition` 才是系统记着的**还原外框**，`showCmd` 顺带告诉我
//! 是不是最大化 —— 这正是我们要存的那两件事，且外框语义自洽（存与还原都用外框，
//! 不会因为边框宽度在「保存→恢复」之间反复缩水）。
//!
//! 坐标是 workspace 物理像素（本进程 per-monitor DPI aware，与 Tauri 的 monitor 坐标同一套）。

use std::mem::size_of;

use tauri::WebviewWindow;
use windows::Win32::Foundation::{HWND, POINT, RECT};
use windows::Win32::UI::WindowsAndMessaging::{
  GetWindowPlacement, SW_HIDE, SW_SHOWMAXIMIZED, SetWindowPlacement, WINDOWPLACEMENT,
  WINDOWPLACEMENT_FLAGS,
};

use crate::config::WindowGeom;

fn raw(win: &WebviewWindow) -> Option<HWND> {
  win.hwnd().ok().map(|h| HWND(h.0))
}

fn placement(h: HWND) -> Option<WINDOWPLACEMENT> {
  let mut wp = WINDOWPLACEMENT {
    length: size_of::<WINDOWPLACEMENT>() as u32,
    ..Default::default()
  };
  unsafe { GetWindowPlacement(h, &mut wp).ok()? };
  Some(wp)
}

/// 读当前几何（还原外框 + 是否最大化）。最小化 / 隐藏 / 尺寸为 0 时不记。
pub fn read(win: &WebviewWindow) -> Option<WindowGeom> {
  if win.is_minimized().unwrap_or(false) || !win.is_visible().unwrap_or(true) {
    return None;
  }
  let wp = placement(raw(win)?)?;
  let r = wp.rcNormalPosition;
  let w = (r.right - r.left).max(0) as u32;
  let h = (r.bottom - r.top).max(0) as u32;
  if w == 0 || h == 0 {
    return None;
  }
  Some(WindowGeom {
    x: r.left,
    y: r.top,
    w,
    h,
    maximized: wp.showCmd == SW_SHOWMAXIMIZED.0 as u32,
  })
}

/// 摆好位置与还原尺寸（**不显示窗口**：显示/最大化由调用方决定，见 `winstate::restore`）。
pub fn apply(win: &WebviewWindow, g: WindowGeom) {
  let Some(h) = raw(win) else {
    return;
  };
  let wp = WINDOWPLACEMENT {
    length: size_of::<WINDOWPLACEMENT>() as u32,
    flags: WINDOWPLACEMENT_FLAGS(0),
    showCmd: SW_HIDE.0 as u32,
    ptMinPosition: POINT::default(),
    ptMaxPosition: POINT::default(),
    rcNormalPosition: RECT {
      left: g.x,
      top: g.y,
      right: g.x + g.w as i32,
      bottom: g.y + g.h as i32,
    },
  };
  unsafe {
    let _ = SetWindowPlacement(h, &wp);
  }
}
