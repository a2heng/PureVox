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

//! Linux：本进程私有字节（`/proc/self/smaps_rollup` 的 `Private_*` 之和）。
//!
//! 口径对应 Windows 的 `PrivateUsage`：**只算本进程独占、不与别人共享的页**。
//! 差别说清楚（不假装等价）：Windows 统计的是「已提交的私有字节」，含已换出部分；
//! Linux 的 `smaps_rollup` 只统计**常驻**页，所以换出后这个数会变小。
//! `Shared_*` 一律不计 —— 那部分内存有别人一起用，算进来会误导「本进程吃了多少」。
//! 内核 ≥ 4.14 才有 `smaps_rollup`（26.04 内核 7.x 自然有）；读不到就明确报不可用。

use super::Probe;

/// 计入的键：干净私有页 + 脏私有页 + 大页（若有）。单位都是 kB。
const KEYS: [&str; 3] = ["Private_Clean:", "Private_Dirty:", "Private_Hugetlb:"];

pub(super) fn private_bytes() -> Probe<u64> {
  let text = match std::fs::read_to_string("/proc/self/smaps_rollup") {
    Ok(t) => t,
    Err(e) => return Probe::unavailable(format!("读取 /proc/self/smaps_rollup 失败：{e}")),
  };
  let mut total_kb: u64 = 0;
  let mut hit = false;
  for line in text.lines() {
    let Some((key, rest)) = line.split_once(':') else {
      continue;
    };
    let key = format!("{key}:");
    if !KEYS.contains(&key.as_str()) {
      continue;
    }
    hit = true;
    if let Ok(kb) = rest.trim().trim_end_matches("kB").trim().parse::<u64>() {
      total_kb += kb;
    }
  }
  if !hit {
    return Probe::unavailable("smaps_rollup 里没有 Private_* 字段（内核过旧？）");
  }
  Probe::ok(total_kb * 1024)
}
