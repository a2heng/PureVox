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

//! 调试状态的唯一数据源（AGENTS.md 第 1 节）。
//!
//! UI 面板（Tauri 命令 `debug_snapshot`）与本机 HTTP 接口序列化的都是
//! [`DebugHub::snapshot`] 返回的同一个 [`DebugSnapshot`]，禁止另算一套。

pub mod http;
pub mod system;
#[cfg(windows)]
mod gpu_win;

use serde::Serialize;
use std::sync::{Arc, RwLock};
use std::time::{Instant, SystemTime, UNIX_EPOCH};

use crate::devices::DeviceList;
use system::SystemInfo;

/// 本机 HTTP 调试接口端口（只绑 127.0.0.1）。改动须同步 AGENTS.md 1.2 节。
pub const DEBUG_HTTP_PORT: u16 = 47821;

/// 一项采集结果：成功 / 采集中 / 不可用（带原因）。不可用时禁止伪造 0。
#[derive(Clone, Debug, Serialize)]
#[serde(tag = "state", rename_all = "snake_case")]
pub enum Probe<T> {
  Pending,
  Ok { value: T },
  Unavailable { reason: String },
}

impl<T> Probe<T> {
  pub fn ok(value: T) -> Self {
    Probe::Ok { value }
  }
  pub fn unavailable(reason: impl Into<String>) -> Self {
    Probe::Unavailable { reason: reason.into() }
  }
}

#[derive(Clone, Debug, Serialize)]
pub struct AppInfo {
  pub name: &'static str,
  pub version: &'static str,
  pub pid: u32,
  /// 调试接口地址；绑定失败时为不可用 + 原因。
  pub debug_http: Probe<String>,
}

/// 单路音频流的调试数据。字段先于实现定下，见 AGENTS.md 1.2 节。
#[allow(dead_code)]
#[derive(Clone, Debug, Serialize)]
pub struct StreamInfo {
  pub id: String,
  pub direction: &'static str,
  pub device_id: String,
  pub sample_rate: u32,
  pub channels: u32,
  pub sample_format: String,
  pub frames_processed: u64,
  pub peak_dbfs: f32,
  pub rms_dbfs: f32,
  pub underruns: u64,
  pub overruns: u64,
  pub buffer_level_ms: f32,
  pub latency_ms: f32,
  pub inference_ms_avg: f32,
  pub inference_ms_max: f32,
  /// 最近一段波形（48 kHz 单声道，按 10 ms 整数倍截取）。
  pub waveform: Vec<f32>,
  /// 最近一帧频谱（dB）。
  pub spectrum_db: Vec<f32>,
}

#[derive(Clone, Debug, Serialize)]
pub struct AudioInfo {
  pub engine: Probe<String>,
  pub streams: Vec<StreamInfo>,
}

#[derive(Clone, Debug, Serialize)]
pub struct DebugSnapshot {
  pub ts: u64,
  pub uptime_ms: u64,
  pub app: AppInfo,
  pub system: SystemInfo,
  pub audio: AudioInfo,
  pub devices: Probe<DeviceList>,
}

#[derive(Clone)]
struct State {
  system: SystemInfo,
  devices: Probe<DeviceList>,
  http: Probe<String>,
}

pub struct DebugHub {
  start: Instant,
  state: RwLock<State>,
}

pub type SharedHub = Arc<DebugHub>;

pub fn now_ms() -> u64 {
  SystemTime::now()
    .duration_since(UNIX_EPOCH)
    .map(|d| d.as_millis() as u64)
    .unwrap_or(0)
}

impl DebugHub {
  pub fn new() -> SharedHub {
    Arc::new(DebugHub {
      start: Instant::now(),
      state: RwLock::new(State {
        system: SystemInfo::default(),
        devices: Probe::Pending,
        http: Probe::Pending,
      }),
    })
  }

  pub fn uptime_ms(&self) -> u64 {
    self.start.elapsed().as_millis() as u64
  }

  pub fn set_system(&self, v: SystemInfo) {
    self.state.write().unwrap().system = v;
  }

  pub fn set_devices(&self, v: Probe<DeviceList>) {
    self.state.write().unwrap().devices = v;
  }

  pub fn set_http(&self, v: Probe<String>) {
    self.state.write().unwrap().http = v;
  }

  pub fn snapshot(&self) -> DebugSnapshot {
    let st = self.state.read().unwrap().clone();
    DebugSnapshot {
      ts: now_ms(),
      uptime_ms: self.uptime_ms(),
      app: AppInfo {
        name: "PureVox",
        version: env!("CARGO_PKG_VERSION"),
        pid: std::process::id(),
        debug_http: st.http,
      },
      system: st.system,
      audio: AudioInfo {
        engine: Probe::unavailable("音频引擎尚未实现"),
        streams: Vec::new(),
      },
      devices: st.devices,
    }
  }
}
