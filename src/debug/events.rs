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

//! 进程内事件日志（调试接口的唯一日志源）。
//!
//! 迁移期最重要的一条是「看不见的状态等于不存在的状态」：release 构建没有控制台，
//! 后台线程的失败如果只 `eprintln!` 就等于没有。这里把所有值得知道的事件收进一个有界
//! 环形缓冲，界面经 `/debug` 的 `events`、脚本经 `/debug/logs` 都能读到。
//!
//! 纪律：
//! - **有界**：最多 [`CAPACITY`] 条，满了丢最旧；不做磁盘写入（写盘会阻塞调用者）。
//! - **不在音频线程做重活**：只是取锁 + 推一条；真正的音频路径仍然只写原子计数。
//! - **panic 也进这里**：`install_panic_hook()` 在 `main` 里调用，线程名/位置/消息都留下。

use std::collections::VecDeque;
use std::sync::{Mutex, OnceLock};

use super::now_ms;

/// 环形缓冲容量。足够覆盖一次「点几下按钮、复现一次故障」的窗口。
pub const CAPACITY: usize = 4096;

#[derive(Clone, Debug, serde::Serialize)]
pub struct Event {
  /// 毫秒时间戳（与快照同一个时钟）
  pub ts: u64,
  /// `debug` / `info` / `warn` / `error`
  pub level: &'static str,
  /// 模块名（`engine` / `capture` / `mixer` / `device-pool` …）
  pub module: String,
  pub message: String,
}

fn ring() -> &'static Mutex<VecDeque<Event>> {
  static RING: OnceLock<Mutex<VecDeque<Event>>> = OnceLock::new();
  RING.get_or_init(|| Mutex::new(VecDeque::with_capacity(CAPACITY)))
}

/// 追加一条事件。任何线程都可调用；锁中毒时仍然继续（日志不能反过来拖垮程序）。
pub fn log(level: &'static str, module: &str, message: impl Into<String>) {
  let event = Event {
    ts: now_ms(),
    level,
    module: module.to_string(),
    message: message.into(),
  };
  if let Ok(mut r) = ring().lock() {
    if r.len() >= CAPACITY {
      r.pop_front();
    }
    r.push_back(event);
  }
}

/// 最近 `limit` 条（按时间正序，最新在最后）。
pub fn recent(limit: usize) -> Vec<Event> {
  let Ok(r) = ring().lock() else {
    return Vec::new();
  };
  let start = r.len().saturating_sub(limit);
  r.iter().skip(start).cloned().collect()
}

/// 安装 panic hook：把线程名、位置、消息写进事件日志，再交给原来的 hook。
///
/// **只装一次**；重复调用是无害的（后装的会覆盖，但 `main` 只调一次）。
pub fn install_panic_hook() {
  let previous = std::panic::take_hook();
  std::panic::set_hook(Box::new(move |info| {
    let thread = std::thread::current()
      .name()
      .unwrap_or("<unnamed>")
      .to_string();
    let location = info
      .location()
      .map(|l| format!("{}:{}", l.file(), l.line()))
      .unwrap_or_else(|| "<unknown>".to_string());
    let payload = info
      .payload()
      .downcast_ref::<&str>()
      .map(|s| (*s).to_string())
      .or_else(|| info.payload().downcast_ref::<String>().cloned())
      .unwrap_or_else(|| "<non-string panic payload>".to_string());
    log(
      "error",
      "panic",
      format!("线程 {thread} 在 {location} panic：{payload}"),
    );
    previous(info);
  }));
}

#[cfg(test)]
mod tests {
  use super::*;

  #[test]
  fn event_ring_is_bounded_and_ordered() {
    for i in 0..(CAPACITY + 17) {
      log("info", "test", format!("第 {i} 条"));
    }
    let all = recent(CAPACITY + 100);
    assert_eq!(all.len(), CAPACITY, "环形缓冲必须有上限");
    assert!(
      all.last().unwrap().message.contains("第 4112 条"),
      "最新的在最后：{:?}",
      all.last()
    );
    assert!(
      !all.iter().any(|e| e.message.contains("第 0 条")),
      "最旧的应当被丢掉"
    );
  }

  #[test]
  fn recent_limit_returns_newest_tail() {
    // 环形缓冲是进程级共享的，其他测试可能并发写；这里只断言相对顺序，
    // 不断言「最后两条一定是它们」（以前那样在并行测试下会偶发失败）。
    log("info", "test-limit", "unit-a");
    log("warn", "test-limit", "unit-b");
    let v = recent(CAPACITY);
    let ia = v
      .iter()
      .position(|e| e.message == "unit-a")
      .expect("a 应存在");
    let ib = v
      .iter()
      .position(|e| e.message == "unit-b")
      .expect("b 应存在");
    assert!(ia < ib, "事件必须保持写入顺序");
    assert_eq!(v[ib].level, "warn");
  }
}
