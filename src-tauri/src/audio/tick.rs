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

//! 10 ms 节拍：给**合成信号源**（测试音，以及将来别的生成器）用的实时钟。
//!
//! ## 为什么合成源需要定时器，而真实源不需要
//!
//! 整条链路是「生产者 - 消费者」没错，但**节拍必须来自某个真实时钟**：
//! 真实输入（麦克风 / 回环）由设备回调驱动，回调本身就是 10 ms 网格；输出由设备回调
//! 拉动。测试音没有设备，它自己就是起点的生产者 —— 没有定时器它只能全速空转。
//!
//! 出站网络**没有**定时器（引擎每出一 hop 立刻发一包，`hub::OutboundSub`），
//! 这条与那边无关：这里造的是「源的一拍」，不是「发送的节奏」。
//!
//! ## 为什么不能用 `thread::sleep(10ms)`
//!
//! Windows 默认时钟刻度是 **15.6 ms**，`sleep(10 ms)` 会被量化成 15.6 ms：
//! 于是平均虽然还是 100 拍/s，但**实际是一拍 15.6 ms、时不时一拍补两格**（突发）。
//! 接收端只有一个 20 ms 的环，突发两格立刻冲爆（丢一格 = 咔），随后又饿死（补 PLC = 糊）
//! —— 实测 1 kHz 听感就是「不连续」，`/debug/audio` 里 `late` 32/s、欠载 5/s。
//! 用 **高精度可等待定时器**（`CREATE_WAITABLE_TIMER_HIGH_RESOLUTION`，Win10 1803+）
//! 可以拿到 ~1 ms 精度且**不改全局时钟分辨率**（不是 `timeBeginPeriod` 那条路：
//! 那是进程级副作用，别为了一个测试音把整机定时器改掉）。
//!
//! 落后时的策略：**不补发**。落后超过两个周期（被挂起、调试器停顿）就从此刻重新对齐；
//! 落后一点点只发一格，下一个网格点照原计划 —— 绝不为了「追时间」连发两格。

use std::time::{Duration, Instant};

#[cfg(windows)]
#[path = "tick_windows.rs"]
mod imp;
#[cfg(not(windows))]
#[path = "tick_other.rs"]
mod imp;

/// 读**系统时钟中断粒度**（ms），给 `/debug/system` 的 `timer_resolution_ms`。
///
/// Windows 用 `GetSystemTimeAdjustment`：`TimeIncrement` 是每次时钟中断的 100 ns 数
/// （默认 156250 = **15.625 ms**）。这正是 `Sleep()` / `std::thread::sleep` 精度的上限 ——
/// 想一秒确认可以用 Sysinternals 的 `ClockRes`，读的是同一个值。
#[cfg(windows)]
pub fn system_timer_granularity_ms() -> Option<f64> {
  use windows::Win32::System::SystemInformation::GetSystemTimeAdjustment;
  let mut adj: u32 = 0;
  let mut inc: u32 = 0;
  let mut disabled = windows::core::BOOL(0);
  // SAFETY: 三个都是指向本栈变量的有效指针，函数只写不读。
  let ok = unsafe { GetSystemTimeAdjustment(&mut adj, &mut inc, &mut disabled) };
  if ok.is_err() || inc == 0 {
    return None;
  }
  Some(inc as f64 / 10_000.0)
}

#[cfg(not(windows))]
pub fn system_timer_granularity_ms() -> Option<f64> {
  // Linux 的 `nanosleep` 精度是微秒级，不存在 Windows 那种时钟节拍限制
  None
}

/// 上面那个读数的 Probe 包装（失败给明确原因，不拿 0 冒充）。
pub fn timer_resolution_probe() -> crate::debug::Probe<f64> {
  match system_timer_granularity_ms() {
    Some(ms) => crate::debug::Probe::ok(ms),
    None => crate::debug::Probe::unavailable("本平台没有系统时钟节拍这一说（或读取失败）"),
  }
}

/// 固定周期节拍器（单调时钟 + 高精度等待）。
pub struct Tick {
  period: Duration,
  next: Instant,
  waiter: imp::Waiter,
}

impl Tick {
  pub fn new(period: Duration) -> Self {
    Self {
      period,
      next: Instant::now() + period,
      waiter: imp::Waiter::new(),
    }
  }

  /// 等到下一个网格点。返回本次实际等了多久（诊断用）。
  ///
  /// 注意：**每次调用只放行一拍**。晚到的调用不会因为「欠了几拍」而连发。
  pub fn wait(&mut self) -> Duration {
    let t0 = Instant::now();
    let now = Instant::now();
    if now < self.next {
      self.waiter.wait(self.next - now);
      self.next += self.period;
    } else {
      // **已经晚了**（线程被调度拖住 / 上一拍超时）：从此刻重新起算。
      //
      // 这里**绝不能** `next += period` 去「追平」：那会让下一拍几乎立刻返回，
      // 两拍连发（实测间隔 0.001 ms），然后空 20 ms —— 这就是「1 kHz 不像一条直线」
      // 的直接来源。宁可平均慢一点点（少发的那点会留在源侧环里），也绝不连发。
      self.next = now + self.period;
    }
    t0.elapsed()
  }

  /// 刚过去的那一拍**该在的时刻**（诊断用：算「这一拍迟到了多少 ms」）。
  ///
  /// `wait()` 返回之后调用它，拿到的是本次等到的那一格；第一拍之前是「现在」。
  pub fn due(&self) -> Instant {
    self.next - self.period
  }

  /// 拿到的是不是高精度等待（Windows 上退化时是 `false`，要能在日志里看见）。
  pub fn high_res(&self) -> bool {
    self.waiter.is_high_res()
  }
}
