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
mod config;
mod debug;
mod devices;
mod dsp;
mod engine;
mod infer;
mod plan;
mod recorder;
mod wav;

use std::sync::Arc;

use audio::AudioManager;
use debug::{DebugHub, DebugSnapshot, SharedHub};
use plan::Plan;

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

/// 读取当前会话计划。
#[tauri::command]
fn get_plan(mgr: tauri::State<'_, Arc<AudioManager>>) -> Plan {
  mgr.plan()
}

/// 应用新计划（结构性变更 → 重建会话）。返回逐行问题（非致命）。
#[tauri::command]
async fn apply_plan(plan: Plan, mgr: tauri::State<'_, Arc<AudioManager>>) -> Result<Vec<String>, String> {
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
    .map(|(f, l)| ModelInfo { file: (*f).into(), label: (*l).into() })
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
            engine::registry::ParamKind::Number { lo, hi, step, .. } => (Some(lo), Some(hi), Some(step)),
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
fn record_tse_reference(seconds: f64, mgr: tauri::State<'_, Arc<AudioManager>>) -> Result<String, String> {
  mgr.record_reference(seconds)
}

/// 测 AEC 延时：采集 3 s（最大搜索 500 ms），结果见 `/debug` 的 `calib` 字段。
#[tauri::command]
fn calibrate_aec_delay(mgr: tauri::State<'_, Arc<AudioManager>>) -> Result<String, String> {
  mgr.calibrate_aec_delay()
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
      ui_report,
      open_devtools,
      get_plan,
      apply_plan,
      list_models,
      list_nodes,
      record_tse_reference,
      calibrate_aec_delay
    ])
    .run(tauri::generate_context!())
    .expect("error while running tauri application");
}
