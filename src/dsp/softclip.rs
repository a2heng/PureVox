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

//! 前置增益之后的**软限幅**（soft clip）：任何一次处理/输出之前，样本都不许超出 ±1。
//!
//! 为什么不是 `clamp(-1, 1)`：硬削波在 0 dBFS 处产生一个直角，瞬态（敲桌子 / 鼠标 /
//! 键盘）会炸出一片高次谐波（听感是「啪」而不是「闷」），而这些瞬态正是降噪要处理的
//! 对象。这里用一条**有理函数软拐点**：
//!
//! ```text
//! |x| ≤ KNEE : y = x                                  （完全透明）
//! |x| > KNEE : y = K + (|x|-K)(1-K) / ((1-K) + (|x|-K))，带原符号
//! ```
//!
//! - 在 `|x| = K` 处一阶连续（斜率都是 1），没有折点；
//! - `|x| → ∞` 时 `|y| → 1`，永远不超过 0 dBFS；
//! - 每样本只有一次比较；超过拐点才做一次除法（没有 `tanh` / `exp`，实时线程随便跑）。
//!
//! 代价：0 dBFS 的正弦会落到约 −0.22 dB（`K = 0.95` 的软拐点），人耳不可闻；
//! 正常语音（约 −20 dBFS）连分支都不会进。
//!
//! 非有限值（NaN / Inf）一律当 0：它们不是「超出 1」，而是坏数据，不能让它污染模型缓存。

/// 软拐点：`|x| ≤ KNEE` 完全透明。
pub const KNEE: f32 = 0.95;

/// 单个样本软限幅到 `[-1, 1]`。
#[inline]
pub fn soft_clip(x: f32) -> f32 {
  if !x.is_finite() {
    return 0.0;
  }
  let a = x.abs();
  if a <= KNEE {
    return x;
  }
  let over = a - KNEE;
  let span = 1.0 - KNEE;
  let y = KNEE + over * span / (span + over);
  if x < 0.0 { -y } else { y }
}

/// 整段就地软限幅（hop / 设备块通用）。
#[inline]
pub fn soft_clip_in_place(buf: &mut [f32]) {
  for x in buf.iter_mut() {
    *x = soft_clip(*x);
  }
}

#[cfg(test)]
mod tests {
  use super::*;

  #[test]
  fn transparent_below_knee() {
    for x in [-KNEE, -0.5, -0.01, 0.0, 0.01, 0.5, KNEE] {
      assert_eq!(soft_clip(x), x, "{x} 在拐点以内必须逐位不变");
    }
  }

  #[test]
  fn never_exceeds_full_scale() {
    for x in [1.0f32, 1.2, 2.0, 10.0, 1e6, f32::MAX] {
      let y = soft_clip(x);
      assert!((0.0..=1.0).contains(&y), "{x} → {y} 超出 [-1,1]");
      let ny = soft_clip(-x);
      assert_eq!(ny, -y, "软限幅必须奇对称");
    }
  }

  #[test]
  fn continuous_at_knee() {
    // 拐点两侧极限相同（一阶连续的必要条件）
    let a = soft_clip(KNEE);
    let b = soft_clip(KNEE + 1e-6);
    assert!((a - b).abs() < 2e-6, "拐点处跳变：{a} vs {b}");
  }

  #[test]
  fn non_finite_becomes_silence() {
    assert_eq!(soft_clip(f32::NAN), 0.0);
    assert_eq!(soft_clip(f32::INFINITY), 0.0);
    assert_eq!(soft_clip(f32::NEG_INFINITY), 0.0);
  }

  #[test]
  fn monotonic() {
    let mut prev = -1.2f32;
    let mut y_prev = soft_clip(prev);
    while prev < 10.0 {
      prev += 0.01;
      let y = soft_clip(prev);
      assert!(y >= y_prev, "软限幅必须单调：{prev} → {y} < {y_prev}");
      y_prev = y;
    }
  }
}
