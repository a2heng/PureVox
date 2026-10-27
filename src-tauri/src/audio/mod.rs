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
/// 实测时钟（设备 vs 系统）：只在测试里编译，见模块注释。
#[cfg(test)]
mod clock_probe;
/// 设备池（DESIGN.md §6.5）：每个 `(设备, 角色)` 只打开一次 + 引用计数
pub mod device_pool;
pub mod fanout;
pub mod health;
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
/// 输出设备的混音：多列 → 一个设备（逐源对齐后相加）
pub mod mixer;
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

/// Linux 上 Pulse / PipeWire 命令行工具（`parec` / `pacat` / `pactl`）缺失时的提示。
///
/// **只装了 PipeWire 的系统同样需要这个包**：`pipewire-pulse` 只提供兼容服务，而这些客户端
/// 工具在 `pulseaudio-utils` 里（Ubuntu 26.04 默认就不装它，实测回环采集直接
/// 「没有那个文件或目录」）。deb / rpm 已把它放进 `recommends`（apt/dnf 默认会装），
/// AppImage 装不了系统包，所以运行时的报错必须说清**该装什么**。
#[cfg(target_os = "linux")]
pub const PULSE_TOOLS_HINT: &str = "缺少 PulseAudio 命令行工具（parec / pacat / pactl）：Debian / Ubuntu 装 pulseaudio-utils，Fedora 装 pulseaudio-utils";

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
  if last.contains("busy") || last.contains("Busy") || last.contains("占用") {
    return Err(format!(
      "{last}（设备被其它程序独占：Linux 输入已走 PipeWire 共享；输出仍走 ALSA，可改用「系统默认输出（PipeWire 共享）」或先关闭占用它的程序）"
    ));
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

  /// 线程**起不来**时的退化句柄：`stop` 已是停，join 可缺。
  /// 与 [`WorkerHandle::new`] 分开是因为 `new` 要求一定有 join 句柄。
  pub fn dead(stop: Arc<AtomicBool>) -> Self {
    WorkerHandle { stop, join: None }
  }
}

struct State {
  plan: Plan,
  /// 运行中才有会话；停止时为空
  session: Option<Session>,
  running: bool,
  /// 每次启停自增。校准「临时启动会话」的收尾据此判断：这期间有没有被用户或计划改动过
  /// ——改过就不能再由我们停掉（否则会把用户刚起的会话也停了）。
  epoch: u64,
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
    hub.set_plan(plan.clone());
    hub.set_engine_stopped();
    AudioManager {
      hub,
      rec,
      calib,
      state: Mutex::new(State {
        plan,
        session: None,
        running: false,
        epoch: 0,
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

  /// 当前运行代号（每次启停 +1）。见 [`State::epoch`]。
  pub fn run_epoch(&self) -> u64 {
    self.state.lock().unwrap().epoch
  }

  /// 代号仍是 `expect` 且在运行 → 说明这期间没人动过引擎，我们可以安全收尾。
  fn untouched_since(&self, expect: u64) -> bool {
    let g = self.state.lock().unwrap();
    g.running && g.epoch == expect
  }

  /// 启动：按当前计划构建并运行会话。返回逐行问题。
  ///
  /// **先建后停**（DESIGN.md §6.5 的差异重启）：设备由 `device_pool` 按引用计数持有，
  /// 先建才能让「没变的设备」**复用同一条流**（不关不重开 → 其它列的 `started_at` 都不动）。
  /// 从前必须「先停旧会话再建新会话」是因为 ALSA 排他 —— 现在排他由池的单开 + 引用计数承担；
  /// 而设备 id 是本仓库自己枚举出来的（同一硬件永远是同一个 id），换了 id 就是换了 PCM，
  /// 短暂并存也不会撞排他。
  pub fn start(&self) -> Result<Vec<String>, String> {
    let _rebuild = self.rebuild.lock().unwrap();
    let plan = self.plan();
    let old = self.state.lock().unwrap().session.take();
    let generation = self.state.lock().unwrap().epoch + 1;
    let outcome = Session::build(
      self.hub.clone(),
      &plan,
      self.rec.clone(),
      self.calib.clone(),
      generation,
    );
    let mut g = self.state.lock().unwrap();
    g.session = Some(outcome.session);
    g.running = true;
    g.epoch = generation;
    drop(g);
    // 提交媒体槽位：旧代次的（已删/已换位置的媒体行）在这里清掉
    if let Some(s) = self.state.lock().unwrap().session.as_ref() {
      crate::engine::media::hub().commit(s.media_gen);
    }
    self.hub.set_engine_running(generation);
    self.hub.set_plan(plan);
    // 旧会话在这里才停：池的引用计数只在「真没人用」时才关设备
    if let Some(mut s) = old {
      s.stop();
    }
    self.hub.set_running(true);
    Ok(outcome.problems)
  }

  /// 停止：停会话、释放设备。
  pub fn stop(&self) {
    let _rebuild = self.rebuild.lock().unwrap();
    let mut g = self.state.lock().unwrap();
    if let Some(mut s) = g.session.take() {
      s.stop();
    }
    g.running = false;
    g.epoch += 1;
    drop(g);
    self.hub.set_running(false);
    // 显式停引擎才清媒体槽位与列概要（`Session::stop` 自己不清：先建后停时会误伤新会话）
    crate::engine::media::hub().clear();
    self.hub.set_engine_stopped();
  }

  /// 录制 TSE 参考（降噪后、原样，48 kHz 单声道）；返回目标文件路径。
  ///
  /// **必须引擎在跑**：参考取自列线程的实时链路，没跑就没有样本（legacy 也是这个语义，
  /// 并明确提示「请先启动音频处理」）。同时挂看门狗：10 秒内收不到音频就明确失败，
  /// 不会永远停在「录制中 0.0/10 s」。
  pub fn record_reference(&self, seconds: f64) -> Result<String, String> {
    if !self.is_running() {
      return Err("请先启动音频处理，再录制参考（TSE 参考取自运行中的链路）".into());
    }
    let plan = self.plan();
    let path = config::default_tse_reference();
    let token = self.rec.start(&plan, seconds, path.clone())?;
    let rec = self.rec.clone();
    let secs = seconds.max(1.0);
    std::thread::Builder::new()
      .name("tse-rec-watchdog".into())
      .spawn(move || {
        std::thread::sleep(std::time::Duration::from_secs_f64(secs + 2.0));
        rec.timeout_if_recording(
          token,
          format!(
            "录制失败：{secs:.0} 秒内没有收到音频（确认引擎已启动、TSE 行已启用、麦克风有输入）"
          ),
        );
      })
      .ok();
    Ok(path.display().to_string())
  }

  /// 测 AEC 延时：对计划里第一个 AEC 行采集 1.5 s，并向会话所有输出设备送扫频探针；
  /// 结果见 `/debug` 的 `calib`。
  ///
  /// **不需要先手动启动**：校准必须有列工作线程把「原始近端 hop + 远端窗口」喂进
  /// `calib::feed`（`engine/session.rs`），没有会话就没人喂。所以会话未运行时这里
  /// **临时启动**一个，第二个返回值是 `Some(运行代号)` 表示「本次是临时的，测完要收尾
  /// 停掉」——交给 [`Self::finish_temp_session`]（看门狗线程在 `main.rs`）。
  pub fn calibrate_aec_delay(&self) -> Result<(String, Option<u64>), String> {
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
    // 会话没在跑 → **临时启动**一个：校准要列线程喂数据，没有会话永远停在「采集中」。
    // 先把「有没有 AEC 行 / 有没有输出可放探针」这些前置条件查完再启动，避免白启一次。
    let temp_epoch = if self.is_running() {
      None
    } else {
      let problems = self.start()?;
      self.hub.push_ui(
        "info",
        format!(
          "校准需要会话在跑（列线程喂近端/远端），已临时启动{}",
          if problems.is_empty() {
            String::new()
          } else {
            format!("（有 {} 行未配置）", problems.len())
          }
        ),
      );
      Some(self.run_epoch())
    };
    // 固定参考延时（见 calib::CALIB_REF_MS）：校准测的是绝对延时，不依赖当前存储值。
    // 设备标识只进报告（诊断 / AI 复现），不参与算法。
    let mic_dev = plan.columns[ci].rows[ri]
      .device
      .clone()
      .filter(|d| !d.is_empty());
    let far_dev = Some(far.clone()).filter(|d| !d.is_empty());
    if let Err(e) = self.calib.start(crate::engine::calib::CalibStart {
      col: ci,
      row: ri,
      seconds: 1.6,
      current_delay_ms: crate::engine::calib::CALIB_REF_MS,
      mic_device: mic_dev,
      far_device: far_dev,
    }) {
      // 启动失败要把临时会话收回去，不能让它一直跑着
      if let Some(epoch) = temp_epoch
        && self.untouched_since(epoch)
      {
        self.stop();
      }
      return Err(e);
    }
    crate::engine::calib::play_probe(self.hub.clone(), outs, far_sink);
    Ok((
      format!("第 {} 列第 {} 行（探针已送出）", ci + 1, ri + 1),
      temp_epoch,
    ))
  }

  /// 校准是否还在进行中（采集中 **或** 正在算）——看门狗据此决定何时收尾。
  pub fn calib_in_flight(&self) -> bool {
    self.calib.in_flight()
  }

  /// 校准跑完后收尾：**只停自己临时启动的会话**。
  ///
  /// - `Some(epoch)`：本次是临时启动的，且代号没变（期间用户没启停过、计划也没重建）
  ///   → 停回未运行，保持用户原本的状态；
  /// - 期间用户自己点了「启动」（代号变了）→ **什么都不做**，不能把用户的会话停掉；
  /// - `None`：本来就是用户在跑的，不动。
  ///
  /// 调用方是看门狗线程：校准**有独立超时**（`calib` 内部 deadline + 这里的硬上限），
  /// 所以这里一定会被调到——不会出现「临时会话一直跑着」。
  pub fn finish_temp_session(&self, temp: Option<u64>) {
    let Some(epoch) = temp else {
      return;
    };
    if !self.untouched_since(epoch) {
      self
        .hub
        .push_ui("info", "校准期间引擎被改动过，保留当前运行状态".into());
      return;
    }
    self.stop();
    self
      .hub
      .push_ui("info", "校准完成，已停掉临时启动的会话".into());
  }

  /// 应用新计划：保存配置；运行中则重建会话（结构性变更），停止时只存计划。
  ///
  /// **事务性**：运行中先构建新会话；只要有**关键失败**（启用的行拿不到设备/模型），
  /// 就停掉新会话、保留旧会话与旧计划，返回错误——绝不让一次坏改动把正在工作的链路停掉。
  /// 成功才提交媒体槽位、停旧会话、落盘。返回构建过程中的问题（非致命，逐行展示）。
  pub fn apply_plan(&self, plan: Plan) -> Result<Vec<String>, String> {
    let _rebuild = self.rebuild.lock().unwrap();
    let running = self.state.lock().unwrap().running;
    if !running {
      self.state.lock().unwrap().plan = plan.clone();
      let _ = config::save_plan(&plan);
      self.hub.set_plan(plan);
      return Ok(vec![]);
    }
    let old = self.state.lock().unwrap().session.take();
    let old_generation = self.state.lock().unwrap().epoch;
    let generation = old_generation + 1;
    let outcome = Session::build(
      self.hub.clone(),
      &plan,
      self.rec.clone(),
      self.calib.clone(),
      generation,
    );
    if !outcome.critical.is_empty() {
      // 回滚：新会话从未提交。它的列线程已经起来，先停干净；媒体槽位只删自己这一代。
      let mut new_session = outcome.session;
      crate::engine::media::hub().abort(new_session.media_gen);
      new_session.stop();
      {
        let mut g = self.state.lock().unwrap();
        g.session = old;
        g.running = true;
        // 代次不变：这次改动没有生效
        g.epoch = old_generation;
      }
      self.hub.set_engine_running(old_generation);
      self.hub.clear_columns();
      self.hub.set_plan(self.plan());
      let msg = format!(
        "新计划未应用（旧会话保持运行）：{}",
        outcome.critical.join("；")
      );
      crate::debug::events::log("error", "engine", msg.clone());
      return Err(msg);
    }
    let media_gen = outcome.session.media_gen;
    {
      let mut g = self.state.lock().unwrap();
      g.session = Some(outcome.session);
      g.plan = plan.clone();
      g.epoch = generation;
    }
    crate::engine::media::hub().commit(media_gen);
    self.hub.set_engine_running(generation);
    self.hub.set_plan(plan.clone());
    let _ = config::save_plan(&plan);
    if let Some(mut s) = old {
      s.stop();
    }
    crate::debug::events::log(
      "info",
      "engine",
      format!(
        "计划已应用：第 {generation} 代，{} 个提醒",
        outcome.problems.len()
      ),
    );
    Ok(outcome.problems)
  }

  /// 运行中改一个**热参数**（与模型无关的标量：`gain` 行的 `gain_db`、`loopback` 行的
  /// `volume_db`，见 `engine/hot.rs`）：写进计划并落盘，运行中的会话**不重建**、下一 hop 生效。
  ///
  /// 为什么要有这条路：从前**所有**参数改动都走 [`apply_plan`]，而 `apply_plan` 是结构性
  /// 重建（关掉全部设备再重开）。Linux 上 ALSA 排他、一次重建几百毫秒起步（2 列撞同一张卡
  /// 时实测 7.25 s），界面按每个 `change` 事件调一次 —— 改增益每 1 dB 重建一次，命令队列
  /// 越堆越长，表现就是「改音量卡死、连停止都点不动」（2026-10-18 用户实测）。
  ///
  /// 结构性参数（模型 / 设备 / 参考音频…）**不许**走这里：调用方照
  /// [`crate::engine::registry::is_hot`] 决定走哪条路，本函数也自己把这道门守住。
  pub fn set_row_param(
    &self,
    col: usize,
    row: usize,
    key: &str,
    value: &str,
  ) -> Result<(), String> {
    let ptype = {
      let g = self.state.lock().unwrap();
      let r = g
        .plan
        .columns
        .get(col)
        .and_then(|c| c.rows.get(row))
        .ok_or_else(|| format!("计划里没有第 {} 列第 {} 行", col + 1, row + 1))?;
      if !crate::engine::registry::is_hot(&r.ptype, key) {
        return Err(format!(
          "{} 的 {key} 不是热参数（要重建会话：走 apply_plan）",
          r.ptype
        ));
      }
      r.ptype.clone()
    };
    // 取值约束来自注册表（界面有 min/max，后端不能信界面）
    let (lo, hi, _) = crate::engine::registry::number_spec(&ptype, key)
      .ok_or_else(|| format!("{ptype} 的 {key} 不是数值参数"))?;
    let db: f64 = value
      .trim()
      .parse()
      .map_err(|_| format!("{key} 取值不是数字：{value}"))?;
    if !db.is_finite() || db < lo || db > hi {
      return Err(format!("{key} 必须在 {lo} ~ {hi} 之间，收到 {value}"));
    }
    let db = db as f32;

    let mut g = self.state.lock().unwrap();
    if let Some(r) = g
      .plan
      .columns
      .get_mut(col)
      .and_then(|c| c.rows.get_mut(row))
    {
      r.params.insert(key.to_string(), value.trim().to_string());
    }
    let plan = g.plan.clone();
    let running = g.session.is_some();
    // 运行中：写单元格（**不重建**）。会话里没登记这个热参数 = 内部不一致，说清楚。
    let result = match g.session.as_ref() {
      Some(s) => s
        .set_hot_param(col, row, key, db)
        .ok_or_else(|| format!("第 {} 列第 {} 行没有登记热参数 {key}", col + 1, row + 1)),
      None => Ok(()),
    };
    drop(g);
    // 运行中合并落盘（拖动滑杆时每 10 ms 一次 fsync 会把命令线程堵死）；
    // 未运行没有实时性要求，直接写，保证 `get_plan` 立刻能读到。
    if running {
      config::save_plan_debounced(plan);
    } else {
      let _ = config::save_plan(&plan);
    }
    result
  }

  /// 热参数落盘（**不重建会话**，DESIGN.md §5）：音量 / 续播位置等改动用它。
  /// 同时更新内存计划，保证 `get_plan` 与运行中实例一致。
  pub fn persist_plan(&self, plan: Plan) -> Result<(), String> {
    let _rebuild = self.rebuild.lock().unwrap();
    self.state.lock().unwrap().plan = plan.clone();
    drop(_rebuild);
    config::save_plan_debounced(plan);
    Ok(())
  }
}

#[cfg(test)]
mod tests {
  use super::*;

  /// 测试用的管理器：**自给自足的计划**，而且**绝不碰用户真实配置**。
  ///
  /// `AudioManager::new` 读的是 `config::config_dir()`：测试构建里它指向**本线程专属**的临时
  /// 目录（见 `config.rs` 的 `test_dir`），所以既不会依赖本机配置（本机若有 AEC 行 / 选了真实
  /// 设备，断言就会飘），也**不会写回**开发者真实的 `~/.purevox/session.json`
  /// —— 一次 `cargo test` 把会话计划覆盖成默认计划，2026-10-18 实际踩过。
  /// 目录按线程分是因为测试并行跑：共用一份的话两个测试的 `persist_plan` 会互相覆盖。
  fn mgr() -> AudioManager {
    let m = AudioManager::new(crate::debug::DebugHub::new());
    m.persist_plan(Plan::default()).expect("写入测试计划");
    m
  }

  /// 临时会话的收尾纪律：`Some(epoch)` 只在**代号没变**时才停回原状。
  ///
  /// 校准期间用户自己点了「启动」就会推高代号；那时若还照停，会把用户刚起的会话
  /// 也停掉——这比「临时会话没收干净」严重得多。
  #[test]
  fn finish_temp_session_respects_run_epoch() {
    let m = mgr();
    assert!(!m.is_running(), "起点应是未运行");
    m.start().expect("启动会话");
    let epoch = m.run_epoch();
    assert!(m.untouched_since(epoch));

    // 用户在校准期间自己启停了一次 → 代号变了
    m.stop();
    m.start().expect("用户自己重新启动");
    assert_ne!(m.run_epoch(), epoch, "启停必须推高代号");
    assert!(!m.untouched_since(epoch), "代号变了就不能再由我们停");

    m.finish_temp_session(Some(epoch));
    assert!(m.is_running(), "期间被用户改动过 → 必须保留用户的运行状态");
    m.stop();
  }

  #[test]
  fn finish_temp_session_is_noop_for_user_started_session() {
    let m = mgr();
    m.start().expect("启动会话");
    // None = 本来就是用户在跑的会话，什么都不做
    m.finish_temp_session(None);
    assert!(m.is_running(), "不该停掉用户自己启动的会话");
    m.stop();
  }

  #[test]
  fn finish_temp_session_stops_untouched_temp_session() {
    let m = mgr();
    m.start().expect("临时启动");
    let epoch = m.run_epoch();
    m.finish_temp_session(Some(epoch));
    assert!(
      !m.is_running(),
      "临时会话测完必须停回未运行，不能一直占着音频设备"
    );
  }

  /// 带一条 `gain` 处理行的计划：热参数测试用（`tone` 输入不需要设备，输出行不配置即可）。
  fn plan_with_gain(db: &str) -> Plan {
    use crate::plan::{ColumnSpec, RowKind, RowSpec};
    let row = |kind, ptype: &str, params: Vec<(&str, &str)>| RowSpec {
      kind,
      ptype: ptype.into(),
      device: None,
      enabled: true,
      params: params
        .into_iter()
        .map(|(k, v)| (k.to_string(), v.to_string()))
        .collect(),
    };
    Plan {
      columns: vec![ColumnSpec {
        enabled: true,
        rows: vec![
          row(RowKind::Input, "tone", vec![]),
          row(RowKind::Process, "gain", vec![("gain_db", db)]),
          row(RowKind::Output, "audio_output", vec![]),
        ],
      }],
    }
  }

  /// **热参数**（`gain_db` / `volume_db`）走 `set_row_param`：只写计划 + 写单元格，
  /// **不重建会话**；结构性参数（模型/设备…）一律拒绝，且拒绝要说清原因。
  ///
  /// 动机：以前所有参数都走 `apply_plan` 全量重建，改增益每 1 dB 重建一次 ——
  /// Linux 上 ALSA 排他，一次重建几百毫秒起步（2 列撞同一张卡时实测 7.25 s），
  /// 队列越堆越长，表现就是「改音量卡死、连停止都点不动」。
  #[test]
  fn set_row_param_only_accepts_hot_scalars() {
    let m = mgr();
    m.apply_plan(plan_with_gain("0")).expect("写计划");
    // 未运行也能改（只落盘，下次启动生效）
    m.set_row_param(0, 1, "gain_db", "-6").expect("热参数");
    let saved = crate::config::load_plan();
    assert_eq!(
      saved.columns[0].rows[1]
        .params
        .get("gain_db")
        .map(|s| s.as_str()),
      Some("-6"),
      "热参数要落盘"
    );
    // 结构性参数不许走这条路
    let e = m.set_row_param(0, 1, "model", "x").unwrap_err();
    assert!(e.contains("不是热参数"), "要说清为什么不行：{e}");
    // 取值范围由注册表说了算（界面有 min/max，后端不能信界面）
    assert!(m.set_row_param(0, 1, "gain_db", "999").is_err(), "越界拒绝");
    assert!(
      m.set_row_param(0, 1, "gain_db", "abc").is_err(),
      "非数字拒绝"
    );
    assert!(
      m.set_row_param(0, 9, "gain_db", "0").is_err(),
      "行号越界拒绝"
    );
  }

  /// 运行中改热参数要**落到会话的单元格**（`Session::set_hot_param` 的寻址没接错）。
  /// 会话里没登记这个参数时报明确的内部错误，而不是静默成功。
  #[test]
  fn set_row_param_reaches_running_session() {
    let m = mgr();
    m.apply_plan(plan_with_gain("0")).expect("写计划");
    m.start().expect("启动会话");
    assert!(
      m.set_row_param(0, 1, "gain_db", "-6").is_ok(),
      "运行中的 gain 行必须登记了热参数单元格"
    );
    m.stop();
  }

  /// 校准**不必先手动启动**：没有 AEC 行时报的是「没有 AEC 行」而不是「会话未运行」，
  /// 说明前置检查排在临时启动之前，不会白启一次会话。
  #[test]
  fn calibrate_without_aec_row_reports_row_problem_not_running() {
    let m = mgr();
    // 计划由 `mgr()` 置成 `Plan::default()`（一条输入 + 一条输出、没有 AEC 行），
    // 而配置目录已被 `use_test_dir()` 指到临时目录 —— 测试**不读也不写**用户真实配置。
    let e = m.calibrate_aec_delay().expect_err("没有 AEC 行应当报错");
    assert!(e.contains("AEC"), "应报缺 AEC 行，实际：{e}");
    assert!(!m.is_running(), "前置检查失败不该留下运行中的会话");
  }
}
