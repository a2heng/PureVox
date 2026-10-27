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

//! 网络中枢（DESIGN.md §4.1）：**两个方向各一个固定深度环** + 远程输入开关 + 全量统计。
//!
//! - 入站（手机 → 电脑）：网络任务塞样本，列工作线程按 10 ms 节拍取（[`Playout`]）。
//! - 出站（电脑 → 手机）：引擎推一个 hop，连接任务**立刻**编码发走（**没有队列、没有定时器**）。
//!   缓冲只在接收方（手机上的环）。
//!
//! 所有计数都只做**观测**：`jitter` / `max_gap` / `late` / `trim` / `plc` 都不参与控制
//! （AGENTS §1.4：看得见才谈得上判断）。

use std::sync::atomic::{AtomicBool, AtomicI64, AtomicU16, AtomicU64, Ordering::Relaxed};
use std::sync::{Arc, Mutex};
use std::time::Instant;

use crate::audio::HOP;
use crate::dsp::meter::RateMeter;
use crate::net::rt::{Playout, PlayoutStats, RING_MS};

/// 发送交接格的上限（hop 数）：**只是防呆**。
///
/// 正常恒为 0 —— 引擎每出一 hop，发送任务立刻编码发走。连接任务偶尔被调度慢
/// （GC / 别的任务抢核）时不能把引擎线程堵住，所以留一个极小的缓冲；
/// 4 hop = 40 ms，超过就丢新的并计数（客户端从 seq 缺口也看得见）。
const MAX_PENDING_HOPS: usize = 4;

/// 一路出站订阅的统计（中枢出 `/debug`，与发送任务共享）。
#[derive(Clone, Debug, serde::Serialize)]
pub struct TxSubStat {
  /// 待发 hop 数（正常 0）：>0 说明发送任务被调度慢了
  pub pending_hops: u64,
  /// 这一路累计发出的包数
  pub sent: u64,
  /// 交接格顶格时丢掉的 hop 数（每个 = 10 ms 音频）
  pub dropped: u64,
}

/// 一路订阅的**统计**句柄（**故意不含 `tx`**：接收端攥着发送端的话，退订后通道永不关闭）。
#[derive(Clone)]
struct SubStat {
  pending: Arc<AtomicU64>,
  sent: Arc<AtomicU64>,
  dropped: Arc<AtomicU64>,
}

struct SubHandle {
  tx: tokio::sync::mpsc::UnboundedSender<Vec<f32>>,
  stat: SubStat,
}

/// 一路出站订阅（连接任务持有）。
pub struct OutboundSub {
  rx: tokio::sync::mpsc::UnboundedReceiver<Vec<f32>>,
  stat: SubStat,
}

impl OutboundSub {
  /// 等下一个 hop。`None` = 订阅已移除。
  pub async fn recv(&mut self) -> Option<Vec<f32>> {
    let hop = self.rx.recv().await?;
    self.stat.pending.fetch_sub(1, Relaxed);
    Some(hop)
  }

  pub fn note_sent(&self) {
    self.stat.sent.fetch_add(1, Relaxed);
  }
}

struct Outbound {
  subs: Mutex<Vec<(u64, SubHandle)>>,
  next_id: AtomicU64,
}

impl Outbound {
  fn subscribe(&self) -> (u64, OutboundSub) {
    let id = self.next_id.fetch_add(1, Relaxed);
    let (tx, rx) = tokio::sync::mpsc::unbounded_channel();
    let stat = SubStat {
      pending: Arc::new(AtomicU64::new(0)),
      sent: Arc::new(AtomicU64::new(0)),
      dropped: Arc::new(AtomicU64::new(0)),
    };
    self.subs.lock().unwrap().push((
      id,
      SubHandle {
        tx,
        stat: stat.clone(),
      },
    ));
    (id, OutboundSub { rx, stat })
  }

  fn unsubscribe(&self, id: u64) {
    self.subs.lock().unwrap().retain(|(sid, _)| *sid != id);
  }

  /// 引擎线程调用：**非阻塞**地把这一 hop 交给每一路订阅。
  fn push_hop(&self, hop: &[f32]) {
    let subs = self.subs.lock().unwrap();
    for (_, h) in subs.iter() {
      if h.stat.pending.load(Relaxed) >= MAX_PENDING_HOPS as u64 {
        h.stat.dropped.fetch_add(1, Relaxed);
        continue;
      }
      h.stat.pending.fetch_add(1, Relaxed);
      if h.tx.send(hop.to_vec()).is_err() {
        h.stat.pending.fetch_sub(1, Relaxed);
      }
    }
  }

  fn count(&self) -> usize {
    self.subs.lock().unwrap().len()
  }

  fn pending_hops(&self) -> u64 {
    self
      .subs
      .lock()
      .unwrap()
      .iter()
      .map(|(_, h)| h.stat.pending.load(Relaxed))
      .sum()
  }

  fn sum_dropped(&self) -> u64 {
    self
      .subs
      .lock()
      .unwrap()
      .iter()
      .map(|(_, h)| h.stat.dropped.load(Relaxed))
      .sum()
  }

  fn stats(&self) -> Vec<TxSubStat> {
    self
      .subs
      .lock()
      .unwrap()
      .iter()
      .map(|(_, h)| TxSubStat {
        pending_hops: h.stat.pending.load(Relaxed),
        sent: h.stat.sent.load(Relaxed),
        dropped: h.stat.dropped.load(Relaxed),
      })
      .collect()
  }
}

/// 客户端上报的**自己那侧**状态（`st` 控制消息，2 Hz）。
///
/// 手机 / 网页的最后一跳（AudioTrack / AudioWorklet）在客户端上，服务端看不见；
/// 没有这份上报，「扬声器为什么还是慢」只能靠猜。
#[derive(Clone, Debug, Default, serde::Serialize)]
pub struct ClientReport {
  /// 客户端播放环的水位 / 深度（ms）——深度应该恒等于 [`RING_MS`]
  pub level_ms: f32,
  pub target_ms: f32,
  /// 欠载 / PLC 掩盖 / 来晚的包（累计）
  pub underruns: u64,
  pub concealed: u64,
  pub late: u64,
  /// 客户端实测的**单向延迟**（电脑发出 → 收到并开始播，ms）
  pub one_way_ms: Option<f64>,
  /// 客户端实测的**音频时钟偏差**（ppm，正 = 客户端比电脑快）；测不出来就不带
  pub clock_ppm: Option<f64>,
  /// 上报时刻（电脑时钟 ms，只用于判断新鲜度）
  pub at_ms: u64,
}

/// 网络统计（`/debug` 的 `net` 段）。**字段只增不改名**（AGENTS §1.2）。
#[derive(Clone, Debug, serde::Serialize)]
pub struct NetStats {
  pub port: u16,
  pub running: bool,
  pub clients: usize,
  pub subs: usize,

  // ---- 入站（手机 → 电脑）----
  pub rx_packets: u64,
  pub rx_samples: u64,
  pub rx_level: usize,
  pub rx_level_ms: f32,
  pub rx_target_ms: f32,
  pub rx_hops: u64,
  pub rx_underruns: u64,
  pub rx_plc: u64,
  pub rx_trims: u64,
  pub rx_dropped: u64,
  pub rx_late: u64,
  pub rx_lost: u64,
  pub rx_jitter_ms: f32,
  pub rx_worst_jitter_ms: f32,
  pub rx_max_gap_ms: f32,
  pub rx_latency_ms: Option<f64>,
  pub rx_verdict: crate::debug::Health,

  // ---- 出站（电脑 → 手机）----
  pub tx_packets: u64,
  pub tx_level: usize,
  pub tx_cap_ms: f32,
  pub tx_hops: u64,
  pub tx_dropped: u64,
  pub tx_subs: Vec<TxSubStat>,
  pub verdict: crate::debug::Health,

  // ---- 客户端上报 ----
  pub client: Option<ClientReport>,
  pub clock_offset_us: Option<i64>,
  pub rtt_ms: Option<f64>,

  // ---- 速率（3 s 窗口）----
  pub tx_packets_per_s: crate::debug::Probe<f64>,
  pub rx_packets_per_s: crate::debug::Probe<f64>,
  pub rx_trims_per_s: crate::debug::Probe<f64>,

  // ---- 远程输入 / 其它 ----
  pub keys_injected: u64,
  pub keys_unmapped: u64,
  pub chars_typed: u64,
  pub remote_input: bool,
  pub last_error: Option<String>,
  pub codec: crate::debug::Probe<String>,
  pub client_caps: Option<String>,
}

impl Default for NetStats {
  fn default() -> Self {
    NetStats {
      port: crate::net::proto::PORT,
      running: false,
      clients: 0,
      subs: 0,
      rx_packets: 0,
      rx_samples: 0,
      rx_level: 0,
      rx_level_ms: 0.0,
      rx_target_ms: RING_MS as f32,
      rx_hops: 0,
      rx_underruns: 0,
      rx_plc: 0,
      rx_trims: 0,
      rx_dropped: 0,
      rx_late: 0,
      rx_lost: 0,
      rx_jitter_ms: 0.0,
      rx_worst_jitter_ms: 0.0,
      rx_max_gap_ms: 0.0,
      rx_latency_ms: None,
      rx_verdict: crate::debug::Health::ok("网络服务未启动"),
      tx_packets: 0,
      tx_level: 0,
      tx_cap_ms: 0.0,
      tx_hops: 0,
      tx_dropped: 0,
      tx_subs: Vec::new(),
      verdict: crate::debug::Health::ok("网络服务未启动"),
      client: None,
      clock_offset_us: None,
      rtt_ms: None,
      tx_packets_per_s: crate::debug::Probe::Pending,
      rx_packets_per_s: crate::debug::Probe::Pending,
      rx_trims_per_s: crate::debug::Probe::Pending,
      keys_injected: 0,
      keys_unmapped: 0,
      chars_typed: 0,
      remote_input: false,
      last_error: None,
      codec: crate::debug::Probe::Pending,
      client_caps: None,
    }
  }
}

struct NetRates {
  rx_packets: RateMeter,
  tx_packets: RateMeter,
  rx_trims: RateMeter,
}

/// 网络中枢：引擎与网络任务之间唯一的共享状态。
pub struct NetHub {
  inbound: Mutex<Playout>,
  outbound: Outbound,
  remote_input: AtomicBool,
  stats: Mutex<NetStats>,
  client: Mutex<Option<ClientReport>>,
  offset_us: AtomicI64,
  has_offset: AtomicBool,
  /// **刚推给客户端的那个 hop**（网络麦克风勾了 AEC 时的「远端」参考源）
  tx_last: Mutex<([f32; HOP], u64)>,
  running: AtomicBool,
  port: AtomicU16,
  ca_pem: Mutex<Option<String>>,
  client_caps: Mutex<Option<String>>,
  rates: Mutex<NetRates>,
}

impl Default for NetHub {
  fn default() -> Self {
    NetHub::new()
  }
}

impl NetHub {
  pub fn new() -> Self {
    NetHub {
      inbound: Mutex::new(Playout::new(RING_MS)),
      outbound: Outbound {
        subs: Mutex::new(Vec::new()),
        next_id: AtomicU64::new(1),
      },
      remote_input: AtomicBool::new(false),
      stats: Mutex::new(NetStats::default()),
      client: Mutex::new(None),
      offset_us: AtomicI64::new(0),
      has_offset: AtomicBool::new(false),
      tx_last: Mutex::new(([0.0f32; HOP], 0u64)),
      running: AtomicBool::new(false),
      port: AtomicU16::new(crate::net::proto::PORT),
      ca_pem: Mutex::new(None),
      client_caps: Mutex::new(None),
      rates: Mutex::new(NetRates {
        rx_packets: RateMeter::new(),
        tx_packets: RateMeter::new(),
        rx_trims: RateMeter::new(),
      }),
    }
  }

  // ---- 生命周期 ----

  pub fn set_running(&self, v: bool) {
    self.running.store(v, Relaxed);
    if !v {
      self
        .inbound
        .lock()
        .unwrap_or_else(|e| e.into_inner())
        .reset();
      *self.client.lock().unwrap_or_else(|e| e.into_inner()) = None;
      self.has_offset.store(false, Relaxed);
    }
  }

  pub fn is_running(&self) -> bool {
    self.running.load(Relaxed)
  }

  pub fn port(&self) -> u16 {
    self.port.load(Relaxed)
  }

  pub fn set_port(&self, v: u16) {
    self.port.store(v, Relaxed);
  }

  pub fn set_ca_pem(&self, pem: String) {
    if let Ok(mut slot) = self.ca_pem.lock() {
      *slot = Some(pem);
    }
  }

  pub fn ca_pem(&self) -> Option<String> {
    self.ca_pem.lock().ok().and_then(|p| p.clone())
  }

  pub fn set_client_caps(&self, s: String) {
    if let Ok(mut slot) = self.client_caps.lock() {
      *slot = Some(s);
    }
  }

  // ---- 远程输入 ----

  pub fn set_remote_input(&self, on: bool) {
    self.remote_input.store(on, Relaxed);
    self.stats.lock().unwrap().remote_input = on;
  }

  pub fn remote_input(&self) -> bool {
    self.remote_input.load(Relaxed)
  }

  // ---- 时钟偏移（`sync`/`syncreply` 估出来，只用于把单向延迟算给人看）----

  pub fn set_clock_offset(&self, us: i64) {
    self.offset_us.store(us, Relaxed);
    self.has_offset.store(true, Relaxed);
  }

  pub fn clock_offset(&self) -> Option<i64> {
    if self.has_offset.load(Relaxed) {
      Some(self.offset_us.load(Relaxed))
    } else {
      None
    }
  }

  // ---- 入站 ----

  pub fn push_rx(&self, samples: &[f32]) {
    self.push_rx_at(samples, Instant::now())
  }

  fn push_rx_at(&self, samples: &[f32], now: Instant) {
    let mut q = self.inbound.lock().unwrap_or_else(|e| e.into_inner());
    q.push(samples, now);
    let st = q.stats();
    drop(q);
    let mut s = self.stats.lock().unwrap();
    s.rx_packets = st.packets;
    s.rx_samples += samples.len() as u64;
    s.rx_trims = st.trims;
    s.rx_dropped = st.dropped;
    s.rx_late = st.late;
  }

  pub fn take_rx_hop(&self, out: &mut [f32]) {
    self.take_rx_hop_at(out)
  }

  fn take_rx_hop_at(&self, out: &mut [f32]) {
    let mut q = self.inbound.lock().unwrap_or_else(|e| e.into_inner());
    q.pull(out);
    let st = q.stats();
    drop(q);
    let mut s = self.stats.lock().unwrap();
    s.rx_hops = st.hops;
    s.rx_underruns = st.underruns;
    s.rx_plc = st.plc;
  }

  /// 记一次入站 PLC（服务端按 seq 缺口补帧时调）。
  pub fn note_rx_plc(&self) {
    self
      .inbound
      .lock()
      .unwrap_or_else(|e| e.into_inner())
      .note_plc();
  }

  /// 记一次入站单向延迟（µs，`sync` 偏移 + 帧头时间戳算出来的）。
  pub fn note_rx_latency(&self, us: i64) {
    if let Ok(mut s) = self.stats.lock() {
      s.rx_latency_ms = Some(us as f64 / 1000.0);
    }
  }

  /// 记一次入站丢包（seq 缺口，已用 PLC 补）。**不动音频缓冲**。
  pub fn note_rx_lost(&self, n: u64) {
    if n > 0
      && let Ok(mut s) = self.stats.lock()
    {
      s.rx_lost += n;
    }
  }

  // ---- 出站 ----

  pub fn push_tx_hop(&self, hop: &[f32]) {
    if let Ok(mut slot) = self.tx_last.lock() {
      let n = HOP.min(hop.len());
      slot.0[..n].copy_from_slice(&hop[..n]);
      for x in slot.0.iter_mut().skip(n) {
        *x = 0.0;
      }
      slot.1 = slot.1.wrapping_add(1);
    }
    self.outbound.push_hop(hop);
  }

  pub fn tx_last(&self) -> ([f32; HOP], u64) {
    *self.tx_last.lock().unwrap_or_else(|e| e.into_inner())
  }

  pub fn subscribe_tx(&self) -> (u64, OutboundSub) {
    self.outbound.subscribe()
  }

  pub fn unsubscribe_tx(&self, id: u64) {
    self.outbound.unsubscribe(id);
  }

  pub fn sub_count(&self) -> usize {
    self.outbound.count()
  }

  pub fn count_tx_packet(&self) {
    self.stats.lock().unwrap().tx_packets += 1;
  }

  /// 待发 hop 数（样本口径）：正常 0。
  pub fn tx_level(&self) -> usize {
    self.outbound.pending_hops() as usize * HOP
  }

  // ---- 客户端上报（`st`）----

  pub fn set_client_report(&self, r: ClientReport) {
    *self.client.lock().unwrap_or_else(|e| e.into_inner()) = Some(r);
  }

  // ---- 连接与统计 ----

  pub fn client_connected(&self) {
    let mut s = self.stats.lock().unwrap();
    s.clients += 1;
    s.last_error = None;
  }

  pub fn client_disconnected(&self) {
    let mut s = self.stats.lock().unwrap();
    s.clients = s.clients.saturating_sub(1);
    drop(s);
    self
      .inbound
      .lock()
      .unwrap_or_else(|e| e.into_inner())
      .reset();
  }

  pub fn set_last_error(&self, e: impl Into<String>) {
    self.stats.lock().unwrap().last_error = Some(e.into());
  }

  pub fn set_rtt(&self, ms: f64) {
    self.stats.lock().unwrap().rtt_ms = Some(ms);
  }

  pub fn count_key(&self, injected: bool) {
    let mut s = self.stats.lock().unwrap();
    if injected {
      s.keys_injected += 1;
    } else {
      s.keys_unmapped += 1;
    }
  }

  pub fn count_chars(&self, n: usize) {
    self.stats.lock().unwrap().chars_typed += n as u64;
  }

  /// 出站结论（推来即发，所以只看「有没有积压 / 丢没丢」）。
  fn tx_verdict(&self, subs: &[TxSubStat], hops: u64) -> crate::debug::Health {
    if subs.is_empty() {
      return crate::debug::Health::ok(
        "没有客户端在收电脑音频（「网络扬声器」那一行还没有订阅者）",
      );
    }
    if hops == 0 {
      return crate::debug::Health::warn(format!(
        "{} 路订阅，但还没有音频推给客户端（「网络扬声器」那一行没接上信号）",
        subs.len()
      ));
    }
    let dropped: u64 = subs.iter().map(|s| s.dropped).sum();
    let pending: u64 = subs.iter().map(|s| s.pending_hops).sum();
    if pending > 0 {
      return crate::debug::Health::warn(format!(
        "{} 路订阅里有 {pending} 个 hop 还没发出去（发送任务被调度慢了？）",
        subs.len()
      ));
    }
    if dropped > 0 {
      return crate::debug::Health::warn(format!(
        "{} 路订阅正常，但累计丢了 {dropped} 个 hop（每个 10 ms）",
        subs.len()
      ));
    }
    crate::debug::Health::ok(format!("{} 路订阅正常：推来即发，无积压", subs.len()))
  }

  /// 入站结论：单向延迟 + 水位 + 抖动/间隔 + 丢包/裁剪，一起给人话。
  fn rx_verdict(&self, st: &PlayoutStats, lost: u64, lat: Option<f64>) -> crate::debug::Health {
    if st.packets == 0 {
      return crate::debug::Health::ok("还没有收到网络麦克风的数据");
    }
    let mut parts = Vec::new();
    if let Some(l) = lat {
      parts.push(format!("单向 {l:.0} ms"));
    }
    parts.push(format!("水位 {:.0}/{:.0} ms", st.level_ms, st.target_ms));
    parts.push(format!("抖动 {:.1} ms", st.jitter_ms));
    if st.max_gap_ms > 20.0 {
      parts.push(format!("最大间隔 {:.0} ms", st.max_gap_ms));
    }
    if lost > 0 {
      parts.push(format!("丢 {lost} 包"));
    }
    if st.trims > 0 {
      parts.push(format!("裁 {} 格", st.trims));
    }
    if st.underruns > 0 {
      parts.push(format!("欠载 {}", st.underruns));
    }
    let text = parts.join("，");
    if st.underruns > 0 || lost > 0 || st.trims > 0 {
      crate::debug::Health::warn(text)
    } else {
      crate::debug::Health::ok(text)
    }
  }

  /// 调试快照用的统计克隆（同时带上 opus 库探测）。
  pub fn stats(&self) -> NetStats {
    let mut s = self.stats.lock().unwrap().clone();
    s.port = self.port();
    s.running = self.is_running();
    s.subs = self.sub_count();
    s.remote_input = self.remote_input();

    let in_st = self
      .inbound
      .lock()
      .unwrap_or_else(|e| e.into_inner())
      .stats();
    s.rx_level_ms = in_st.level_ms as f32;
    s.rx_level = (in_st.level_ms * crate::audio::SAMPLE_RATE as f64 / 1000.0) as usize;
    s.rx_target_ms = in_st.target_ms as f32;
    s.rx_jitter_ms = in_st.jitter_ms as f32;
    s.rx_worst_jitter_ms = in_st.worst_jitter_ms as f32;
    s.rx_max_gap_ms = in_st.max_gap_ms as f32;
    s.rx_hops = in_st.hops;
    s.rx_underruns = in_st.underruns;
    s.rx_plc = in_st.plc;
    s.rx_trims = in_st.trims;
    s.rx_dropped = in_st.dropped;
    s.rx_late = in_st.late;
    s.rx_verdict = self.rx_verdict(&in_st, s.rx_lost, s.rx_latency_ms);

    s.tx_level = self.tx_level();
    s.tx_hops = self.tx_last.lock().map(|v| v.1).unwrap_or(0);
    s.tx_cap_ms = 0.0; // 出站无缓冲（字段保留）
    s.tx_dropped = self.outbound.sum_dropped();
    s.tx_subs = self.outbound.stats();
    s.verdict = self.tx_verdict(&s.tx_subs, s.tx_hops);

    s.client = self.client.lock().ok().and_then(|c| c.clone());
    s.clock_offset_us = self.clock_offset();
    {
      let now = Instant::now();
      let mut r = self.rates.lock().unwrap_or_else(|e| e.into_inner());
      s.rx_packets_per_s = r.rx_packets.push(now, s.rx_packets);
      s.tx_packets_per_s = r.tx_packets.push(now, s.tx_packets);
      s.rx_trims_per_s = r.rx_trims.push(now, s.rx_trims);
    }
    s.codec = match super::opus_sys::probe() {
      Ok(v) => {
        let path = super::opus_sys::loaded_path().unwrap_or_default();
        crate::debug::Probe::ok(format!("{v}（{path}）"))
      }
      Err(e) => crate::debug::Probe::unavailable(e),
    };
    s.client_caps = self.client_caps.lock().ok().and_then(|c| c.clone());
    s
  }
}

#[cfg(test)]
mod tests {
  use super::*;
  use std::time::Duration;

  /// 模拟时钟：每步 10 ms（与真实 hop 节拍一致）。
  struct Clock(Instant);
  impl Clock {
    fn new() -> Self {
      Clock(Instant::now())
    }
    fn step(&mut self) -> Instant {
      self.0 += Duration::from_millis(10);
      self.0
    }
  }

  #[test]
  fn rx_priming_then_playback_is_exact() {
    let hub = NetHub::new();
    let mut c = Clock::new();
    let mut hop = [9.0f32; HOP];
    hub.take_rx_hop_at(&mut hop);
    assert!(hop.iter().all(|x| *x == 0.0), "起播前是静音");
    assert_eq!(hub.stats().rx_underruns, 0, "起播前的静音不算欠载");
    for _ in 0..2 {
      hub.push_rx_at(&[0.5; HOP], c.step());
    }
    hub.take_rx_hop_at(&mut hop);
    assert!(hop.iter().all(|x| (*x - 0.5).abs() < 1e-6), "样本必须原样");
  }

  #[test]
  fn inbound_steady_state_is_lossless() {
    let hub = NetHub::new();
    let mut c = Clock::new();
    let mut out = [0.0f32; HOP];
    for _ in 0..2 {
      hub.push_rx_at(&[0.3; HOP], c.step());
    }
    for _ in 0..2000 {
      let now = c.step();
      hub.push_rx_at(&[0.3; HOP], now);
      hub.take_rx_hop_at(&mut out);
    }
    let s = hub.stats();
    assert_eq!(s.rx_underruns, 0, "稳态不该欠载");
    assert_eq!(s.rx_trims, 0, "稳态不该裁剪");
    assert_eq!(s.rx_hops, 2000);
    assert!(
      s.rx_level_ms <= (RING_MS + 10.0) as f32,
      "水位应贴住固定深度：{:.1} ms",
      s.rx_level_ms
    );
  }

  #[tokio::test]
  async fn outbound_pushes_immediately() {
    let hub = NetHub::new();
    let (id, mut sub) = hub.subscribe_tx();
    hub.push_tx_hop(&[0.25; HOP]);
    let hop = tokio::time::timeout(Duration::from_millis(50), sub.recv())
      .await
      .expect("推来即发：不该等定时器")
      .expect("订阅还在");
    assert!(hop.iter().all(|x| (*x - 0.25).abs() < 1e-6));
    assert_eq!(hub.stats().tx_subs[0].pending_hops, 0, "发完就没有积压");
    hub.unsubscribe_tx(id);
    assert!(sub.recv().await.is_none(), "退订后 recv 应结束");
  }

  #[tokio::test]
  async fn outbound_drops_instead_of_blocking_engine() {
    let hub = NetHub::new();
    let (_id, mut sub) = hub.subscribe_tx();
    for _ in 0..10 {
      hub.push_tx_hop(&[1.0; HOP]);
    }
    let st = hub.stats();
    assert_eq!(st.tx_subs[0].pending_hops, MAX_PENDING_HOPS as u64);
    assert_eq!(st.tx_subs[0].dropped, 10 - MAX_PENDING_HOPS as u64);
    for _ in 0..MAX_PENDING_HOPS {
      let _ = sub.recv().await;
    }
    assert_eq!(hub.stats().tx_subs[0].pending_hops, 0);
  }

  #[test]
  fn remote_input_defaults_off() {
    let hub = NetHub::new();
    assert!(!hub.remote_input());
    hub.set_remote_input(true);
    assert!(hub.remote_input());
    assert!(hub.stats().remote_input);
  }

  #[test]
  fn client_report_reaches_snapshot() {
    let hub = NetHub::new();
    assert!(hub.stats().client.is_none(), "没上报时应为 null");
    hub.set_client_report(ClientReport {
      level_ms: 21.0,
      target_ms: RING_MS as f32,
      underruns: 0,
      concealed: 2,
      late: 1,
      one_way_ms: Some(14.5),
      clock_ppm: Some(-27.0),
      at_ms: 1234,
    });
    let st = hub.stats().client.expect("上报后应可见");
    assert!((st.level_ms - 21.0).abs() < 1e-6);
    assert_eq!(st.concealed, 2);
    assert_eq!(st.clock_ppm, Some(-27.0));
  }

  /// 深度是常量：不管有没有抖动 / 欠载，`rx_target_ms` 必须恒等于 `RING_MS`。
  #[test]
  fn depth_is_constant() {
    let hub = NetHub::new();
    let mut c = Clock::new();
    let mut out = [0.0f32; HOP];
    for k in 0..200 {
      // 制造大抖动（0/30 ms 交替），再穿插几次欠载
      c.0 += Duration::from_millis(if k % 2 == 0 { 0 } else { 30 });
      if k % 7 != 0 {
        hub.push_rx_at(&[0.1; HOP], c.0);
      }
      hub.take_rx_hop_at(&mut out);
    }
    let s = hub.stats();
    assert_eq!(s.rx_target_ms, RING_MS as f32, "深度不许被动态调整");
    assert!(s.rx_jitter_ms > 0.0, "抖动要能观测到");
  }

  #[test]
  fn stop_clears_audio_state() {
    let hub = NetHub::new();
    for _ in 0..8 {
      hub.push_rx(&[0.2; HOP]);
    }
    assert!(hub.stats().rx_level > 0);
    hub.set_running(false);
    assert_eq!(hub.stats().rx_level, 0, "停服务后缓冲必须清空");
  }

  #[test]
  fn hop_is_ten_ms() {
    assert_eq!(HOP, crate::audio::SAMPLE_RATE as usize / 100);
  }
}
