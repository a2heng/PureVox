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

//! 关键指标的短历史（`/debug/metrics`）：回答「刚才那次咔哒是哪一层」。
//!
//! 快照只有**现在**：一次欠载发生在 3 秒前，等人打开面板时计数已经涨上去了、速率也回到 0。
//! 这里按固定窗口保留每一条序列的采样（默认 120 s），面板/脚本能回看波形。
//!
//! 采样频率 1 Hz（由 `debug::system` 的采样线程驱动），每条序列有硬上限；
//! 只存 `f64`，不存字符串/结构体，避免把调试历史变成第二个状态源。

use std::collections::{BTreeMap, VecDeque};
use std::sync::{Mutex, OnceLock};

use super::now_ms;

/// 历史窗口：2 分钟。
pub const WINDOW_MS: u64 = 120_000;
/// 单条序列最多存这么多个点（5 Hz × 120 s，留足余量）。
const MAX_POINTS: usize = 1200;

#[derive(Default)]
struct Series {
  points: VecDeque<(u64, f64)>,
}

fn store() -> &'static Mutex<BTreeMap<String, Series>> {
  static STORE: OnceLock<Mutex<BTreeMap<String, Series>>> = OnceLock::new();
  STORE.get_or_init(|| Mutex::new(BTreeMap::new()))
}

/// 记录一个采样点。`name` 是稳定标识（`input:xxx.underrun_per_s`），不要放易变的显示名。
pub fn record(name: impl Into<String>, value: f64) {
  record_at(name, now_ms(), value);
}

pub fn record_at(name: impl Into<String>, ts: u64, value: f64) {
  if !value.is_finite() {
    return;
  }
  let Ok(mut g) = store().lock() else {
    return;
  };
  let s = g.entry(name.into()).or_default();
  s.points.push_back((ts, value));
  while s.points.len() > MAX_POINTS {
    s.points.pop_front();
  }
  while let Some((t, _)) = s.points.front() {
    if ts.saturating_sub(*t) <= WINDOW_MS {
      break;
    }
    s.points.pop_front();
  }
}

/// 一条序列的只读视图（`(时间戳 ms, 值)`，按时间正序）。
#[derive(Clone, Debug, serde::Serialize)]
pub struct MetricsInfo {
  pub window_ms: u64,
  pub ts: u64,
  /// 序列名 → 采样点。名字是稳定标识，UI 自己挑要画的。
  pub series: BTreeMap<String, Vec<(u64, f64)>>,
}

pub fn snapshot() -> MetricsInfo {
  let now = now_ms();
  let mut series = BTreeMap::new();
  if let Ok(g) = store().lock() {
    for (name, s) in g.iter() {
      let points: Vec<(u64, f64)> = s
        .points
        .iter()
        .filter(|(t, _)| now.saturating_sub(*t) <= WINDOW_MS)
        .copied()
        .collect();
      if !points.is_empty() {
        series.insert(name.clone(), points);
      }
    }
  }
  MetricsInfo {
    window_ms: WINDOW_MS,
    ts: now,
    series,
  }
}

#[cfg(test)]
mod tests {
  use super::*;

  #[test]
  fn series_is_time_windowed_and_bounded() {
    // 用一个独立的序列名，避免与其他测试互相污染
    let name = "unit-test.series";
    let base = crate::debug::now_ms();
    for i in 0..(MAX_POINTS + 100) {
      record_at(name, base + i as u64, i as f64);
    }
    let m = snapshot();
    let pts = m.series.get(name).expect("序列应存在");
    assert!(pts.len() <= MAX_POINTS, "单条序列必须有上限");
    assert_eq!(pts.last().unwrap().1, (MAX_POINTS + 99) as f64);
  }

  #[test]
  fn non_finite_values_are_rejected() {
    let name = "unit-test.nan";
    record_at(name, 10, f64::NAN);
    record_at(name, 11, f64::INFINITY);
    let m = snapshot();
    assert!(!m.series.contains_key(name), "NaN/Inf 不该进历史");
  }
}
