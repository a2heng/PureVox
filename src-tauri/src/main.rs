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

mod debug;
mod devices;

use debug::{DebugHub, DebugSnapshot, SharedHub};

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

fn main() {
  let hub = DebugHub::new();
  debug::system::spawn_sampler(hub.clone());
  debug::http::spawn(hub.clone());
  devices::spawn_refresh(hub.clone());

  tauri::Builder::default()
    .runtime(tauri_runtime_wry::Wry::default())
    .manage(hub)
    .invoke_handler(tauri::generate_handler![debug_snapshot, refresh_devices])
    .run(tauri::generate_context!())
    .expect("error while running tauri application");
}
