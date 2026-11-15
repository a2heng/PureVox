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

//! 实时线程的「保活」：让系统别把我们的音频线程当普通后台线程调度。
//!
//! ## 为什么需要
//!
//! 实测（2026-10-27）：链路稳态（发包 100.00/s、无欠载）时仍每隔几秒~十几秒出现一次
//! ~30~55 ms 的**整进程停顿**，看门狗线程（什么都不干、不拿锁）也会跟着停 ——
//! 说明不是我们的锁，而是调度/电源层面把进程当成了「后台」。
//!
//! ## Windows 上的两条标准做法
//!
//! 1. **MMCSS**（多媒体类调度服务）：音频线程调 `AvSetMmThreadCharacteristicsW("Pro Audio")`，
//!    系统把线程登记成实时音频任务并提升优先级（`AvSetMmThreadPriority` = CRITICAL）。
//!    DAW / 播放器在 Windows 上都是这么做的 —— 这不是「提高 nice」，是向系统**声明用途**。
//! 2. **执行保活**：`PowerCreateRequest` + `PowerSetRequest(PowerRequestExecutionRequired)`，
//!    在音频跑着的时候持有，等价于安卓的 WakeLock：系统不会把进程按效率模式挂着。
//!
//! 两者都**只影响本进程**，不改任何全局注册表/电源计划（`timeBeginPeriod` 那种全局副作用不做）。
//!
//! 非 Windows：这里不做事（Linux 的实时性靠 cgroup/`sched_setscheduler`，另有安排）。
//!
//! 平台实现见 `rt_guard_windows.rs` / `rt_guard_linux.rs`（后者：`SCHED_FIFO` 或 rtkit，
//! 对应 MMCSS「Pro Audio」；「执行保活」在 Linux 无对应物，见该文件注释）。

#[cfg(windows)]
#[path = "rt_guard_windows.rs"]
mod imp;

#[cfg(target_os = "linux")]
#[path = "rt_guard_linux.rs"]
mod imp;

#[cfg(not(any(windows, target_os = "linux")))]
#[path = "rt_guard_other.rs"]
mod imp;

/// 把**当前线程**登记成实时音频线程（MMCSS Pro Audio）。Drop 时撤销。
///
/// 用法：线程函数第一行 `let _rt = rt_guard::enter_audio_thread();`。
/// 失败不会 panic，也不阻塞任何功能，只是拿不到提升（原因用 [`last_error`]）。
// 只为了让 `Drop` 按声明的顺序走 imp 的实现；字段本身不读，是刻意的（RAII 句柄）。
#[allow(dead_code)]
pub struct AudioThreadGuard(imp::ThreadGuard);

impl Drop for AudioThreadGuard {
  fn drop(&mut self) {
    // 撤销在 imp 的 Drop 里做（句柄归它管）
  }
}

/// 登记当前线程。失败返回的 guard 也能安全丢弃（只是没有提升）。
pub fn enter_audio_thread() -> AudioThreadGuard {
  AudioThreadGuard(imp::enter())
}

/// 进程级「执行保活」句柄：持有期间系统不按效率模式挂起本进程。Drop 时释放。
#[allow(dead_code)]
pub struct ExecutionRequest(imp::ExecGuard);

impl Drop for ExecutionRequest {
  fn drop(&mut self) {
    // 释放在 imp 的 Drop 里做
  }
}

/// 申请执行保活（音频会话开始时）。失败也只是没有保活，不致命。
pub fn acquire_execution_required() -> ExecutionRequest {
  ExecutionRequest(imp::acquire_exec())
}

/// 最近一次失败原因（`None` = 没失败过）。进事件日志，不静默。
pub fn last_error() -> Option<&'static str> {
  imp::last_error()
}
