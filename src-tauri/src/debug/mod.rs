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

#[cfg(windows)]
mod gpu_win;
pub mod http;
pub mod system;

use serde::Serialize;
use std::collections::BTreeMap;
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
    Probe::Unavailable {
      reason: reason.into(),
    }
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

/// 单路音频流的调试数据，见 AGENTS.md 1.2 节。测不到的项一律 Probe 不可用，不填 0。
#[derive(Clone, Debug, Serialize)]
pub struct StreamInfo {
  pub id: String,
  pub direction: &'static str,
  pub device_id: String,
  pub device_name: String,
  /// running / 不可用 + 错误原因
  pub state: Probe<String>,
  pub started_at: u64,
  /// 输出流播放的信号源名称；输入流为 None
  pub source: Option<String>,
  /// 本流的降噪状态：ok = 运行中的模型名；unavailable = 未启用或失败原因（输出流恒为不适用）
  pub denoise: Probe<String>,
  /// 设备以此原生格式打开
  pub sample_rate: u32,
  pub channels: u32,
  pub sample_format: String,
  /// 进入本流重采样器的实测帧率（3 s 滑动窗口）。输入流 = 设备侧；输出流 = 引擎 48k 侧
  pub measured_input_rate: Probe<f64>,
  /// 引擎内部采样率（恒 48000）
  pub output_rate: u32,
  /// 离开本流重采样器的实测帧率（3 s 滑动窗口）。输入流 = 引擎 48k 侧（应≈48000）；输出流 = 设备侧
  pub measured_output_rate: Probe<f64>,
  pub resampler: String,
  pub resampler_delay_ms: f64,
  /// 输出流时钟伺服对重采样比例的当前修正（ppm，正值 = 消耗更快）
  pub asrc_adjust_ppm: Probe<f64>,
  pub callbacks: u64,
  /// 设备回调块大小（帧）：最近 / 最小 / 最大
  pub callback_frames: Probe<crate::audio::capture::CallbackFrames>,
  /// 已收到的原生帧数
  pub frames_in: u64,
  /// 已产出的 48 kHz 帧数（恒为 HOP 整数倍）
  pub frames_processed: u64,
  pub hops: u64,
  /// 已重采样但不足一个 hop 的剩余帧（恒 < 480）
  pub pending_frames: u32,
  /// 最近一个发布周期（200 ms）内
  pub peak_dbfs: Probe<f32>,
  pub rms_dbfs: Probe<f32>,
  /// 输出流：设备回调取不到数据而补静音的样本数（预热完成后才计）
  pub underruns: Probe<u64>,
  /// 输入流：环形缓冲满丢弃的样本数；输出流：48k 缓冲超过封顶丢弃的最旧样本数
  pub overruns: u64,
  /// 输出流：信号源断流后静音并重新预热的次数
  pub resyncs: Probe<u64>,
  pub stream_errors: u64,
  pub last_error: Option<String>,
  /// 输入流：回调 → 工作线程环形缓冲水位；输出流：48k 源缓冲水位（伺服目标 40 ms）
  pub buffer_level_ms: f32,
  /// 输出流：工作线程 → 设备回调的设备侧缓冲水位
  pub device_buffer_ms: Probe<f32>,
  pub latency_ms: Probe<f32>,
  pub inference_ms_avg: Probe<f32>,
  pub inference_ms_max: Probe<f32>,
  /// 最近 50 ms 波形（48 kHz 单声道，5 个 hop）
  pub waveform: Vec<f32>,
  /// 最近一个发布周期的平均频谱（dBFS，481 个 bin，NFFT = 960）
  pub spectrum_db: Vec<f32>,
  pub spectrum_bin_hz: f32,
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
  /// 界面自报的最近消息（新在前，最多 50 条；见 `ui_report` 命令）
  pub ui: Vec<String>,
  /// 音频引擎是否运行中（启动/停止状态）
  pub running: bool,
  /// TSE 参考录音状态/进度
  pub recorder: Probe<String>,
  /// AEC 延时校准状态/结果
  pub calib: Probe<String>,
  /// 网络（手机 ⇄ 电脑，DESIGN.md §4.1）：端口、客户端数、进出计数、远程输入开关、Opus 探测
  pub net: crate::net::hub::NetStats,
}

#[derive(Clone)]
struct State {
  system: SystemInfo,
  devices: Probe<DeviceList>,
  http: Probe<String>,
  /// 列（引擎处理图）的概要，由列工作线程发布
  column: Probe<String>,
  /// 界面自报的最近消息（迁移期定位前端问题用；新在前，最多 50 条）
  ui: Vec<String>,
  /// 音频引擎是否运行中
  running: bool,
  /// TSE 参考录音状态/进度
  recorder: Probe<String>,
  /// AEC 延时校准状态/结果
  calib: Probe<String>,
  streams: BTreeMap<String, StreamInfo>,
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
        column: Probe::Pending,
        ui: Vec::new(),
        running: false,
        recorder: Probe::Pending,
        calib: Probe::Pending,
        streams: BTreeMap::new(),
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

  pub fn set_column(&self, v: Probe<String>) {
    self.state.write().unwrap().column = v;
  }

  pub fn set_recorder(&self, v: Probe<String>) {
    self.state.write().unwrap().recorder = v;
  }

  pub fn set_calib(&self, v: Probe<String>) {
    self.state.write().unwrap().calib = v;
  }

  pub fn set_running(&self, v: bool) {
    self.state.write().unwrap().running = v;
  }

  /// 追加一条界面自报消息（新在前，最多 50 条）。
  pub fn push_ui(&self, level: &str, message: String) {
    let mut st = self.state.write().unwrap();
    st.ui.insert(0, format!("[{level}] {message}"));
    st.ui.truncate(50);
  }

  pub fn set_stream(&self, v: StreamInfo) {
    self.state.write().unwrap().streams.insert(v.id.clone(), v);
  }

  pub fn remove_stream(&self, id: &str) {
    self.state.write().unwrap().streams.remove(id);
  }

  pub fn snapshot(&self) -> DebugSnapshot {
    let st = self.state.read().unwrap().clone();
    // 设备表的「已打开」由当前活动流推出，避免两处各存一份
    let mut devices = st.devices;
    if let Probe::Ok { value } = &mut devices {
      for d in &mut value.devices {
        d.opened = st
          .streams
          .values()
          .any(|s| s.device_id == d.id && s.direction == d.direction);
      }
    }
    let st_ui = st.ui.clone();
    let st_recorder = st.recorder.clone();
    let st_calib = st.calib.clone();
    let st_running = st.running;
    let engine = match st.column {
      Probe::Ok { value } => Probe::ok(value),
      _ => Probe::unavailable("列未启动"),
    };
    let streams: Vec<StreamInfo> = st.streams.into_values().collect();
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
      audio: AudioInfo { engine, streams },
      devices,
      ui: st_ui,
      running: st_running,
      recorder: st_recorder,
      calib: st_calib,
      net: crate::net::hub().stats(),
    }
  }
}
