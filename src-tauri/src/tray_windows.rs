// PureVox — AI 麦克风降噪工具
// Copyright (C) 2024-2026 a2heng <752848283@qq.com>
//
// PureVox is licensed under the GNU General Public License v3.0 or
// later (GPL-3.0-or-later).  See LICENSE for details.
//
// SPDX-License-Identifier: GPL-3.0-or-later

//! Windows 系统托盘。
//!
//! - `tray-icon` 负责图标与菜单（它依赖外部消息循环，GPUI 的 win32 循环正好在跑）；
//! - 菜单事件由 GPUI 的 80 ms tick 调用 [`poll`] 取走，不另开线程；
//! - 主窗口 WndProc 子类化只拦 `WM_SYSCOMMAND / SC_MINIMIZE`，改成隐藏到托盘；
//!   关闭仍由 GPUI 的 `on_window_should_close` 决定（见 `main.rs`）。

use std::cell::RefCell;
use std::sync::atomic::{AtomicBool, AtomicIsize, Ordering};

use windows::Win32::Foundation::{HWND, LPARAM, LRESULT, WPARAM};
use windows::Win32::UI::WindowsAndMessaging::{
  CallWindowProcW, DefWindowProcW, GWLP_WNDPROC, IsWindowVisible, PostMessageW, SC_MINIMIZE,
  SW_HIDE, SW_SHOW, SetForegroundWindow, SetWindowLongPtrW, ShowWindow, WM_CLOSE, WM_SYSCOMMAND,
  WNDPROC,
};

use crate::debug::{SharedHub, UiCommand};

static MAIN_HWND: AtomicIsize = AtomicIsize::new(0);
static OLD_WNDPROC: AtomicIsize = AtomicIsize::new(0);
static QUITTING: AtomicBool = AtomicBool::new(false);
static HUB: std::sync::OnceLock<SharedHub> = std::sync::OnceLock::new();

thread_local! {
  static TRAY: RefCell<Option<tray_icon::TrayIcon>> = const { RefCell::new(None) };
}

fn hwnd() -> HWND {
  HWND(MAIN_HWND.load(Ordering::Relaxed) as *mut core::ffi::c_void)
}

/// 建托盘。必须在 GPUI 主线程、消息循环所在线程调用。
pub fn spawn(main_hwnd: isize, hub: SharedHub) {
  if MAIN_HWND.load(Ordering::Relaxed) != 0 {
    return;
  }
  let _ = HUB.set(hub.clone());
  tray_icon::menu::MenuEvent::set_event_handler(Some(|event: tray_icon::menu::MenuEvent| {
    on_menu(event.id().0.as_str());
  }));

  use tray_icon::menu::{Menu, MenuItem};
  let show = MenuItem::with_id("show", "显示 / 隐藏", true, None);
  let engine = MenuItem::with_id("engine", "启动 / 停止引擎", true, None);
  let quit = MenuItem::with_id("quit", "退出 PureVox", true, None);
  let Ok(menu) = Menu::with_items(&[&show, &engine, &quit]) else {
    hub.push_ui("warn", "托盘：菜单创建失败".into());
    return;
  };

  let icon = image::load_from_memory(include_bytes!("../icons/32x32.png"))
    .ok()
    .and_then(|img| {
      let rgba = img.to_rgba8();
      tray_icon::Icon::from_rgba(rgba.as_raw().clone(), rgba.width(), rgba.height()).ok()
    });

  let mut builder = tray_icon::TrayIconBuilder::new()
    .with_menu(Box::new(menu))
    .with_menu_on_left_click(true)
    .with_tooltip("PureVox");
  if let Some(icon) = icon {
    builder = builder.with_icon(icon);
  }
  match builder.build() {
    Ok(tray) => {
      TRAY.with(|t| *t.borrow_mut() = Some(tray));
      // 只有托盘真的建起来了，才允许「关闭 = 隐藏」；否则关掉窗口就找不回来了。
      MAIN_HWND.store(main_hwnd, Ordering::Relaxed);
      subclass_main_window(main_hwnd);
    }
    Err(e) => hub.push_ui("warn", format!("托盘创建失败（关闭窗口将正常退出）：{e}")),
  }
}

fn on_menu(id: &str) {
  match id {
    "show" => toggle_main(),
    "engine" => {
      if let Some(hub) = HUB.get() {
        hub.push_ui_command(UiCommand::ToggleEngine);
      }
    }
    "quit" => {
      QUITTING.store(true, Ordering::Relaxed);
      let _ = unsafe { PostMessageW(Some(hwnd()), WM_CLOSE, WPARAM(0), LPARAM(0)) };
    }
    _ => {}
  }
}

/// 把主窗口藏进托盘；没有托盘（非 Windows / 创建失败）返回 false。
pub fn hide_main() -> bool {
  if MAIN_HWND.load(Ordering::Relaxed) == 0 {
    return false;
  }
  let _ = unsafe { ShowWindow(hwnd(), SW_HIDE) };
  true
}

pub fn quitting() -> bool {
  QUITTING.load(Ordering::Relaxed)
}

fn toggle_main() {
  if MAIN_HWND.load(Ordering::Relaxed) == 0 {
    return;
  }
  let h = hwnd();
  if unsafe { IsWindowVisible(h) }.as_bool() {
    let _ = unsafe { ShowWindow(h, SW_HIDE) };
  } else {
    let _ = unsafe { ShowWindow(h, SW_SHOW) };
    let _ = unsafe { SetForegroundWindow(h) };
  }
}

fn subclass_main_window(main_hwnd: isize) {
  if OLD_WNDPROC.load(Ordering::Relaxed) != 0 {
    return;
  }
  let old = unsafe {
    SetWindowLongPtrW(
      HWND(main_hwnd as *mut core::ffi::c_void),
      GWLP_WNDPROC,
      main_wndproc as *const () as usize as isize,
    )
  };
  OLD_WNDPROC.store(old, Ordering::Relaxed);
}

unsafe extern "system" fn main_wndproc(
  h: HWND,
  msg: u32,
  wparam: WPARAM,
  lparam: LPARAM,
) -> LRESULT {
  // 最小化（-）不进任务栏，直接隐藏到托盘。关闭（x）由 GPUI 的回调处理。
  if msg == WM_SYSCOMMAND && (wparam.0 as u32 & 0xFFF0) == SC_MINIMIZE {
    let _ = unsafe { ShowWindow(h, SW_HIDE) };
    return LRESULT(0);
  }
  let old = OLD_WNDPROC.load(Ordering::Relaxed);
  if old != 0 {
    let proc: WNDPROC = unsafe { std::mem::transmute(old) };
    unsafe { CallWindowProcW(proc, h, msg, wparam, lparam) }
  } else {
    unsafe { DefWindowProcW(h, msg, wparam, lparam) }
  }
}
