// PureVox — AI 麦克风降噪工具
// Copyright (C) 2024-2026 a2heng <752848283@qq.com>
//
// PureVox is licensed under the GNU General Public License v3.0 or
// later (GPL-3.0-or-later).  See LICENSE for details.
//
// SPDX-License-Identifier: GPL-3.0-or-later

//! Linux 托盘：ksni（StatusNotifierItem / D-Bus），不引 GTK。
//!
//! GNOME 需要 AppIndicator 扩展；托盘菜单「显示窗口」或左键点击图标会把主窗口
//! 拉到前台 / 再点一次隐藏。关闭窗口直接移除（真正的托盘隐藏），进程与引擎继续跑，
//! 退出走托盘菜单。

use std::sync::OnceLock;
use std::sync::atomic::{AtomicBool, Ordering};

use crate::debug::{SharedHub, UiCommand};

static HUB: OnceLock<SharedHub> = OnceLock::new();
/// 托盘只建一次：窗口关掉再重开（同一进程）不重复注册 SNI。
static SPAWNED: AtomicBool = AtomicBool::new(false);

fn on_menu(id: &str) {
  let Some(hub) = HUB.get() else {
    return;
  };
  match id {
    "show" => hub.push_ui_command(UiCommand::ShowWindow),
    "toggle" => hub.push_ui_command(UiCommand::ToggleWindow),
    "engine" => hub.push_ui_command(UiCommand::ToggleEngine),
    "quit" => hub.push_ui_command(UiCommand::Quit),
    _ => {}
  }
}

pub fn spawn(_hwnd: isize, hub: SharedHub) {
  if SPAWNED.swap(true, Ordering::Relaxed) {
    return;
  }
  let _ = HUB.set(hub.clone());
  tray_icon::menu::MenuEvent::set_event_handler(Some(|event: tray_icon::menu::MenuEvent| {
    on_menu(event.id().0.as_str());
  }));
  // SNI 的左键 Activate（GNOME 的 AppIndicator 里通常直接打开菜单，但 KDE 等会发 Activate）。
  tray_icon::TrayIconEvent::set_event_handler(Some(|event: tray_icon::TrayIconEvent| {
    if let tray_icon::TrayIconEvent::Click {
      button: tray_icon::MouseButton::Left,
      button_state: tray_icon::MouseButtonState::Up,
      ..
    } = event
    {
      on_menu("toggle");
    }
  }));

  use tray_icon::menu::{Menu, MenuItem};
  let show = MenuItem::with_id("show", "显示窗口", true, None);
  let engine = MenuItem::with_id("engine", "启动 / 停止引擎", true, None);
  let quit = MenuItem::with_id("quit", "退出 PureVox", true, None);
  let Ok(menu) = Menu::with_items(&[&show, &engine, &quit]) else {
    hub.push_ui("warn", "托盘：菜单创建失败".into());
    return;
  };

  // 应用图标（32×32 RGBA，构建时从 icons/32x32.png 转出，运行时不解码图片）。
  let icon = tray_icon::Icon::from_rgba(include_bytes!("../icons/tray.rgba").to_vec(), 32, 32).ok();
  let mut builder = tray_icon::TrayIconBuilder::new()
    .with_menu(Box::new(menu))
    .with_title("PureVox")
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

/// Linux 没有统一 hide 协议：关闭窗口由 `main.rs` 直接移除，这里不再使用。
#[allow(dead_code)]
pub fn hide_main() -> bool {
  false
}

pub fn quitting() -> bool {
  false
}

/// Linux 托盘由 ksni 管理，暂不按运行状态换图标（与 Windows 不同）：保持同一套调用点。
pub fn set_running(_on: bool) {}
