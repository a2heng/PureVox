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

pub mod events;
pub mod http;
pub mod metrics;
pub mod screenshot;
pub mod system;
pub mod threads;

use serde::Serialize;
use std::collections::BTreeMap;
use std::sync::{Arc, RwLock};
use std::time::{Instant, SystemTime, UNIX_EPOCH};

use crate::devices::DeviceList;
use crate::plan::Plan;
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

/// 一路流 / 整个音频引擎的体检结论。
///
/// `level` 只用来着色（`ok` / `warn`），`text` 是人话结论且**必须带关键数字** ——
/// 「稳定」和「稳定（设备侧 48000 Hz，无丢弃）」的信息量差一个数量级。
/// 判定统一在 [`crate::audio::health`] 里做一处，面板与 HTTP 接口渲染同一份
/// （AGENTS.md 1.3「单一数据源」；前端禁止自己另算一套）。
#[derive(Clone, Debug, Serialize)]
pub struct Health {
  /// `ok` = 正常；`warn` = 需要看一眼（面板着黄）
  pub level: &'static str,
  pub text: String,
}

impl Health {
  pub fn ok(text: impl Into<String>) -> Self {
    Health {
      level: "ok",
      text: text.into(),
    }
  }
  pub fn warn(text: impl Into<String>) -> Self {
    Health {
      level: "warn",
      text: text.into(),
    }
  }
}

/// 一行（输入 / 处理 / 输出）的结构化状态。
///
/// 这是「异常 N」的替代品：界面与脚本不用再解析中文概要串，直接按
/// `columns[].rows[].health` 上色、按 `state` 定位（AGENTS §1.3 单一数据源）。
#[derive(Clone, Debug, Serialize)]
pub struct RowInfo {
  pub index: usize,
  /// `input` / `process` / `output`
  pub kind: String,
  pub ptype: String,
  pub label: String,
  pub enabled: bool,
  /// `ok("running")` / `ok("disabled")` / `unavailable(原因)`
  pub state: Probe<String>,
  pub device: Option<String>,
  /// 这一行自己的环水位（ms）：输入行 = 行输入环 / AEC mic / 网络入站；输出行 = 混音源环
  pub buffer_ms: Option<f32>,
  pub dropped: u64,
  pub dropped_per_s: Probe<f64>,
  pub inference_ms_avg: Probe<f32>,
  pub inference_ms_max: Probe<f32>,
  /// 超过一跳预算（`TICK_MS * 0.8`）的次数与速率
  pub inference_over: u64,
  pub inference_over_per_s: Probe<f64>,
  /// 人话细节（AEC 对齐/ρ、网络水位等）；与 `health` 不重复
  pub detail: String,
  pub health: Probe<Health>,
}

/// 一列的结构化状态（含节拍质量）。
#[derive(Clone, Debug, Serialize)]
pub struct ColumnInfo {
  pub index: usize,
  pub enabled: bool,
  /// `running` / `degraded` / `stopped`
  pub state: String,
  /// 实测 hop/s（正常 100）
  pub hop_rate: f64,
  pub ticks: u64,
  /// 本轮会话累计的「开始处理时已经晚于 10 ms 网格」次数
  pub tick_overruns: u64,
  pub tick_overrun_per_s: Probe<f64>,
  /// 最近 / 最大节拍迟到（ms）
  pub last_late_ms: f32,
  pub max_late_ms: f32,
  /// 单拍处理耗时最大值（ms）：模型/解码一慢，这里先炸
  pub max_work_ms: f32,
  pub rows: Vec<RowInfo>,
  /// 人话概要（仍给旧面板/日志用；由结构派生）
  pub text: String,
}

/// 引擎整体（所有列 + 线程存活 + 模型池）。`/debug/audio` 的 `engine` 文本是它的派生。
#[derive(Clone, Debug, Serialize, Default)]
pub struct EngineInfo {
  pub generation: u64,
  /// `running` / `degraded` / `stopped`
  pub state: String,
  pub started_at: u64,
  pub columns: Vec<ColumnInfo>,
  pub threads: Vec<threads::ThreadInfo>,
  pub models: Vec<crate::infer::ModelStat>,
  /// 派生文本：`列 1：… ｜ 列 2：…`
  pub text: String,
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
  /// 健康结论：把下面的计数与速率翻成一句人话（含关键数字），由 `audio::health` 判定
  pub health: Probe<Health>,
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
  /// 输入流：设备环丢最旧 —— 环满（回调侧）+ **延迟封顶**（[`crate::audio::capture`]
  /// 的 `DEV_MAX_MS`，超过硬顶的过时音频直接丢，不慢慢放出去）；
  /// 输出流：48k 缓冲超过封顶丢弃的最旧样本数
  pub overruns: u64,
  /// 输出流：信号源断流后静音并重新预热的次数
  pub resyncs: Probe<u64>,
  pub stream_errors: u64,
  /// 上面几个计数的实测速率（次/秒，3 s 滑动窗口）：累计值只会越涨越大，
  /// 单看它永远看不出「现在稳不稳」——速率与结论要一起看。没数据时为采集中。
  /// 与计数同名同义：输入流的欠载/重同步本就不适用，速率也给不可用 + 原因。
  pub underrun_per_s: Probe<f64>,
  pub resync_per_s: Probe<f64>,
  pub overrun_per_s: Probe<f64>,
  pub stream_error_per_s: Probe<f64>,
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
  /// 兼容字段：全部列的人话概要（由 `engine_info` 派生，字段只增不改名）
  pub engine: Probe<String>,
  /// 结构化引擎状态（列 / 行 / 节拍 / 线程 / 模型）——新代码应读这里
  pub engine_info: EngineInfo,
  /// 全部流的汇总结论（最坏优先）：一眼看出「整个音频链路现在有没有问题」
  pub verdict: Probe<Health>,
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
  /// 进程内事件日志（时间正序，最多 200 条；完整日志见 `/debug/logs`）
  pub events: Vec<events::Event>,
  /// 当前会话计划（含校验现场；见 `AudioManager` 的 `set_plan`）
  pub plan: Probe<Plan>,
  /// 音频引擎是否运行中（启动/停止状态）
  pub running: bool,
  /// 设备刷新状态（正在刷 / 上次成功于 / 上次耗时 / 上次失败原因）
  pub device_refresh: crate::devices::RefreshState,
  /// TSE 参考录音状态/进度
  pub recorder: Probe<String>,
  /// AEC 延时校准状态/结果（人话串，向后兼容）
  pub calib: Probe<String>,
  /// AEC 校准的结构化结果（AGENTS.md §1.4：同一份进 `/debug/calib` 并落盘
  /// `~/.purevox/calib_last.json`；界面画图与回填都读它，不再解析上面那句串）
  pub calib_report: Probe<crate::engine::calib::CalibReport>,
  /// 网络（手机 ⇄ 电脑，DESIGN.md §4.1）：端口、客户端数、进出计数、远程输入开关、Opus 探测
  pub net: crate::net::hub::NetStats,
  /// 更新检查（GitHub Releases）：阶段、本机版本 / 最新 tag、下载进度、失败原因
  pub update: crate::update::Status,
  /// Linux 虚拟麦克风（PipeWire）状态；非 Linux 或不可用时为 unavailable + 原因
  pub virtual_mic: Probe<crate::audio::virtual_mic::Status>,
}

#[derive(Clone)]
struct State {
  system: SystemInfo,
  devices: Probe<DeviceList>,
  http: Probe<String>,
  /// 列（引擎处理图）的结构化状态，由列工作线程发布；索引 → 列
  columns: BTreeMap<usize, ColumnInfo>,
  /// 引擎代次 / 状态 / 起步时刻（`Session` 构建与监督线程维护）
  engine_generation: u64,
  engine_state: String,
  engine_started_at: u64,
  /// 当前会话计划（`AudioManager` 每次加载/应用/回滚都同步过来）
  plan: Probe<Plan>,
  /// 界面自报的最近消息（迁移期定位前端问题用；新在前，最多 50 条）
  ui: Vec<String>,
  /// 音频引擎是否运行中
  running: bool,
  /// TSE 参考录音状态/进度
  recorder: Probe<String>,
  /// AEC 延时校准状态/结果
  calib: Probe<String>,
  /// AEC 校准的结构化报告（与 `calib` 同一时刻发布；快照 / 端点 / 落盘同源）
  calib_report: Probe<crate::engine::calib::CalibReport>,
  /// Linux 虚拟麦克风状态
  virtual_mic: Probe<crate::audio::virtual_mic::Status>,
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
        columns: BTreeMap::new(),
        engine_generation: 0,
        engine_state: "stopped".to_string(),
        engine_started_at: 0,
        plan: Probe::Pending,
        ui: Vec::new(),
        running: false,
        recorder: Probe::Pending,
        calib: Probe::Pending,
        calib_report: Probe::Pending,
        virtual_mic: Probe::Pending,
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

  /// 新的会话开始：代次 +1，状态置 `running`。旧会话随后停止时**不许**覆盖它
  /// （`Session::stop` 不再清列；只有 `AudioManager::stop` 显式停引擎才清）。
  pub fn set_engine_running(&self, generation: u64) {
    let mut st = self.state.write().unwrap();
    st.engine_generation = generation;
    st.engine_state = "running".to_string();
    st.engine_started_at = now_ms();
  }

  /// 引擎降级（线程死亡 / 关键行失败）。保留 `generation`，便于区分是哪一代出的问题。
  pub fn set_engine_degraded(&self, reason: impl Into<String>) {
    let reason = reason.into();
    let mut st = self.state.write().unwrap();
    if st.engine_state != "stopped" {
      st.engine_state = format!("degraded：{reason}");
    }
    drop(st);
    events::log("error", "engine", format!("引擎降级：{reason}"));
  }

  pub fn set_engine_stopped(&self) {
    let mut st = self.state.write().unwrap();
    st.engine_state = "stopped".to_string();
    st.engine_started_at = 0;
    st.columns.clear();
  }

  /// 列工作线程发布（200 ms 一次）。索引相同的后写覆盖。
  pub fn set_column_info(&self, info: ColumnInfo) {
    self.state.write().unwrap().columns.insert(info.index, info);
  }

  /// 清掉列概要（回滚/停引擎）：旧会话的列线程会在下一拍重新发布。
  pub fn clear_columns(&self) {
    self.state.write().unwrap().columns.clear();
  }

  pub fn set_plan(&self, plan: Plan) {
    self.state.write().unwrap().plan = Probe::ok(plan);
  }

  pub fn set_recorder(&self, v: Probe<String>) {
    self.state.write().unwrap().recorder = v;
  }

  pub fn set_calib(&self, v: Probe<String>) {
    self.state.write().unwrap().calib = v;
  }

  /// 结构化校准报告（与 `set_calib` 成对发布；`/debug/calib` 与快照读同一份）
  pub fn set_calib_report(&self, v: Probe<crate::engine::calib::CalibReport>) {
    self.state.write().unwrap().calib_report = v;
  }

  pub fn set_virtual_mic(&self, v: Probe<crate::audio::virtual_mic::Status>) {
    self.state.write().unwrap().virtual_mic = v;
  }

  pub fn set_running(&self, v: bool) {
    self.state.write().unwrap().running = v;
  }

  /// 追加一条界面自报消息（新在前，最多 50 条）。
  pub fn push_ui(&self, level: &str, message: String) {
    let mut st = self.state.write().unwrap();
    st.ui.insert(0, format!("[{level}] {message}"));
    st.ui.truncate(50);
    drop(st);
    events::log("ui", "ui", message);
  }

  pub fn set_stream(&self, v: StreamInfo) {
    self.state.write().unwrap().streams.insert(v.id.clone(), v);
  }

  pub fn remove_stream(&self, id: &str) {
    self.state.write().unwrap().streams.remove(id);
  }

  /// 1 Hz 指标采样（由 `debug::system` 的采样线程调用）：把关键标量写进短历史。
  pub fn record_metrics(&self) {
    let Ok(st) = self.state.read() else {
      return;
    };
    fn rec(name: String, v: f64) {
      metrics::record(name, v);
    }
    fn probe_f64(name: String, p: &Probe<f64>) {
      if let Probe::Ok { value } = p {
        metrics::record(name, *value);
      }
    }
    fn probe_f32(name: String, p: &Probe<f32>) {
      if let Probe::Ok { value } = p {
        metrics::record(name, *value as f64);
      }
    }

    for s in st.streams.values() {
      let id = s.id.replace(':', "_");
      rec(format!("stream.{id}.buffer_ms"), s.buffer_level_ms as f64);
      probe_f32(format!("stream.{id}.device_buffer_ms"), &s.device_buffer_ms);
      probe_f64(format!("stream.{id}.underrun_per_s"), &s.underrun_per_s);
      probe_f64(format!("stream.{id}.resync_per_s"), &s.resync_per_s);
      probe_f64(format!("stream.{id}.overrun_per_s"), &s.overrun_per_s);
      probe_f64(
        format!("stream.{id}.stream_error_per_s"),
        &s.stream_error_per_s,
      );
      probe_f64(
        format!("stream.{id}.measured_input_rate"),
        &s.measured_input_rate,
      );
      probe_f64(
        format!("stream.{id}.measured_output_rate"),
        &s.measured_output_rate,
      );
      probe_f32(format!("stream.{id}.peak_dbfs"), &s.peak_dbfs);
    }
    for (i, c) in st.columns.iter() {
      rec(format!("column.{i}.hop_rate"), c.hop_rate);
      rec(format!("column.{i}.tick_overruns"), c.tick_overruns as f64);
      probe_f64(
        format!("column.{i}.tick_overrun_per_s"),
        &c.tick_overrun_per_s,
      );
      rec(format!("column.{i}.max_late_ms"), c.max_late_ms as f64);
      rec(format!("column.{i}.max_work_ms"), c.max_work_ms as f64);
      for r in &c.rows {
        let rid = format!("{i}.{}", r.index);
        if let Some(v) = r.buffer_ms {
          rec(format!("row.{rid}.buffer_ms"), v as f64);
        }
        probe_f32(format!("row.{rid}.inference_ms_avg"), &r.inference_ms_avg);
        probe_f64(format!("row.{rid}.dropped_per_s"), &r.dropped_per_s);
        probe_f64(
          format!("row.{rid}.inference_over_per_s"),
          &r.inference_over_per_s,
        );
      }
    }
    for t in threads::snapshot() {
      rec(
        format!("thread.{}.age_ms", t.name.replace(' ', "_")),
        t.age_ms as f64,
      );
    }
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
        // 设备池：引用数与逐源状态（DESIGN.md §6.5「必须看得见」）
        let pool = crate::audio::device_pool::hub().stat(&d.id);
        d.refs = pool.refs;
        d.mix_sources = pool.sources;
      }
    }
    let st_ui = st.ui.clone();
    let st_recorder = st.recorder.clone();
    let st_calib = st.calib.clone();
    let st_calib_report = st.calib_report.clone();
    let st_virtual_mic = st.virtual_mic.clone();
    let st_running = st.running;
    let st_plan = st.plan.clone();
    let columns: Vec<ColumnInfo> = st.columns.into_values().collect();
    let text = columns
      .iter()
      .map(|c| c.text.clone())
      .collect::<Vec<_>>()
      .join(" ｜ ");
    let engine_info = EngineInfo {
      generation: st.engine_generation,
      state: st.engine_state.clone(),
      started_at: st.engine_started_at,
      columns,
      threads: threads::snapshot(),
      models: crate::infer::models_stats(),
      text: text.clone(),
    };
    let engine = if st.engine_state == "stopped" || text.is_empty() {
      Probe::unavailable(if st.engine_state == "stopped" {
        "列未启动".to_string()
      } else {
        "列还没有发布状态".to_string()
      })
    } else {
      Probe::ok(text)
    };
    let streams: Vec<StreamInfo> = st.streams.into_values().collect();
    let verdict = crate::audio::health::overall(&streams, &engine_info);
    DebugSnapshot {
      ts: now_ms(),
      uptime_ms: self.uptime_ms(),
      app: AppInfo {
        name: "PureVox",
        version: crate::version::VERSION,
        pid: std::process::id(),
        debug_http: st.http,
      },
      system: st.system,
      audio: AudioInfo {
        engine,
        engine_info,
        verdict,
        streams,
      },
      devices,
      ui: st_ui,
      events: events::recent(200),
      plan: st_plan,
      running: st_running,
      device_refresh: crate::devices::refresh_state(),
      recorder: st_recorder,
      calib: st_calib,
      calib_report: st_calib_report,
      net: crate::net::hub().stats(),
      update: crate::update::status(),
      virtual_mic: st_virtual_mic,
    }
  }
}
