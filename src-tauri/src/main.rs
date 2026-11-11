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
#[cfg(windows)]
#[path = "sound_panel_windows.rs"]
mod sound_panel;
#[cfg(target_os = "linux")]
#[path = "sound_panel_linux.rs"]
mod sound_panel;
#[cfg(not(any(windows, target_os = "linux")))]
#[path = "sound_panel_other.rs"]
mod sound_panel;
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
use gpui_kit::{Bounds, TitlebarOptions, WindowBounds, WindowOptions, px, size};

/// 主窗口默认几何（第一次启动 / 还没记住几何时）。
const DEFAULT_W: f32 = 1280.0;
const DEFAULT_H: f32 = 800.0;

fn main() {
  // panic 也进事件日志：release 没有控制台，后台线程 panic 不能「只消失不报」。
  debug::events::install_panic_hook();
  let hub = DebugHub::new();
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
    .run(move |cx| {
      // 组件库（主题 / 图标 / 本地化）必须先初始化；本应用只有深色界面，
      // 不跟随系统亮色（否则组件默认前景是深色，落在深色卡片上就是「黑字看不见」）。
      gpui_kit::init(cx);
      Theme::change(ThemeMode::Dark, None, cx);
      cx.bind_keys(ui::key_bindings());
      cx.on_window_closed(|cx, _window_id| {
        if cx.windows().is_empty() {
          cx.quit();
        }
      })
      .detach();

      let bounds = Bounds::centered(None, size(px(DEFAULT_W), px(DEFAULT_H)), cx);
      let hub = hub_for_ui.clone();
      let audio = audio_for_ui.clone();
      if let Err(e) = gpui_kit::open_window(
        WindowOptions {
          window_bounds: Some(WindowBounds::Windowed(bounds)),
          window_min_size: Some(size(px(900.), px(560.))),
          titlebar: Some(TitlebarOptions {
            title: Some(format!("PureVox Mix {}", version::VERSION).into()),
            ..Default::default()
          }),
          ..Default::default()
        },
        cx,
        move |window, cx| {
          window.set_window_title(&format!("PureVox Mix {}", version::VERSION));
          let audio = audio.clone();
          let hub = hub.clone();
          cx.new(|cx| ui::Workspace::new(audio, hub, window, cx))
        },
      ) {
        eprintln!("打开主窗口失败：{e}");
        cx.quit();
        return;
      }
      cx.activate(true);

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
