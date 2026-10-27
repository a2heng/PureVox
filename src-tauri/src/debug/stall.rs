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

//! 「整个进程被冻住」的观测与对策。
//!
//! ## 为什么需要它
//!
//! 实测（2026-10-27）：音频链路本身已经稳态（发包 100.00/s、无欠载），但每隔 **几秒到十几秒**
//! 会出现一次 ~50 ms 的停顿 —— 同一时刻列线程的推送间隔与发送任务的写出间隔都出现同样量级的
//! 空档（`src_max_gap_ms` 与 `tx_max_gap_ms` 数值接近），然后补发两包。听感就是「1 kHz
//! 偶尔不连贯」。这种「多个线程**同时**停住」的形状不像是某一个锁，更像**进程被整体降频/挂起**：
//!
//! - Windows 会把「后台进程」判为 EcoQoS（效率模式），调度优先级被压，连定时器都更粗；
//! - 应用在托盘里（`start_hidden`）时尤其容易被这样对待 —— 而放音频正是我们最主要的使用场景。
//!
//! 对策：[`opt_out_of_throttling`] 显式声明本进程需要满速执行（媒体类应用的标准做法），
//! 再加一个 [`spawn_watchdog`] 每 10 ms 量一次「实际睡了多久 vs 应该睡多久」，
//! 把冻住的幅度与次数写进 `/debug/system` —— 修没修好不靠猜。

use std::sync::atomic::{AtomicU64, Ordering::Relaxed};
use std::sync::Arc;
use std::time::{Duration, Instant};

#[cfg(windows)]
#[path = "stall_windows.rs"]
mod imp;

#[cfg(not(windows))]
#[path = "stall_other.rs"]
mod imp;

/// 看门狗周期。10 ms 与音频网格同量级，够看清 20~50 ms 的冻住，又不会自己抢 CPU。
const WATCH_PERIOD: Duration = Duration::from_millis(10);

/// 超过这个量才算「冻住」（正常调度抖动在 1~3 ms 量级）。
const STALL_MS: f64 = 15.0;

/// 观测结果（只读，`/debug/system` 里序列化）。
#[derive(Default)]
pub struct StallStat {
  /// 见过的最坏超时（ms）
  pub worst_ms: AtomicU64,
  /// 超时次数（累计；速率看 `/debug` 的两次采样差）
  pub count: AtomicU64,
  /// 最近一次超时（ms）
  pub last_ms: AtomicU64,
}

impl StallStat {
  fn note(&self, ms: f64) {
    // f64 → u64（毫秒的千分之一），只为无锁存一个数；读出来再除回去
    let v = (ms * 1000.0) as u64;
    self
      .worst_ms
      .fetch_max(v, Relaxed);
    if ms >= STALL_MS {
      self.count.fetch_add(1, Relaxed);
      self.last_ms.store(v, Relaxed);
    }
  }

  pub fn worst(&self) -> f64 {
    self.worst_ms.load(Relaxed) as f64 / 1000.0
  }

  pub fn last(&self) -> f64 {
    self.last_ms.load(Relaxed) as f64 / 1000.0
  }

  pub fn count(&self) -> u64 {
    self.count.load(Relaxed)
  }
}

/// 全局唯一的一份读数（`/debug/system` 与看门狗共用）。
static STAT: std::sync::OnceLock<Arc<StallStat>> = std::sync::OnceLock::new();

pub fn stat() -> &'static Arc<StallStat> {
  STAT.get_or_init(|| Arc::new(StallStat::default()))
}

/// 起看门狗（用全局读数那份）。
pub fn spawn_watchdog_default() {
  spawn_watchdog(stat().clone())
}

/// 让本进程**不被电源节流**（EcoQoS）。失败返回人话原因，调用方写进事件日志。
pub fn opt_out_of_throttling() -> Result<(), String> {
  imp::opt_out_of_throttling()
}

/// 起一个看门狗线程，把「整个进程被冻住」的幅度记进 `stat`。
///
/// 它量的是**本线程**的 `sleep(10ms)` 超时：本线程没有任何业务、不拿任何锁，
/// 所以它看到的超时只可能来自「进程/系统层面」的停顿 —— 这正是要分清的那件事：
/// 音频线程停顿 == 看门狗也有同样停顿 → 不是我们的代码卡住，是调度/电源问题。
pub fn spawn_watchdog(stat: Arc<StallStat>) {
  std::thread::Builder::new()
    .name("stall-watch".into())
    .spawn(move || {
      let mut next = Instant::now() + WATCH_PERIOD;
      loop {
        let now = Instant::now();
        if next > now {
          std::thread::sleep(next - now);
        }
        let now = Instant::now();
        let woke = now.saturating_duration_since(next);
        stat.note(woke.as_secs_f64() * 1000.0);
        // 落后太多就重新对齐（长时间挂起后不补发，同 `audio/tick.rs` 的道理）
        next = if woke > WATCH_PERIOD * 3 {
          now + WATCH_PERIOD
        } else {
          next + WATCH_PERIOD
        };
      }
    })
    .map_err(|e| eprintln!("PureVox: 看门狗线程起不来：{e}"))
    .ok();
}
