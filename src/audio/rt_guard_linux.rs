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

//! Linux 实现：把音频线程提成实时（`SCHED_FIFO`），对应 Windows 的 MMCSS「Pro Audio」。
//!
//! 两条路，按权限从高到低试，**都没有就如实上报**（不静默、也不拖垮功能）：
//! 1. 直接 `sched_setscheduler(SCHED_FIFO)`：需要 `RLIMIT_RTPRIO > 0` 或 `CAP_SYS_NICE`。
//!    桌面默认都是 0（本机实测 `ulimit -r = 0` → EPERM），发行版做音频优化时才成立。
//! 2. **rtkit**（`org.freedesktop.RealtimeKit1`）：桌面默认装着的实时权限代理，
//!    PipeWire / PulseAudio 自己也走它。未提权进程把「自己的 tid + 想要的优先级」报给
//!    rtkit-daemon，由它以特权身份改调度策略 —— 这是 Linux 上真正的 MMCSS 等价物。
//!    **必须由线程自己申请**（rtkit 会校验 caller 与目标线程的归属），所以调用点在
//!    线程函数内部，不能集中到主线程代劳。
//!
//! 与 Windows 的差异（写清楚）：
//! - Windows 的「执行保活」（`PowerRequestExecutionRequired`）在 Linux 没有对应物：
//!   普通前台进程不会被 EcoQoS 式降频，休眠由 logind/PipeWire 各自持有 inhibitor。
//!   因此 [`acquire_exec`] 是空实现，且**不谎报成功**（也不报错，因为确实无事可做）。
//! - Linux 的调度策略是**每线程**的，线程退出即消失，不需要（也无法）像 Windows 那样
//!   `AvRevertMmThreadCharacteristics` 撤销；`ThreadGuard` 只作语法占位。

use std::sync::atomic::Ordering;
use std::sync::{Mutex, OnceLock};

/// 直接申请时用的优先级（rtkit 的默认上限是 20，见下）。
const WANT_PRIO: i32 = 40;

/// 首次失败原因（`None` = 没失败过）。要 `&'static str`：进事件日志，生命周期不跟调用方。
static LAST: Mutex<Option<&'static str>> = Mutex::new(None);

/// 是否有线程成功拿到实时优先级。这是**进程级**结论：某个线程失败（例如临时资源紧张）
/// 不该让界面报「实时优先级不可用」—— 只要有一个音频线程拿到了，就说明这条路是通的。
static GRANTED: std::sync::atomic::AtomicBool = std::sync::atomic::AtomicBool::new(false);

pub struct ThreadGuard;

pub struct ExecGuard;

pub fn last_error() -> Option<&'static str> {
  if GRANTED.load(std::sync::atomic::Ordering::Relaxed) {
    return None;
  }
  *LAST.lock().unwrap_or_else(|e| e.into_inner())
}

fn note_fail(reason: &'static str) {
  let mut g = LAST.lock().unwrap_or_else(|e| e.into_inner());
  if g.is_none() {
    *g = Some(reason);
  }
}

pub fn enter() -> ThreadGuard {
  // 已经是实时线程（重复调用 / 外层已设过）就不用再申请。
  if is_realtime() {
    GRANTED.store(true, Ordering::Relaxed);
    return ThreadGuard;
  }
  if set_fifo(WANT_PRIO).is_ok() {
    GRANTED.store(true, Ordering::Relaxed);
    return ThreadGuard;
  }
  match rtkit_realtime() {
    Ok(()) => GRANTED.store(true, Ordering::Relaxed),
    Err(reason) => note_fail(reason),
  }
  ThreadGuard
}

/// 本线程当前是否已经是实时策略（`SCHED_FIFO` / `SCHED_RR`）。
fn is_realtime() -> bool {
  let policy = unsafe { libc::sched_getscheduler(0) };
  policy == libc::SCHED_FIFO || policy == libc::SCHED_RR
}

/// 直接设 `SCHED_FIFO`（需要权限）。
fn set_fifo(prio: i32) -> Result<(), ()> {
  let param = libc::sched_param {
    sched_priority: prio,
  };
  let rc = unsafe { libc::sched_setscheduler(0, libc::SCHED_FIFO, &param) };
  if rc == 0 { Ok(()) } else { Err(()) }
}

/// 通过 rtkit 给自己的线程申请实时优先级。
fn rtkit_realtime() -> Result<(), &'static str> {
  let conn =
    bus().ok_or("实时优先级未授予：rtkit 不可用（sch_guard: 需 rtkit-daemon 或 RLIMIT_RTPRIO）")?;
  let tid = unsafe { libc::syscall(libc::SYS_gettid) } as u64;
  let prio = max_priority().clamp(1, 20);
  conn
    .call_method(
      Some("org.freedesktop.RealtimeKit1"),
      "/org/freedesktop/RealtimeKit1",
      Some("org.freedesktop.RealtimeKit1"),
      "MakeThreadRealtime",
      &(tid, prio),
    )
    .map(|_| ())
    .map_err(|e| {
      // 具体原因进事件日志（`last_error` 只能放 `&'static str`，留摘要）
      crate::debug::events::log(
        "warn",
        "rt",
        format!("rtkit 拒绝 tid={tid} prio={prio}：{e}"),
      );
      "实时优先级未授予：rtkit 拒绝了请求（音频线程按普通优先级运行）"
    })
}

/// 系统总线连接（进程内复用；rtkit 是系统服务）。
fn bus() -> Option<&'static zbus::blocking::Connection> {
  static BUS: OnceLock<Option<zbus::blocking::Connection>> = OnceLock::new();
  BUS
    .get_or_init(|| zbus::blocking::Connection::system().ok())
    .as_ref()
}

/// rtkit 的 `MaxRealtimePriority`（Ubuntu 默认 20）；读不到就按 20 试。
fn max_priority() -> u32 {
  static MAX: OnceLock<u32> = OnceLock::new();
  *MAX.get_or_init(|| {
    bus()
      .and_then(|c| {
        c.call_method(
          Some("org.freedesktop.RealtimeKit1"),
          "/org/freedesktop/RealtimeKit1",
          Some("org.freedesktop.DBus.Properties"),
          "Get",
          &("org.freedesktop.RealtimeKit1", "MaxRealtimePriority"),
        )
        .ok()
      })
      .and_then(|m| m.body().deserialize::<u32>().ok())
      .unwrap_or(20)
  })
}

pub fn acquire_exec() -> ExecGuard {
  // 见模块注释：Linux 没有 EcoQoS 式后台降频，不需要「执行保活」。
  ExecGuard
}
