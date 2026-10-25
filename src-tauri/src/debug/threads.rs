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

//! 工作线程存活登记（调试接口的「谁还活着」）。
//!
//! 以前线程 panic 后只是悄悄消失：快照里那条流的最后一个 `state="running"` 会一直留着，
//! 用户看到的是「界面没报错、但没有声音」。每个长跑线程现在都拿一个 [`ThreadToken`]：
//! 心跳是**原子写**（不加锁、不进音频回调），退出（含 panic）由 `Drop` 标记。
//!
//! 快照里的 `alive` 由「最近一次心跳 + 是否已退出」共同判定；阈值 [`STALE_MS`] 只在这里。

use std::sync::atomic::{AtomicBool, AtomicU64, Ordering::Relaxed};
use std::sync::{Arc, Mutex, OnceLock};

use super::now_ms;

/// 超过这么久没有心跳即视为「卡住/已死」。
pub const STALE_MS: u64 = 3_000;
/// 登记表上限：只保留最近这么多线程（退出的旧线程会被挤掉）。
const MAX_THREADS: usize = 64;

#[derive(Clone, Debug, serde::Serialize)]
pub struct ThreadInfo {
  pub name: String,
  pub started_at: u64,
  pub last_beat_ms: u64,
  pub beats: u64,
  pub exited: bool,
  /// 距离最近一次心跳的毫秒数（快照生成时计算）
  pub age_ms: u64,
  /// `exited=false && age_ms <= STALE_MS`
  pub alive: bool,
}

struct Entry {
  name: String,
  started_at: u64,
  last: Arc<AtomicU64>,
  beats: Arc<AtomicU64>,
  exited: Arc<AtomicBool>,
}

fn registry() -> &'static Mutex<Vec<Entry>> {
  static REG: OnceLock<Mutex<Vec<Entry>>> = OnceLock::new();
  REG.get_or_init(|| Mutex::new(Vec::new()))
}

/// 一个线程的心跳句柄。`Drop` 即标记退出（panic 展开也会走到）。
pub struct ThreadToken {
  last: Arc<AtomicU64>,
  beats: Arc<AtomicU64>,
  exited: Arc<AtomicBool>,
}

impl ThreadToken {
  /// 登记一个线程。同名线程会各自占一条（例如两列的同名工作线程带不同后缀）。
  pub fn new(name: impl Into<String>) -> Self {
    let name = name.into();
    let last = Arc::new(AtomicU64::new(now_ms()));
    let beats = Arc::new(AtomicU64::new(0));
    let exited = Arc::new(AtomicBool::new(false));
    if let Ok(mut reg) = registry().lock() {
      // 先淘汰已退出且最旧的条目，避免长期运行后无限增长
      if reg.len() >= MAX_THREADS {
        reg.retain(|e| !e.exited.load(Relaxed));
        if reg.len() >= MAX_THREADS {
          reg.remove(0);
        }
      }
      reg.push(Entry {
        name: name.clone(),
        started_at: now_ms(),
        last: last.clone(),
        beats: beats.clone(),
        exited: exited.clone(),
      });
    }
    ThreadToken {
      last,
      beats,
      exited,
    }
  }

  /// 心跳：一次原子写 + 一次原子加。可以在 10 ms 循环里每拍调用。
  #[inline]
  pub fn beat(&self) {
    self.last.store(now_ms(), Relaxed);
    self.beats.fetch_add(1, Relaxed);
  }
}

impl Drop for ThreadToken {
  fn drop(&mut self) {
    self.exited.store(true, Relaxed);
    self.last.store(now_ms(), Relaxed);
  }
}

/// 当前登记表快照（按登记顺序；退出的也保留到下一次淘汰）。
pub fn snapshot() -> Vec<ThreadInfo> {
  let now = now_ms();
  let Ok(reg) = registry().lock() else {
    return Vec::new();
  };
  reg
    .iter()
    .map(|e| {
      let last = e.last.load(Relaxed);
      let exited = e.exited.load(Relaxed);
      let age = now.saturating_sub(last);
      ThreadInfo {
        name: e.name.clone(),
        started_at: e.started_at,
        last_beat_ms: last,
        beats: e.beats.load(Relaxed),
        exited,
        age_ms: age,
        alive: !exited && age <= STALE_MS,
      }
    })
    .collect()
}

#[cfg(test)]
mod tests {
  use super::*;

  #[test]
  fn live_thread_is_alive_and_exit_is_visible() {
    let t = ThreadToken::new("unit-test-thread");
    t.beat();
    let info = snapshot()
      .into_iter()
      .find(|i| i.name == "unit-test-thread")
      .expect("登记表里应有该线程");
    assert!(info.alive, "刚 beat 过的线程应当是活的");
    assert!(info.beats >= 1);

    drop(t);
    let info = snapshot()
      .into_iter()
      .find(|i| i.name == "unit-test-thread")
      .expect("退出后条目仍应保留到淘汰");
    assert!(info.exited, "Drop 必须标记退出");
    assert!(!info.alive, "退出后不能算活着");
  }
}
