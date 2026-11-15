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

//! 会话：按 [`Plan`] 构建并运行若干独立的列（ARCHITECTURE.md §3、§5）。
//!
//! 每列一个工作线程：系统时钟每 10 ms 自上而下执行该列的行——
//! 输入行取 1 hop 并按列内输入数等权混合、处理行就地处理、输出行把当前信号推给该行扇出。
//! 结构性变更（增删/排序行、换设备、改型号、增删列）由上层重建整个 Session。

use std::collections::BTreeMap;
use std::panic::{AssertUnwindSafe, catch_unwind};
use std::sync::atomic::{AtomicBool, Ordering::Relaxed};
use std::sync::{Arc, Mutex};
use std::thread::JoinHandle;
use std::time::{Duration, Instant};

use rtrb::Consumer;

use super::aec::{AecRow, AecStage, FAR_HIST_SAMPLES, FarHistory, FarWin, spawn_far_pump};
use super::calib::CalibHub;
use super::latency::{LatencyAnalyzer, LatencyTap, RowTapInfo};
use super::media::{self, MediaCmd, MediaSlot};
use super::registry::{self, ParamValue, Params};
use super::stage::{FrameContext, Stage, StageError};
use crate::audio::fanout::MultiFanout;
use crate::audio::health;
use crate::audio::{HOP, WorkerHandle, capture, loopback};
use crate::debug::{ColumnInfo, Probe, RowInfo, SharedHub, events, metrics, threads};
use crate::dsp::meter::RateMeter;
use crate::infer::aec::Aec;
use crate::plan::{ColumnSpec, Plan, RowKind};
use crate::recorder::{Phase, RecorderHub};

/// 列工作线程节拍（= 1 hop）。
const TICK: Duration = Duration::from_millis(10);
/// 列概要发布周期。
const PUBLISH: Duration = Duration::from_millis(200);
/// 行输入环水位：目标 2 hop，超过 4 hop 丢最旧（ARCHITECTURE.md §8）。
const ROW_TARGET: usize = 2 * HOP;
const ROW_CAP: usize = 4 * HOP;
/// 运行期重新打开设备的退避下限 / 上限（**绝不在音频 tick 里直接开设备**）。
/// 第一次失败后 1 s 重试，连续失败翻倍，最长 30 s；成功后清零。
const REOPEN_BACKOFF_MIN: Duration = Duration::from_secs(1);
const REOPEN_BACKOFF_MAX: Duration = Duration::from_secs(30);
/// 单拍处理耗时超过「一跳预算 - 2 ms」即计入节拍超时（留 2 ms 给调度）。
const TICK_LATE_WARN: Duration = Duration::from_millis(2);

/// 输入行「租约还在、却这么久一点 hop 都没拿到」= 流已经停了（设备拔出、休眠回来驱动没恢复、
/// 驱动重置）。超过它就把租约放掉、让运行期重试重新打开（ARCHITECTURE.md §6.6）。
///
/// 为什么按「有没有 hop」判而不是看回调计数：设备活着时**静音也在出数据**，只有流真停了
/// 才会一格都拿不到；这样不会把「用户不说话」误判成设备掉线。
const INPUT_STALL_REOPEN: Duration = Duration::from_secs(5);

#[derive(Clone, Copy)]
struct InputRef {
  src: usize,
  from_port: u8,
  to_port: u8,
}

impl Row {
  /// 本节点自身的算法延迟（样本）；多通道取通道间最大值（各通道 Stage 相同）。
  fn stage_latency(&self) -> usize {
    // 停用的行是直通（不跑 Stage），实际延迟 0；声明值必须跟着变，否则 PDC 会
    // 白白给并行支路补延迟，反而对不齐。
    if !self.enabled {
      return 0;
    }
    if let Some(a) = self.aec_stage.as_ref() {
      return a.latency_samples();
    }
    self
      .stage
      .iter()
      .map(|s| s.latency_samples())
      .max()
      .unwrap_or(0)
  }
}

/// 一条入线的整数样本延迟（PDC 用）。
///
/// 环长 = delay + 1：写当前样本、读 `delay` 个样本之前的那一个。`delay = 0` 时不建环。
struct DelayLine {
  buf: Vec<f32>,
  pos: usize,
  delay: usize,
}

impl DelayLine {
  fn new(delay: usize) -> Self {
    DelayLine {
      buf: vec![0.0; delay + 1],
      pos: 0,
      delay,
    }
  }

  fn process(&mut self, input: &[f32; HOP]) -> [f32; HOP] {
    let mut out = [0.0f32; HOP];
    let n = self.buf.len();
    for i in 0..HOP {
      self.buf[self.pos] = input[i];
      let read = (self.pos + n - self.delay) % n;
      out[i] = self.buf[read];
      self.pos = (self.pos + 1) % n;
    }
    out
  }
}

struct Row {
  /// 稳定节点 ID（连线引用它；调试 / 热参数仍按 (组, 行) 寻址）。
  id: u32,
  /// 入线所指的前驱（构建时已解析成行下标 + 目标端口）。
  inputs: Vec<InputRef>,
  kind: RowKind,
  /// 注册表类型（`audio_input` / `denoise` / …）：调试接口按它识别行
  ptype: String,
  label: String,
  enabled: bool,
  /// 上一次的 enabled（列工作线程据此把开关翻转转发给媒体行）
  enabled_prev: bool,
  error: Option<String>,
  /// 未配置（如未选择设备）：不是错误，只是还没配好
  unconfigured: bool,
  /// 这一行是否是**配置好的输入源**（与当前租约是否拿到无关）。
  /// 列内混合系数用它计数，而不是用「活着的租约数」——否则一个设备打不开，
  /// 其余输入会突然 +6 dB（ARCHITECTURE.md §3.2 明确要求按配置的输入数）。
  source: bool,
  /// 处理行：本行推理耗时 EMA / 最大（ms），进调试概要
  inf_ms: f32,
  inf_max_ms: f32,
  /// 推理耗时超过一跳预算的次数（累计 + 速率）
  inf_over: u64,
  inf_over_rate: RateMeter,
  /// 本节点的音频通道数（端口 = 通道）
  channels: u32,
  /// 输入行：本行源的 48k hop 读端（**只包含下面 `input_channels` 里的通道**）
  input: Vec<Consumer<f32>>,
  /// 订阅的物理通道号（与 `input` 一一对应）
  input_channels: Vec<usize>,
  /// 设备原生通道数（用于「单声道设备铺到多通道节点」）
  src_channels: usize,
  /// 构建期算出的「需要订阅的通道」：只包含下游真正用到的端口
  sub_channels: Vec<usize>,
  /// 输入行：行输入环累计丢弃的样本数（水位超 `ROW_CAP` 丢最旧）。
  ///
  /// 这个计数以前没有 ——源「一大块到达」时环被瞬间灌满、按策略丢最旧，**丢多少完全看不见**，
  /// 用户看到的只是「声音规律地卡」（2026-10-18 实测：Linux 回环 100 ms 一块，每块丢掉大半）。
  ring_dropped: u64,
  /// 行输入环丢弃的实测速率（3 s 窗口；累计只涨不降，速率才看得出还在不在丢）
  ring_drop_rate: RateMeter,
  /// 本行绑的设备 ID（重启用它按需取回设备租约；输入/输出行才有）
  device: Option<String>,
  /// 本行的采集是否**池化**（普通采集行 = 是；AEC 行 / 回环行各自持有采集路径，见 §6.5 待做）
  pooled_input: bool,
  /// 本行的输出是否池化（普通输出行 = 是）
  pooled_output: bool,
  /// 输入行的设备租约（池持有设备，行只拿这个；停用即放掉 → 引用归零才真关）
  in_lease: Option<crate::audio::device_pool::InputLease>,
  /// 输出行的设备租约（内含该设备唯一那条混音的这一路源）
  out_lease: Option<crate::audio::device_pool::OutputLease>,
  /// 运行期重新打开（**后台线程**，绝不阻塞 10 ms tick）：
  /// 正在尝试时是接收端；失败后按指数退避，`*_retry_at` 之前不重试。
  in_pending:
    Option<std::sync::mpsc::Receiver<Result<crate::audio::device_pool::InputLease, String>>>,
  in_retry_at: Option<Instant>,
  in_failures: u32,
  /// 设备打开失败、正按退避自动重试（设备不在 / 被独占 / USB 还没枚举完）。
  ///
  /// 这类失败**不进 `critical`**：整盘回滚会让用户连改设备、删列都做不到（issue #10：
  /// 两列同时「设备不在」时，改好一列就被回滚，永远修不完），而设备回来之后运行期的
  /// 退避重试会自己接上（§6.3）。模型类失败仍然 critical —— 那不会自己好。
  device_pending: bool,
  /// 输入行最后一次真正拿到 hop 的时刻：长于 [`INPUT_STALL_REOPEN`] 没数据 = 流已停，
  /// 放掉租约重新打开（§6.6 自愈）。
  last_hop_at: Instant,
  out_pending:
    Option<std::sync::mpsc::Receiver<Result<crate::audio::device_pool::OutputLease, String>>>,
  out_retry_at: Option<Instant>,
  out_failures: u32,
  /// AEC 输入行：mic 读端 + far 历史 + 本行模型状态
  aec: Option<AecRow>,
  /// `remote_mic` 输入行：网络入站队列（NETWORK.md §4.1）
  net_input: Option<Arc<crate::net::NetHub>>,
  /// 节点输入微调（Element 的 node input gain；热参数）
  input_gain: Option<Arc<crate::engine::hot::HotGain>>,
  /// 处理行：逐通道一个 Stage（各通道 DSP/模型状态独立）
  stage: Vec<Box<dyn Stage>>,
  /// Session V2：AEC 是两输入处理节点（近端/远端都来自连线）
  aec_stage: Option<AecStage>,
  /// 媒体行（soundpad / player）：运行时命令通道（界面经 MediaHub 寻址）
  media_slot: Option<Arc<MediaSlot>>,
  /// 输出行：逐通道扇出（设备混音从这里订阅）
  output: Option<Arc<MultiFanout>>,
  /// `remote_speaker` 输出行：推给网络中枢（无扇出、无播放线程）
  net_output: bool,
  /// 本节点当前输出峰值（线性；快起慢落，界面电平条用）
  peak: f32,
  /// 本节点输出在整条路径上的累计算法延迟（样本）；PDC 用
  latency: usize,
  /// 每条入线要补的延迟（样本）；与 `inputs` 一一对应
  edge_delays: Vec<usize>,
  /// 每条入线的延迟缓冲（`edge_delays[i] == 0` 时是 `None`，不分配）
  delay_lines: Vec<Option<DelayLine>>,
}

struct ColumnRuntime {
  stop: Arc<AtomicBool>,
  join: Option<JoinHandle<()>>,
  /// 本列拥有的流：Session 停止时一并关闭（池化设备不在这里）
  captures: Vec<WorkerHandle>,
  /// 线程是否还活着（由列线程的 guard 置 false；监督线程据此报 degraded）
  alive: Arc<AtomicBool>,
  /// 这一列的延迟探针（列线程写波形，分析线程读）
  latency: Arc<LatencyTap>,
}

/// 一个已构建并正在运行的会话。
pub struct Session {
  columns: Vec<ColumnRuntime>,
  publisher_stop: Arc<AtomicBool>,
  publisher: Option<JoinHandle<()>>,
  /// 热参数单元格：`(列, 行, 参数名) → 值`。`set_hot_param` 写它，行工作线程下一 hop 生效
  /// （见 `engine/hot.rs`：热参数**不重建会话**）。
  hot: std::collections::HashMap<(usize, usize, String), Arc<crate::engine::hot::HotGain>>,
  /// EQ 行的整条曲线单元格：`(列, 行) → HotEq`（无锁原子，见 `engine/hot.rs`）
  hot_eq: std::collections::HashMap<(usize, usize), Arc<crate::engine::hot::HotEq>>,
  /// 本次构建的媒体槽位代次（提交/回滚由 `AudioManager` 决定）
  pub media_gen: u64,
  /// 本次构建的引擎代次（进调试快照）
  pub generation: u64,
  /// 逐列延迟分析线程（读到 stop 为止）
  latency: Option<LatencyAnalyzer>,
}

/// 一次构建的结果。
pub struct BuildOutcome {
  pub session: Session,
  /// 非致命问题（未配置的行、可恢复的失败……）：界面逐行显示
  pub problems: Vec<String>,
  /// 关键失败（启用的行拿不到设备/模型/参考）：`apply_plan` 据此**回滚**，保留旧会话
  pub critical: Vec<String>,
}

impl Session {
  /// 按计划构建并启动；返回构建结果（不修改计划本身）。
  ///
  /// `generation`：本次会话的代次（`AudioManager` 的运行代号），进调试快照。
  pub fn build(
    hub: SharedHub,
    plan: &Plan,
    rec: Arc<RecorderHub>,
    calib: Arc<CalibHub>,
    generation: u64,
  ) -> BuildOutcome {
    let mut problems = crate::plan::validate(plan);
    let mut critical = Vec::new();
    // 媒体槽位按代次注册：构建成功由 `AudioManager` 提交，失败/回滚只删自己这一代。
    // **绝不在这里 clear()**：先建后停期间旧会话的媒体控件必须仍然可用。
    let media_gen = media::hub().begin();
    // 可视化帧按**引擎代次**换代：新会话开始时清旧帧，旧会话线程（先建后停期间还在跑）
    // 的发布带旧代次，会被快照过滤掉。
    crate::engine::viz::hub().begin(generation);
    let mut columns = Vec::new();
    let mut hot = std::collections::HashMap::new();
    let mut hot_eq = std::collections::HashMap::new();
    for (i, spec) in plan.columns.iter().enumerate() {
      columns.push(build_column(
        &hub,
        i,
        spec,
        &mut problems,
        &mut critical,
        &rec,
        &calib,
        &mut hot,
        &mut hot_eq,
        media_gen,
        generation,
        // 只有多列时才绑核（见 `run_column` 里的说明）：单列绑核只会被钉到忙核上
        plan.columns.len() > 1,
      ));
    }

    // 延迟分析线程：每 500 ms 把各列「输入 hop / 输出 hop」做互相关，并把各段缓冲
    // 水位合成预算（DEBUG.md §1.4）。它只读探针、不在音频线程里做任何分析。
    let latency = LatencyAnalyzer::spawn(
      hub.clone(),
      columns
        .iter()
        .enumerate()
        .map(|(i, c)| (i, c.latency.clone()))
        .collect(),
    );

    // 监督线程：列线程死了（panic / 意外退出）必须在一秒内变成 `degraded`，
    // 而不是让快照继续显示最后一帧「100 hop/s」。
    let watches: Vec<(usize, Arc<AtomicBool>, Arc<AtomicBool>)> = columns
      .iter()
      .enumerate()
      .map(|(i, c)| (i, c.alive.clone(), c.stop.clone()))
      .collect();
    let publisher_stop = Arc::new(AtomicBool::new(false));
    let (ps, hub2) = (publisher_stop.clone(), hub.clone());
    let publisher = std::thread::Builder::new()
      .name("session-supervisor".into())
      .spawn(move || {
        let token = threads::ThreadToken::new("session-supervisor");
        let mut reported = vec![false; watches.len()];
        while !ps.load(Relaxed) {
          for (i, alive, stop) in &watches {
            if !alive.load(Relaxed) && !stop.load(Relaxed) && !reported[*i] {
              reported[*i] = true;
              let reason = format!("第 {} 列的列工作线程意外退出（panic 或内部错误）", i + 1);
              hub2.set_engine_degraded(reason.clone());
              metrics::record("engine.column_dead", 1.0);
            }
          }
          token.beat();
          std::thread::sleep(PUBLISH);
        }
      })
      .ok();

    // 引擎状态与计划由 `AudioManager` 在**提交**时写入（回滚时不能把新计划写进快照）。
    events::log(
      "info",
      "engine",
      format!(
        "会话构建完成：第 {generation} 代，{} 列，{} 个问题，{} 个关键失败",
        plan.columns.len(),
        problems.len(),
        critical.len()
      ),
    );

    BuildOutcome {
      session: Session {
        columns,
        publisher_stop,
        publisher,
        hot,
        hot_eq,
        media_gen,
        generation,
        latency: Some(latency),
      },
      problems,
      critical,
    }
  }

  /// 运行中改一个热参数（见 `engine/hot.rs`）：写单元格，行工作线程**下一 hop**生效。
  /// 计划里没有这个 `(列,行,参数名)` → `None`（调用方据此报「这一行没有这个热参数」）。
  pub fn set_hot_param(&self, col: usize, row: usize, key: &str, value: f32) -> Option<()> {
    let cell = self.hot.get(&(col, row, key.to_string()))?;
    cell.set_db(value);
    Some(())
  }

  /// 运行中改一整条 EQ 曲线（不重建会话）：写 `HotEq` 单元格，列线程下一 hop 看到新版本。
  pub fn set_hot_eq(
    &self,
    col: usize,
    row: usize,
    settings: &crate::engine::hot::EqSettings,
  ) -> Option<()> {
    let cell = self.hot_eq.get(&(col, row))?;
    cell.set(settings);
    Some(())
  }

  pub fn stop(&mut self) {
    // 先停分析线程：它只读探针，停它不会影响列线程收尾
    if let Some(mut l) = self.latency.take() {
      l.stop();
    }
    self.publisher_stop.store(true, Relaxed);
    if let Some(j) = self.publisher.take() {
      let _ = j.join();
    }
    for c in &mut self.columns {
      c.stop.store(true, Relaxed);
      if let Some(j) = c.join.take() {
        let _ = j.join();
      }
      for h in c.captures.drain(..) {
        h.stop();
      }
    }
    self.columns.clear();
    // **不要在这里清媒体槽位 / 列概要**：先建后停时，旧会话的 stop 可能晚于新会话构建，
    // 清掉就会把新会话的音效板/播放器/状态一起清掉（2026-10-25 的确定性 bug）。
    // 只有用户显式停引擎（`AudioManager::stop`）才清。
    events::log(
      "info",
      "engine",
      format!("第 {} 代会话已停止", self.generation),
    );
  }
}

fn to_params(map: &BTreeMap<String, String>) -> Params {
  map
    .iter()
    .map(|(k, v)| (k.clone(), ParamValue::Text(v.clone())))
    .collect()
}

/// 从行输入环取 1 hop（超上限丢最旧到目标）；不足一 hop 返回 false。
/// 行输入环丢弃注记（空串 = 没丢过）。
///
/// **丢了不报 = 只剩一句「声音卡」**：丢弃发生在 10 ms 网格之外，累计数只涨不降，
/// 所以速率一起给（速率才看得出「现在还在不在丢」）。
fn ring_drop_note(dropped: u64, rate: &crate::debug::Probe<f64>) -> String {
  if dropped == 0 {
    return String::new();
  }
  match rate {
    crate::debug::Probe::Ok { value } => format!("，环丢 {dropped}（{value:.0}/s）"),
    _ => format!("，环丢 {dropped}"),
  }
}

/// 从行输入环取一个 hop。
///
/// 水位超过 [`ROW_CAP`] 就丢最旧、降到 [`ROW_TARGET`]（ARCHITECTURE.md §8）—— 顺带把丢了多少
/// 报出来：丢样本发生在 10 ms 网格之外，**不报出来就只是一句「声音卡」**。
/// 返回 `(是否取到 hop, 本轮丢弃的样本数)`。
fn take_hop(cons: &mut Consumer<f32>, out: &mut [f32; HOP]) -> (bool, usize) {
  let mut dropped = 0usize;
  if cons.slots() > ROW_CAP {
    let drop = cons.slots() - ROW_TARGET;
    if let Ok(c) = cons.read_chunk(drop) {
      c.commit_all();
      dropped = drop;
    }
  }
  if cons.slots() >= HOP
    && let Ok(c) = cons.read_chunk(HOP)
  {
    let (a, b) = c.as_slices();
    out[..a.len()].copy_from_slice(a);
    out[a.len()..].copy_from_slice(b);
    c.commit_all();
    return (true, dropped);
  }
  (false, dropped)
}

/// 输入行的设备租约（**构建期**，同步）：只试一次；失败交给运行期 `poll_input_lease` 退避重试。
fn apply_input_lease(r: &mut Row, hub: &SharedHub, idx: usize, ri: usize) {
  if !r.pooled_input {
    return;
  }
  let Some(dev) = r.device.clone() else {
    return;
  };
  if !r.enabled {
    r.in_lease = None;
    r.input.clear();
    return;
  }
  if r.in_lease.is_some() {
    return;
  }
  match crate::audio::device_pool::hub().acquire_input(hub.clone(), &dev) {
    Ok(lease) => {
      let phys = lease.channels();
      // 只订阅下游真正用到的通道（稀疏也行）。
      let chans: Vec<usize> = r
        .sub_channels
        .iter()
        .copied()
        .filter(|c| *c < phys)
        .collect();
      let chans = if chans.is_empty() { vec![0] } else { chans };
      r.src_channels = phys;
      r.input_channels = chans.clone();
      r.input = lease.subscribe_list(&format!("col{idx}row{ri}"), &chans);
      r.in_lease = Some(lease);
      r.error = None;
      r.in_failures = 0;
      r.device_pending = false;
      r.last_hop_at = Instant::now();
      events::log("info", "engine", format!("输入设备已打开：{dev}"));
    }
    Err(e) => {
      r.error = Some(e.clone());
      r.in_failures = 1;
      r.device_pending = true;
      r.in_retry_at = Some(Instant::now() + backoff(1));
      events::log("error", "engine", format!("输入设备打开失败：{dev}：{e}"));
    }
  }
}

/// 运行期输入租约（**永不阻塞 tick**）：启动后台打开线程 → 下一拍非阻塞收结果；
/// 失败按指数退避（1 s → 30 s），绝不每 10 ms 去枚举设备/重试打开。
fn poll_input_lease(r: &mut Row, hub: &SharedHub, idx: usize, ri: usize, now: Instant) {
  if !r.pooled_input {
    return;
  }
  let Some(dev) = r.device.clone() else {
    return;
  };
  if !r.enabled {
    if r.in_lease.is_some() {
      r.in_lease = None;
      r.input.clear();
      events::log(
        "info",
        "engine",
        format!("输入行已停用，放掉设备引用：{dev}"),
      );
    }
    r.in_pending = None;
    r.in_retry_at = None;
    // 停用期间不产出：重新启用时重新计时，免得一开就撞上「停太久」的自愈判定
    r.last_hop_at = now;
    return;
  }
  if r.in_lease.is_some() {
    return;
  }
  if let Some(rx) = r.in_pending.as_ref() {
    match rx.try_recv() {
      Ok(Ok(lease)) => {
        let phys = lease.channels();
        let chans: Vec<usize> = r
          .sub_channels
          .iter()
          .copied()
          .filter(|c| *c < phys)
          .collect();
        let chans = if chans.is_empty() { vec![0] } else { chans };
        r.src_channels = phys;
        r.input_channels = chans.clone();
        r.input = lease.subscribe_list(&format!("col{idx}row{ri}"), &chans);
        r.in_lease = Some(lease);
        r.in_pending = None;
        r.in_retry_at = None;
        r.in_failures = 0;
        r.error = None;
        r.device_pending = false;
        r.last_hop_at = Instant::now();
        events::log("info", "engine", format!("输入设备已恢复：{dev}"));
      }
      Ok(Err(e)) => {
        r.in_pending = None;
        r.in_failures = r.in_failures.saturating_add(1);
        r.device_pending = true;
        r.in_retry_at = Some(now + backoff(r.in_failures));
        r.error = Some(e.clone());
        events::log(
          "error",
          "engine",
          format!(
            "输入设备打开失败（第 {} 次，{} s 后重试）：{dev}：{e}",
            r.in_failures,
            backoff(r.in_failures).as_secs()
          ),
        );
      }
      Err(std::sync::mpsc::TryRecvError::Empty) => {}
      Err(std::sync::mpsc::TryRecvError::Disconnected) => {
        r.in_pending = None;
        r.in_failures = r.in_failures.saturating_add(1);
        r.device_pending = true;
        r.in_retry_at = Some(now + backoff(r.in_failures));
      }
    }
    return;
  }
  if let Some(at) = r.in_retry_at
    && now < at
  {
    return;
  }
  // 起一个一次性打开线程：设备打开可能几百毫秒到几秒，绝不能占住 10 ms tick。
  let (tx, rx) = std::sync::mpsc::channel();
  let h = hub.clone();
  let d = dev.clone();
  let name = format!("open-in-{idx}-{ri}");
  let spawned = std::thread::Builder::new().name(name).spawn(move || {
    let r = crate::audio::device_pool::hub().acquire_input(h, &d);
    let _ = tx.send(r);
  });
  match spawned {
    Ok(_) => r.in_pending = Some(rx),
    Err(e) => {
      r.in_failures = r.in_failures.saturating_add(1);
      r.device_pending = true;
      r.in_retry_at = Some(now + backoff(r.in_failures));
      r.error = Some(format!("起设备打开线程失败：{e}"));
    }
  }
}

/// 输出行的设备租约（**构建期**，同步）；与输入侧同一条纪律。
fn apply_output_lease(r: &mut Row, hub: &SharedHub, idx: usize, ri: usize) {
  if !r.pooled_output {
    return;
  }
  let Some(dev) = r.device.clone() else {
    return;
  };
  if !r.enabled {
    r.out_lease = None;
    r.output = None;
    return;
  }
  if r.out_lease.is_some() {
    return;
  }
  match crate::audio::device_pool::hub().acquire_output(hub.clone(), &dev, r.channels as usize) {
    Ok(lease) => {
      r.output = Some(lease.out.clone());
      r.out_lease = Some(lease);
      r.error = None;
      r.out_failures = 0;
      r.device_pending = false;
      events::log("info", "engine", format!("输出设备已打开：{dev}"));
    }
    Err(e) => {
      r.error = Some(e.clone());
      r.out_failures = 1;
      r.device_pending = true;
      r.out_retry_at = Some(Instant::now() + backoff(1));
      events::log("error", "engine", format!("输出设备打开失败：{dev}：{e}"));
    }
  }
  let _ = (idx, ri);
}

/// 运行期输出租约：同输入侧，后台打开 + 指数退避。
fn poll_output_lease(r: &mut Row, hub: &SharedHub, idx: usize, ri: usize, now: Instant) {
  if !r.pooled_output {
    return;
  }
  let Some(dev) = r.device.clone() else {
    return;
  };
  if !r.enabled {
    if r.out_lease.is_some() {
      r.out_lease = None;
      r.output = None;
      events::log(
        "info",
        "engine",
        format!("输出行已停用，放掉设备引用：{dev}"),
      );
    }
    r.out_pending = None;
    r.out_retry_at = None;
    return;
  }
  if r.out_lease.is_some() {
    return;
  }
  if let Some(rx) = r.out_pending.as_ref() {
    match rx.try_recv() {
      Ok(Ok(lease)) => {
        r.output = Some(lease.out.clone());
        r.out_lease = Some(lease);
        r.out_pending = None;
        r.out_retry_at = None;
        r.out_failures = 0;
        r.error = None;
        r.device_pending = false;
        events::log("info", "engine", format!("输出设备已恢复：{dev}"));
      }
      Ok(Err(e)) => {
        r.out_pending = None;
        r.out_failures = r.out_failures.saturating_add(1);
        r.device_pending = true;
        r.out_retry_at = Some(now + backoff(r.out_failures));
        r.error = Some(e.clone());
        events::log(
          "error",
          "engine",
          format!(
            "输出设备打开失败（第 {} 次，{} s 后重试）：{dev}：{e}",
            r.out_failures,
            backoff(r.out_failures).as_secs()
          ),
        );
      }
      Err(std::sync::mpsc::TryRecvError::Empty) => {}
      Err(std::sync::mpsc::TryRecvError::Disconnected) => {
        r.out_pending = None;
        r.out_failures = r.out_failures.saturating_add(1);
        r.device_pending = true;
        r.out_retry_at = Some(now + backoff(r.out_failures));
      }
    }
    return;
  }
  if let Some(at) = r.out_retry_at
    && now < at
  {
    return;
  }
  let (tx, rx) = std::sync::mpsc::channel();
  let h = hub.clone();
  let d = dev.clone();
  let ch = r.channels as usize;
  let name = format!("open-out-{idx}-{ri}");
  let spawned = std::thread::Builder::new().name(name).spawn(move || {
    let r = crate::audio::device_pool::hub().acquire_output(h, &d, ch);
    let _ = tx.send(r);
  });
  match spawned {
    Ok(_) => r.out_pending = Some(rx),
    Err(e) => {
      r.out_failures = r.out_failures.saturating_add(1);
      r.device_pending = true;
      r.out_retry_at = Some(now + backoff(r.out_failures));
      r.error = Some(format!("起设备打开线程失败：{e}"));
    }
  }
}

/// 指数退避：1、2、4、8、16、30、30… 秒。
fn backoff(failures: u32) -> Duration {
  let exp = failures.saturating_sub(1).min(5);
  let secs = (1u64 << exp).min(REOPEN_BACKOFF_MAX.as_secs());
  Duration::from_secs(secs.max(REOPEN_BACKOFF_MIN.as_secs()))
}

/// 设备名兜底：枚举里暂时查不到这只设备时，用**上次选择时记下的显示名**（`RowSpec.device_name`），
/// 连它也没有才退回设备 ID。名字只影响显示，不参与匹配（ARCHITECTURE.md §6.2）。
fn saved_device_name(r: &crate::plan::RowSpec, dev: &str) -> String {
  r.device_name
    .as_deref()
    .map(str::trim)
    .filter(|s| !s.is_empty())
    .map(str::to_string)
    .unwrap_or_else(|| dev.to_string())
}

/// 这一行的失败要不要**回滚整份计划**（ARCHITECTURE.md §6.3）。
///
/// 只有「启用了、也配好了、但又不会自己好」的失败才算：模型加载失败、AEC 缺参考等。
/// 设备类失败（`device_pending`）由运行期退避重开自己接上，回滚只会让用户失去修复能力
/// （issue #10：两列同时「设备不在」，改一列就被打回原样）。
fn blocks_plan(enabled: bool, unconfigured: bool, device_pending: bool) -> bool {
  enabled && !unconfigured && !device_pending
}

#[allow(clippy::too_many_arguments)] // 构建期一次性传参；拆结构只会把同一件事分散到多处
fn build_column(
  hub: &SharedHub,
  idx: usize,
  spec: &ColumnSpec,
  problems: &mut Vec<String>,
  critical: &mut Vec<String>,
  rec: &Arc<RecorderHub>,
  calib: &Arc<CalibHub>,
  hot: &mut std::collections::HashMap<(usize, usize, String), Arc<crate::engine::hot::HotGain>>,
  hot_eq: &mut std::collections::HashMap<(usize, usize), Arc<crate::engine::hot::HotEq>>,
  media_gen: u64,
  generation: u64,
  pin_core: bool,
) -> ColumnRuntime {
  let mut rows: Vec<Row> = Vec::new();
  let mut captures = Vec::new();
  // 下游真正用到的端口（源节点只订阅这些通道；10 通道设备接 2 路时不再为 8 路做重采样）
  let mut needed: std::collections::HashMap<u32, std::collections::BTreeSet<u8>> =
    std::collections::HashMap::new();
  for r in &spec.rows {
    for e in &r.inputs {
      needed.entry(e.from).or_default().insert(e.from_port);
    }
  }
  // `remote_mic` 入站是单队列，全会话只允许一行（SPEC 约束）
  let mut remote_mic_used = false;

  // 列头开关：整列停用 = 这一列不产出、也不占设备引用（行级开关照旧各自生效）
  let column_on = spec.enabled;
  for (ri, r) in spec.rows.iter().enumerate() {
    // 热参数登记口：本行建出来的单元格（增益 / 音量）收进会话，供运行中改值
    let mut sink = crate::engine::hot::HotSink::new();
    let mut row = Row {
      id: r.id,
      inputs: Vec::new(),
      channels: r.channels.max(1),
      kind: r.kind,
      ptype: r.ptype.clone(),
      label: r.ptype.clone(),
      enabled: r.enabled && column_on,
      enabled_prev: r.enabled,
      error: None,
      unconfigured: false,
      source: false,
      inf_ms: 0.0,
      inf_max_ms: 0.0,
      inf_over: 0,
      inf_over_rate: RateMeter::new(),
      input: Vec::new(),
      input_channels: Vec::new(),
      src_channels: 1,
      sub_channels: Vec::new(),
      device: None,
      pooled_input: false,
      pooled_output: false,
      in_lease: None,
      out_lease: None,
      in_pending: None,
      in_retry_at: None,
      in_failures: 0,
      device_pending: false,
      last_hop_at: Instant::now(),
      out_pending: None,
      out_retry_at: None,
      out_failures: 0,
      aec: None,
      aec_stage: None,
      input_gain: None,
      net_input: None,
      ring_dropped: 0,
      ring_drop_rate: RateMeter::new(),
      stage: Vec::new(),
      media_slot: None,
      output: None,
      net_output: false,
      peak: 0.0,
      latency: 0,
      edge_delays: Vec::new(),
      delay_lines: Vec::new(),
    };
    if r.kind == RowKind::Input {
      row.sub_channels = if r.channels <= 1 {
        // 单声道节点：立体声设备取 L/R 平均；多声道设备也只取前两路（不吞 10 路把 CPU 拉满）
        vec![0, 1]
      } else {
        let mut v: Vec<usize> = needed
          .get(&r.id)
          .map(|s| s.iter().map(|p| *p as usize).collect())
          .unwrap_or_default();
        if v.is_empty() {
          v.push(0);
        }
        v
      };
      row.input_channels = row.sub_channels.clone();
    }
    let device = r.device.as_deref().filter(|d| !d.is_empty());
    let trim: f64 = r.input_gain_db.trim().parse().unwrap_or(0.0);
    row.input_gain = Some(sink.gain("input_gain_db", trim));
    match r.kind {
      RowKind::Input if r.ptype == "echo_cancel" => {
        // AEC 输入行：本行 device = mic，params.far_device = 远端参考（回环或另一路输入）。
        let far = r.params.get("far_device").cloned().unwrap_or_default();
        let delay_ms: f64 = r
          .params
          .get("far_delay_ms")
          .and_then(|v| v.trim().parse().ok())
          .unwrap_or(0.0);
        let model = crate::infer::known_model(
          crate::infer::aec::aec_models(),
          r.params
            .get("model")
            .map(String::as_str)
            .unwrap_or(crate::infer::aec::MODEL_AEC),
        )
        .to_string();
        let db = |k: &str| {
          r.params
            .get(k)
            .and_then(|v| v.trim().parse::<f64>().ok())
            .unwrap_or(0.0)
        };
        let mic_gain = 10f32.powf((db("mic_gain_db") / 20.0) as f32);
        let far_gain = 10f32.powf((db("far_gain_db") / 20.0) as f32);
        let bypass = r
          .params
          .get("bypass")
          .map(|v| {
            let v = v.trim();
            v == "1" || v.eq_ignore_ascii_case("true")
          })
          .unwrap_or(false);
        match device {
          None => row.unconfigured = true,
          Some(mic_dev) => {
            row.source = true;
            // 远端：扬声器（输出设备）→ 自动 WASAPI 回环；也可填输入设备当参考
            let far_loop = crate::engine::aec::far_loopback_id(&far);
            let mic = crate::devices::display_name(mic_dev, true);
            let far_ok: Result<String, String> = if let Some(id) = &far_loop {
              if id.is_empty() {
                Ok("默认输出回环".to_string())
              } else {
                Ok(
                  crate::devices::find(id, false)
                    .map(|(_, n)| n)
                    .unwrap_or_else(|_| "输出回环".to_string()),
                )
              }
            } else if far.is_empty() {
              Err("未选择远端设备".to_string())
            } else {
              crate::devices::display_name(&far, true)
            };
            match (mic, far_ok) {
              (Ok(mic_name), Ok(far_name)) => {
                match capture::spawn(hub.clone(), mic_dev.to_string(), format!("c{idx}r{ri}m")) {
                  Ok(mic_cap) => {
                    let far_cap = match &far_loop {
                      Some(id) => loopback::spawn(
                        hub.clone(),
                        if id.is_empty() {
                          None
                        } else {
                          Some(id.clone())
                        },
                        format!("c{idx}r{ri}f"),
                      ),
                      None => capture::spawn(hub.clone(), far.clone(), format!("c{idx}r{ri}f")),
                    };
                    match far_cap {
                      Ok(far_cap) => {
                        let hist = Arc::new(Mutex::new(FarHistory::new(FAR_HIST_SAMPLES)));
                        let pump = spawn_far_pump(
                          far_cap.fanout.channel(0).expect("远端至少 1 通道"),
                          hist.clone(),
                          &format!("c{idx}r{ri}"),
                          calib.epoch(),
                        );
                        match Aec::load(&model) {
                          Ok(engine) => {
                            row.label = format!("AEC：{mic_name}（远端 {far_name}）");
                            row.aec = Some(AecRow {
                              engine,
                              hist,
                              // 延时取整到 10 ms（= 1 hop 的整数倍），far 窗口与 hop 网格对齐
                              delay_samples: (delay_ms / 10.0).round() as i64 * HOP as i64,
                              mic_hops: 0,
                              mic: mic_cap
                                .fanout
                                .channel(0)
                                .expect("麦克风至少 1 通道")
                                .subscribe(&format!("c{idx}r{ri}-mic")),
                              mic_gain,
                              far_gain,
                              bypass,
                              exact: 0,
                              latest: 0,
                              pass: 0,
                              echo: crate::engine::aec::EchoMetrics::default(),
                            });
                          }
                          Err(e) => row.error = Some(e),
                        }
                        captures.push(mic_cap.handle);
                        captures.push(far_cap.handle);
                        captures.push(pump);
                      }
                      Err(e) => {
                        captures.push(mic_cap.handle);
                        row.error = Some(e);
                      }
                    }
                  }
                  Err(e) => row.error = Some(e),
                }
              }
              (Err(e), _) | (_, Err(e)) => row.error = Some(e),
            }
          }
        }
      }
      RowKind::Input if r.ptype == "tone" => {
        // 测试音源（全局共享线程），无需设备
        row.label = "测试音（1kHz 测试用）".into();
        row.source = true;
        row.input = vec![crate::audio::tone::shared().subscribe(&format!("col{idx}row{ri}"))];
        row.input_channels = vec![0];
        row.src_channels = 1;
      }
      RowKind::Input if r.ptype == "remote_mic" => {
        // 网络麦克风：不需要虚拟声卡驱动，网络流直接进引擎（NETWORK.md §4.1）。
        // 允许一个：入站是单队列，多行会互相抢样本。
        // **不做 AEC**：要消回声就在图里再挂一个 `echo_cancel` 节点，近端接这里、
        // 远端接 `loopback` / 另一路输入（SESSION-V2.md §3.3）。
        if remote_mic_used {
          row.error = Some("网络麦克风只允许一个输入行".into());
        } else {
          remote_mic_used = true;
          row.source = true;
          row.label = "网络".into();
          row.net_input = Some(crate::net::hub().clone());
        }
      }
      RowKind::Input if r.ptype == "loopback" => {
        // 电脑播放音频：捕获输出设备正在播放的系统混音（复用 AEC 的回环采集）。
        // `device` = `loopback`（默认输出）/ `loopback:<端点ID｜sink>`；行内增益默认 -6 dB。
        let target = match device {
          None => None,
          Some(d) if d == "loopback" || d.is_empty() => None,
          Some(d) => Some(d.strip_prefix("loopback:").unwrap_or(d).to_string()),
        };
        let name = match &target {
          None => "系统默认输出".to_string(),
          Some(id) => crate::devices::find(id, false)
            .map(|(_, n)| n)
            .unwrap_or_else(|_| id.clone()),
        };
        row.label = format!("扬声器（{name}）");
        row.source = true;
        match loopback::spawn(hub.clone(), target, format!("c{idx}r{ri}")) {
          Ok(cap) => {
            let phys = cap.fanout.len();
            let chans: Vec<usize> = row
              .sub_channels
              .iter()
              .copied()
              .filter(|c| *c < phys)
              .collect();
            let chans = if chans.is_empty() { vec![0] } else { chans };
            row.src_channels = phys;
            row.input_channels = chans.clone();
            row.input = cap
              .fanout
              .subscribe_list(&format!("col{idx}row{ri}"), &chans);
            captures.push(cap.handle);
          }
          Err(e) => row.error = Some(e),
        }
      }
      RowKind::Input => match device {
        None => row.unconfigured = true,
        Some(dev) => {
          row.source = true;
          // 设备由**池**持有（同一设备多列共用一条采集流），行只拿租约（ARCHITECTURE.md §6.5）
          row.device = Some(dev.to_string());
          row.pooled_input = true;
          // 名字**只用于显示**：枚举里暂时没有这只设备（USB 还没枚举完 / 端点被重建）也照样
          // 建行 —— 拿不到名字就退回上次选择时记下的名字，运行期会按退避一直重开（§6.3）。
          row.label = match crate::devices::display_name(dev, true) {
            Ok(name) => name,
            Err(_) => saved_device_name(r, dev),
          };
          crate::engine::session::apply_input_lease(&mut row, hub, idx, ri);
        }
      },
      RowKind::Process if r.ptype == "echo_cancel" => {
        row.label = "AEC（近端 + 远端）".into();
        match AecStage::new(&r.params) {
          Ok(st) => row.aec_stage = Some(st),
          Err(e) => row.error = Some(e),
        }
      }
      RowKind::Process => {
        // 媒体行（音效板 / 播放器）：注册 (列,行) 槽位，供界面投递运行时命令
        let slot = match r.ptype.as_str() {
          "soundpad" => Some(media::hub().register(media_gen, idx, ri, "soundpad")),
          "player" => Some(media::hub().register(media_gen, idx, ri, "player")),
          _ => None,
        };
        row.media_slot = slot.clone();
        // 逐通道一个 Stage：模型 / 滤波器状态必须按通道独立（一个立体声降噪不能
        // 两个通道共用一份 LSTM 状态）。热参数单元格反而要**共享**，第 1 个通道
        // 建完后把单元格交回 sink，其余通道复用（见 `HotSink::seed`）。
        let params = to_params(&r.params);
        let n = row.channels.max(1) as usize;
        let mut stages: Vec<Box<dyn Stage>> = Vec::with_capacity(n);
        let mut first_err = None;
        for c in 0..n {
          match registry::create_stage(
            &r.ptype,
            &params,
            slot.clone(),
            &mut sink,
            (idx, ri, generation),
          ) {
            Ok(st) => {
              stages.push(st);
              if c == 0 {
                let gains = sink.take();
                let eq = sink.take_eq();
                for (k, cell) in gains {
                  sink.seed(k, cell);
                }
                if let Some(cell) = eq {
                  sink.seed_eq(cell);
                }
              }
            }
            Err(e) => {
              first_err = Some(e);
              break;
            }
          }
        }
        match first_err {
          Some(e) => row.error = Some(e),
          None => {
            row.label = registry::get_spec(&r.ptype)
              .map(|s| s.label.to_string())
              .unwrap_or(r.ptype.clone());
            row.stage = stages;
          }
        }
      }
      RowKind::Output if r.ptype == "remote_speaker" => {
        // 网络扬声器：按位置 tap 取信号，交给网络中枢广播给订阅客户端
        row.label = "网络扬声器".into();
        row.net_output = true;
      }
      RowKind::Output => {
        match device {
          None => row.unconfigured = true,
          Some(dev) => {
            // 输出也由池持有：一个设备**一条**混音，本行只是往里登记一路源（ARCHITECTURE.md §6.5）
            row.device = Some(dev.to_string());
            row.pooled_output = true;
            row.label = match crate::devices::display_name(dev, false) {
              Ok(name) => name,
              Err(_) => saved_device_name(r, dev),
            };
            crate::engine::session::apply_output_lease(&mut row, hub, idx, ri);
          }
        }
      }
    }
    if let Some(e) = &row.error {
      problems.push(format!("第 {} 列第 {} 行：{e}", idx + 1, ri + 1));
      // 关键失败：这一行是启用状态、已经配置了设备/模型，却打不开 —— 新会话不能提交。
      // 未配置（还没选设备）与停用行不算（那是用户的正常中间态）。
      //
      // **模型类处理行不再例外**（2026-10-30 起，denoise / TSE 都一样）：
      // 旧实现是 `let model_optional = matches!(row.ptype.as_str(), "denoise" | "tse");`
      // 且 `if row.enabled && !row.unconfigured && !model_optional` —— 模型加载失败时
      // 链路直通、只在调试面板留一行状态。2026-10-30 实锤：TSE 主图与注册编码器不同源，
      // 用户录了参考、界面也显示「已生效」，实际全程直通，完全无感。
      // 现在与其它行一致：启用了且已配置却起不来 = 关键失败，会话回滚并明确报错
      // （UX.md「失败不静默」）。代价是模型坏了会挡住切设备 —— 这正是要的：先修模型，
      // 不静默降级。
      //
      // **设备类失败是例外（2026-11-03，issue #10）**：设备不在 / 被独占 / USB 还没枚举完
      // 是**可自愈**的资源问题，运行期本来就会按 1→30 s 退避一直重开（`poll_*_lease`）。
      // 把它当关键失败会让整份计划回滚，结果是「两列都指向一只暂时不在的设备」时用户
      // 改一列就被打回原样，连删列、关开关都做不到。这里只把原因报给界面（`problems`），
      // 会话照常提交，坏的那一行自己等设备回来。模型失败不会自己好，仍走 critical。
      if blocks_plan(row.enabled, row.unconfigured, row.device_pending) {
        critical.push(format!(
          "第 {} 列第 {} 行「{}」：{e}",
          idx + 1,
          ri + 1,
          row.label
        ));
      }
    }
    // 热参数单元格收进会话（(列,行,参数名) 寻址；见 `engine/hot.rs`）
    for (key, cell) in sink.take() {
      hot.insert((idx, ri, key), cell);
    }
    if let Some(cell) = sink.take_eq() {
      hot_eq.insert((idx, ri), cell);
    }
    rows.push(row);
  }

  // 连线：把节点 ID 解析成列内行下标（计划已按拓扑顺序排好）。
  {
    let index: std::collections::HashMap<u32, usize> =
      rows.iter().enumerate().map(|(i, r)| (r.id, i)).collect();
    let edges_by_row: Vec<Vec<(u32, u8, u8)>> = spec
      .rows
      .iter()
      .map(|r| {
        r.inputs
          .iter()
          .map(|e| (e.from, e.from_port, e.to_port))
          .collect()
      })
      .collect();
    for (row, edges) in rows.iter_mut().zip(edges_by_row) {
      row.inputs = edges
        .into_iter()
        .filter_map(|(id, from_port, to_port)| {
          index.get(&id).copied().map(|src| InputRef {
            src,
            from_port,
            to_port,
          })
        })
        .collect();
    }
  }

  // 延迟补偿：每条入线要补多少样本（Element GraphBuilder 同一条规则）
  compute_pdc(&mut rows);
  // 活跃区间缓冲复用（Element GraphBuilder 同一条规则）：槽在最后一个读者之后回收。
  let (slot_of, _slot_count) = assign_slots(&rows);
  let slot_of = Arc::new(slot_of);

  let rows = Arc::new(Mutex::new(rows));
  let stop = Arc::new(AtomicBool::new(false));
  let alive = Arc::new(AtomicBool::new(true));
  let (rows2, stop2, alive2) = (rows.clone(), stop.clone(), alive.clone());
  let rec2 = rec.clone();
  let calib2 = calib.clone();
  let hub3 = hub.clone();
  let latency = Arc::new(LatencyTap::new(idx));
  let latency3 = latency.clone();
  // 一个模型一路核（见 `engine/affinity.rs`）：列线程 = 这一列推理的宿主线程，
  // 从 1 号核开始轮转分配，避免多列的模型被调度到同一个核上互相打断
  // （2026-10-26：后台 CPU 忙时两列抢核 → 采集环涨到硬顶 → 丢样本）。
  //
  // **只有多列时才绑核**：绑核的目的是「别让两列的推理挤同一个核」，只有一列时
  // 它只会在一种情况下帮倒忙 —— 被钉到一个正忙的核 / 小核上（实测列线程节拍迟到
  // 最高 94.7 ms、源侧发包间隔出现 0.001 ms 连发 + 89 ms 空档）。单列交给系统调度。
  let core = super::affinity::next_core();
  let join = match std::thread::Builder::new()
    .name(format!("column-{}@cpu{}", idx + 1, core))
    .spawn(move || {
      let _guard = AliveGuard(alive2);
      if pin_core && let Err(e) = super::affinity::pin_current(core) {
        events::log(
          "warn",
          "engine",
          format!("第 {} 列绑定 CPU {core} 失败（回到系统调度）：{e}", idx + 1),
        );
      }
      let token = threads::ThreadToken::new(format!("column-{}@cpu{}", idx + 1, core));
      let r = catch_unwind(AssertUnwindSafe(|| {
        run_column(
          idx, column_on, rows2, slot_of, stop2, rec2, calib2, hub3, token, latency3,
        );
      }));
      if r.is_err() {
        events::log(
          "error",
          "engine",
          format!("第 {} 列的列工作线程 panic（详见事件日志）", idx + 1),
        );
      }
    }) {
    Ok(j) => Some(j),
    Err(e) => {
      alive.store(false, Relaxed);
      events::log(
        "error",
        "engine",
        format!("第 {} 列线程起不来：{e}", idx + 1),
      );
      None
    }
  };

  ColumnRuntime {
    stop,
    join,
    captures,
    alive,
    latency,
  }
}

/// 活跃区间缓冲分配：返回每行的槽号，以及槽总数。
fn assign_slots(rows: &[Row]) -> (Vec<Vec<usize>>, usize) {
  // 一个「单元」= (行, 通道)。端口 = 通道，所以缓冲复用也必须按通道算：
  // 某个通道的最后一个读者过去之后，那条缓冲马上可以给后来的单元用。
  let mut base = Vec::with_capacity(rows.len());
  let mut units = 0usize;
  for r in rows {
    base.push(units);
    units += r.channels.max(1) as usize;
  }
  let mut last_use = vec![0usize; units];
  for (i, r) in rows.iter().enumerate() {
    for e in &r.inputs {
      let Some(&b) = base.get(e.src) else {
        continue;
      };
      let sc = rows[e.src].channels.max(1) as usize;
      let ch = (e.from_port as usize).min(sc.saturating_sub(1));
      last_use[b + ch] = i;
    }
  }
  let mut slot_of: Vec<Vec<usize>> = Vec::with_capacity(rows.len());
  let mut slot_last: Vec<usize> = Vec::new();
  for (i, r) in rows.iter().enumerate() {
    let n = r.channels.max(1) as usize;
    let mut row_slots = Vec::with_capacity(n);
    for c in 0..n {
      let u = base[i] + c;
      // 严格 `< i`：同一行的不同通道不能共用同一条缓冲（它们在同一拍同时写）。
      let slot = (0..slot_last.len()).find(|&s| slot_last[s] < i);
      let s = slot.unwrap_or_else(|| {
        slot_last.push(0);
        slot_last.len() - 1
      });
      slot_last[s] = last_use[u].max(i);
      row_slots.push(s);
    }
    slot_of.push(row_slots);
  }
  (slot_of, slot_last.len().max(1))
}

/// 延迟补偿（PDC），与 Element `GraphBuilder` 同一条规则：
/// 一个节点的输入延迟 = max(各入线来源的累计算延迟)；较快的入线补
/// `max - 该来源延迟` 个样本；本节点输出延迟 = max + 本 Stage 的算法延迟。
///
/// 例：降噪（1 hop）与直通两路汇合 → 直通那一路补 480 样本，两路才同相。
/// 单路链不受影响（max = 该路延迟，补 0）。
fn compute_pdc(rows: &mut [Row]) {
  let mut lat = vec![0usize; rows.len()];
  for i in 0..rows.len() {
    let max_in = rows[i]
      .inputs
      .iter()
      .map(|e| lat.get(e.src).copied().unwrap_or(0))
      .max()
      .unwrap_or(0);
    let delays: Vec<usize> = rows[i]
      .inputs
      .iter()
      .map(|e| max_in.saturating_sub(lat.get(e.src).copied().unwrap_or(0)))
      .collect();
    let own = rows[i].stage_latency();
    rows[i].edge_delays = delays;
    rows[i].latency = max_in + own;
    lat[i] = rows[i].latency;
  }
  for r in rows.iter_mut() {
    r.delay_lines = r
      .edge_delays
      .iter()
      .map(|&d| if d > 0 { Some(DelayLine::new(d)) } else { None })
      .collect();
  }
}

/// 节点输入微调（dB → 线性；热参数每 hop 读一次）。
fn apply_trim(r: &Row, buf: &mut [f32; HOP]) {
  if let Some(g) = r.input_gain.as_ref() {
    let g = g.linear();
    if g != 1.0 {
      for x in buf.iter_mut() {
        *x *= g;
      }
    }
  }
}

struct AliveGuard(Arc<AtomicBool>);
impl Drop for AliveGuard {
  fn drop(&mut self) {
    self.0.store(false, Relaxed);
  }
}

#[allow(clippy::too_many_arguments)] // 线程入口：参数就是这一列的完整上下文
#[allow(clippy::needless_range_loop)]
fn run_column(
  idx: usize,
  column_on: bool,
  rows: Arc<Mutex<Vec<Row>>>,
  slot_of: Arc<Vec<Vec<usize>>>,
  stop: Arc<AtomicBool>,
  rec: Arc<RecorderHub>,
  calib: Arc<CalibHub>,
  hub: SharedHub,
  token: threads::ThreadToken,
  latency: Arc<LatencyTap>,
) {
  // 实时音频线程：向系统声明用途（Windows = MMCSS "Pro Audio"），
  // 否则播放/推流时会被当普通后台线程调度，出现几十毫秒的整体停顿（`debug/stall.rs` 量得到）。
  let _rt = crate::audio::rt_guard::enter_audio_thread();
  let ctx = FrameContext::default();
  let mut tmp = [0.0f32; HOP];
  let mut last_publish = Instant::now() - PUBLISH;
  // 10 ms 节拍：**必须用高精度等待**（`audio/tick.rs`）。
  //
  // 以前是 `sleep(next - now)` + 「落后一点就立刻跑下一轮追平」：Windows 默认时钟刻度
  // 15.6 ms，睡过头之后那一轮会**连发两拍**（实测发包间隔出现 0.00 ms，同时空档 86 ms）。
  // 连发是「超前」，接收端多深的缓冲都吃不掉 —— 听感就是 1 kHz「不像一条直线」。
  // `Tick` 只放行一拍、落后不补发，超过两个周期就从此刻重新对齐。
  let mut tick = crate::audio::tick::Tick::new(TICK);
  let mut ticks: u64 = 0;
  let mut last_ticks: u64 = 0;
  // Session V2：每个节点一条输出缓冲；入线在**本拍**按拓扑顺序已经写好。
  let mut outbufs: Vec<[f32; HOP]> =
    vec![[0.0f32; HOP]; slot_of.iter().flatten().max().copied().unwrap_or(0) + 1];
  // 逐行逐通道的当前拍累加缓冲（预分配；通道数固定，运行期不增长）
  let mut accs: Vec<Vec<[f32; HOP]>> = {
    let g = rows.lock().unwrap();
    g.iter()
      .map(|r| vec![[0.0f32; HOP]; r.channels.max(1) as usize])
      .collect()
  };
  let mut src_hops: Vec<[f32; HOP]> = Vec::new();
  // 节拍质量：迟到（上一拍处理超时/被挂起）与单拍处理耗时。这两项是「为什么卡」的第一现场。
  let mut tick_overruns: u64 = 0;
  let mut tick_overrun_rate = RateMeter::new();
  let mut last_late_ms: f32 = 0.0;
  let mut max_late_ms: f32 = 0.0;
  let mut max_work_ms: f32 = 0.0;
  // 重置代号：会话起步 / 每次校准都让 far 网格与 mic 序号从同一原点计时
  let calib_epoch = calib.epoch();
  let mut epoch_seen = calib_epoch.load(Relaxed);
  let mut aec_boot = true;

  while !stop.load(Relaxed) {
    ticks += 1;
    let tick_start = Instant::now();
    // 这一拍「该在的时刻」由节拍器给（诊断只用，不参与控制）
    let next = tick.due();
    let late = tick_start.saturating_duration_since(next);
    if late > TICK_LATE_WARN {
      tick_overruns += 1;
      last_late_ms = late.as_secs_f32() * 1000.0;
      max_late_ms = max_late_ms.max(last_late_ms);
    }
    token.beat();
    let mut column_peak = 0.0f32;
    for b in &mut outbufs {
      b.fill(0.0);
    }
    // 延迟探针：本 tick 的第一路有数据的输入 hop / 第一路输出行取到的 hop（原样拷贝）
    let mut probe_in = [0.0f32; HOP];
    let mut probe_in_ok = false;
    let mut probe_out = [0.0f32; HOP];
    let mut probe_out_ok = false;
    // 发布周期要写进延迟预算的「第一路输入 / 第一路输出」
    let mut lat_in: Option<RowTapInfo> = None;
    let mut lat_out: Option<RowTapInfo> = None;
    {
      let mut g = rows.lock().unwrap();
      let recording = rec.active();
      let calibrating = calib.active();

      for (ri, r) in g.iter_mut().enumerate() {
        let nch = r.channels.max(1) as usize;
        let row_slots = &slot_of[ri];
        for c in 0..nch {
          accs[ri][c].fill(0.0);
        }
        // 入线求和（输入行没有入线；AEC 自己按近端/远端收，见下面分支）。
        // 端口 = 通道：from_port 选来源通道，to_port 落到目标通道；每条线按 PDC 补延迟。
        if r.kind != RowKind::Input && !(r.kind == RowKind::Process && r.ptype == "echo_cancel") {
          for ei in 0..r.inputs.len() {
            let e = r.inputs[ei];
            let Some(ssrc) = slot_of.get(e.src) else {
              continue;
            };
            if ssrc.is_empty() {
              continue;
            }
            let fp = (e.from_port as usize).min(ssrc.len() - 1);
            let Some(src) = outbufs.get(ssrc[fp]) else {
              continue;
            };
            let delayed = match r.delay_lines.get_mut(ei) {
              Some(Some(line)) => Some(line.process(src)),
              _ => None,
            };
            let s = delayed.as_ref().unwrap_or(src);
            let dst = &mut accs[ri][(e.to_port as usize).min(nch - 1)];
            for i in 0..HOP {
              dst[i] += s[i];
            }
          }
        }
        let mut inf_sample: Option<f32> = None;
        // 行启用开关翻转 → 转发给媒体行（音效板淡出在播声部；播放器硬旁路）
        if r.enabled != r.enabled_prev {
          r.enabled_prev = r.enabled;
          if let Some(slot) = r.media_slot.as_ref() {
            slot.send(MediaCmd::Enabled(r.enabled));
          }
        }
        match r.kind {
          RowKind::Input => {
            poll_input_lease(r, &hub, idx, ri, tick_start);
            if r.enabled
              && r.in_lease.is_some()
              && tick_start.saturating_duration_since(r.last_hop_at) >= INPUT_STALL_REOPEN
            {
              let secs = tick_start
                .saturating_duration_since(r.last_hop_at)
                .as_secs();
              events::log(
                "warn",
                "engine",
                format!(
                  "第 {} 组第 {} 节点：输入流已 {} s 没有数据，重新打开设备{}",
                  idx + 1,
                  ri + 1,
                  secs,
                  r.device
                    .as_deref()
                    .map(|d| format!("（{d}）"))
                    .unwrap_or_default()
                ),
              );
              r.in_lease = None;
              r.input.clear();
              r.in_retry_at = Some(tick_start);
              r.last_hop_at = tick_start;
            }
            if !r.enabled {
              // 停用 = 不产出（静音）
            } else if let Some(aec) = r.aec.as_mut() {
              // 旧版 AEC 输入行（迁移前的兼容路径；V2 起 AEC 是两输入 Process 节点）
              let mut near = [0.0f32; HOP];
              let (got, dropped) = take_hop(&mut aec.mic, &mut tmp);
              r.ring_dropped += dropped as u64;
              if got {
                if !probe_in_ok {
                  probe_in.copy_from_slice(&tmp);
                  probe_in_ok = true;
                }
                if aec_boot {
                  aec_boot = false;
                  calib_epoch.fetch_add(1, Relaxed);
                }
                let e = calib_epoch.load(Relaxed);
                if e != epoch_seen {
                  epoch_seen = e;
                  aec.mic_hops = 0;
                  aec.engine.reset();
                }
                let hop_idx = aec.mic_hops;
                aec.mic_hops += 1;
                if aec.bypass {
                  if aec.mic_gain != 1.0 {
                    for x in tmp.iter_mut() {
                      *x *= aec.mic_gain;
                    }
                  }
                  near.copy_from_slice(&tmp);
                } else {
                  let delay_samples = if calibrating {
                    crate::engine::calib::CALIB_REF_SAMPLES
                  } else {
                    aec.delay_samples
                  };
                  let start = hop_idx as i64 * HOP as i64 - delay_samples;
                  let mut win_buf = [0.0f32; HOP];
                  let win = {
                    let h = aec.hist.lock().unwrap();
                    h.window_or_latest(start, &mut win_buf)
                  };
                  if calibrating {
                    calib.feed(idx, ri, &tmp, &win_buf, win == FarWin::Exact);
                  }
                  if aec.mic_gain != 1.0 {
                    for x in tmp.iter_mut() {
                      *x *= aec.mic_gain;
                    }
                  }
                  if aec.far_gain != 1.0 {
                    for x in win_buf.iter_mut() {
                      *x *= aec.far_gain;
                    }
                  }
                  match win {
                    FarWin::None => {
                      aec.pass += 1;
                      near.copy_from_slice(&tmp);
                    }
                    w => {
                      if w == FarWin::Latest {
                        aec.latest += 1;
                      } else {
                        aec.exact += 1;
                      }
                      let t = Instant::now();
                      match aec.engine.process(&tmp, &win_buf) {
                        Ok(out) => {
                          let elapsed = t.elapsed().as_secs_f32() * 1000.0;
                          inf_sample = Some(elapsed);
                          aec.echo.push(&tmp, &win_buf, out);
                          near.copy_from_slice(out);
                        }
                        Err(e) => r.error = Some(e),
                      }
                    }
                  }
                }
              }
              accs[ri][0] = near;
            } else if let Some(net) = r.net_input.as_ref() {
              net.take_rx_hop(&mut tmp);
              if !probe_in_ok {
                probe_in.copy_from_slice(&tmp);
                probe_in_ok = true;
              }
              accs[ri][0] = tmp;
            } else if !r.input.is_empty() {
              // 设备通道 → 节点通道（端口 = 通道）
              let sc = r.input.len();
              src_hops.resize(sc, [0.0f32; HOP]);
              let mut got_any = false;
              for (i, cons) in r.input.iter_mut().enumerate() {
                let (got, dropped) = take_hop(cons, &mut src_hops[i]);
                r.ring_dropped += dropped as u64;
                if got {
                  got_any = true;
                } else {
                  src_hops[i].fill(0.0);
                }
              }
              if got_any {
                r.last_hop_at = tick_start;
              }
              if !probe_in_ok {
                probe_in.copy_from_slice(&src_hops[0]);
                probe_in_ok = true;
              }
              if nch == 1 {
                // 单声道节点 = 订阅到的通道平均（立体声设备就是 L/R 平均）
                if sc == 1 {
                  accs[ri][0] = src_hops[0];
                } else {
                  for i in 0..HOP {
                    let mut sum = 0.0f32;
                    for h in &src_hops {
                      sum += h[i];
                    }
                    accs[ri][0][i] = sum / sc as f32;
                  }
                }
              } else if r.src_channels == 1 {
                // 单声道设备喂多通道节点：铺到所有通道
                for c in 0..nch {
                  accs[ri][c] = src_hops[0];
                }
              } else {
                // 稀疏订阅：src_hops[i] 对应物理通道 input_channels[i]，落到同名端口
                for i in 0..sc {
                  let ch = r.input_channels.get(i).copied().unwrap_or(i);
                  if ch < nch {
                    accs[ri][ch] = src_hops[i];
                  }
                }
              }
            }
            for c in 0..nch {
              if r.enabled {
                apply_trim(r, &mut accs[ri][c]);
              }
            }
            if let Some(ms) = inf_sample {
              r.inf_ms = if r.inf_ms == 0.0 {
                ms
              } else {
                r.inf_ms * 0.9 + ms * 0.1
              };
              r.inf_max_ms = r.inf_max_ms.max(ms);
            }
          }
          RowKind::Process if r.ptype == "echo_cancel" => {
            // AEC：近端 = 端口 0，远端 = 端口 1（每条入线各自补 PDC 延迟）
            let mut near = [0.0f32; HOP];
            let mut far = [0.0f32; HOP];
            for ei in 0..r.inputs.len() {
              let e = r.inputs[ei];
              let Some(ssrc) = slot_of.get(e.src) else {
                continue;
              };
              if ssrc.is_empty() {
                continue;
              }
              let fp = (e.from_port as usize).min(ssrc.len() - 1);
              let Some(src) = outbufs.get(ssrc[fp]) else {
                continue;
              };
              let delayed = match r.delay_lines.get_mut(ei) {
                Some(Some(line)) => Some(line.process(src)),
                _ => None,
              };
              let s = delayed.as_ref().unwrap_or(src);
              let dst = if e.to_port == 1 { &mut far } else { &mut near };
              for i in 0..HOP {
                dst[i] += s[i];
              }
            }
            if r.enabled {
              // 跳过（enabled=false）= 不处理、也不做输入微调：信号原样直通。
              apply_trim(r, &mut near);
              apply_trim(r, &mut far);
            }
            if r.enabled
              && !r.inputs.is_empty()
              && let Some(st) = r.aec_stage.as_mut()
            {
              if recording {
                rec.tap(idx, ri, Phase::Pre, &near);
              }
              let t = Instant::now();
              match st.process(
                &mut near,
                &far,
                calibrating,
                Some((calib.as_ref(), idx, ri)),
              ) {
                Ok(()) => r.error = None,
                Err(e) => r.error = Some(e),
              }
              let ms = t.elapsed().as_secs_f32() * 1000.0;
              r.inf_ms = if r.inf_ms == 0.0 {
                ms
              } else {
                r.inf_ms * 0.9 + ms * 0.1
              };
              if ms > r.inf_max_ms {
                r.inf_max_ms = ms;
              }
              if ms > TICK.as_secs_f32() * 1000.0 * 0.8 {
                r.inf_over += 1;
              }
              if recording {
                rec.tap(idx, ri, Phase::Post, &near);
              }
            }
            accs[ri][0] = near;
          }
          RowKind::Process => {
            // 有入线的节点：输入微调属于「本节点处理」的一部分，跳过时连它也不做。
            // 纯媒体源（音效板 / 播放器）没有入线：微调作用在它自己生成的信号上，
            // 所以放到 Stage 之后再乘（否则只会乘到一段空缓冲）。
            let media_only = r.media_slot.is_some() && r.inputs.is_empty();
            if r.enabled && !media_only {
              for c in 0..nch {
                apply_trim(r, &mut accs[ri][c]);
              }
            }
            // 没有任何入线的处理节点（也不是媒体源）= 输出静音：不要白跑模型。
            let runs = !r.inputs.is_empty() || r.media_slot.is_some();
            if r.enabled && !r.stage.is_empty() && runs {
              if recording {
                rec.tap(idx, ri, Phase::Pre, &accs[ri][0]);
              }
              let t = Instant::now();
              for c in 0..nch {
                if let Some(st) = r.stage.get_mut(c) {
                  match st.process(&mut accs[ri][c], &ctx) {
                    Ok(()) => {}
                    Err(StageError::Unavailable(_)) => {}
                    Err(StageError::Fatal(m)) => {
                      r.error = Some(m);
                      break;
                    }
                  }
                }
              }
              if media_only {
                for c in 0..nch {
                  apply_trim(r, &mut accs[ri][c]);
                }
              }
              let ms = t.elapsed().as_secs_f32() * 1000.0;
              r.inf_ms = if r.inf_ms == 0.0 {
                ms
              } else {
                r.inf_ms * 0.9 + ms * 0.1
              };
              if ms > r.inf_max_ms {
                r.inf_max_ms = ms;
              }
              if ms > TICK.as_secs_f32() * 1000.0 * 0.8 {
                r.inf_over += 1;
              }
              if recording {
                rec.tap(idx, ri, Phase::Post, &accs[ri][0]);
              }
            }
            if let Some(ms) = inf_sample {
              r.inf_ms = if r.inf_ms == 0.0 {
                ms
              } else {
                r.inf_ms * 0.9 + ms * 0.1
              };
              r.inf_max_ms = r.inf_max_ms.max(ms);
            }
          }
          RowKind::Output => {
            poll_output_lease(r, &hub, idx, ri, tick_start);
            if r.enabled {
              for c in 0..nch {
                apply_trim(r, &mut accs[ri][c]);
              }
              if !probe_out_ok {
                probe_out.copy_from_slice(&accs[ri][0]);
                probe_out_ok = true;
              }
              if let Some(f) = r.output.as_ref() {
                for c in 0..nch {
                  f.push_channel(c, &accs[ri][c]);
                }
              }
              if r.net_output {
                crate::net::hub().push_tx_hop(&accs[ri][0]);
              }
            }
          }
        }
        let mut row_peak = 0.0f32;
        for c in 0..nch {
          let p = accs[ri][c].iter().fold(0.0f32, |a, b| a.max(b.abs()));
          row_peak = row_peak.max(p);
          outbufs[row_slots[c]] = accs[ri][c];
        }
        // 快起慢落：10 ms 一格，峰值条不会闪一下就消失
        r.peak = row_peak.max(r.peak * 0.85);
        if r.kind == RowKind::Output && r.enabled {
          column_peak = column_peak.max(row_peak);
        }
      }

      let peak = column_peak;
      let now = Instant::now();
      if now.duration_since(last_publish) >= PUBLISH {
        // 速率要按**实测**间隔算：拿固定 PUBLISH 去除，publish 偶尔晚一拍就会报出 105 hop/s
        // （2026-10-18 实测：同一段会话里 100 / 105 交替出现 —— 是量法的问题，不是节拍的问题）
        let elapsed = now.duration_since(last_publish).as_secs_f64().max(1e-3);
        last_publish = now;
        let n_in = g
          .iter()
          .filter(|r| {
            r.kind == RowKind::Input
              && (!r.input.is_empty() || r.aec.is_some() || r.net_input.is_some())
          })
          .count();
        let n_out = g
          .iter()
          .filter(|r| r.kind == RowKind::Output && (r.output.is_some() || r.net_output))
          .count();
        let n_proc = g.iter().filter(|r| r.kind == RowKind::Process).count();
        let unconf = g.iter().filter(|r| r.unconfigured).count();
        let errs = g.iter().filter(|r| r.error.is_some()).count();
        let mut s = format!("列 {}：输入 {n_in}，处理 {n_proc}，输出 {n_out}", idx + 1);
        let rate = (ticks - last_ticks) as f64 / elapsed;
        last_ticks = ticks;
        s.push_str(&format!("，{rate:.0} hop/s"));
        if unconf > 0 {
          s.push_str(&format!("，未配置 {unconf}"));
        }
        if errs > 0 {
          s.push_str(&format!("，异常 {errs}"));
        }
        if tick_overruns > 0 {
          s.push_str(&format!(
            "，节拍超时 {tick_overruns}（最大迟到 {max_late_ms:.1} ms）"
          ));
        }
        // 行状态：结构化（`rows`）+ 人话（`detail`）。失败的行必须把原因带出来。
        let mut rows_info: Vec<RowInfo> = Vec::with_capacity(g.len());
        let mut notes: Vec<String> = Vec::new();
        for (ri, r) in g.iter_mut().enumerate() {
          let kind = match r.kind {
            RowKind::Input => "input",
            RowKind::Process => "process",
            RowKind::Output => "output",
          };
          let state = if !r.enabled {
            Probe::ok("disabled".to_string())
          } else if let Some(e) = &r.error {
            Probe::unavailable(e.clone())
          } else if r.unconfigured {
            Probe::unavailable("未配置设备".to_string())
          } else {
            Probe::ok("running".to_string())
          };
          let buffer_ms = if let Some(aec) = r.aec.as_ref() {
            Some(aec.mic.slots() as f32 * 1000.0 / crate::audio::SAMPLE_RATE as f32)
          } else if let Some(net) = r.net_input.as_ref() {
            Some(net.stats().rx_level as f32 * 1000.0 / crate::audio::SAMPLE_RATE as f32)
          } else if let Some(cons) = r.input.first() {
            Some(cons.slots() as f32 * 1000.0 / crate::audio::SAMPLE_RATE as f32)
          } else {
            r.output.as_ref().map(|f| f.max_level_ms())
          };
          // 延迟预算：第一路启用的输入 / 输出行（流 id 精确到这一行，见 `RowTapInfo`）
          let has_source = !r.input.is_empty() || r.aec.is_some() || r.net_input.is_some();
          let has_sink = r.output.is_some() || r.net_output;
          if r.enabled && (has_source || has_sink) {
            match r.kind {
              RowKind::Input if lat_in.is_none() && has_source => {
                let stream_id = if r.pooled_input {
                  r.device.as_ref().map(|d| format!("input:{d}"))
                } else if r.ptype == "loopback" {
                  Some(format!("input:c{idx}r{ri}"))
                } else if r.aec.is_some() {
                  r.device.as_ref().map(|_| format!("input:c{idx}r{ri}m"))
                } else {
                  None
                };
                lat_in = Some(RowTapInfo {
                  stream_id,
                  buffer_ms,
                  label: r.label.clone(),
                });
              }
              RowKind::Output if lat_out.is_none() && has_sink => {
                let stream_id = if r.pooled_output {
                  r.device.as_ref().map(|d| format!("output:{d}"))
                } else {
                  None
                };
                lat_out = Some(RowTapInfo {
                  stream_id,
                  buffer_ms,
                  label: r.label.clone(),
                });
              }
              _ => {}
            }
          }
          let dropped_per_s = r.ring_drop_rate.push(now, r.ring_dropped);
          // 可视化行走的是同一个 Stage，但它没有模型：别把它算进「推理耗时」那一栏
          let has_model = (r.kind == RowKind::Process && r.ptype != "viz") || r.aec.is_some();
          let inference_ms_avg = if has_model && r.inf_ms > 0.0 {
            Probe::ok(r.inf_ms)
          } else if has_model {
            Probe::Pending
          } else {
            Probe::unavailable("不适用（本行无模型）")
          };
          let inference_ms_max = if has_model && r.inf_max_ms > 0.0 {
            Probe::ok(r.inf_max_ms)
          } else if has_model {
            Probe::Pending
          } else {
            Probe::unavailable("不适用（本行无模型）")
          };
          let inference_over_per_s = if has_model {
            r.inf_over_rate.push(now, r.inf_over)
          } else {
            Probe::unavailable("不适用（本行无模型）")
          };
          let mut detail = if let Some(e) = r.error.as_ref() {
            format!("⚠ {e}")
          } else if r.kind == RowKind::Process {
            let st = r
              .stage
              .first()
              .and_then(|s| s.status())
              .unwrap_or_else(|| "运行中".to_string());
            format!("{st}，{:.1} ms（最大 {:.1}）", r.inf_ms, r.inf_max_ms)
          } else if let Some(aec) = r.aec.as_ref() {
            let ftotal = aec.hist.lock().map(|h| h.total()).unwrap_or(0);
            let mic_idx = aec.mic_hops * crate::audio::HOP as u64;
            format!(
              "精确 {}，回退 {}，直通 {}，延时 {} ms ｜ 远端解释度 ρ={:.2}，回声抑制 {:.1} dB ｜ 网格 far={} mic={}（差 {}）{}",
              aec.exact,
              aec.latest,
              aec.pass,
              aec.delay_samples / 48,
              aec.echo.rho,
              aec.echo.supp_db,
              ftotal,
              mic_idx,
              mic_idx as i64 - ftotal as i64,
              ring_drop_note(r.ring_dropped, &dropped_per_s)
            )
          } else if let Some(net) = r.net_input.as_ref() {
            let ns = net.stats();
            format!(
              "入站 {} 包 / {:.0} ms，欠载 {}，已取 {} hop",
              ns.rx_packets,
              ns.rx_level as f64 / crate::audio::SAMPLE_RATE as f64 * 1000.0,
              ns.rx_underruns,
              ns.rx_hops
            )
          } else if r.net_output {
            format!(
              "已发 {} 包，订阅 {}",
              crate::net::hub().stats().tx_packets,
              crate::net::hub().sub_count()
            )
          } else if let Some(ms) = buffer_ms {
            format!(
              "{} ms{}",
              ms,
              ring_drop_note(r.ring_dropped, &dropped_per_s)
            )
          } else {
            String::new()
          };
          if detail.is_empty() {
            detail = "—".to_string();
          }
          if let Some(e) = r.error.as_ref() {
            notes.push(format!("⚠ {}：{e}", r.label));
          } else {
            notes.push(format!("{}：{detail}", r.label));
          }
          let mut info = RowInfo {
            index: ri,
            kind: kind.to_string(),
            ptype: r.ptype.clone(),
            label: r.label.clone(),
            channels: r.channels,
            peak: r.peak,
            declared_latency_ms: r.latency as f32 * 1000.0 / crate::audio::SAMPLE_RATE as f32,
            input_delays_ms: r
              .edge_delays
              .iter()
              .map(|d| *d as f32 * 1000.0 / crate::audio::SAMPLE_RATE as f32)
              .collect(),
            enabled: r.enabled,
            state,
            device: r.device.clone(),
            buffer_ms,
            dropped: r.ring_dropped,
            dropped_per_s,
            inference_ms_avg,
            inference_ms_max,
            inference_over: r.inf_over,
            inference_over_per_s,
            detail,
            health: Probe::Pending,
          };
          health::annotate_row(&mut info);
          rows_info.push(info);
        }
        if !notes.is_empty() {
          s.push_str(&format!(" ｜ {}", notes.join("；")));
        }
        hub.set_column_info(ColumnInfo {
          index: idx,
          peak,
          enabled: column_on,
          state: if stop.load(Relaxed) {
            "stopped".to_string()
          } else {
            "running".to_string()
          },
          latency: latency.result(),
          hop_rate: rate,
          ticks,
          tick_overruns,
          tick_overrun_per_s: tick_overrun_rate.push(now, tick_overruns),
          last_late_ms,
          max_late_ms,
          max_work_ms,
          rows: rows_info,
          text: s,
        });
        // 分析线程据此决定「测哪一路进 / 哪一路出」；停用列也要写（它据此报 disabled）
        latency.set_rows(column_on && !stop.load(Relaxed), lat_in, lat_out);
      }
    }

    // 探针写入放在行锁之外：分析线程从另一头取走，音频线程不等它
    latency.push(
      if probe_in_ok {
        Some(&probe_in[..])
      } else {
        None
      },
      if probe_out_ok {
        Some(&probe_out[..])
      } else {
        None
      },
    );

    let work_ms = tick_start.elapsed().as_secs_f32() * 1000.0;
    max_work_ms = max_work_ms.max(work_ms);
    // 等到下一拍。**这里绝不能再有「立刻跑下一轮追平」的分支** —— 那就是连发。
    tick.wait();
  }
}

#[cfg(test)]
mod tests {
  use super::*;

  /// 设备类失败**不回滚整份计划**（issue #10 的根因回归）。
  ///
  /// 两列都指向一只暂时不在的设备时，回滚会让「每改一列就被打回原样」——改设备、删列、
  /// 关开关全都做不了。设备会回来，运行期退避重开自己接上（§6.3）。
  #[test]
  fn device_failures_do_not_block_the_plan() {
    assert!(
      !blocks_plan(true, false, true),
      "设备打开失败（会自动重试）不能算关键失败"
    );
    assert!(!blocks_plan(true, true, false), "没配设备不算失败");
    assert!(!blocks_plan(false, false, false), "停用的行不算失败");
    assert!(
      blocks_plan(true, false, false),
      "模型/参考这类不会自愈的失败必须回滚"
    );
  }

  /// PDC 的延迟线：延迟 N 个样本就是**精确** N 个样本，不是近似。
  #[test]
  fn delay_line_delays_exactly_n_samples() {
    let mut line = DelayLine::new(3);
    let mut first = [0.0f32; HOP];
    for (i, x) in first.iter_mut().enumerate() {
      *x = i as f32;
    }
    let out = line.process(&first);
    assert_eq!(out[0], 0.0);
    assert_eq!(out[1], 0.0);
    assert_eq!(out[2], 0.0);
    assert_eq!(out[3], 0.0, "前 3 个样本必须是静音");
    assert_eq!(out[4], 1.0, "第 4 个样本输出第 1 个输入");
    let mut second = [0.0f32; HOP];
    second[0] = 100.0;
    let out = line.process(&second);
    assert_eq!(out[0], first[HOP - 3], "跨 hop 的延迟必须连续");
    assert_eq!(out[1], first[HOP - 2]);
    assert_eq!(out[2], first[HOP - 1]);
    assert_eq!(out[3], 100.0);
  }

  /// 设备名兜底：枚举里查不到时用**上次记住的名字**，连名字都没有才退 ID。
  #[test]
  fn saved_device_name_prefers_remembered_name() {
    let mut row = crate::plan::RowSpec {
      kind: crate::plan::RowKind::Input,
      ptype: "audio_input".into(),
      device: Some("wasapi:{0.0.1.00000000}.{guid}".into()),
      device_name: Some("  麦克风 (2- MMX 150)  ".into()),
      enabled: true,
      params: Default::default(),
      ..Default::default()
    };
    assert_eq!(saved_device_name(&row, "wasapi:x"), "麦克风 (2- MMX 150)");
    row.device_name = Some("   ".into());
    assert_eq!(saved_device_name(&row, "wasapi:x"), "wasapi:x");
    row.device_name = None;
    assert_eq!(saved_device_name(&row, "wasapi:x"), "wasapi:x");
  }
}
