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
use std::sync::atomic::{AtomicBool, Ordering::Relaxed};
use std::sync::{Arc, Mutex};
use std::thread::JoinHandle;
use std::time::{Duration, Instant};

use rtrb::Consumer;

use super::aec::{AecRow, FAR_HIST_SAMPLES, FarHistory, FarWin, spawn_far_pump};
use super::calib::CalibHub;
use super::registry::{self, ParamValue, Params};
use super::stage::{FrameContext, Stage, StageError};
use crate::audio::fanout::Fanout;
use crate::audio::{HOP, WorkerHandle, capture, loopback, playback};
use crate::debug::SharedHub;
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

struct Row {
  kind: RowKind,
  label: String,
  enabled: bool,
  error: Option<String>,
  /// 未配置（如未选择设备）：不是错误，只是还没配好
  unconfigured: bool,
  /// 处理行：本行推理耗时 EMA / 最大（ms），进调试概要
  inf_ms: f32,
  inf_max_ms: f32,
  /// 输入行：本行源的 48k hop 读端
  input: Option<Consumer<f32>>,
  /// AEC 输入行：mic 读端 + far 历史 + 本行模型状态
  aec: Option<AecRow>,
  /// 处理行
  stage: Option<Box<dyn Stage>>,
  /// 输出行：本行扇出（该行的播放线程从这里订阅）
  output: Option<Arc<Fanout>>,
}

struct ColumnRuntime {
  summary: Arc<Mutex<String>>,
  stop: Arc<AtomicBool>,
  join: Option<JoinHandle<()>>,
  /// 本列拥有的流：Session 停止时一并关闭
  captures: Vec<WorkerHandle>,
  playbacks: Vec<WorkerHandle>,
}

/// 一个已构建并正在运行的会话。
pub struct Session {
  columns: Vec<ColumnRuntime>,
  publisher_stop: Arc<AtomicBool>,
  publisher: Option<JoinHandle<()>>,
}

impl Session {
  /// 按计划构建并启动；返回构建过程中收集到的问题（非致命）。不修改计划本身。
  pub fn build(
    hub: SharedHub,
    plan: &Plan,
    rec: Arc<RecorderHub>,
    calib: Arc<CalibHub>,
  ) -> (Session, Vec<String>) {
    let mut problems = crate::plan::validate(plan);
    let mut columns = Vec::new();
    for (i, spec) in plan.columns.iter().enumerate() {
      columns.push(build_column(&hub, i, spec, &mut problems, &rec, &calib));
    }

    // 汇总各列概要，定期发布到调试接口
    let summaries: Vec<Arc<Mutex<String>>> = columns.iter().map(|c| c.summary.clone()).collect();
    let publisher_stop = Arc::new(AtomicBool::new(false));
    let (ps, hub2) = (publisher_stop.clone(), hub.clone());
    let publisher = std::thread::Builder::new()
      .name("session-publish".into())
      .spawn(move || {
        while !ps.load(Relaxed) {
          let parts: Vec<String> = summaries
            .iter()
            .map(|s| s.lock().unwrap().clone())
            .collect();
          hub2.set_column(crate::debug::Probe::ok(parts.join(" ｜ ")));
          std::thread::sleep(Duration::from_millis(200));
        }
      })
      .ok();

    (
      Session {
        columns,
        publisher_stop,
        publisher,
      },
      problems,
    )
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
      for h in c.playbacks.drain(..) {
        h.stop();
      }
      for h in c.captures.drain(..) {
        h.stop();
      }
    }
    self.columns.clear();
  }
}

fn to_params(map: &BTreeMap<String, String>) -> Params {
  map
    .iter()
    .map(|(k, v)| (k.clone(), ParamValue::Text(v.clone())))
    .collect()
}

/// 从行输入环取 1 hop（超上限丢最旧到目标）；不足一 hop 返回 false。
fn take_hop(cons: &mut Consumer<f32>, out: &mut [f32; HOP]) -> bool {
  if cons.slots() > ROW_CAP {
    let drop = cons.slots() - ROW_TARGET;
    if let Ok(c) = cons.read_chunk(drop) {
      c.commit_all();
    }
  }
  if cons.slots() >= HOP
    && let Ok(c) = cons.read_chunk(HOP)
  {
    let (a, b) = c.as_slices();
    out[..a.len()].copy_from_slice(a);
    out[a.len()..].copy_from_slice(b);
    c.commit_all();
    return true;
  }
  false
}

fn build_column(
  hub: &SharedHub,
  idx: usize,
  spec: &ColumnSpec,
  problems: &mut Vec<String>,
  rec: &Arc<RecorderHub>,
  calib: &Arc<CalibHub>,
) -> ColumnRuntime {
  let mut rows: Vec<Row> = Vec::new();
  let mut captures = Vec::new();
  let mut playbacks = Vec::new();

  for (ri, r) in spec.rows.iter().enumerate() {
    let mut row = Row {
      kind: r.kind,
      label: r.ptype.clone(),
      enabled: r.enabled,
      error: None,
      unconfigured: false,
      inf_ms: 0.0,
      inf_max_ms: 0.0,
      input: None,
      aec: None,
      stage: None,
      output: None,
    };
    let device = r.device.as_deref().filter(|d| !d.is_empty());
    match r.kind {
      RowKind::Input if r.ptype == "echo_cancel" => {
        // AEC 输入行：本行 device = mic，params.far_device = 远端参考（回环或另一路输入）
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
            // 远端：扬声器（输出设备）→ 自动 WASAPI 回环；也可填输入设备当参考
            let far_loop = crate::engine::aec::far_loopback_id(&far);
            let mic = crate::devices::find(mic_dev, true);
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
              crate::devices::find(&far, true).map(|(_, n)| n)
            };
            match (mic, far_ok) {
              (Ok((_, mic_name)), Ok(far_name)) => {
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
                        let pump =
                          spawn_far_pump(&far_cap.fanout, hist.clone(), &format!("c{idx}r{ri}"));
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
        row.input = Some(crate::audio::tone::shared().subscribe(&format!("col{idx}row{ri}")));
      }
      RowKind::Input => match device {
        None => row.unconfigured = true,
        Some(dev) => match crate::devices::find(dev, true) {
          Err(e) => row.error = Some(e),
          Ok((_, name)) => {
            row.label = name;
            match capture::spawn(hub.clone(), dev.to_string(), format!("c{idx}r{ri}")) {
              Ok(cap) => {
                row.input = Some(cap.fanout.subscribe(&format!("col{idx}row{ri}")));
                captures.push(cap.handle);
              }
              Err(e) => row.error = Some(e),
            }
          }
        },
      },
      RowKind::Process => match registry::create_stage(&r.ptype, &to_params(&r.params)) {
        Ok(st) => {
          row.label = registry::get_spec(&r.ptype)
            .map(|s| s.label.to_string())
            .unwrap_or(r.ptype.clone());
          row.stage = Some(st);
        }
        Err(e) => row.error = Some(e),
      },
      RowKind::Output => match device {
        None => row.unconfigured = true,
        Some(dev) => match crate::devices::find(dev, false) {
          Err(e) => row.error = Some(e),
          Ok((_, name)) => {
            row.label = name;
            let fan = Arc::new(Fanout::new(format!("列{}输出", idx + 1)));
            match playback::spawn(
              hub.clone(),
              dev.to_string(),
              fan.clone(),
              format!("c{idx}r{ri}"),
            ) {
              Ok(h) => {
                row.output = Some(fan);
                playbacks.push(h);
              }
              Err(e) => row.error = Some(e),
            }
          }
        },
      },
    }
    if let Some(e) = &row.error {
      problems.push(format!("第 {} 列第 {} 行：{e}", idx + 1, ri + 1));
    }
    rows.push(row);
  }

  let rows = Arc::new(Mutex::new(rows));
  let summary = Arc::new(Mutex::new(format!("列 {}：构建中", idx + 1)));
  let stop = Arc::new(AtomicBool::new(false));
  let (rows2, sum2, stop2) = (rows.clone(), summary.clone(), stop.clone());
  let rec2 = rec.clone();
  let calib2 = calib.clone();
  let join = std::thread::Builder::new()
    .name(format!("column-{}", idx + 1))
    .spawn(move || run_column(idx, rows2, sum2, stop2, rec2, calib2))
    .ok();

  ColumnRuntime {
    summary,
    stop,
    join,
    captures,
    playbacks,
  }
}

fn run_column(
  idx: usize,
  rows: Arc<Mutex<Vec<Row>>>,
  summary: Arc<Mutex<String>>,
  stop: Arc<AtomicBool>,
  rec: Arc<RecorderHub>,
  calib: Arc<CalibHub>,
) {
  let ctx = FrameContext::default();
  let mut tmp = [0.0f32; HOP];
  let mut last_publish = Instant::now() - PUBLISH;
  // 绝对时刻节拍：每轮对齐到 10 ms 网格，消除 sleep 过冲的累积漂移
  // （固定 sleep(10ms-work) 会每轮多睡一点，实测产 hop 只有 ~96/s，输出侧 ±3% 伺服补不回来 → 周期性欠载）。
  let mut next = Instant::now() + TICK;
  let mut ticks: u64 = 0;
  let mut last_ticks: u64 = 0;

  while !stop.load(Relaxed) {
    ticks += 1;
    let mut acc = [0.0f32; HOP];
    {
      let mut g = rows.lock().unwrap();
      let n_inputs = g
        .iter()
        .filter(|r| r.kind == RowKind::Input && (r.input.is_some() || r.aec.is_some()))
        .count();
      let k = 1.0 / n_inputs.max(1) as f32;
      let recording = rec.active();
      let calibrating = calib.active();

      for (ri, r) in g.iter_mut().enumerate() {
        match r.kind {
          RowKind::Input => {
            if let Some(aec) = r.aec.as_mut() {
              // AEC 行：mic hop 与 far 窗口对齐后过模型；far 历史不足则直通 mic
              if take_hop(&mut aec.mic, &mut tmp) {
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
                  let start = hop_idx as i64 * HOP as i64 - aec.delay_samples;
                  let mut far = [0.0f32; HOP];
                  let win = {
                    let h = aec.hist.lock().unwrap();
                    h.window_or_latest(start, &mut far)
                  };
                  // 校准：喂「原始近端 + 当前延时下的远端窗口」（与运行同一坐标、且在增益前）。
                  // 不要求精确窗口（否则延时不对时会永远取不到 → 校准卡死）；回退时用最近段也能测。
                  if calibrating {
                    calib.feed(idx, ri, &tmp, &far);
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
                      match aec.engine.process(&tmp, &far) {
                        Ok(out) => {
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
            } else if let Some(cons) = r.input.as_mut()
              && take_hop(cons, &mut tmp)
            {
              for i in 0..HOP {
                acc[i] += tmp[i] * k;
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
              if let Err(StageError::Fatal(m)) = st.process(&mut acc, &ctx) {
                r.error = Some(m);
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
              if recording {
                rec.tap(idx, ri, Phase::Post, &acc);
              }
            }
          }
          RowKind::Output => {
            if r.enabled
              && let Some(f) = r.output.as_ref()
            {
              f.push_hop(&acc);
            }
          }
        }
      }

      let now = Instant::now();
      if now.duration_since(last_publish) >= PUBLISH {
        last_publish = now;
        let n_in = g
          .iter()
          .filter(|r| r.kind == RowKind::Input && (r.input.is_some() || r.aec.is_some()))
          .count();
        let n_out = g
          .iter()
          .filter(|r| r.kind == RowKind::Output && r.output.is_some())
          .count();
        let n_proc = g.iter().filter(|r| r.kind == RowKind::Process).count();
        let unconf = g.iter().filter(|r| r.unconfigured).count();
        let errs = g.iter().filter(|r| r.error.is_some()).count();
        let mut s = format!("列 {}：输入 {n_in}，处理 {n_proc}，输出 {n_out}", idx + 1);
        let rate = (ticks - last_ticks) as f64 / PUBLISH.as_secs_f64();
        last_ticks = ticks;
        s.push_str(&format!("，{rate:.0} hop/s"));
        if unconf > 0 {
          s.push_str(&format!("，未配置 {unconf}"));
        }
        if errs > 0 {
          s.push_str(&format!("，异常 {errs}"));
        }
        // 行状态：处理行（模型/参考 + 推理耗时）、AEC 行（对齐/延时），进调试接口便于观测
        let mut notes: Vec<String> = Vec::new();
        for r in g.iter() {
          if r.kind == RowKind::Process {
            let st = r
              .stage
              .as_ref()
              .and_then(|s| s.status())
              .unwrap_or_else(|| "运行中".to_string());
            notes.push(format!(
              "{}：{st}，{:.1} ms（最大 {:.1}）",
              r.label, r.inf_ms, r.inf_max_ms
            ));
          } else if let Some(aec) = r.aec.as_ref() {
            notes.push(format!(
              "{}：精确 {}，回退 {}，直通 {}，延时 {} ms",
              r.label,
              aec.exact,
              aec.latest,
              aec.pass,
              aec.delay_samples / 48
            ));
          }
        }
        if !notes.is_empty() {
          s.push_str(&format!(" ｜ {}", notes.join("；")));
        }
        *summary.lock().unwrap() = s;
      }
    }

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
