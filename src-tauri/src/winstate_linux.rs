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

//! Linux（GTK）：窗口几何走 Tauri 的通用 API（`outer_position` + `inner_size`）。
//!
//! GTK 没有 `WINDOWPLACEMENT` 那套「还原尺寸」概念，最大化时读到的就是整屏；这里的退化
//! 处理见 `winstate::spawn_saver`：最大化时只翻标记、不覆盖尺寸（保存的仍是上一次的还原尺寸）。

use tauri::WebviewWindow;

use crate::config::WindowGeom;

pub fn read(win: &WebviewWindow) -> Option<WindowGeom> {
  if win.is_minimized().unwrap_or(false) || !win.is_visible().unwrap_or(true) {
    return None;
  }
  let pos = win.outer_position().ok()?;
  let size = win.inner_size().ok()?;
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
  let _ = win.set_size(tauri::PhysicalSize::new(g.w, g.h));
  let _ = win.set_position(tauri::PhysicalPosition::new(g.x, g.y));
}
