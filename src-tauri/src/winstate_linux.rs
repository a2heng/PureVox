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

//! Linux（GTK）：几何走 Tauri 的通用 API，但**尺寸量 webview 客户端区**，不量 `inner_size()`。
//!
//! 为什么不能用 `inner_size()`：tao 在 Linux 上把 GTK `configure-event` 的窗口尺寸当作
//! `inner_size`，**包含 CSD 标题栏与边框**（本机 2x 缩放实测多出 52×89 逻辑像素）；
//! 而 `set_size()` 通过 `gtk_window_resize` 设的是**客户端区**。读大、写小 → 每保存一次
//! 窗口就凭空长一圈，下次启动再长一圈（本机实测 1200×800 → 1652×1008 → 1756×1186）。
//! `Webview::size()`（wry `bounds()`）在 Wayland 下取 `allocated_size()`、X11 下取
//! `XGetWindowAttributes`，两种会话都是 webview 客户端区，与 `set_size()` 同一坐标系。
//!
//! **不做任何「跟显示器比对」的推断**（夹尺寸 / 猜最大化）：窗口能不能放下、要不要按
//! 最大化处理，交给合成器；这里只如实读写「是什么」。GTK 没有 `WINDOWPLACEMENT` 那套
//! 「还原尺寸」概念，最大化时读到的就是整屏，退化处理见 `winstate::spawn_saver`：
//! 最大化时只翻标记、不覆盖尺寸。
//!
//! 位置在 **Wayland 下不可控**：合成器不告诉应用窗口在哪，`outer_position()` 恒为 (0,0)，
//! `set_position()` 也基本无效 —— 位置记忆实际只在 X11 会话里生效。

use tauri::WebviewWindow;

use crate::config::WindowGeom;

pub fn read(win: &WebviewWindow) -> Option<WindowGeom> {
  if win.is_minimized().unwrap_or(false) || !win.is_visible().unwrap_or(true) {
    return None;
  }
  let pos = win.outer_position().ok()?;
  let webview: &tauri::Webview<_> = win.as_ref();
  let size = webview.size().ok()?;
  if size.width == 0 || size.height == 0 {
    return None;
  }
  Some(WindowGeom {
    x: pos.x,
    y: pos.y,
    w: size.width,
    h: size.height,
    maximized: win.is_maximized().unwrap_or(false),
  })
}

pub fn apply(win: &WebviewWindow, g: WindowGeom) {
  let _ = win.set_size(tauri::PhysicalSize::new(g.w.max(320), g.h.max(240)));
  let _ = win.set_position(tauri::PhysicalPosition::new(g.x, g.y));
}
