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

//! 会话：按 [`Plan`] 构建并运行若干独立的列（DESIGN.md §3、§5）。
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

use super::aec::{AecRow, FAR_HIST_SAMPLES, FarHistory, FarWin, spawn_far_pump};
use super::calib::CalibHub;
use super::media::{self, MediaCmd, MediaSlot};
use super::registry::{self, ParamValue, Params};
use super::stage::{FrameContext, Stage, StageError};
use crate::audio::fanout::Fanout;
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
/// 行输入环水位：目标 2 hop，超过 4 hop 丢最旧（DESIGN.md §8）。
const ROW_TARGET: usize = 2 * HOP;
const ROW_CAP: usize = 4 * HOP;
/// 运行期重新打开设备的退避下限 / 上限（**绝不在音频 tick 里直接开设备**）。
/// 第一次失败后 1 s 重试，连续失败翻倍，最长 30 s；成功后清零。
const REOPEN_BACKOFF_MIN: Duration = Duration::from_secs(1);
const REOPEN_BACKOFF_MAX: Duration = Duration::from_secs(30);
/// 单拍处理耗时超过「一跳预算 - 2 ms」即计入节拍超时（留 2 ms 给调度）。
const TICK_LATE_WARN: Duration = Duration::from_millis(2);

struct Row {
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
  /// 其余输入会突然 +6 dB（DESIGN.md §3.2 明确要求按配置的输入数）。
  source: bool,
  /// 处理行：本行推理耗时 EMA / 最大（ms），进调试概要
  inf_ms: f32,
  inf_max_ms: f32,
  /// 推理耗时超过一跳预算的次数（累计 + 速率）
  inf_over: u64,
  inf_over_rate: RateMeter,
  /// 输入行：本行源的 48k hop 读端
  input: Option<Consumer<f32>>,
  /// 输入行：行输入环累计丢弃的样本数（水位超 `ROW_CAP` 丢最旧）。
  ///
  /// 这个计数以前没有 ——源「一大块到达」时环被瞬间灌满、按策略丢最旧，**丢多少完全看不见**，
  /// 用户看到的只是「声音规律地卡」（2026-10-18 实测：Linux 回环 100 ms 一块，每块丢掉大半）。
  ring_dropped: u64,
  /// 行输入环丢弃的实测速率（3 s 窗口；累计只涨不降，速率才看得出还在不在丢）
  ring_drop_rate: RateMeter,
  /// 输入行（loopback）：行内增益（系统混音偏响，默认 -6 dB）——**热参数**
  /// （`engine/hot.rs`：界面改它只写这个单元格，不重建会话）
  gain: Option<Arc<crate::engine::hot::HotGain>>,
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
  out_pending:
    Option<std::sync::mpsc::Receiver<Result<crate::audio::device_pool::OutputLease, String>>>,
  out_retry_at: Option<Instant>,
  out_failures: u32,
  /// AEC 输入行：mic 读端 + far 历史 + 本行模型状态
  aec: Option<AecRow>,
  /// `remote_mic` 输入行：网络入站队列（DESIGN.md §4.1）
  net_input: Option<Arc<crate::net::NetHub>>,
  /// 处理行
  stage: Option<Box<dyn Stage>>,
  /// 媒体行（soundpad / player）：运行时命令通道（界面经 MediaHub 寻址）
  media_slot: Option<Arc<MediaSlot>>,
  /// 输出行：本行扇出（该行的播放线程从这里订阅）
  output: Option<Arc<Fanout>>,
  /// `remote_speaker` 输出行：推给网络中枢（无扇出、无播放线程）
  net_output: bool,
}

struct ColumnRuntime {
  stop: Arc<AtomicBool>,
  join: Option<JoinHandle<()>>,
  /// 本列拥有的流：Session 停止时一并关闭（池化设备不在这里）
  captures: Vec<WorkerHandle>,
  /// 线程是否还活着（由列线程的 guard 置 false；监督线程据此报 degraded）
  alive: Arc<AtomicBool>,
}

/// 一个已构建并正在运行的会话。
pub struct Session {
  columns: Vec<ColumnRuntime>,
  publisher_stop: Arc<AtomicBool>,
  publisher: Option<JoinHandle<()>>,
  /// 热参数单元格：`(列, 行, 参数名) → 值`。`set_hot_param` 写它，行工作线程下一 hop 生效
  /// （见 `engine/hot.rs`：热参数**不重建会话**）。
  hot: std::collections::HashMap<(usize, usize, String), Arc<crate::engine::hot::HotGain>>,
  /// 本次构建的媒体槽位代次（提交/回滚由 `AudioManager` 决定）
  pub media_gen: u64,
  /// 本次构建的引擎代次（进调试快照）
  pub generation: u64,
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
    let mut columns = Vec::new();
    let mut hot = std::collections::HashMap::new();
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
        media_gen,
      ));
    }

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
        media_gen,
        generation,
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

  pub fn stop(&mut self) {
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

/// 网络麦克风行勾了 AEC 时要用的参数（一包，免得 `build_net_aec_row` 参数列太长）。
struct NetAecParams {
  model: String,
  far: String,
  delay_ms: f64,
  mic_gain: f32,
  far_gain: f32,
  bypass: bool,
}

impl NetAecParams {
  fn from(row: &crate::plan::RowSpec) -> Self {
    let num = |k: &str| {
      row
        .params
        .get(k)
        .and_then(|v| v.trim().parse::<f64>().ok())
        .unwrap_or(0.0)
    };
    Self {
      model: row
        .params
        .get("model")
        .cloned()
        .unwrap_or_else(|| crate::infer::aec::MODEL_AEC.to_string()),
      far: row
        .params
        .get("far_device")
        .cloned()
        .unwrap_or_else(|| "loopback".into()),
      delay_ms: num("far_delay_ms"),
      mic_gain: 10f32.powf((num("mic_gain_db") / 20.0) as f32),
      far_gain: 10f32.powf((num("far_gain_db") / 20.0) as f32),
      bypass: row
        .params
        .get("bypass")
        .map(|v| v == "1" || v.eq_ignore_ascii_case("true"))
        .unwrap_or(false),
    }
  }
}

/// 加载模型并挂到行上（网络 AEC 用）。近端固定取 `near` 扇出 —— 网络麦克风的近端
/// 就是它自己，卡片上不可改。
fn aec_attach(
  row: &mut Row,
  p: &NetAecParams,
  hist: Arc<Mutex<FarHistory>>,
  near: &Arc<Fanout>,
) -> Result<(), String> {
  let engine = Aec::load(&p.model)?;
  row.aec = Some(AecRow {
    engine,
    hist,
    // 延时取整到 10 ms（= 1 hop 的整数倍），far 窗口与 hop 网格对齐
    delay_samples: (p.delay_ms / 10.0).round() as i64 * HOP as i64,
    mic_hops: 0,
    mic: near.subscribe("net-mic"),
    mic_gain: p.mic_gain,
    far_gain: p.far_gain,
    bypass: p.bypass,
    exact: 0,
    latest: 0,
    pass: 0,
    echo: crate::engine::aec::EchoMetrics::default(),
  });
  Ok(())
}

/// 网络麦克风行勾了 AEC（`params.aec == "1"`）时建行：近端 = 网络入站（锁死），
/// 远端按 `far_device` 取。
///
/// 近/远端都经一层 `Fanout`（`spawn_net_pumps`），所以这一支和 `echo_cancel` 设备行
/// 完全同构 —— 差别只是扇出的内容来自网络而不是采集设备，`AecRow` 一个字段都不用改。
///
/// far 选设备回环时**不能**拿网络泵那个没人灌的 far 扇出当参考（历史一直空，AEC 会
/// 以为没有回声、静默失效），所以那种情况改用设备回环自己的扇出。
fn build_net_aec_row(
  row: &mut Row,
  spec: &crate::plan::RowSpec,
  hub: &SharedHub,
  captures: &mut Vec<WorkerHandle>,
  epoch: Arc<std::sync::atomic::AtomicU64>,
  tag: String,
) {
  let p = NetAecParams::from(spec);
  let net_far = p.far == crate::engine::aec::NET_FAR;
  let near_fan = Arc::new(Fanout::new(format!("{tag}-netmic")));
  let net = crate::net::hub();

  // —— 情况 1：远端取网络出站。近/远端由同一个网络泵在同一次 tick 各推一格，——
  //    所以两条时间轴天然锁在一起，`far_delay_ms` 才有个稳定基准。
  if net_far {
    let far_fan = Arc::new(Fanout::new(format!("{tag}-netfar")));
    let hist = Arc::new(Mutex::new(FarHistory::new(FAR_HIST_SAMPLES)));
    captures.push(crate::engine::aec::spawn_far_pump(
      &far_fan,
      hist.clone(),
      &tag,
      epoch,
    ));
    match aec_attach(row, &p, hist, &near_fan) {
      Ok(()) => {
        captures.push(crate::engine::aec::spawn_net_pumps(net, near_fan, far_fan));
        row.label = "网络麦克风 + AEC（远端 网络出站）".into();
      }
      Err(e) => row.error = Some(e),
    }
    return;
  }

  // —— 情况 2：远端取设备（输出回环，或直接填了一个输入设备）——
  let opened: Result<crate::audio::capture::Capture, String> =
    match crate::engine::aec::far_loopback_id(&p.far) {
      Some(id) => loopback::spawn(
        hub.clone(),
        if id.is_empty() { None } else { Some(id) },
        format!("{tag}-f"),
      ),
      None => Err("未选择远端设备".into()),
    };
  let cap = match opened {
    Ok(c) => {
      captures.push(c.handle);
      c.fanout
    }
    Err(_) => match capture::spawn(hub.clone(), p.far.clone(), format!("{tag}-f")) {
      Ok(c) => {
        captures.push(c.handle);
        c.fanout
      }
      Err(e) => {
        row.error = Some(e);
        return;
      }
    },
  };
  let hist = Arc::new(Mutex::new(FarHistory::new(FAR_HIST_SAMPLES)));
  captures.push(crate::engine::aec::spawn_far_pump(
    &cap,
    hist.clone(),
    &tag,
    epoch,
  ));
  match aec_attach(row, &p, hist, &near_fan) {
    Ok(()) => {
      // far 由设备提供，只让网络泵灌近端；第二个扇出没人灌、不参与
      captures.push(crate::engine::aec::spawn_net_pumps(
        net,
        near_fan,
        Arc::new(Fanout::new(format!("{tag}-unused-far"))),
      ));
      row.label = "网络麦克风 + AEC".into();
    }
    Err(e) => row.error = Some(e),
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
/// 水位超过 [`ROW_CAP`] 就丢最旧、降到 [`ROW_TARGET`]（DESIGN.md §8）—— 顺带把丢了多少
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
    r.input = None;
    return;
  }
  if r.in_lease.is_some() {
    return;
  }
  match crate::audio::device_pool::hub().acquire_input(hub.clone(), &dev) {
    Ok(lease) => {
      r.input = Some(lease.subscribe(&format!("col{idx}row{ri}")));
      r.in_lease = Some(lease);
      r.error = None;
      r.in_failures = 0;
      events::log("info", "engine", format!("输入设备已打开：{dev}"));
    }
    Err(e) => {
      r.error = Some(e.clone());
      r.in_failures = 1;
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
      r.input = None;
      events::log(
        "info",
        "engine",
        format!("输入行已停用，放掉设备引用：{dev}"),
      );
    }
    r.in_pending = None;
    r.in_retry_at = None;
    return;
  }
  if r.in_lease.is_some() {
    return;
  }
  if let Some(rx) = r.in_pending.as_ref() {
    match rx.try_recv() {
      Ok(Ok(lease)) => {
        r.input = Some(lease.subscribe(&format!("col{idx}row{ri}")));
        r.in_lease = Some(lease);
        r.in_pending = None;
        r.in_retry_at = None;
        r.in_failures = 0;
        r.error = None;
        events::log("info", "engine", format!("输入设备已恢复：{dev}"));
      }
      Ok(Err(e)) => {
        r.in_pending = None;
        r.in_failures = r.in_failures.saturating_add(1);
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
  match crate::audio::device_pool::hub().acquire_output(hub.clone(), &dev) {
    Ok(lease) => {
      r.output = Some(lease.out.clone());
      r.out_lease = Some(lease);
      r.error = None;
      r.out_failures = 0;
      events::log("info", "engine", format!("输出设备已打开：{dev}"));
    }
    Err(e) => {
      r.error = Some(e.clone());
      r.out_failures = 1;
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
        events::log("info", "engine", format!("输出设备已恢复：{dev}"));
      }
      Ok(Err(e)) => {
        r.out_pending = None;
        r.out_failures = r.out_failures.saturating_add(1);
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
  let name = format!("open-out-{idx}-{ri}");
  let spawned = std::thread::Builder::new().name(name).spawn(move || {
    let r = crate::audio::device_pool::hub().acquire_output(h, &d);
    let _ = tx.send(r);
  });
  match spawned {
    Ok(_) => r.out_pending = Some(rx),
    Err(e) => {
      r.out_failures = r.out_failures.saturating_add(1);
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
  media_gen: u64,
) -> ColumnRuntime {
  let mut rows: Vec<Row> = Vec::new();
  let mut captures = Vec::new();
  // `remote_mic` 入站是单队列，全会话只允许一行（SPEC 约束）
  let mut remote_mic_used = false;

  // 列头开关：整列停用 = 这一列不产出、也不占设备引用（行级开关照旧各自生效）
  let column_on = spec.enabled;
  for (ri, r) in spec.rows.iter().enumerate() {
    // 热参数登记口：本行建出来的单元格（增益 / 音量）收进会话，供运行中改值
    let mut sink = crate::engine::hot::HotSink::new();
    let mut row = Row {
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
      input: None,
      gain: None,
      device: None,
      pooled_input: false,
      pooled_output: false,
      in_lease: None,
      out_lease: None,
      in_pending: None,
      in_retry_at: None,
      in_failures: 0,
      out_pending: None,
      out_retry_at: None,
      out_failures: 0,
      aec: None,
      net_input: None,
      ring_dropped: 0,
      ring_drop_rate: RateMeter::new(),
      stage: None,
      media_slot: None,
      output: None,
      net_output: false,
    };
    let device = r.device.as_deref().filter(|d| !d.is_empty());
    match r.kind {
      RowKind::Input if r.ptype == "echo_cancel" => {
        // AEC 输入行：本行 device = mic，params.far_device = 远端参考（回环或另一路输入）。
        let far = r.params.get("far_device").cloned().unwrap_or_default();
        let delay_ms: f64 = r
          .params
          .get("far_delay_ms")
          .and_then(|v| v.trim().parse().ok())
          .unwrap_or(0.0);
        let model = r
          .params
          .get("model")
          .cloned()
          .unwrap_or_else(|| crate::infer::aec::MODEL_AEC.to_string());
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
                          &far_cap.fanout,
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
                              mic: mic_cap.fanout.subscribe(&format!("c{idx}r{ri}-mic")),
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
        row.label = "测试音 1 kHz".into();
        row.source = true;
        row.input = Some(crate::audio::tone::shared().subscribe(&format!("col{idx}row{ri}")));
      }
      RowKind::Input if r.ptype == "remote_mic" => {
        // 网络麦克风：不需要虚拟声卡驱动，网络流直接进引擎（DESIGN.md §4.1）
        // 允许一个：入站是单队列，多行会互相抢样本
        //
        // **勾了 AEC（`params.aec == "1"`）时**（2026-10-12 加）：不再是直通，而是建
        // 一张 AEC 行 —— 近端锁死为网络（卡片上不可改，这正是这张卡的意义），远端按
        // `far_device` 取。近/远端各经一层 `Fanout`（`spawn_net_pumps`），所以 AEC
        // 主循环与设备麦克风那条路完全同构。
        if remote_mic_used {
          row.error = Some("网络麦克风只允许一个输入行".into());
        } else {
          remote_mic_used = true;
          row.source = true;
          if r.params.get("aec").map(|v| v == "1").unwrap_or(false) {
            build_net_aec_row(
              &mut row,
              r,
              hub,
              &mut captures,
              calib.epoch(),
              format!("c{idx}r{ri}"),
            );
          } else {
            row.label = "网络麦克风".into();
            row.net_input = Some(crate::net::hub().clone());
          }
        }
      }
      RowKind::Input if r.ptype == "loopback" => {
        // 电脑播放音频：捕获输出设备正在播放的系统混音（复用 AEC 的回环采集）。
        // `device` = `loopback`（默认输出）/ `loopback:<端点ID｜sink>`；行内增益默认 -6 dB。
        let db: f64 = r
          .params
          .get("volume_db")
          .and_then(|v| v.trim().parse().ok())
          .unwrap_or(-6.0);
        row.gain = Some(sink.gain("volume_db", db));
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
        row.label = format!("电脑播放音频（{name}）");
        row.source = true;
        match loopback::spawn(hub.clone(), target, format!("c{idx}r{ri}")) {
          Ok(cap) => {
            row.input = Some(cap.fanout.subscribe(&format!("col{idx}row{ri}")));
            captures.push(cap.handle);
          }
          Err(e) => row.error = Some(e),
        }
      }
      RowKind::Input => match device {
        None => row.unconfigured = true,
        Some(dev) => match crate::devices::display_name(dev, true) {
          Err(e) => row.error = Some(e),
          Ok(name) => {
            row.label = name;
            row.source = true;
            // 设备由**池**持有（同一设备多列共用一条采集流），行只拿租约（DESIGN.md §6.5）
            row.device = Some(dev.to_string());
            row.pooled_input = true;
            crate::engine::session::apply_input_lease(&mut row, hub, idx, ri);
          }
        },
      },
      RowKind::Process => {
        // 媒体行（音效板 / 播放器）：注册 (列,行) 槽位，供界面投递运行时命令
        let slot = match r.ptype.as_str() {
          "soundpad" => Some(media::hub().register(media_gen, idx, ri, "soundpad")),
          "player" => Some(media::hub().register(media_gen, idx, ri, "player")),
          _ => None,
        };
        row.media_slot = slot.clone();
        match registry::create_stage(&r.ptype, &to_params(&r.params), slot, &mut sink) {
          Ok(st) => {
            row.label = registry::get_spec(&r.ptype)
              .map(|s| s.label.to_string())
              .unwrap_or(r.ptype.clone());
            row.stage = Some(st);
          }
          Err(e) => row.error = Some(e),
        }
      }
      RowKind::Output if r.ptype == "remote_speaker" => {
        // 网络扬声器：按位置 tap 取信号，交给网络中枢广播给订阅客户端
        row.label = "网络扬声器".into();
        row.net_output = true;
      }
      RowKind::Output => match device {
        None => row.unconfigured = true,
        Some(dev) => match crate::devices::find(dev, false) {
          Err(e) => row.error = Some(e),
          Ok((_, name)) => {
            row.label = name;
            // 输出也由池持有：一个设备**一条**混音，本行只是往里登记一路源（DESIGN.md §6.5）
            row.device = Some(dev.to_string());
            row.pooled_output = true;
            crate::engine::session::apply_output_lease(&mut row, hub, idx, ri);
          }
        },
      },
    }
    if let Some(e) = &row.error {
      problems.push(format!("第 {} 列第 {} 行：{e}", idx + 1, ri + 1));
      // 关键失败：这一行是启用状态、已经配置了设备/模型，却打不开 —— 新会话不能提交。
      // 未配置（还没选设备）与停用行不算（那是用户的正常中间态）。
      //
      // **模型类处理行例外**：denoise / TSE 的 `.onnx` 初始化失败时，这一行没有 Stage，
      // 链路按直通继续（声音还在，行状态红色）。不让它回滚整个计划 —— 否则用户只是
      // 想切换输出设备，也会因为模型版本不对被拒绝，界面显示新设备、引擎却还在旧设备
      // （2026-10-25 实测：切到 R20 后调试面板仍是 USB Audio，R20 没声音）。
      let model_optional = matches!(row.ptype.as_str(), "denoise" | "tse");
      if row.enabled && !row.unconfigured && !model_optional {
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
    rows.push(row);
  }

  let rows = Arc::new(Mutex::new(rows));
  let stop = Arc::new(AtomicBool::new(false));
  let alive = Arc::new(AtomicBool::new(true));
  let (rows2, stop2, alive2) = (rows.clone(), stop.clone(), alive.clone());
  let rec2 = rec.clone();
  let calib2 = calib.clone();
  let hub3 = hub.clone();
  let join = match std::thread::Builder::new()
    .name(format!("column-{}", idx + 1))
    .spawn(move || {
      let _guard = AliveGuard(alive2);
      let token = threads::ThreadToken::new(format!("column-{}", idx + 1));
      let r = catch_unwind(AssertUnwindSafe(|| {
        run_column(idx, column_on, rows2, stop2, rec2, calib2, hub3, token);
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
  }
}

struct AliveGuard(Arc<AtomicBool>);
impl Drop for AliveGuard {
  fn drop(&mut self) {
    self.0.store(false, Relaxed);
  }
}

#[allow(clippy::too_many_arguments)] // 线程入口：参数就是这一列的完整上下文
fn run_column(
  idx: usize,
  column_on: bool,
  rows: Arc<Mutex<Vec<Row>>>,
  stop: Arc<AtomicBool>,
  rec: Arc<RecorderHub>,
  calib: Arc<CalibHub>,
  hub: SharedHub,
  token: threads::ThreadToken,
) {
  let ctx = FrameContext::default();
  let mut tmp = [0.0f32; HOP];
  let mut last_publish = Instant::now() - PUBLISH;
  // 绝对时刻节拍：每轮对齐到 10 ms 网格，消除 sleep 过冲的累积漂移
  // （固定 sleep(10ms-work) 会每轮多睡一点，实测产 hop 只有 ~96/s，输出侧 ±3% 伺服补不回来 → 周期性欠载）。
  let mut next = Instant::now() + TICK;
  let mut ticks: u64 = 0;
  let mut last_ticks: u64 = 0;
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
    let late = tick_start.saturating_duration_since(next);
    if late > TICK_LATE_WARN {
      tick_overruns += 1;
      last_late_ms = late.as_secs_f32() * 1000.0;
      max_late_ms = max_late_ms.max(last_late_ms);
    }
    token.beat();
    let mut acc = [0.0f32; HOP];
    {
      let mut g = rows.lock().unwrap();
      // 混合系数按**配置的输入源**计数（`source`），不是按当前拿到租约的数量：
      // 一个设备打不开时，其余输入不该突然被抬高 6 dB（DESIGN.md §3.2）。
      let n_inputs = g
        .iter()
        .filter(|r| r.kind == RowKind::Input && r.enabled && r.source)
        .count();
      let k = 1.0 / n_inputs.max(1) as f32;
      let recording = rec.active();
      let calibrating = calib.active();

      for (ri, r) in g.iter_mut().enumerate() {
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
            // 启用/停用：挂摘设备租约。运行期**只在后台打开**（poll），绝不阻塞这一拍。
            poll_input_lease(r, &hub, idx, ri, tick_start);
            if !r.enabled {
              // 停用的输入行不产出：AEC / 回环 / 网络 / 测试音都必须尊重行/列开关
              // （以前只有池化的普通输入看开关，其余几种停用后仍在混音）。
            } else if let Some(aec) = r.aec.as_mut() {
              // AEC 行：mic hop 与 far 窗口对齐后过模型；far 历史不足则直通 mic
              let (got, dropped) = take_hop(&mut aec.mic, &mut tmp);
              r.ring_dropped += dropped as u64;
              if got {
                // 会话起步：让 far 泵的时钟与 mic 序号同起点（否则两者相差设备打开时间，
                // 校准会测出负延时且每次会话不同）；校准开始时代号变化 → 序号同样归零。
                if aec_boot {
                  aec_boot = false;
                  calib_epoch.fetch_add(1, Relaxed);
                }
                let e = calib_epoch.load(Relaxed);
                if e != epoch_seen {
                  epoch_seen = e;
                  aec.mic_hops = 0;
                }
                let hop_idx = aec.mic_hops;
                aec.mic_hops += 1;
                if aec.bypass {
                  // 直通：跳过 AEC，直接过 mic（仍应用近端增益）
                  if aec.mic_gain != 1.0 {
                    for x in tmp.iter_mut() {
                      *x *= aec.mic_gain;
                    }
                  }
                  for i in 0..HOP {
                    acc[i] += tmp[i] * k;
                  }
                } else {
                  // 校准时用固定参考延时取 far 窗口（与存储值无关，见 calib::CALIB_REF_MS）
                  let delay_samples = if calibrating {
                    crate::engine::calib::CALIB_REF_SAMPLES
                  } else {
                    aec.delay_samples
                  };
                  let start = hop_idx as i64 * HOP as i64 - delay_samples;
                  let mut far = [0.0f32; HOP];
                  let win = {
                    let h = aec.hist.lock().unwrap();
                    h.window_or_latest(start, &mut far)
                  };
                  // 校准：喂「原始近端 + 当前延时下的远端窗口」（与运行同一坐标、且在增益前）。
                  // 只让精确窗口参与测量：回退段没有按延时偏移，量到的是绝对偏移，会把延时
                  // 越加越大（far 网格已按实时推进，精确窗口正常可用；一直不可用由 calib 超时兜底）。
                  if calibrating {
                    calib.feed(idx, ri, &tmp, &far, win == FarWin::Exact);
                  }
                  // 端侧增益（进模型前）
                  if aec.mic_gain != 1.0 {
                    for x in tmp.iter_mut() {
                      *x *= aec.mic_gain;
                    }
                  }
                  if aec.far_gain != 1.0 {
                    for x in far.iter_mut() {
                      *x *= aec.far_gain;
                    }
                  }
                  match win {
                    FarWin::None => {
                      aec.pass += 1;
                      for i in 0..HOP {
                        acc[i] += tmp[i] * k;
                      }
                    }
                    w => {
                      if w == FarWin::Latest {
                        aec.latest += 1;
                      } else {
                        aec.exact += 1;
                      }
                      let t = Instant::now();
                      match aec.engine.process(&tmp, &far) {
                        Ok(out) => {
                          let elapsed = t.elapsed().as_secs_f32() * 1000.0;
                          inf_sample = Some(elapsed);
                          aec.echo.push(&tmp, &far, out);
                          for i in 0..HOP {
                            acc[i] += out[i] * k;
                          }
                        }
                        Err(e) => r.error = Some(e),
                      }
                    }
                  }
                }
              }
            } else if let Some(net) = r.net_input.as_ref() {
              // 网络麦克风：按引擎 10 ms 节拍从网络队列取一个 hop（欠载补静音）
              net.take_rx_hop(&mut tmp);
              for i in 0..HOP {
                acc[i] += tmp[i] * k;
              }
            } else if let Some(cons) = r.input.as_mut() {
              let (got, dropped) = take_hop(cons, &mut tmp);
              r.ring_dropped += dropped as u64;
              if got {
                // 行内增益（loopback 源：系统混音偏响）
                if let Some(g) = r.gain.as_ref() {
                  let g = g.linear();
                  for x in tmp.iter_mut() {
                    *x *= g;
                  }
                }
                for i in 0..HOP {
                  acc[i] += tmp[i] * k;
                }
              }
            }
            if let Some(ms) = inf_sample {
              r.inf_ms = if r.inf_ms == 0.0 {
                ms
              } else {
                r.inf_ms * 0.9 + ms * 0.1
              };
              r.inf_max_ms = r.inf_max_ms.max(ms);
              if ms > TICK.as_secs_f32() * 1000.0 * 0.8 {
                r.inf_over += 1;
              }
            }
          }
          RowKind::Process => {
            if r.enabled
              && let Some(st) = r.stage.as_mut()
            {
              // 录音采样点：处理前 = 上游输出（TSE 行的输入即降噪后）
              if recording {
                rec.tap(idx, ri, Phase::Pre, &acc);
              }
              let t = Instant::now();
              match st.process(&mut acc, &ctx) {
                Ok(()) => r.error = None,
                Err(StageError::Unavailable(_)) => {}
                Err(StageError::Fatal(m)) => r.error = Some(m),
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
                rec.tap(idx, ri, Phase::Post, &acc);
              }
            }
          }
          RowKind::Output => {
            // 启用/停用：挂摘设备租约（运行期在后台打开，绝不阻塞这一拍）
            poll_output_lease(r, &hub, idx, ri, tick_start);
            if r.enabled {
              if let Some(f) = r.output.as_ref() {
                f.push_hop(&acc);
              }
              if r.net_output {
                // 网络扬声器：位置 tap（放到哪一行就取到哪里的信号）
                crate::net::hub().push_tx_hop(&acc);
              }
            }
          }
        }
      }

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
              && (r.input.is_some() || r.aec.is_some() || r.net_input.is_some())
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
          } else if let Some(cons) = r.input.as_ref() {
            Some(cons.slots() as f32 * 1000.0 / crate::audio::SAMPLE_RATE as f32)
          } else {
            r.output.as_ref().map(|f| f.max_level_ms())
          };
          let dropped_per_s = r.ring_drop_rate.push(now, r.ring_dropped);
          let has_model = r.kind == RowKind::Process || r.aec.is_some();
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
              .as_ref()
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
          enabled: column_on,
          state: if stop.load(Relaxed) {
            "stopped".to_string()
          } else {
            "running".to_string()
          },
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
      }
    }

    let work_ms = tick_start.elapsed().as_secs_f32() * 1000.0;
    max_work_ms = max_work_ms.max(work_ms);
    let now = Instant::now();
    if next > now {
      std::thread::sleep(next - now);
      next += TICK;
    } else if now.duration_since(next) > Duration::from_millis(200) {
      next = now + TICK; // 长时间被挂起后重新对齐，不补发积压
    } else {
      next += TICK; // 只落后一点：立刻跑下一轮追平平均速率
    }
  }
}
