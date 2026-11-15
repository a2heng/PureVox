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

//! 列工作线程的 CPU 亲和性（**一个模型一路核**）。
//!
//! 背景（2026-10-26 实测）：音频模型必须**单线程**跑（`infer::denoise` 已经把
//! intra/inter op 线程都钉成 1，否则默认线程池会忙等并把 10 ms 一跳拖到 >100 ms），
//! 于是每列的推理都压在**这一列的线程**上。多列 + 后台占用一起来时，Windows 调度器
//! 可能把两列的推理排在同一个核上互相打断，采集侧就表现为「环水位涨到硬顶 → 丢样本」。
//!
//! 做法：每起一列工作线程，就从 1 号核开始**轮转分配一个独占核**并固定上去
//! （0 号核留给系统 / 音频回调 / 界面线程），顺手把优先级提到 ABOVE_NORMAL。
//! 这不是实时隔离，但能保证「两列的模型不在同一个核上打架」。
//!
//! 列数 > 核数时轮转复用（多列共核，退回系统调度）。

use std::sync::atomic::{AtomicUsize, Ordering::Relaxed};

#[cfg(windows)]
#[path = "affinity_windows.rs"]
mod imp;

#[cfg(target_os = "linux")]
#[path = "affinity_linux.rs"]
mod imp;

#[cfg(not(any(windows, target_os = "linux")))]
#[path = "affinity_other.rs"]
mod imp;

/// 轮转游标（进程内单调递增；只用来挑核，不做业务判断）。
static NEXT: AtomicUsize = AtomicUsize::new(0);

/// 给新起的列线程挑一个核：从 1 开始轮转（0 号核留给系统 / 音频回调 / 界面）。
///
/// 只有 1 个逻辑核时返回 0（不隔离，也没有别的核可用）。
pub fn next_core() -> usize {
  let n = std::thread::available_parallelism()
    .map(|v| v.get())
    .unwrap_or(4)
    .max(1);
  if n <= 1 {
    return 0;
  }
  let slot = NEXT.fetch_add(1, Relaxed);
  1 + slot % (n - 1)
}

/// 把**当前线程**固定到 `core`，并把优先级提到 ABOVE_NORMAL（失败不致命：
/// 只是回到系统调度，返回原因由调用方记日志）。
pub fn pin_current(core: usize) -> Result<(), String> {
  imp::pin_current(core)
}

#[cfg(test)]
mod tests {
  use super::*;

  #[test]
  fn cores_rotate_without_zero() {
    let n = std::thread::available_parallelism()
      .map(|v| v.get())
      .unwrap_or(4)
      .max(1);
    if n <= 1 {
      assert_eq!(next_core(), 0);
      return;
    }
    let mut seen = Vec::new();
    for _ in 0..(n - 1) {
      seen.push(next_core());
    }
    assert!(
      seen.iter().all(|&c| c >= 1 && c < n),
      "分配到的核必须在 1..{n}：{seen:?}"
    );
    // 轮转一圈里每个核各一次
    seen.sort_unstable();
    seen.dedup();
    assert_eq!(seen.len(), n - 1, "一轮应该把 1..n 的核各分一次");
  }
}
