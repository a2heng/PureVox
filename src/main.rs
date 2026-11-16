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

// release 版不弹控制台窗口（Windows）
#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]
// 迁移期：实时麦克风引擎 / 更新 / 热键等模块仍在仓库里，但界面还没接回去。
// 这些「未接线」告警会盖住真正的问题；阶段 B 接完删掉本行（AGENTS.md 维护准则）。
#![allow(dead_code)]

mod audio;
mod autostart;
mod config;
mod cues;
mod debug;
mod devices;
mod dsp;
mod engine;
mod hotkey;
mod infer;
mod media;
mod net;
#[cfg(windows)]
#[path = "openurl_windows.rs"]
mod openurl;
#[cfg(target_os = "linux")]
#[path = "openurl_linux.rs"]
mod openurl;
#[cfg(not(any(windows, target_os = "linux")))]
#[path = "openurl_other.rs"]
mod openurl;
mod plan;
mod recorder;
mod rt;
mod single_instance;
#[cfg(windows)]
#[path = "sound_panel_windows.rs"]
mod sound_panel;
#[cfg(target_os = "linux")]
#[path = "sound_panel_linux.rs"]
mod sound_panel;
#[cfg(not(any(windows, target_os = "linux")))]
#[path = "sound_panel_other.rs"]
mod sound_panel;
mod tray;
mod ui;
mod update;
#[cfg(windows)]
#[path = "vb_panel_windows.rs"]
mod vb_panel;
#[cfg(not(windows))]
#[path = "vb_panel_other.rs"]
mod vb_panel;
mod version;
mod wav;

use std::sync::Arc;

use audio::AudioManager;
use debug::{DebugHub, SharedHub};
use gpui_kit::component::{Theme, ThemeMode};
use gpui_kit::prelude::*;
use gpui_kit::{Bounds, TitlebarOptions, WindowBounds, WindowOptions, point, px, size};

/// 主窗口默认几何（第一次启动 / 还没记住几何时）。
const DEFAULT_W: f32 = 1280.0;
const DEFAULT_H: f32 = 800.0;

#[cfg(windows)]
fn native_scale() -> f32 {
  // 旧配置存的是物理像素，按系统 DPI 换算一次；新配置带 logical=true 直接是逻辑像素。
  unsafe { windows::Win32::UI::HiDpi::GetDpiForSystem() as f32 / 96.0 }
}

#[cfg(not(windows))]
fn native_scale() -> f32 {
  1.0
}

/// 主窗口的托盘句柄（窗口被移除后，重开窗口还要用它建菜单）。
static UI_HUB: std::sync::OnceLock<SharedHub> = std::sync::OnceLock::new();

/// 主窗口几何 / 标题栏选项。Windows 与 Linux 都不使用系统标题栏：
/// 由 `ui::title_bar()` 自绘（拖动 / 最小化 / 最大化 / 关闭），两边行为一致。
fn main_window_options(cx: &mut gpui_kit::App) -> WindowOptions {
  let settings = crate::config::load_settings();
  let scale = native_scale();
  let (bounds, maximized) = match settings.window {
    Some(g) => {
      let s = if g.logical { 1.0 } else { scale.max(0.5) };
      let b = Bounds::new(
        point(px(g.x as f32 / s), px(g.y as f32 / s)),
        size(
          px((g.w as f32 / s).max(900.0)),
          px((g.h as f32 / s).max(560.0)),
        ),
      );
      (b, g.maximized)
    }
    None => (
      Bounds::centered(None, size(px(DEFAULT_W), px(DEFAULT_H)), cx),
      false,
    ),
  };
  let mut options = WindowOptions {
    titlebar: Some(TitlebarOptions {
      title: Some(format!("PureVox {}", version::VERSION).into()),
      // 只有自绘标题栏的平台才隐藏系统标题栏；其他平台保持系统原样。
      appears_transparent: cfg!(any(windows, target_os = "linux")),
      traffic_light_position: None,
    }),
    // 拖动 / 双击最大化由标题栏自己接管，系统不要再处理一遍。
    app_owns_titlebar_drag: cfg!(any(windows, target_os = "linux")),
    ..Default::default()
  };
  options.window_bounds = Some(if maximized {
    WindowBounds::Maximized(bounds)
  } else {
    WindowBounds::Windowed(bounds)
  });
  options.window_min_size = Some(size(px(900.), px(560.)));
  #[cfg(target_os = "linux")]
  {
    // Linux 没有系统标题栏（Wayland 默认客户端装饰）；X11 无合成器时窗口管理器
    // 仍可能给服务端装饰，`title_bar()` 会按实际装饰决定是否画按钮。
    options.window_decorations = Some(gpui_kit::WindowDecorations::Client);
  }
  options
}

/// 每个主窗口都要挂的标题 / 托盘 / 关闭行为。
fn install_window_handlers(window: &mut gpui_kit::Window, cx: &mut gpui_kit::App, hub: SharedHub) {
  window.set_window_title(&format!("PureVox {}", version::VERSION));
  #[cfg(windows)]
  {
    let hwnd = raw_window_handle::HasWindowHandle::window_handle(window)
      .ok()
      .and_then(|h| match h.as_raw() {
        raw_window_handle::RawWindowHandle::Win32(w) => Some(w.hwnd.get()),
        _ => None,
      });
    if let Some(hwnd) = hwnd {
      tray::spawn(hwnd, hub);
    }
  }
  #[cfg(not(windows))]
  tray::spawn(0, hub);
  // 关闭（x）：Windows 收进托盘；Linux 允许窗口真正移除（隐藏到托盘），
  // 主视图仍由 `ui::MAIN_VIEW` 持有，托盘「显示窗口」再重开。退出只走托盘菜单。
  window.on_window_should_close(cx, |window, _cx| {
    let _ = crate::config::save_window_geom(crate::config::geom_from_window(window));
    #[cfg(windows)]
    {
      if tray::quitting() {
        true
      } else {
        !tray::hide_main()
      }
    }
    #[cfg(not(windows))]
    {
      true
    }
  });
}

/// 打开主窗口：第一次创建 `Workspace`，之后复用同一个视图。
fn open_main_window(
  cx: &mut gpui_kit::App,
  audio: Arc<AudioManager>,
  hub: SharedHub,
) -> Result<(), String> {
  let _ = UI_HUB.set(hub.clone());
  let existing = ui::current_view();
  let options = main_window_options(cx);
  cx.open_window(options, move |window, cx| {
    install_window_handlers(window, cx, hub.clone());
    match existing {
      Some(view) => {
        let focus = view.read(cx).focus_handle();
        window.focus(&focus, cx);
        view
      }
      None => {
        let view = cx.new(|cx| ui::Workspace::new(audio.clone(), hub.clone(), window, cx));
        ui::remember_view(view.clone());
        view
      }
    }
  })
  .map(|_| ())
  .map_err(|e| e.to_string())
}

/// 托盘「显示窗口」：有窗口就激活，没有就重开（复用同一个 Workspace 视图）。
pub(crate) fn reopen_main_window(cx: &mut gpui_kit::App) {
  if let Some(handle) = cx.windows().first().copied() {
    let _ = cx.update_window(handle, |_, window, _| window.activate_window());
    return;
  }
  let Some(view) = ui::current_view() else {
    return;
  };
  let Some(hub) = UI_HUB.get().cloned() else {
    return;
  };
  let options = main_window_options(cx);
  if let Err(e) = cx.open_window(options, move |window, cx| {
    install_window_handlers(window, cx, hub.clone());
    let focus = view.read(cx).focus_handle();
    window.focus(&focus, cx);
    view
  }) {
    eprintln!("重新打开主窗口失败：{e}");
  }
}

/// 把主窗口收进托盘（Linux 直接移除窗口，主视图仍由 `MAIN_VIEW` 持有）。
pub(crate) fn hide_main_window(cx: &mut gpui_kit::App) -> bool {
  #[cfg(windows)]
  {
    let _ = cx;
    tray::hide_main()
  }
  #[cfg(not(windows))]
  {
    let Some(handle) = cx.windows().first().copied() else {
      return false;
    };
    cx.update_window(handle, |_, window, _| {
      let _ = crate::config::save_window_geom(crate::config::geom_from_window(window));
      window.remove_window();
    })
    .is_ok()
  }
}

fn main() {
  // 单实例：第二个进程只请求已有窗口显示，然后退出（不启动第二套引擎 / 端口）。
  if !single_instance::claim() {
    eprintln!("PureVox 已在运行，本进程退出");
    return;
  }
  // panic 也进事件日志：release 没有控制台，后台线程 panic 不能「只消失不报」。
  debug::events::install_panic_hook();
  let hub = DebugHub::new();
  // 上次校准报告是**数据**：启动就发布，曲线/数值不空白（不自动回填，避免覆盖手工改过的参数）。
  if let Some(report) = engine::calib::load_last_report() {
    hub.set_calib_report(debug::Probe::ok(report));
  }
  debug::system::spawn_sampler(hub.clone());
  debug::http::spawn(hub.clone());
  devices::spawn_refresh(hub.clone());
  devices::spawn_watch(hub.clone());
  let audio = Arc::new(AudioManager::new(hub.clone()));

  // 模型：打包后 `models/` 与可执行文件同级（新打包链，不再有 Tauri 资源目录）。
  if let Ok(exe) = std::env::current_exe()
    && let Some(dir) = exe.parent()
  {
    let models = dir.join("models");
    if models.is_dir() {
      infer::set_model_dir(models);
    }
  }
  infer::preload_all();
  net::load_settings();
  debug::stall::spawn_watchdog_default();
  match debug::stall::opt_out_of_throttling() {
    Ok(()) => {}
    Err(e) => hub.push_ui("warn", e),
  }

  // 无人值守验证钩子（不设环境变量则完全不生效）。
  if std::env::var("PUREVOX_AUTONET").is_ok() {
    match net::start() {
      Ok(m) => hub.push_ui("info", format!("AUTONET 启动网络服务：{m}")),
      Err(e) => hub.push_ui("error", format!("AUTONET 启动网络服务失败：{e}")),
    }
  }
  if let Some(ms) = std::env::var("PUREVOX_AUTOCALIB_MS")
    .ok()
    .and_then(|s| s.parse::<u64>().ok())
  {
    let mgr = audio.clone();
    let hub2 = hub.clone();
    std::thread::spawn(move || {
      if let Err(e) = mgr.start() {
        hub2.push_ui("error", format!("AUTOCALIB 启动会话失败：{e}"));
        return;
      }
      std::thread::sleep(std::time::Duration::from_millis(ms));
      match mgr.calibrate_aec_delay(None) {
        Ok((t, _)) => hub2.push_ui("info", format!("AUTOCALIB：{t}")),
        Err(e) => hub2.push_ui("error", format!("AUTOCALIB：{e}")),
      }
    });
  }
  if std::env::var("PUREVOX_AUTODEBUG").is_ok() {
    std::thread::spawn(move || {
      std::thread::sleep(std::time::Duration::from_millis(800));
      let url = format!("http://127.0.0.1:{}/debug/ui", debug::http_port());
      if let Err(e) = openurl::open(&url) {
        eprintln!("AUTODEBUG 打开调试面板失败：{e}");
      }
    });
  }

  let hub_for_ui: SharedHub = hub.clone();
  let audio_for_ui = audio.clone();
  gpui_kit::application()
    .with_assets(gpui_kit::assets::Assets)
    // 关闭最后一个窗口不退出：Linux 收进托盘后仍要保留进程与引擎（Explicit = 只认 cx.quit）。
    .with_quit_mode(gpui_kit::QuitMode::Explicit)
    .run(move |cx| {
      // 组件库（主题 / 图标 / 本地化）必须先初始化；本应用只有深色界面，
      // 不跟随系统亮色（否则组件默认前景是深色，落在深色卡片上就是「黑字看不见」）。
      gpui_kit::init(cx);
      Theme::change(ThemeMode::Dark, None, cx);
      cx.bind_keys(ui::key_bindings());
      cx.on_window_closed(|cx, _window_id| {
        // Linux：关闭窗口 = 收进托盘，主视图由 `ui::MAIN_VIEW` 持有；
        // 真正退出走托盘菜单（`UiCommand::Quit` 直接 `cx.quit()`）。
        #[cfg(not(target_os = "linux"))]
        if cx.windows().is_empty() {
          cx.quit();
        }
        #[cfg(target_os = "linux")]
        let _ = cx;
      })
      .detach();

      let settings = crate::config::load_settings();
      let hub = hub_for_ui.clone();
      let audio = audio_for_ui.clone();
      if let Err(e) = open_main_window(cx, audio, hub) {
        eprintln!("打开主窗口失败：{e}");
        cx.quit();
        return;
      }
      cx.activate(true);

      // 「开机自启（静默运行）」：登录后自动跑引擎并收进托盘——与 Tauri 版的
      // `start_hidden` 同一行为，设置页只有**一个**开关同时控制自启与静默。
      if settings.start_hidden {
        let audio = audio_for_ui.clone();
        let hub = hub_for_ui.clone();
        match audio.start() {
          Ok(_) => crate::tray::set_running(true),
          Err(e) => hub.push_ui("warn", format!("静默启动引擎失败：{e}")),
        }
        if hide_main_window(cx) {
          hub.push_ui("info", "已静默启动：引擎在跑，窗口收进托盘".into());
        }
      } else {
        crate::tray::set_running(audio_for_ui.is_running());
      }

      // 无人值守验证：`PUREVOX_MIX_AUTOSTART=1` 在窗口起来 1.2 s 后自动启动引擎。
      if std::env::var("PUREVOX_MIX_AUTOSTART").is_ok() {
        let audio = audio_for_ui.clone();
        std::thread::spawn(move || {
          std::thread::sleep(std::time::Duration::from_millis(1200));
          if let Err(e) = audio.start() {
            eprintln!("AUTOSTART 启动引擎失败：{e}");
          }
        });
      }
    });
}
