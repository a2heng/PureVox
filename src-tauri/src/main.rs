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
mod debug;
mod devices;
mod infer;

use std::sync::Arc;

use audio::AudioManager;
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

/// 在阻塞线程池上执行音频管理操作（打开/关闭设备可能耗时，不占 UI 线程）。
async fn blocking<R: Send + 'static>(
  mgr: &tauri::State<'_, Arc<AudioManager>>,
  f: impl FnOnce(&AudioManager) -> R + Send + 'static,
) -> Result<R, String> {
  let mgr = mgr.inner().clone();
  tauri::async_runtime::spawn_blocking(move || f(&mgr))
    .await
    .map_err(|e| format!("音频任务异常：{e}"))
}

/// 开始采集某个输入设备。
#[tauri::command]
async fn start_capture(device_id: String, mgr: tauri::State<'_, Arc<AudioManager>>) -> Result<(), String> {
  blocking(&mgr, move |m| m.start_capture(&device_id)).await?
}

#[tauri::command]
async fn stop_capture(device_id: String, mgr: tauri::State<'_, Arc<AudioManager>>) -> Result<(), String> {
  blocking(&mgr, move |m| m.stop_capture(&device_id)).await
}

/// 在输出设备上播放信号源（"tone" 或正在采集的输入设备 ID）。
#[tauri::command]
async fn start_playback(
  device_id: String,
  source: String,
  mgr: tauri::State<'_, Arc<AudioManager>>,
) -> Result<(), String> {
  blocking(&mgr, move |m| m.start_playback(&device_id, &source)).await?
}

#[tauri::command]
async fn stop_playback(device_id: String, mgr: tauri::State<'_, Arc<AudioManager>>) -> Result<(), String> {
  blocking(&mgr, move |m| m.stop_playback(&device_id)).await
}

/// 开关某路采集的降噪（输入行「降噪」按钮）。
#[tauri::command]
async fn set_denoise(
  device_id: String,
  on: bool,
  mgr: tauri::State<'_, Arc<AudioManager>>,
) -> Result<(), String> {
  blocking(&mgr, move |m| m.set_denoise(&device_id, on)).await?
}

fn main() {
  let hub = DebugHub::new();
  debug::system::spawn_sampler(hub.clone());
  debug::http::spawn(hub.clone());
  devices::spawn_refresh(hub.clone());
  let audio = Arc::new(AudioManager::new(hub.clone()));

  tauri::Builder::default()
    .runtime(tauri_runtime_wry::Wry::default())
    .manage(hub)
    .manage(audio)
    .invoke_handler(tauri::generate_handler![
      debug_snapshot,
      refresh_devices,
      start_capture,
      stop_capture,
      start_playback,
      stop_playback,
      set_denoise
    ])
    .run(tauri::generate_context!())
    .expect("error while running tauri application");
}
