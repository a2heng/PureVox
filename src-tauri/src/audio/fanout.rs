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

//! 信号源扇出：一个源（输入采集 / 测试音）把 48 kHz hop 分发给多个订阅者（输出行 / 列）。
//!
//! **只在工作线程之间使用**（不在实时回调里），加锁可接受；但每条数据面纪律必须守住：
//! - **整 hop 进、整 hop 出**：环里放不下一个 hop 时**丢整个新 hop**并计数，
//!   绝不允许写半个 hop（那会让读端跨 hop 边界，和多路源永久错位）。
//! - **每订阅者可见**：水位 / 丢弃都进 `FanoutStat`，进 `/debug`（谁没在取数据一眼可查）。
//! - 读端被丢弃（输出流已停）的订阅者顺手清掉，避免 `Producer` 堆积。

use std::sync::Mutex;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};

use rtrb::{Consumer, Producer, RingBuffer};

use super::SAMPLE_RATE;

/// 每个订阅者的 48 kHz 环形缓冲容量：1 s（下游自己的封顶逻辑远小于此）。
const SUBSCRIBER_CAPACITY: usize = SAMPLE_RATE as usize;

#[derive(Default)]
struct SubStats {
  /// 因为环满而丢掉的样本数（整 hop 计；单列可见）
  dropped: AtomicU64,
  /// 成功写入的样本数
  pushed: AtomicU64,
}

/// 一个订阅者的即时状态（`/debug/devices` 的逐源/逐订阅者）。
#[derive(Clone, Debug, serde::Serialize)]
pub struct FanoutStat {
  pub id: String,
  /// 当前积压（ms）
  pub level_ms: f32,
  /// 因为环满而丢掉的样本数（累计；速率请由调用方按时间差分）
  pub dropped: u64,
  pub pushed: u64,
  /// 容量（ms）
  pub capacity_ms: f32,
}

struct Subscriber {
  id: String,
  prod: Producer<f32>,
  stats: std::sync::Arc<SubStats>,
}

pub struct Fanout {
  name: String,
  subscribers: Mutex<Vec<Subscriber>>,
  closed: AtomicBool,
}

impl Fanout {
  pub fn new(name: impl Into<String>) -> Self {
    Fanout {
      name: name.into(),
      subscribers: Mutex::new(Vec::new()),
      closed: AtomicBool::new(false),
    }
  }

  /// 源已停止（不会再有数据）。
  pub fn close(&self) {
    self.closed.store(true, Ordering::SeqCst);
  }

  pub fn is_closed(&self) -> bool {
    self.closed.load(Ordering::SeqCst)
  }

  /// 源的显示名（输出流调试信息里展示）。
  pub fn name(&self) -> &str {
    &self.name
  }

  /// 订阅：返回本订阅者的 48 kHz 读端。同一 id 重复订阅会替换旧的。
  pub fn subscribe(&self, id: &str) -> Consumer<f32> {
    let (prod, cons) = RingBuffer::<f32>::new(SUBSCRIBER_CAPACITY);
    let mut subs = self.subscribers.lock().unwrap_or_else(|e| e.into_inner());
    subs.retain(|s| s.id != id);
    subs.push(Subscriber {
      id: id.to_string(),
      prod,
      stats: std::sync::Arc::new(SubStats::default()),
    });
    cons
  }

  pub fn unsubscribe(&self, id: &str) {
    self
      .subscribers
      .lock()
      .unwrap_or_else(|e| e.into_inner())
      .retain(|s| s.id != id);
  }

  /// 分发一个 hop。**整 hop 要么全写、要么整跳丢弃**（环满 = 该订阅者没在取数据）。
  pub fn push_hop(&self, hop: &[f32]) {
    let mut subs = self.subscribers.lock().unwrap_or_else(|e| e.into_inner());
    subs.retain(|s| !s.prod.is_abandoned());
    for s in subs.iter_mut() {
      // 先判空间：rtrb 的 Producer::slots() = 还能写多少样本。放不下就丢**新** hop，
      // 不写半截（半个 hop 会让下游的 10 ms 网格错位）。
      if s.prod.slots() < hop.len() {
        s.stats
          .dropped
          .fetch_add(hop.len() as u64, Ordering::Relaxed);
        continue;
      }
      let mut written = 0usize;
      for &x in hop {
        if s.prod.push(x).is_err() {
          break;
        }
        written += 1;
      }
      s.stats.pushed.fetch_add(written as u64, Ordering::Relaxed);
      if written < hop.len() {
        // 理论上不会发生（slots 已判过），但并发下仍兜底：把剩下的计入丢弃。
        s.stats
          .dropped
          .fetch_add((hop.len() - written) as u64, Ordering::Relaxed);
      }
    }
  }

  /// 逐订阅者状态（水位 / 丢弃）。
  pub fn stats(&self) -> Vec<FanoutStat> {
    let subs = self.subscribers.lock().unwrap_or_else(|e| e.into_inner());
    let cap_ms = SUBSCRIBER_CAPACITY as f32 * 1000.0 / SAMPLE_RATE as f32;
    subs
      .iter()
      .map(|s| {
        let free = s.prod.slots();
        let level = SUBSCRIBER_CAPACITY.saturating_sub(free);
        FanoutStat {
          id: s.id.clone(),
          level_ms: level as f32 * 1000.0 / SAMPLE_RATE as f32,
          dropped: s.stats.dropped.load(Ordering::Relaxed),
          pushed: s.stats.pushed.load(Ordering::Relaxed),
          capacity_ms: cap_ms,
        }
      })
      .collect()
  }

  /// 当前最大积压（ms）——行概要里展示用。
  pub fn max_level_ms(&self) -> f32 {
    self
      .stats()
      .into_iter()
      .map(|s| s.level_ms)
      .fold(0.0, f32::max)
  }
}

#[cfg(test)]
mod tests {
  use super::*;
  use crate::audio::HOP;

  /// 环满时**丢整跳、不写半截**：读到的每个 hop 都必须完整来自同一批推送。
  #[test]
  fn full_ring_drops_whole_hop_not_half() {
    let fan = Fanout::new("test-fanout");
    let mut cons = fan.subscribe("s");
    let capacity_hops = SUBSCRIBER_CAPACITY / HOP;
    // 先塞满（值 1.0），再多推 5 个（值 2.0）——这些必须被整跳丢弃
    for _ in 0..capacity_hops {
      fan.push_hop(&[1.0; HOP]);
    }
    for _ in 0..5 {
      fan.push_hop(&[2.0; HOP]);
    }
    let st = fan.stats();
    assert_eq!(st.len(), 1);
    assert_eq!(
      st[0].dropped,
      (5 * HOP) as u64,
      "满环后的 5 个 hop 必须整跳计入丢弃"
    );
    // 读出来的每个 hop 都只能是 1.0；出现任何 2.0 就说明写了半截/新数据混入
    for _ in 0..capacity_hops {
      let mut hop = [0.0f32; HOP];
      for x in hop.iter_mut() {
        *x = cons.pop().expect("应当读满容量内的 hop");
      }
      assert!(
        hop.iter().all(|x| *x == 1.0),
        "读到了被丢弃或半截的 hop：{:?}",
        &hop[..4]
      );
    }
  }

  #[test]
  fn unsubscribed_reader_is_pruned() {
    let fan = Fanout::new("test-unsub");
    let cons = fan.subscribe("gone");
    assert_eq!(fan.stats().len(), 1);
    drop(cons);
    fan.push_hop(&[0.0; HOP]);
    assert_eq!(fan.stats().len(), 0, "读端丢弃后应自动清掉");
  }
}
