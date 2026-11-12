// PureVox — AI 麦克风降噪工具
// Copyright (C) 2024-2026 a2heng <752848283@qq.com>
//
// PureVox is licensed under the GNU General Public License v3.0 or
// later (GPL-3.0-or-later).  See LICENSE for details.
//
// SPDX-License-Identifier: GPL-3.0-or-later

//! Linux 托盘：ksni（StatusNotifierItem / D-Bus），不引 GTK。
//!
//! Linux 桌面没有统一的「窗口隐藏到托盘」协议（X11/Wayland 差异大），所以这里只提供
//! **启停引擎 / 退出**；关闭窗口仍按桌面正常退出。`hide_main()` 返回 false 让
//! `main.rs` 的关闭回调走正常关闭路径。

use std::sync::OnceLock;

use crate::debug::{SharedHub, UiCommand};

static HUB: OnceLock<SharedHub> = OnceLock::new();

fn on_menu(id: &str) {
  let Some(hub) = HUB.get() else {
    return;
  };
  match id {
    "engine" => hub.push_ui_command(UiCommand::ToggleEngine),
    "quit" => hub.push_ui_command(UiCommand::Quit),
    _ => {}
  }
}

pub fn spawn(_hwnd: isize, hub: SharedHub) {
  let _ = HUB.set(hub.clone());
  tray_icon::menu::MenuEvent::set_event_handler(Some(|event: tray_icon::menu::MenuEvent| {
    on_menu(event.id().0.as_str());
  }));

  use tray_icon::menu::{Menu, MenuItem};
  let engine = MenuItem::with_id("engine", "启动 / 停止引擎", true, None);
  let quit = MenuItem::with_id("quit", "退出 PureVox", true, None);
  let Ok(menu) = Menu::with_items(&[&engine, &quit]) else {
    hub.push_ui("warn", "托盘：菜单创建失败".into());
    return;
  };

  // 32×32 纯色图标（不引 image/GTK；ksni 只需要 RGBA）。
  let rgba = [0x4fu8, 0x8c, 0xff, 0xff].repeat(32 * 32);
  let icon = tray_icon::Icon::from_rgba(rgba, 32, 32).ok();
  let mut builder = tray_icon::TrayIconBuilder::new()
    .with_menu(Box::new(menu))
    .with_tooltip("PureVox");
  if let Some(icon) = icon {
    builder = builder.with_icon(icon);
  }
  match builder.build() {
    // ksni 自己跑线程；进程退出时一起结束。forget 是刻意的：托盘要活到进程结束。
    Ok(tray) => std::mem::forget(tray),
    Err(e) => hub.push_ui("warn", format!("托盘创建失败：{e}")),
  }
}

pub fn hide_main() -> bool {
  false
}

pub fn quitting() -> bool {
  false
}
