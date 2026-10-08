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

//! 网络中枢（DESIGN.md §4.1、§8）：进出两条有界缓冲 + 远程输入开关 + 统计。
//!
//! **主时钟在引擎侧**：网络任务只负责「解码后把样本塞进队列」，由列工作线程按自己的
//! 10 ms 节拍取走（DESIGN.md §2/§3.3）。网络抖动因此被缓冲吸收，不会传导成 hop 节奏抖动；
//! 缓冲欠载时列线程补静音（与设备采集缺帧同一种处理）。
//!
//! 两个方向都是**有界**的：目标 50 ms、硬顶 80 ms，超限丢最旧（保新）。

use std::collections::VecDeque;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering::Relaxed};
use std::sync::{Arc, Mutex};

use crate::audio::{HOP, SAMPLE_RATE};

/// 出站队列上限（hop 数）：8 hop = 80 ms。
const OUT_CAP_HOPS: usize = 8;
/// 入站队列上限（样本）：80 ms。
const IN_CAP: usize = SAMPLE_RATE as usize / 100 * 80;

/// 入站样本队列（手机 → 电脑）。多生产者（多个连接）、单消费者（一列的 `remote_mic` 行）。
#[derive(Default, Debug)]
pub struct SampleQueue {
  buf: VecDeque<f32>,
}

impl SampleQueue {
  /// 追加解码后的样本；超过上限丢最旧。
  pub fn push(&mut self, samples: &[f32]) -> u64 {
    self.buf.extend(samples.iter().copied());
    let mut dropped = 0u64;
    while self.buf.len() > IN_CAP {
      self.buf.pop_front();
      dropped += 1;
    }
    dropped
  }

  /// 取一个 hop；不足返回 false（调用方补静音）。
  pub fn take_hop(&mut self, out: &mut [f32]) -> bool {
    if self.buf.len() < out.len() {
      return false;
    }
    for (i, slot) in out.iter_mut().enumerate() {
      *slot = self.buf[i];
    }
    self.buf.drain(..out.len());
    true
  }

  /// 水位（样本数），进调试接口。
  pub fn level(&self) -> usize {
    self.buf.len()
  }
}

/// 出站 hop 队列（电脑 → 手机）：一订阅者一条。
#[derive(Default)]
struct HopQueue {
  buf: VecDeque<f32>,
}

impl HopQueue {
  fn push(&mut self, hop: &[f32]) -> u64 {
    self.buf.extend(hop.iter().copied());
    let mut dropped = 0u64;
    while self.buf.len() > OUT_CAP_HOPS * HOP {
      self.buf.pop_front();
      dropped += 1;
    }
    dropped
  }

  fn take_hop(&mut self, out: &mut [f32]) -> bool {
    if self.buf.len() < out.len() {
      return false;
    }
    for (i, slot) in out.iter_mut().enumerate() {
      *slot = self.buf[i];
    }
    self.buf.drain(..out.len());
    true
  }

  fn level(&self) -> usize {
    self.buf.len()
  }
}

/// 一个订阅者的句柄（连接任务持有）。
pub struct OutboundSub {
  q: Arc<Mutex<HopQueue>>,
}

impl OutboundSub {
  /// 取一个 hop 准备发送；欠载返回 false（发送静音包，保持时钟不断）。
  pub fn take_hop(&self, out: &mut [f32]) -> bool {
    self.q.lock().unwrap().take_hop(out)
  }
}

struct Outbound {
  subs: Mutex<Vec<(u64, Arc<Mutex<HopQueue>>)>>,
  next_id: AtomicU64,
  dropped: AtomicU64,
}

impl Outbound {
  fn subscribe(&self) -> (u64, OutboundSub) {
    let id = self.next_id.fetch_add(1, Relaxed);
    let q = Arc::new(Mutex::new(HopQueue::default()));
    self.subs.lock().unwrap().push((id, q.clone()));
    (id, OutboundSub { q })
  }

  fn unsubscribe(&self, id: u64) {
    self.subs.lock().unwrap().retain(|(sid, _)| *sid != id);
  }

  fn push_hop(&self, hop: &[f32]) {
    let subs = self.subs.lock().unwrap();
    if subs.is_empty() {
      return;
    }
    for (_, q) in subs.iter() {
      let d = q.lock().unwrap().push(hop);
      if d > 0 {
        self.dropped.fetch_add(d, Relaxed);
      }
    }
  }

  fn count(&self) -> usize {
    self.subs.lock().unwrap().len()
  }
}

/// 网络统计（进调试接口 `/debug` 的 `net` 段）。
#[derive(Clone, Debug, serde::Serialize)]
pub struct NetStats {
  /// WebSocket 监听端口
  pub port: u16,
  /// 已连接客户端数
  pub clients: usize,
  /// 已订阅电脑音频的客户端数
  pub subs: usize,
  /// 手机 → 电脑：收到的 Opus 包数
  pub rx_packets: u64,
  /// 手机 → 电脑：解出的样本数
  pub rx_samples: u64,
  /// 手机 → 电脑：因超出 80 ms 上限丢弃的样本数
  pub rx_dropped: u64,
  /// 手机 → 电脑：入站缓冲水位（样本）
  pub rx_level: usize,
  /// 入站已取走的 hop 数（= 引擎消费）
  pub rx_hops: u64,
  /// 入站欠载次数（缓冲空，补静音）
  pub rx_underruns: u64,
  /// 电脑 → 手机：发出的 Opus 包数
  pub tx_packets: u64,
  /// 电脑 → 手机：出站队列水位合计（样本）
  pub tx_level: usize,
  /// 电脑 → 手机：因超出 80 ms 上限丢弃的样本数
  pub tx_dropped: u64,
  /// 已注入的按键事件数
  pub keys_injected: u64,
  /// 未映射而忽略的手机按键数
  pub keys_unmapped: u64,
  /// 已注入的文本字符数（UTF-16 码元）
  pub chars_typed: u64,
  /// 最近一次往返时延（ms）
  pub rtt_ms: Option<f64>,
  /// 远程输入开关（打字 + 全尺寸键盘）
  pub remote_input: bool,
  /// 最近一次错误原因
  pub last_error: Option<String>,
  /// 库状态：Opus 版本 + 加载到的 dll 路径
  pub codec: crate::debug::Probe<String>,
}

impl Default for NetStats {
  fn default() -> Self {
    NetStats {
      port: crate::net::proto::PORT,
      clients: 0,
      subs: 0,
      rx_packets: 0,
      rx_samples: 0,
      rx_dropped: 0,
      rx_level: 0,
      rx_hops: 0,
      rx_underruns: 0,
      tx_packets: 0,
      tx_level: 0,
      tx_dropped: 0,
      keys_injected: 0,
      keys_unmapped: 0,
      chars_typed: 0,
      rtt_ms: None,
      remote_input: false,
      last_error: None,
      codec: crate::debug::Probe::Pending,
    }
  }
}

/// 网络中枢：引擎与网络任务之间唯一的共享状态。
pub struct NetHub {
  /// 入站样本（手机 → 电脑），由 `remote_mic` 输入行消费
  inbound: Mutex<SampleQueue>,
  /// 出站 hop（电脑 → 手机）
  outbound: Outbound,
  /// 远程输入总开关（默认关，界面显式开启才接受 `text` / `key`）
  remote_input: AtomicBool,
  /// 统计计数（只有持锁者写，调试读时克隆）
  stats: Mutex<NetStats>,
  running: AtomicBool,
}

impl Default for NetHub {
  fn default() -> Self {
    NetHub::new()
  }
}

impl NetHub {
  pub fn new() -> Self {
    NetHub {
      inbound: Mutex::new(SampleQueue::default()),
      outbound: Outbound {
        subs: Mutex::new(Vec::new()),
        next_id: AtomicU64::new(1),
        dropped: AtomicU64::new(0),
      },
      remote_input: AtomicBool::new(false),
      stats: Mutex::new(NetStats::default()),
      running: AtomicBool::new(false),
    }
  }

  // ---- 生命周期 ----

  pub fn set_running(&self, v: bool) {
    self.running.store(v, Relaxed);
  }

  pub fn is_running(&self) -> bool {
    self.running.load(Relaxed)
  }

  // ---- 远程输入开关 ----

  /// 远程输入（打字 + 全尺寸键盘）总开关。**默认关**，界面一键开关。
  pub fn set_remote_input(&self, on: bool) {
    self.remote_input.store(on, Relaxed);
    self.stats.lock().unwrap().remote_input = on;
  }

  pub fn remote_input(&self) -> bool {
    self.remote_input.load(Relaxed)
  }

  // ---- 入站（手机 → 电脑） ----

  /// 网络任务：塞入解码后的样本。
  pub fn push_rx(&self, samples: &[f32]) {
    let dropped = self.inbound.lock().unwrap().push(samples);
    let mut s = self.stats.lock().unwrap();
    s.rx_packets += 1;
    s.rx_samples += samples.len() as u64;
    s.rx_dropped += dropped;
  }

  /// 列工作线程（`remote_mic` 行）：按 10 ms 节拍取一个 hop；欠载补静音并计数。
  pub fn take_rx_hop(&self, out: &mut [f32]) {
    let got = self.inbound.lock().unwrap().take_hop(out);
    let mut s = self.stats.lock().unwrap();
    if got {
      s.rx_hops += 1;
    } else {
      out.fill(0.0);
      s.rx_underruns += 1;
    }
    s.rx_level = self.inbound.lock().unwrap().level();
  }

  /// 入站缓冲水位（样本）。
  pub fn rx_level(&self) -> usize {
    self.inbound.lock().unwrap().level()
  }

  // ---- 出站（电脑 → 手机） ----

  /// 列工作线程（`remote_speaker` 行）：按位置 tap 推一个 hop。
  pub fn push_tx_hop(&self, hop: &[f32]) {
    self.outbound.push_hop(hop);
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

  /// 出站订阅的水位（样本）合计：>0 说明有客户端消费偏慢（界面与调试接口用）。
  pub fn tx_level(&self) -> usize {
    self
      .outbound
      .subs
      .lock()
      .unwrap()
      .iter()
      .map(|(_, q)| q.lock().unwrap().level())
      .sum()
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

  /// 调试快照用的统计克隆（同时带上 opus 库探测）。
  pub fn stats(&self) -> NetStats {
    let mut s = self.stats.lock().unwrap().clone();
    s.port = crate::net::proto::PORT;
    s.subs = self.sub_count();
    s.rx_level = self.rx_level();
    s.remote_input = self.remote_input();
    s.tx_dropped = self.outbound.dropped.load(Relaxed);
    s.tx_level = self.tx_level();
    s.codec = match super::opus_sys::probe() {
      Ok(v) => {
        let path = super::opus_sys::loaded_path().unwrap_or_default();
        crate::debug::Probe::ok(format!("{v}（{path}）"))
      }
      Err(e) => crate::debug::Probe::unavailable(e),
    };
    s
  }
}

#[cfg(test)]
mod tests {
  use super::*;

  #[test]
  fn inbound_queue_keeps_hop_alignment() {
    let mut q = SampleQueue::default();
    let mut hop = [0.0f32; HOP];
    // 不满一个 hop 时不返回
    q.push(&[1.0; 100]);
    assert!(!q.take_hop(&mut hop));
    // 补足后正好出一个 hop
    q.push(&[2.0; HOP - 100]);
    assert!(q.take_hop(&mut hop));
    // 前 100 个是 1.0（新追加的部分），其余 380 个是 2.0
    assert!(hop[..100].iter().all(|x| *x == 1.0), "旧样本应原样保留");
    assert!(hop[100..].iter().all(|x| *x == 2.0), "新样本应接在后面");
    assert!(!q.take_hop(&mut hop));
  }

  #[test]
  fn inbound_queue_is_bounded_and_drops_oldest() {
    let mut q = SampleQueue::default();
    let dropped = q.push(&vec![7.0; IN_CAP + 1000]);
    assert_eq!(dropped, 1000);
    assert_eq!(q.level(), IN_CAP);
    let mut hop = [0.0f32; HOP];
    assert!(q.take_hop(&mut hop));
    // 丢的是最旧的：剩下的全是新值
    assert!(hop.iter().all(|x| *x == 7.0));
  }

  #[test]
  fn outbound_queue_is_bounded_per_subscriber() {
    let hub = NetHub::new();
    let (_id, sub) = hub.subscribe_tx();
    let hop = [1.0f32; HOP];
    for _ in 0..(OUT_CAP_HOPS + 3) {
      hub.push_tx_hop(&hop);
    }
    let mut out = [0.0f32; HOP];
    for _ in 0..OUT_CAP_HOPS {
      assert!(sub.take_hop(&mut out), "应能连续取出封顶内的 hop");
    }
    assert!(!sub.take_hop(&mut out), "超出封顶的应被丢弃");
    assert!(hub.stats().tx_dropped > 0);
  }

  #[test]
  fn remote_input_defaults_off() {
    let hub = NetHub::new();
    assert!(!hub.remote_input(), "远程输入必须默认关闭");
    hub.set_remote_input(true);
    assert!(hub.remote_input());
    assert!(hub.stats().remote_input);
  }

  #[test]
  fn rx_hop_underrun_fills_silence() {
    let hub = NetHub::new();
    let mut hop = [9.0f32; HOP];
    hub.take_rx_hop(&mut hop);
    assert!(hop.iter().all(|x| *x == 0.0), "欠载应补静音");
    assert_eq!(hub.stats().rx_underruns, 1);
    hub.push_rx(&[0.5; HOP]);
    hub.take_rx_hop(&mut hop);
    assert!(hop.iter().all(|x| *x == 0.5));
  }
}
