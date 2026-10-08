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

//! 系统指标采样线程：CPU / 内存 / GPU，周期 1 s，结果写入 DebugHub。

use serde::Serialize;
use std::time::Duration;
use sysinfo::{ProcessRefreshKind, ProcessesToUpdate, System};

use super::{Probe, SharedHub, now_ms};

pub const SAMPLE_PERIOD: Duration = Duration::from_secs(1);

#[derive(Clone, Debug, Serialize)]
pub struct CpuInfo {
  /// 系统总占用（0~100）
  pub system_pct: f32,
  /// 本进程占整机的比例（0~100，已除以逻辑核数，与任务管理器口径一致）
  pub process_pct: f32,
  pub logical_cores: usize,
}

#[derive(Clone, Debug, Serialize)]
pub struct MemoryInfo {
  pub system_total: u64,
  pub system_available: u64,
  /// 本进程工作集（字节）
  pub process_working_set: u64,
  /// 本进程私有字节（字节）
  pub process_private: Probe<u64>,
}

#[derive(Clone, Debug, Serialize)]
pub struct GpuAdapter {
  pub name: String,
  pub vendor_id: String,
  pub luid: String,
  /// 专用显存总量（字节）
  pub dedicated_total: u64,
  /// 整卡专用显存已用（字节）
  pub dedicated_used: Probe<u64>,
  /// 本进程专用显存已用（字节）
  pub process_dedicated_used: Probe<u64>,
  /// 整卡占用（各引擎类型中最高者，0~100，与任务管理器口径一致）
  pub utilization_pct: Probe<f64>,
  /// 本进程占用（0~100）
  pub process_utilization_pct: Probe<f64>,
}

#[derive(Clone, Debug, Serialize)]
pub struct SystemInfo {
  pub sampled_at: Option<u64>,
  pub cpu: Probe<CpuInfo>,
  pub memory: Probe<MemoryInfo>,
  pub gpu: Probe<Vec<GpuAdapter>>,
}

impl Default for SystemInfo {
  fn default() -> Self {
    SystemInfo {
      sampled_at: None,
      cpu: Probe::Pending,
      memory: Probe::Pending,
      gpu: Probe::Pending,
    }
  }
}

// 平台实现分文件（AGENTS.md §4）：私有字节 / GPU 采样。
#[cfg(windows)]
#[path = "gpu_windows.rs"]
mod gpu;
#[cfg(not(windows))]
#[path = "gpu_other.rs"]
mod gpu;
#[cfg(windows)]
#[path = "mem_windows.rs"]
mod mem;
#[cfg(not(windows))]
#[path = "mem_other.rs"]
mod mem;

use gpu::GpuSampler;

pub fn spawn_sampler(hub: SharedHub) {
  std::thread::Builder::new()
    .name("debug-sampler".into())
    .spawn(move || {
      let mut sys = System::new();
      let pid = sysinfo::get_current_pid();
      let mut gpu = GpuSampler::new();
      let mut first = true;
      let mut tick: u64 = 0;
      loop {
        sys.refresh_cpu_usage();
        sys.refresh_memory();
        if let Ok(pid) = pid {
          sys.refresh_processes_specifics(
            ProcessesToUpdate::Some(&[pid]),
            true,
            ProcessRefreshKind::nothing().with_cpu().with_memory(),
          );
        }
        let cores = sys.cpus().len();
        let proc_ = pid.ok().and_then(|p| sys.process(p));

        // CPU 占用是差分值，第一轮没有基准
        let cpu = match (first, proc_, pid) {
          (true, _, _) => Probe::Pending,
          (_, _, Err(e)) => Probe::unavailable(format!("无法取得本进程 PID：{e}")),
          (_, None, _) => Probe::unavailable("无法读取本进程信息"),
          (_, Some(p), _) => Probe::ok(CpuInfo {
            system_pct: sys.global_cpu_usage(),
            process_pct: p.cpu_usage() / cores.max(1) as f32,
            logical_cores: cores,
          }),
        };
        let memory = match proc_ {
          None => Probe::unavailable("无法读取本进程信息"),
          Some(p) => Probe::ok(MemoryInfo {
            system_total: sys.total_memory(),
            system_available: sys.available_memory(),
            process_working_set: p.memory(),
            process_private: mem::private_bytes(),
          }),
        };

        hub.set_system(SystemInfo {
          sampled_at: Some(now_ms()),
          cpu,
          memory,
          gpu: gpu.sample(),
        });
        // 虚拟麦克风状态（Linux 才有）：不必每 tick 都跑 pw-cli 子进程，2 s 一次即可。
        if first || tick.is_multiple_of(2) {
          hub.set_virtual_mic(crate::audio::virtual_mic::status_probe());
        }
        tick += 1;
        first = false;
        std::thread::sleep(SAMPLE_PERIOD);
      }
    })
    .expect("spawn debug-sampler");
}
