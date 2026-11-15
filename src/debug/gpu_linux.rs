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

//! Linux GPU 指标：NVIDIA 走 `nvidia-smi`，AMD 走 sysfs。**不引入新依赖、不碰私有驱动 API。**
//!
//! 为什么是命令行 / sysfs：Windows 侧用 DXGI + PDH（进程内 API）；Linux 上
//! NVML 要链接 `libnvidia-ml.so`（每家发行版路径不同、升级即变），而 `nvidia-smi`
//! 是与驱动同版本发布的稳定出口，读数与 `nvidia-smi` 自己看到的一致。AMD 的
//! `amdgpu` 则直接暴露 `/sys/class/drm/card*/device/{gpu_busy_percent,mem_info_vram_*}`
//! （没有就把该适配器跳过，不伪造 0）。
//!
//! 口径差异（写清楚，不假装和 Windows 一样）：
//! - **整卡占用 / 整卡显存**：与任务管理器同义（各引擎取最高值 / 专用显存已用）；
//! - **本进程显存**：`nvidia-smi --query-compute-apps` 只列**计算**上下文，图形上下文
//!   （WebKitGTK 的 EGL）看不到 → 查得到就填，查不到记 0（确实是「没占 CUDA 显存」）；
//! - **本进程占用**：`nvidia-smi` 不提供单进程占用率 → 明确不可用，不编数。

use std::process::Command;
use std::time::{Duration, Instant};

use super::{GpuAdapter, Probe};

/// `nvidia-smi` 一次查询约 24 ms（本机实测）：调试面板 1 Hz 刷新时复用 2 s 内的结果，
/// 免得每秒都去起一个进程。
const CACHE: Duration = Duration::from_secs(2);

pub struct GpuSampler {
  cache: Option<(Instant, Probe<Vec<GpuAdapter>>)>,
}

impl GpuSampler {
  pub fn new() -> Self {
    GpuSampler { cache: None }
  }

  pub fn sample(&mut self) -> Probe<Vec<GpuAdapter>> {
    if let Some((at, probe)) = &self.cache
      && at.elapsed() < CACHE
    {
      return probe.clone();
    }
    let probe = nvidia().or_else(amd).unwrap_or_else(|| {
      Probe::unavailable("没有可用的 GPU 读数来源（既没有 nvidia-smi，也没有 amdgpu sysfs）")
    });
    self.cache = Some((Instant::now(), probe.clone()));
    probe
  }
}

/// NVIDIA：`nvidia-smi` 的 CSV 输出（列顺序与 [`QUERY_GPU`] 一一对应）。
const QUERY_GPU: &str = "index,name,pci.bus_id,utilization.gpu,memory.total,memory.used";
const QUERY_APPS: &str = "pid,used_memory";

fn nvidia() -> Option<Probe<Vec<GpuAdapter>>> {
  let out = Command::new("nvidia-smi")
    .args([
      &format!("--query-gpu={QUERY_GPU}"),
      "--format=csv,noheader,nounits",
    ])
    .output()
    .ok()?;
  if !out.status.success() {
    return Some(Probe::unavailable(format!(
      "nvidia-smi 查询失败：{}",
      String::from_utf8_lossy(&out.stderr).trim()
    )));
  }
  let mine = nvidia_process_mem();
  let text = String::from_utf8_lossy(&out.stdout);
  let mut list = Vec::new();
  for line in text.lines() {
    let cols: Vec<&str> = line.split(',').map(str::trim).collect();
    if cols.len() < 6 {
      continue;
    }
    let bus = cols[2].to_string();
    list.push(GpuAdapter {
      name: cols[1].to_string(),
      // NVIDIA 的 PCI 厂商号；Linux 侧没有 Windows 的 LUID 概念，用 PCI 总线地址当稳定标识
      vendor_id: "10DE".to_string(),
      luid: bus,
      dedicated_total: mib(cols[4]).unwrap_or(0),
      dedicated_used: mib(cols[5]).map_or_else(
        || Probe::unavailable("nvidia-smi 未给出显存已用"),
        Probe::ok,
      ),
      process_dedicated_used: Probe::ok(mine),
      utilization_pct: num(cols[3]).map_or_else(
        || Probe::unavailable("nvidia-smi 未给出整卡占用"),
        Probe::ok,
      ),
      process_utilization_pct: Probe::unavailable("nvidia-smi 不提供单进程 GPU 占用"),
    });
  }
  if list.is_empty() {
    return Some(Probe::unavailable("nvidia-smi 没有返回任何适配器"));
  }
  Some(Probe::ok(list))
}

/// 本进程在 NVIDIA 卡上的显存（只统计计算上下文；没有条目就是 0 字节）。
fn nvidia_process_mem() -> u64 {
  let Ok(out) = Command::new("nvidia-smi")
    .args([
      &format!("--query-compute-apps={QUERY_APPS}"),
      "--format=csv,noheader,nounits",
    ])
    .output()
  else {
    return 0;
  };
  let me = std::process::id().to_string();
  let text = String::from_utf8_lossy(&out.stdout);
  let mut sum = 0u64;
  for line in text.lines() {
    let mut cols = line.split(',').map(str::trim);
    let (Some(pid), Some(mem)) = (cols.next(), cols.next()) else {
      continue;
    };
    if pid == me {
      sum += mib(mem).unwrap_or(0);
    }
  }
  sum
}

/// AMD（amdgpu）：`/sys/class/drm/card*/device/` 下的整卡读数。
fn amd() -> Option<Probe<Vec<GpuAdapter>>> {
  let entries = std::fs::read_dir("/sys/class/drm").ok()?;
  let mut list = Vec::new();
  for e in entries.flatten() {
    let name = e.file_name().to_string_lossy().to_string();
    // `card0` 是整卡，`card0-DP-1` 是它的输出接头：只要整卡
    if !name.starts_with("card") || name.contains('-') {
      continue;
    }
    let dev = e.path().join("device");
    let total = read_num(&dev.join("mem_info_vram_total"));
    let used = read_num(&dev.join("mem_info_vram_used"));
    let busy = read_num(&dev.join("gpu_busy_percent"));
    if total.is_none() && busy.is_none() {
      continue; // 不是 AMD，或驱动没暴露这套节点
    }
    let driver = std::fs::read_to_string(dev.join("uevent"))
      .ok()
      .and_then(|t| {
        t.lines()
          .find_map(|l| l.strip_prefix("DRIVER=").map(str::to_string))
      })
      .unwrap_or_else(|| "drm".into());
    list.push(GpuAdapter {
      name: format!("AMD GPU（{name}，{driver}）"),
      vendor_id: "1002".to_string(),
      luid: name,
      dedicated_total: total.unwrap_or(0),
      dedicated_used: used.map_or_else(
        || Probe::unavailable("amdgpu 未暴露 mem_info_vram_used"),
        Probe::ok,
      ),
      // 单进程显存要走 /proc/<pid>/fdinfo 的 drm-memory-*，暂不实现（不编数）
      process_dedicated_used: Probe::unavailable("amdgpu 单进程显存未实现"),
      utilization_pct: busy.map_or_else(
        || Probe::unavailable("amdgpu 未暴露 gpu_busy_percent"),
        |v| Probe::ok(v as f64),
      ),
      process_utilization_pct: Probe::unavailable("amdgpu 不提供单进程占用"),
    });
  }
  if list.is_empty() {
    return None;
  }
  Some(Probe::ok(list))
}

/// CSV 里可能是 `[N/A]` / `[Not Supported]`：只有纯数字才算数。
fn num(s: &str) -> Option<f64> {
  s.trim().parse::<f64>().ok()
}

fn mib(s: &str) -> Option<u64> {
  num(s).map(|v| (v.max(0.0) * 1024.0 * 1024.0) as u64)
}

fn read_num(path: &std::path::Path) -> Option<u64> {
  std::fs::read_to_string(path).ok()?.trim().parse().ok()
}
