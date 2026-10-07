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
pub mod loopback;
pub mod playback;
pub mod tone;

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

/// 工作线程句柄：置停止标志并等待线程退出。
pub struct WorkerHandle {
  stop: Arc<AtomicBool>,
  join: Option<JoinHandle<()>>,
}

impl WorkerHandle {
  pub fn new(stop: Arc<AtomicBool>, join: JoinHandle<()>) -> Self {
    WorkerHandle { stop, join: Some(join) }
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
  session: Session,
}

/// 音频子系统：持有当前计划与由其构建的会话。
pub struct AudioManager {
  hub: SharedHub,
  rec: Arc<RecorderHub>,
  calib: Arc<CalibHub>,
  state: Mutex<State>,
}

impl AudioManager {
  pub fn new(hub: SharedHub) -> Self {
    let plan = config::load_plan();
    let rec = Arc::new(RecorderHub::new(hub.clone()));
    let calib = Arc::new(CalibHub::new(hub.clone()));
    let (session, _problems) = Session::build(hub.clone(), &plan, rec.clone(), calib.clone());
    AudioManager { hub, rec, calib, state: Mutex::new(State { plan, session }) }
  }

  pub fn plan(&self) -> Plan {
    self.state.lock().unwrap().plan.clone()
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
        if r.kind == crate::plan::RowKind::Output {
          if let Some(d) = r.device.as_deref().filter(|d| !d.is_empty()) {
            if !outs.iter().any(|x| x == d) {
              outs.push(d.to_string());
            }
          }
        }
      }
    }
    // 远端是输出回环时：探针送到被回环的那只输出（远端与 mic 听同一份，延时才有意义）
    let far = plan.columns[ci].rows[ri]
      .params
      .get("far_device")
      .cloned()
      .unwrap_or_default();
    if let Some(id) = crate::engine::aec::far_loopback_id(&far) {
      let target = if id.is_empty() {
        crate::devices::default_output_id()
      } else {
        Some(id)
      };
      if let Some(t) = target {
        if !outs.iter().any(|x| x == &t) {
          outs.push(t);
        }
      }
    }
    if outs.is_empty() {
      return Err("没有输出设备可播放校准探针".into());
    }
    // 当前延时（自校正的基准）：新延时 = 旧延时 + 残余
    let current_delay: f64 = plan.columns[ci].rows[ri]
      .params
      .get("far_delay_ms")
      .and_then(|v| v.trim().parse().ok())
      .unwrap_or(0.0);
    self.calib.start(ci, ri, 1.6, 1000.0, current_delay)?;
    crate::engine::calib::play_probe(self.hub.clone(), outs);
    Ok(format!("第 {} 列第 {} 行（探针已送出）", ci + 1, ri + 1))
  }

  /// 应用新计划（结构性变更）：保存配置 → 构建新会话 → 停旧会话 → 换新。
  /// 返回构建过程中的问题（非致命，逐行展示）。应在阻塞线程调用（会打开/关闭设备）。
  pub fn apply_plan(&self, plan: Plan) -> Result<Vec<String>, String> {
    let (new_session, problems) =
      Session::build(self.hub.clone(), &plan, self.rec.clone(), self.calib.clone());
    let _ = config::save_plan(&plan);
    let mut g = self.state.lock().unwrap();
    g.session.stop();
    g.session = new_session;
    g.plan = plan;
    Ok(problems)
  }
}
