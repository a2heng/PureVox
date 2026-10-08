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

mod audio;
#[cfg(windows)]
#[path = "autostart_windows.rs"]
mod autostart;
#[cfg(not(windows))]
#[path = "autostart_other.rs"]
mod autostart;
// 打开链接的平台实现分文件（AGENTS.md §4）。
mod config;
mod cues;
mod debug;
mod devices;
mod dsp;
mod engine;
mod hotkey;
mod infer;
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
mod wav;

use std::sync::{Arc, Mutex};

use audio::AudioManager;
use debug::{DebugHub, DebugSnapshot, SharedHub};
use plan::Plan;
use tauri::Manager;

/// 热键宿主（可重启）：设置变更时先停旧的再起新的。
struct HotkeyHost(Mutex<hotkey::Hotkeys>);

/// UI 调试面板取数：与 HTTP 接口同一个快照。
#[tauri::command]
fn debug_snapshot(hub: tauri::State<'_, SharedHub>) -> DebugSnapshot {
  hub.snapshot()
}

/// 刷新设备列表（后台执行，立即返回）。
#[tauri::command]
fn refresh_devices(hub: tauri::State<'_, SharedHub>) {
  devices::spawn_refresh(hub.inner().clone());
}

/// 界面自报错误/状态，追加进调试快照（`/debug` 的 `ui` 数组），便于无控制台时定位前端问题。
#[tauri::command]
fn ui_report(kind: String, message: String, hub: tauri::State<'_, SharedHub>) {
  hub.push_ui(&kind, message);
}

/// 打开 WebView2 开发者工具（JS 交互式调试）。
#[tauri::command]
fn open_devtools(window: tauri::WebviewWindow) {
  window.open_devtools();
}

/// 开机自启开关（写/删当前用户 Run 键）。
#[tauri::command]
fn set_autostart(on: bool) -> Result<(), String> {
  autostart::set(on)
}

#[tauri::command]
fn get_autostart() -> bool {
  autostart::get()
}

/// 读取当前会话计划。
#[tauri::command]
fn get_plan(mgr: tauri::State<'_, Arc<AudioManager>>) -> Plan {
  mgr.plan()
}

/// 应用新计划（结构性变更 → 重建会话）。返回逐行问题（非致命）。
#[tauri::command]
async fn apply_plan(
  plan: Plan,
  mgr: tauri::State<'_, Arc<AudioManager>>,
) -> Result<Vec<String>, String> {
  let mgr = mgr.inner().clone();
  tauri::async_runtime::spawn_blocking(move || mgr.apply_plan(plan))
    .await
    .map_err(|e| format!("应用计划异常：{e}"))?
}

#[derive(serde::Serialize)]
struct ModelInfo {
  file: String,
  label: String,
}

/// 某类处理节点的可选模型（供界面下拉）。
#[tauri::command]
fn list_models(ptype: String) -> Vec<ModelInfo> {
  let list: &[(&str, &str)] = match ptype.as_str() {
    "tse" => infer::tse::tse_models(),
    "echo_cancel" => infer::aec::aec_models(),
    _ => infer::denoise_models(),
  };
  list
    .iter()
    .map(|(f, l)| ModelInfo {
      file: (*f).into(),
      label: (*l).into(),
    })
    .collect()
}

#[derive(serde::Serialize)]
struct ParamInfo {
  key: String,
  label: String,
  kind: String,
  default: String,
  #[serde(skip_serializing_if = "Option::is_none")]
  min: Option<f64>,
  #[serde(skip_serializing_if = "Option::is_none")]
  max: Option<f64>,
  #[serde(skip_serializing_if = "Option::is_none")]
  step: Option<f64>,
}

#[derive(serde::Serialize)]
struct NodeInfo {
  ptype: String,
  label: String,
  kind: String,
  params: Vec<ParamInfo>,
}

/// 注册表里的节点类型（供界面构建下拉与参数编辑，单一来源）。
#[tauri::command]
fn list_nodes() -> Vec<NodeInfo> {
  engine::registry::all_specs()
    .iter()
    .map(|s| NodeInfo {
      ptype: s.ptype.into(),
      label: s.label.into(),
      kind: s.kind.as_str().into(),
      params: s
        .params
        .iter()
        .map(|p| {
          let (min, max, step) = match p.kind {
            engine::registry::ParamKind::Number { lo, hi, step, .. } => {
              (Some(lo), Some(hi), Some(step))
            }
            _ => (None, None, None),
          };
          ParamInfo {
            key: p.key.into(),
            label: p.label.into(),
            kind: p.kind_str().into(),
            default: p.default_text(),
            min,
            max,
            step,
          }
        })
        .collect(),
    })
    .collect()
}

/// 录制 TSE 参考：录「降噪后、TSE 前」的信号 `seconds` 秒，做音量归一化后写
/// `~/.purevox/tse_reference.wav`；进度见 `/debug` 的 `recorder` 字段。
#[tauri::command]
fn record_tse_reference(
  seconds: f64,
  mgr: tauri::State<'_, Arc<AudioManager>>,
) -> Result<String, String> {
  mgr.record_reference(seconds)
}

/// 测 AEC 延时：采集 3 s（最大搜索 500 ms），结果见 `/debug` 的 `calib` 字段。
#[tauri::command]
fn calibrate_aec_delay(mgr: tauri::State<'_, Arc<AudioManager>>) -> Result<String, String> {
  mgr.calibrate_aec_delay()
}

/// 启动网络服务（手机 ⇄ 电脑，端口 59123）。幂等。
#[tauri::command]
fn net_start() -> Result<String, String> {
  net::start()
}

/// 停网络服务，并强制关掉远程输入。
#[tauri::command]
fn net_stop() -> Result<String, String> {
  net::stop();
  Ok("已停止".into())
}

/// 网络状态（界面展示；明细见 `/debug` 的 `net` 字段）。
#[tauri::command]
fn net_status() -> String {
  net::status()
}

/// 远程输入总开关：手机输入法打字 + 手机实体键当全尺寸键盘。**默认关**。
#[tauri::command]
fn net_set_remote_input(on: bool) -> Result<String, String> {
  net::hub().set_remote_input(on);
  Ok(if on {
    "已开启远程输入：同网段的手机现在可以向这台电脑打字和发按键".to_string()
  } else {
    "已关闭远程输入".to_string()
  })
}

/// Linux 虚拟麦克风（PipeWire）当前状态；非 Linux / 不可用时带原因。
#[tauri::command]
fn virtual_mic_status() -> debug::Probe<audio::virtual_mic::Status> {
  audio::virtual_mic::status_probe()
}

/// 创建 Linux 虚拟麦克风（幂等），返回创建后的状态。
#[tauri::command]
fn virtual_mic_create(
  hub: tauri::State<'_, SharedHub>,
) -> Result<audio::virtual_mic::Status, String> {
  let s = audio::virtual_mic::create()?;
  hub.set_virtual_mic(debug::Probe::ok(s.clone()));
  Ok(s)
}

/// 移除 Linux 虚拟麦克风（幂等），返回移除后的状态。
#[tauri::command]
fn virtual_mic_remove(
  hub: tauri::State<'_, SharedHub>,
) -> Result<audio::virtual_mic::Status, String> {
  let s = audio::virtual_mic::remove()?;
  hub.set_virtual_mic(debug::Probe::ok(s.clone()));
  Ok(s)
}

/// AEC 远端可选的回环目标（某个输出/sink 的 monitor；「系统默认输出」由界面加）。
#[tauri::command]
fn list_loopback_targets() -> Vec<devices::LoopbackTarget> {
  devices::loopback_targets()
}

/// 当前平台标识（界面「驱动」页据此切换平台实现）：`linux` / `windows` / `macos`。
#[tauri::command]
fn get_platform() -> String {
  std::env::consts::OS.to_string()
}

/// 用系统默认浏览器打开链接（驱动下载 / 教程）。
#[tauri::command]
fn open_url(url: String) -> Result<(), String> {
  if !(url.starts_with("https://") || url.starts_with("http://")) {
    return Err("只允许打开 http(s) 链接".into());
  }
  openurl::open(&url)
}

fn main() {
  let hub = DebugHub::new();
  debug::system::spawn_sampler(hub.clone());
  debug::http::spawn(hub.clone());
  devices::spawn_refresh(hub.clone());
  let audio = Arc::new(AudioManager::new(hub.clone()));
  let hotkey_host = HotkeyHost(Mutex::new(hotkey::Hotkeys::start(
    "",
    hub.clone(),
    Arc::new(|| {}),
  )));

  tauri::Builder::default()
    .runtime(tauri_runtime_wry::Wry::default())
    .manage(hub)
    .manage(audio)
    .manage(hotkey_host)
    .setup(|app| {
      // 打包后模型位于 Tauri 资源目录（Windows：安装目录/models；Linux deb：/usr/lib/<productName>/models），
      // 注册给模型查找（开发态该目录不存在，走仓库 models/）。
      if let Ok(res) = app.path().resource_dir() {
        let models = res.join("models");
        if models.is_dir() {
          infer::set_model_dir(models);
        }
      }
      setup_tray(app)?;
      start_hotkey(app.handle(), &app.state::<HotkeyHost>());
      // 初始状态同步窗口图标
      sync_tray(app.handle(), app.state::<Arc<AudioManager>>().is_running());
      // 启动自运行并隐藏窗口（收到托盘）
      if config::load_settings().start_hidden {
        let mgr = app.state::<Arc<AudioManager>>().inner().clone();
        let _ = mgr.start();
        sync_tray(app.handle(), true);
        if let Some(w) = app.get_webview_window("main") {
          let _ = w.hide();
        }
      }
      Ok(())
    })
    // 最小化 / 关闭都收到托盘（不退出）
    .on_window_event(|window, event| match event {
      tauri::WindowEvent::CloseRequested { api, .. } => {
        api.prevent_close();
        let _ = window.hide();
      }
      tauri::WindowEvent::Resized(_) if window.is_minimized().unwrap_or(false) => {
        let _ = window.hide();
      }
      _ => {}
    })
    .invoke_handler(tauri::generate_handler![
      debug_snapshot,
      refresh_devices,
      ui_report,
      open_devtools,
      set_autostart,
      get_autostart,
      set_running,
      get_running,
      set_language,
      get_settings,
      set_settings,
      list_cues,
      get_plan,
      apply_plan,
      list_models,
      list_nodes,
      record_tse_reference,
      calibrate_aec_delay,
      net_start,
      net_stop,
      net_status,
      net_set_remote_input,
      virtual_mic_status,
      virtual_mic_create,
      virtual_mic_remove,
      list_loopback_targets,
      get_platform,
      open_url
    ])
    .run(tauri::generate_context!())
    .expect("error while running tauri application");
}

/// 系统托盘：启动/停止、显示/隐藏主窗口、退出；图标随运行状态变化。
fn setup_tray(app: &tauri::App) -> Result<(), Box<dyn std::error::Error>> {
  use tauri::tray::TrayIconBuilder;

  let mgr = app.state::<Arc<AudioManager>>().inner().clone();
  let menu = build_tray_menu(app, &config::load_lang(), mgr.is_running())?;
  let mgr2 = mgr.clone();
  let app_ev = app.handle().clone();
  let mut builder = TrayIconBuilder::with_id("main")
    .tooltip("PureVox")
    .menu(&menu)
    // 左键单击 = 显隐窗口；右键 = 弹菜单
    .show_menu_on_left_click(false)
    .on_menu_event(move |app, event| match event.id.as_ref() {
      "run" => toggle_running(app, &mgr2),
      "show" => toggle_window(app),
      "quit" => app.exit(0),
      _ => {}
    })
    .on_tray_icon_event(move |_tray, event| {
      if let tauri::tray::TrayIconEvent::Click {
        button: tauri::tray::MouseButton::Left,
        button_state: tauri::tray::MouseButtonState::Up,
        ..
      } = event
      {
        toggle_window(&app_ev);
      }
    });
  if let Some(icon) = tray_icon(mgr.is_running()) {
    builder = builder.icon(icon);
  }
  builder.build(app)?;
  Ok(())
}

const TRAY_RUN_PNG: &[u8] = include_bytes!("../icons/tray_running.png");
const TRAY_STOP_PNG: &[u8] = include_bytes!("../icons/tray_stopped.png");

fn tray_icon(running: bool) -> Option<tauri::image::Image<'static>> {
  tauri::image::Image::from_bytes(if running { TRAY_RUN_PNG } else { TRAY_STOP_PNG }).ok()
}

/// 显隐主窗口（托盘左键 / 菜单「显示/隐藏」共用）。
fn toggle_window(app: &tauri::AppHandle) {
  if let Some(w) = app.get_webview_window("main") {
    if w.is_visible().unwrap_or(true) {
      let _ = w.hide();
    } else {
      let _ = w.unminimize();
      let _ = w.show();
      let _ = w.set_focus();
    }
  }
}

/// 托盘菜单（随语言与运行状态重建）。
fn build_tray_menu<R: tauri::Runtime, M: Manager<R>>(
  mgr: &M,
  lang: &str,
  running: bool,
) -> tauri::Result<tauri::menu::Menu<R>> {
  use tauri::menu::{Menu, MenuItem};
  let en = lang == "en";
  let run = if running {
    if en { "Stop" } else { "停止" }
  } else if en {
    "Start"
  } else {
    "启动"
  };
  let show = if en {
    "Show / Hide window"
  } else {
    "显示 / 隐藏窗口"
  };
  let quit = if en { "Quit PureVox" } else { "退出 PureVox" };
  let run = MenuItem::with_id(mgr, "run", run, true, None::<&str>)?;
  let show = MenuItem::with_id(mgr, "show", show, true, None::<&str>)?;
  let quit = MenuItem::with_id(mgr, "quit", quit, true, None::<&str>)?;
  Menu::with_items(mgr, &[&run, &show, &quit])
}

/// 启动 / 停止：改引擎状态 + 提示音 + 同步托盘图标与菜单。
fn apply_running(app: &tauri::AppHandle, mgr: &AudioManager, on: bool) {
  if on {
    let _ = mgr.start();
  } else {
    mgr.stop();
  }
  let s = config::load_settings();
  if s.cue_on {
    cues::play(
      if on { &s.cue_start } else { &s.cue_stop },
      if on { "start" } else { "stop" },
    );
  }
  sync_tray(app, on);
}

/// 按当前设置（重）启动全局热键。
fn start_hotkey(app: &tauri::AppHandle, host: &HotkeyHost) {
  let s = config::load_settings();
  let spec = if s.hotkey_on {
    s.hotkey.clone()
  } else {
    String::new()
  };
  let hub = app.state::<SharedHub>().inner().clone();
  let app2 = app.clone();
  let mgr = app.state::<Arc<AudioManager>>().inner().clone();
  let cb: Arc<dyn Fn() + Send + Sync> = Arc::new(move || toggle_running(&app2, &mgr));
  let hk = hotkey::Hotkeys::start(&spec, hub, cb);
  let mut guard = host.0.lock().unwrap();
  guard.stop();
  *guard = hk;
}

#[tauri::command]
fn get_settings() -> config::AppSettings {
  config::load_settings()
}

#[derive(serde::Serialize)]
struct CueInfo {
  id: String,
  label: String,
}

/// 可选提示音预设（供设置面板下拉）。
#[tauri::command]
fn list_cues() -> Vec<CueInfo> {
  cues::PRESETS
    .iter()
    .map(|(id, label)| CueInfo {
      id: (*id).into(),
      label: (*label).into(),
    })
    .collect()
}

/// 保存设置：归一化热键 → 落盘 → 重启热键 → 刷新托盘菜单（语言）。
#[tauri::command]
fn set_settings(mut settings: config::AppSettings, app: tauri::AppHandle) -> Result<(), String> {
  settings.hotkey = hotkey::normalize_spec(&settings.hotkey);
  config::save_settings(&settings)?;
  start_hotkey(&app, &app.state::<HotkeyHost>());
  let running = app.state::<Arc<AudioManager>>().is_running();
  if let Some(tray) = app.tray_by_id("main")
    && let Ok(menu) = build_tray_menu(&app, &settings.lang, running)
  {
    let _ = tray.set_menu(Some(menu));
  }
  Ok(())
}

fn toggle_running(app: &tauri::AppHandle, mgr: &AudioManager) {
  apply_running(app, mgr, !mgr.is_running());
}

fn sync_tray(app: &tauri::AppHandle, running: bool) {
  // 托盘图标 + 窗口（标题栏/任务栏）图标都随运行状态切换
  if let Some(icon) = tray_icon(running) {
    if let Some(tray) = app.tray_by_id("main") {
      let _ = tray.set_icon(Some(icon.clone()));
    }
    if let Some(w) = app.get_webview_window("main") {
      let _ = w.set_icon(icon);
    }
  }
  if let Ok(menu) = build_tray_menu(app, &config::load_lang(), running)
    && let Some(tray) = app.tray_by_id("main")
  {
    let _ = tray.set_menu(Some(menu));
  }
}

/// 启动 / 停止音频引擎（界面按钮、托盘、快捷键共用）。
#[tauri::command]
async fn set_running(
  on: bool,
  app: tauri::AppHandle,
  mgr: tauri::State<'_, Arc<AudioManager>>,
) -> Result<(), String> {
  let mgr = mgr.inner().clone();
  tauri::async_runtime::spawn_blocking(move || apply_running(&app, &mgr, on))
    .await
    .map_err(|e| format!("启动/停止异常：{e}"))
}

#[tauri::command]
fn get_running(mgr: tauri::State<'_, Arc<AudioManager>>) -> bool {
  mgr.is_running()
}

/// 切换界面语言：保存 + 更新托盘菜单（界面文案由前端 i18n 处理）。
#[tauri::command]
fn set_language(lang: String, app: tauri::AppHandle) -> Result<(), String> {
  let _ = config::save_lang(&lang);
  let running = app.state::<Arc<AudioManager>>().is_running();
  if let Some(tray) = app.tray_by_id("main") {
    let menu = build_tray_menu(&app, &lang, running).map_err(|e| e.to_string())?;
    tray.set_menu(Some(menu)).map_err(|e| e.to_string())?;
  }
  Ok(())
}
