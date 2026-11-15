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
//! - 左键单击 = 显示 / 隐藏主窗口，右键 = 菜单（菜单事件同样走主线程回调）；
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
/// 引擎是否在运行：托盘图标据此在「亮 / 暗」两张图之间切换。
static RUNNING: AtomicBool = AtomicBool::new(false);
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
  tray_icon::TrayIconEvent::set_event_handler(Some(|event: tray_icon::TrayIconEvent| {
    use tray_icon::{MouseButton, MouseButtonState, TrayIconEvent};
    // 左键抬起才切换：Windows 会先给 Down 再给 Up，只认 Up 避免一次点击切两次。
    if let TrayIconEvent::Click {
      button: MouseButton::Left,
      button_state: MouseButtonState::Up,
      ..
    } = event
    {
      toggle_main();
    }
  }));

  use tray_icon::menu::{Menu, MenuItem};
  let show = MenuItem::with_id("show", "显示 / 隐藏", true, None);
  let engine = MenuItem::with_id("engine", "启动 / 停止引擎", true, None);
  let quit = MenuItem::with_id("quit", "退出 PureVox", true, None);
  let Ok(menu) = Menu::with_items(&[&show, &engine, &quit]) else {
    hub.push_ui("warn", "托盘：菜单创建失败".into());
    return;
  };

  let icon = icon_for(RUNNING.load(Ordering::Relaxed));

  let mut builder = tray_icon::TrayIconBuilder::new()
    .with_menu(Box::new(menu))
    // 左键自己处理成显示/隐藏，菜单只在右键弹出。
    .with_menu_on_left_click(false)
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
  // 第二个进程启动时广播「显示已有窗口」：恢复被收进托盘的窗口并抢到前台。
  let activate = crate::single_instance::activate_message();
  if activate != 0 && msg == activate {
    let _ = unsafe { ShowWindow(h, SW_SHOW) };
    let _ = unsafe { SetForegroundWindow(h) };
    return LRESULT(0);
  }
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

/// 托盘图标：运行中 = 亮（`tray_running.png`），已停止 = 暗（`tray_stopped.png`）。
fn icon_for(on: bool) -> Option<tray_icon::Icon> {
  let bytes: &[u8] = if on {
    include_bytes!("../icons/tray_running.png")
  } else {
    include_bytes!("../icons/tray_stopped.png")
  };
  image::load_from_memory(bytes).ok().and_then(|img| {
    let rgba = img.to_rgba8();
    tray_icon::Icon::from_rgba(rgba.as_raw().clone(), rgba.width(), rgba.height()).ok()
  })
}

/// 引擎启停时换托盘图标（明 = 运行中，暗 = 已停止）；托盘还没建好时只记状态。
pub fn set_running(on: bool) {
  RUNNING.store(on, Ordering::Relaxed);
  TRAY.with(|t| {
    if let Some(tray) = t.borrow().as_ref() {
      let _ = tray.set_icon(icon_for(on));
    }
  });
}
