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

//! PI 时钟伺服：**水位（ms）→ 重采样比例修正量**。全项目唯一实现。
//!
//! 两处被控对象不同、控制律相同：
//! - `audio/playback.rs`：被控对象是设备输出的 48k 源环（设定点 40 ms，限幅 ±3%）；
//! - `net/rt.rs`：被控对象是网络收发缓冲（设定点由自适应 jitter 估计给，限幅 ±2%）。
//!
//! **为什么要有它**：两条链上「生产」与「消费」是两套时钟（设备晶振 vs 系统定时器、
//! 手机晶振 vs 电脑定时器），标称都是 48 kHz 但实际差几十到几百 ppm。两者速率不等时
//! 水位是**单向漂移**的随机游走：漂出去以后只能靠丢样本 / 补静音收拾，听感就是周期性
//! 咔哒或短暂静音。用重采样比例把消耗速率连续调到和生产速率一致，才能**稳态一点不丢**。
//!
//! 调参：二阶闭环，误差单位 ms，输出是「比例修正量」的无量纲小量。
//! 增益取 (KP, KI) = (2e-4, 2e-5) 时闭环时间常数约 5 s、阻尼约 0.7（见 playback 的实测），
//! 也就是说 ppm 级漂移在几秒内被吸收，而不会像快积分那样把比例调得听得出来。

/// 默认增益 / 限幅 / EMA 时间常数（设备输出链路的口径，网络链路另行 tuned）。
pub const DEFAULT_KP: f64 = 2.0e-4;
pub const DEFAULT_KI: f64 = 2.0e-5;
pub const DEFAULT_LIMIT: f64 = 0.03;
pub const DEFAULT_TAU_S: f64 = 1.0;

/// PI 伺服：设定点 + 水位 EMA + 积分，输出 `adj`（正 = **消耗更快**，即水位在涨时多消耗）。
#[derive(Debug, Clone)]
pub struct Servo {
  target_ms: f64,
  ema_ms: f64,
  integral: f64,
  adj: f64,
  kp: f64,
  ki: f64,
  limit: f64,
  tau_s: f64,
}

impl Servo {
  /// 用设备输出链路的默认增益建一个（保持 `audio/playback.rs` 的历史行为不变）。
  pub fn new(target_ms: f64) -> Self {
    Self::tuned(
      target_ms,
      DEFAULT_KP,
      DEFAULT_KI,
      DEFAULT_LIMIT,
      DEFAULT_TAU_S,
    )
  }

  /// 显式给定全部参数（网络链路：目标会随 jitter 估计变化，增益另行调）。
  pub fn tuned(target_ms: f64, kp: f64, ki: f64, limit: f64, tau_s: f64) -> Self {
    Servo {
      target_ms,
      ema_ms: target_ms,
      integral: 0.0,
      adj: 0.0,
      kp,
      ki,
      limit,
      tau_s,
    }
  }

  /// 预热完成 / 重同步之后，把 EMA 拉回设定点（保留积分）。
  pub fn reset_level(&mut self) {
    self.ema_ms = self.target_ms;
  }

  /// 每个控制周期调一次。`level_ms` = 当前水位，`dt_s` = 距上次调用的秒数。
  pub fn update(&mut self, level_ms: f64, dt_s: f64) {
    if dt_s <= 0.0 || !level_ms.is_finite() {
      return;
    }
    let a = (dt_s / self.tau_s).clamp(0.0, 1.0);
    self.ema_ms += a * (level_ms - self.ema_ms);
    let e = self.ema_ms - self.target_ms;
    self.integral += e * dt_s;
    // 抗饱和：积分项单独不超过限幅（否则大幅超调后要很久才回来）
    let i_lim = self.limit / self.ki.max(1e-12);
    self.integral = self.integral.clamp(-i_lim, i_lim);
    self.adj = (self.kp * e + self.ki * self.integral).clamp(-self.limit, self.limit);
  }

  /// 比例修正量（正 = 消耗更快）。稳态时它就是两套时钟的 ppm 差。
  pub fn adj(&self) -> f64 {
    self.adj
  }
}

#[cfg(test)]
mod tests {
  use super::*;

  /// 水位恒定高于目标 → 一直要求「消耗更快」，且被限幅夹住。
  #[test]
  fn pushes_toward_target_when_high() {
    let mut s = Servo::tuned(30.0, DEFAULT_KP, DEFAULT_KI, 0.02, 1.0);
    for _ in 0..500 {
      s.update(60.0, 0.01);
    }
    assert!(s.adj() > 0.0, "水位偏高应加速消耗：{}", s.adj());
    assert!(s.adj() <= 0.02 + 1e-12, "必须限幅：{}", s.adj());
  }

  /// 水位正好在目标上 → 修正量收敛到 0（不无端变速）。
  #[test]
  fn settles_at_target() {
    let mut s = Servo::new(40.0);
    for _ in 0..2000 {
      s.update(40.0, 0.01);
    }
    assert!(s.adj().abs() < 1e-9, "稳态不该有修正：{}", s.adj());
  }
}
