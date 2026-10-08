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

//! 音频子系统（DESIGN.md §1）：L0 设备与流 + 会话（多列）。
//!
//! 输入采集线程（原生→48k hop）→ 列工作线程（混合 + 处理链）→ 输出播放线程。

pub mod capture;
pub mod fanout;
/// AEC 远端参考的输出回环采集（远端扬声器 → far 参考）。**实现按平台分文件**：
/// `loopback_windows.rs`（WASAPI loopback）/ `loopback_linux.rs`（PipeWire monitor，`parec`）/
/// `loopback_other.rs`（明确不可用）。
#[cfg(windows)]
#[path = "loopback_windows.rs"]
pub mod loopback;
#[cfg(target_os = "linux")]
#[path = "loopback_linux.rs"]
pub mod loopback;
#[cfg(not(any(windows, target_os = "linux")))]
#[path = "loopback_other.rs"]
pub mod loopback;
pub mod playback;
pub mod probe;
pub mod tone;
/// Linux 虚拟麦克风（PipeWire）的创建 / 移除（「驱动」页面）。
pub mod virtual_mic;

use std::sync::atomic::{AtomicBool, Ordering::Relaxed};
use std::sync::{Arc, Mutex};
use std::thread::JoinHandle;

use crate::config;
use crate::debug::SharedHub;
use crate::engine::calib::CalibHub;
use crate::engine::session::Session;
use crate::plan::Plan;
use crate::recorder::RecorderHub;

/// 引擎内部采样率。
pub const SAMPLE_RATE: u32 = 48_000;
/// 10 ms hop，按时间派生（48 kHz 下 480 样本）。
pub const HOP: usize = (SAMPLE_RATE / 100) as usize;

/// 打开设备时重试：Linux ALSA 的设备是**排他**的，而 cpal 关闭流是异步的
/// （真正的 `snd_pcm_close` 在 cpal 内部线程上），重建会话时紧接着打开会得到
/// 「读取默认格式失败：The requested device is temporarily busy」。
/// 只对「busy」类错误退避重试（最多 ~1.5 s），其它错误立即返回。
pub(crate) fn with_device_retry<T>(mut f: impl FnMut() -> Result<T, String>) -> Result<T, String> {
  let mut last = String::new();
  for attempt in 0..20 {
    match f() {
      Ok(v) => return Ok(v),
      Err(e) => {
        // 设备还在异步关闭（cpal 的 ALSA 真正 close 在它自己的线程上）时会看到
        // 「temporarily busy」或「not available / may have been disconnected」；
        // 都退避重试，别的错误立即返回。
        let retryable = ["busy", "Busy", "not available", "disconnected", "占用"];
        let busy = retryable.iter().any(|k| e.contains(k));
        last = e;
        if !busy {
          return Err(last);
        }
        // 退避 150 ms，给 cpal 把旧流真正关掉
        std::thread::sleep(std::time::Duration::from_millis(150));
        let _ = attempt;
      }
    }
  }
  Err(last)
}

/// 工作线程句柄：置停止标志并等待线程退出。
pub struct WorkerHandle {
  stop: Arc<AtomicBool>,
  join: Option<JoinHandle<()>>,
}

impl WorkerHandle {
  pub fn new(stop: Arc<AtomicBool>, join: JoinHandle<()>) -> Self {
    WorkerHandle {
      stop,
      join: Some(join),
    }
  }

  pub fn stop(mut self) {
    self.stop.store(true, Relaxed);
    if let Some(j) = self.join.take() {
      let _ = j.join();
    }
  }
}

struct State {
  plan: Plan,
  /// 运行中才有会话；停止时为空
  session: Option<Session>,
  running: bool,
}

/// 音频子系统：持有当前计划与由其构建的会话（可启动 / 停止）。
pub struct AudioManager {
  hub: SharedHub,
  rec: Arc<RecorderHub>,
  calib: Arc<CalibHub>,
  state: Mutex<State>,
  /// 重建串行化：界面连续改参数会并发调 `apply_plan`，两次重建若交错打开同一 ALSA 设备
  /// 就会「temporarily busy」（同一设备排他）。启动/停止也走这把锁。
  rebuild: Mutex<()>,
}

impl AudioManager {
  pub fn new(hub: SharedHub) -> Self {
    let plan = config::load_plan();
    let rec = Arc::new(RecorderHub::new(hub.clone()));
    let calib = Arc::new(CalibHub::new(hub.clone()));
    AudioManager {
      hub,
      rec,
      calib,
      state: Mutex::new(State {
        plan,
        session: None,
        running: false,
      }),
      rebuild: Mutex::new(()),
    }
  }

  pub fn plan(&self) -> Plan {
    self.state.lock().unwrap().plan.clone()
  }

  pub fn is_running(&self) -> bool {
    self.state.lock().unwrap().running
  }

  /// 先释放旧会话：Linux 的 ALSA **同一设备不能被打开两次**，旧的还占着，新的
  /// `default_input_config/output_config` 就会报「读取默认格式失败」（Windows 的 WASAPI
  /// 共享模式容忍并发打开，所以那边没暴露）。重建/启动一律先停旧、再建新。
  fn release_session(&self) {
    let old = self.state.lock().unwrap().session.take();
    if let Some(mut s) = old {
      s.stop();
    }
  }

  /// 启动：按当前计划构建并运行会话。返回逐行问题。
  pub fn start(&self) -> Result<Vec<String>, String> {
    let _rebuild = self.rebuild.lock().unwrap();
    let plan = self.plan();
    self.release_session();
    let (session, problems) = Session::build(
      self.hub.clone(),
      &plan,
      self.rec.clone(),
      self.calib.clone(),
    );
    let mut g = self.state.lock().unwrap();
    g.session = Some(session);
    g.running = true;
    drop(g);
    self.hub.set_running(true);
    Ok(problems)
  }

  /// 停止：停会话、释放设备。
  pub fn stop(&self) {
    let _rebuild = self.rebuild.lock().unwrap();
    let mut g = self.state.lock().unwrap();
    if let Some(mut s) = g.session.take() {
      s.stop();
    }
    g.running = false;
    drop(g);
    self.hub.set_running(false);
    // 清掉列概要，界面显示「列未启动」
    self
      .hub
      .set_column(crate::debug::Probe::unavailable("列未启动".to_string()));
  }

  /// 录制 TSE 参考（降噪后、音量归一化，48 kHz 单声道）；返回目标文件路径。
  pub fn record_reference(&self, seconds: f64) -> Result<String, String> {
    let plan = self.plan();
    let path = config::default_tse_reference();
    self.rec.start(&plan, seconds, path.clone())?;
    Ok(path.display().to_string())
  }

  /// 测 AEC 延时：对计划里第一个 AEC 行采集 1.5 s，并向会话所有输出设备送扫频探针；
  /// 结果见 `/debug` 的 `calib`。
  pub fn calibrate_aec_delay(&self) -> Result<String, String> {
    if !self.is_running() {
      return Err("会话未运行：先点顶栏「启动」，再点「校准」".into());
    }
    let plan = self.plan();
    let mut target = None;
    'outer: for (ci, c) in plan.columns.iter().enumerate() {
      for (ri, r) in c.rows.iter().enumerate() {
        if r.ptype == "echo_cancel" {
          target = Some((ci, ri));
          break 'outer;
        }
      }
    }
    let (ci, ri) = target.ok_or_else(|| "计划里没有 AEC 行；请先加一个再测延时".to_string())?;
    // 探针送到会话里所有输出设备：保证 far 与 mic 都能收到同一段扫频
    let mut outs: Vec<String> = Vec::new();
    for c in &plan.columns {
      for r in &c.rows {
        if r.kind == crate::plan::RowKind::Output
          && let Some(d) = r.device.as_deref().filter(|d| !d.is_empty())
          && !outs.iter().any(|x| x == d)
        {
          outs.push(d.to_string());
        }
      }
    }
    // 远端是输出回环时：探针必须送到被回环的那只输出（远端与 mic 听同一份，延时才有意义）。
    // Linux 用 `pacat` 直送 Pulse sink（cpal 打不开 sink 名）；Windows 目标本就是 cpal 输出端点。
    let far = plan.columns[ci].rows[ri]
      .params
      .get("far_device")
      .cloned()
      .unwrap_or_default();
    let mut far_sink: Option<String> = None;
    if let Some(id) = crate::engine::aec::far_loopback_id(&far) {
      if crate::audio::probe::NATIVE {
        // Linux：探针**只**从 far 的这只 sink 发出（探针发声的设备 = AEC 要消除的设备），
        // 不走 cpal 的输出（cpal 也打不开 Pulse sink 名）。
        far_sink = Some(id); // "" = 默认 sink
        outs.clear();
      } else {
        let target = if id.is_empty() {
          crate::devices::default_output_id()
        } else {
          Some(id)
        };
        if let Some(t) = target
          && !outs.iter().any(|x| x == &t)
        {
          outs.push(t);
        }
      }
    }
    if outs.is_empty() && far_sink.is_none() {
      return Err("没有输出设备可播放校准探针".into());
    }
    // 固定参考延时（见 calib::CALIB_REF_MS）：校准测的是绝对延时，不依赖当前存储值
    self
      .calib
      .start(ci, ri, 1.6, 1000.0, crate::engine::calib::CALIB_REF_MS)?;
    crate::engine::calib::play_probe(self.hub.clone(), outs, far_sink);
    Ok(format!("第 {} 列第 {} 行（探针已送出）", ci + 1, ri + 1))
  }

  /// 应用新计划：保存配置；运行中则重建会话（结构性变更），停止时只存计划。
  /// 返回构建过程中的问题（非致命，逐行展示）。应在阻塞线程调用（会打开/关闭设备）。
  pub fn apply_plan(&self, plan: Plan) -> Result<Vec<String>, String> {
    let _rebuild = self.rebuild.lock().unwrap();
    let _ = config::save_plan(&plan);
    let running = self.state.lock().unwrap().running;
    if !running {
      self.state.lock().unwrap().plan = plan;
      return Ok(vec![]);
    }
    // 先停旧会话（Linux ALSA 同一设备不能同时打开，见 `release_session`），再建新会话
    self.release_session();
    let (new_session, problems) = Session::build(
      self.hub.clone(),
      &plan,
      self.rec.clone(),
      self.calib.clone(),
    );
    let mut g = self.state.lock().unwrap();
    g.session = Some(new_session);
    g.plan = plan;
    Ok(problems)
  }
}
