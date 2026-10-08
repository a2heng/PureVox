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

//! Windows GPU 指标。
//!
//! - 适配器列表、显存总量、本进程显存：DXGI（`IDXGIAdapter3::QueryVideoMemoryInfo`）
//! - 整卡/本进程占用率、整卡显存已用：PDH 计数器 `GPU Engine` / `GPU Adapter Memory`
//!   （任务管理器同源）。占用率是差分值，第一轮采样为「采集中」。
//!
//! 实例名形如 `pid_1234_luid_0x00000000_0x0000C3F1_phys_0_eng_0_engtype_3D`，
//! 按 LUID 与 DXGI 适配器对应。

use std::collections::{HashMap, HashSet};

use windows::Win32::Graphics::Dxgi::{
  CreateDXGIFactory1, DXGI_ADAPTER_FLAG_SOFTWARE, DXGI_ERROR_NOT_FOUND,
  DXGI_MEMORY_SEGMENT_GROUP_LOCAL, DXGI_QUERY_VIDEO_MEMORY_INFO, IDXGIAdapter3, IDXGIFactory1,
};
use windows::Win32::System::Performance::{
  PDH_CSTATUS_NEW_DATA, PDH_CSTATUS_VALID_DATA, PDH_FMT_COUNTERVALUE_ITEM_W, PDH_FMT_DOUBLE,
  PDH_HCOUNTER, PDH_HQUERY, PDH_MORE_DATA, PdhAddEnglishCounterW, PdhCollectQueryData,
  PdhGetFormattedCounterArrayW, PdhOpenQueryW,
};
use windows::core::{Interface, PCWSTR, w};

use super::Probe;
use super::system::GpuAdapter;

const ENGINE_PATH: PCWSTR = w!("\\GPU Engine(*)\\Utilization Percentage");
const ADAPTER_MEM_PATH: PCWSTR = w!("\\GPU Adapter Memory(*)\\Dedicated Usage");

struct Pdh {
  query: PDH_HQUERY,
  engine: PDH_HCOUNTER,
  adapter_mem: PDH_HCOUNTER,
  primed: bool,
}

// PDH 句柄只在采样线程内使用
unsafe impl Send for Pdh {}

impl Pdh {
  fn open() -> Result<Self, String> {
    unsafe {
      let mut query = PDH_HQUERY::default();
      let st = PdhOpenQueryW(PCWSTR::null(), 0, &mut query);
      if st != 0 {
        return Err(format!("PdhOpenQuery 失败：0x{st:08X}"));
      }
      let mut engine = PDH_HCOUNTER::default();
      let st = PdhAddEnglishCounterW(query, ENGINE_PATH, 0, &mut engine);
      if st != 0 {
        return Err(format!(
          "PDH 计数器 GPU Engine 不可用：0x{st:08X}（需 Windows 10 1709+ 与 WDDM 2.x 驱动）"
        ));
      }
      let mut adapter_mem = PDH_HCOUNTER::default();
      let st = PdhAddEnglishCounterW(query, ADAPTER_MEM_PATH, 0, &mut adapter_mem);
      if st != 0 {
        return Err(format!("PDH 计数器 GPU Adapter Memory 不可用：0x{st:08X}"));
      }
      Ok(Pdh {
        query,
        engine,
        adapter_mem,
        primed: false,
      })
    }
  }

  fn read_array(counter: PDH_HCOUNTER) -> Result<Vec<(String, f64)>, String> {
    unsafe {
      let mut size = 0u32;
      let mut count = 0u32;
      let st = PdhGetFormattedCounterArrayW(counter, PDH_FMT_DOUBLE, &mut size, &mut count, None);
      if st == 0 {
        return Ok(Vec::new());
      }
      if st != PDH_MORE_DATA {
        return Err(format!("PdhGetFormattedCounterArray 失败：0x{st:08X}"));
      }
      // u64 缓冲保证对齐；字符串数据也在同一块缓冲内
      let mut buf: Vec<u64> = vec![0; (size as usize).div_ceil(8)];
      let items_ptr = buf.as_mut_ptr() as *mut PDH_FMT_COUNTERVALUE_ITEM_W;
      let st = PdhGetFormattedCounterArrayW(
        counter,
        PDH_FMT_DOUBLE,
        &mut size,
        &mut count,
        Some(items_ptr),
      );
      if st != 0 {
        return Err(format!("PdhGetFormattedCounterArray 失败：0x{st:08X}"));
      }
      let items = std::slice::from_raw_parts(items_ptr, count as usize);
      let mut out = Vec::with_capacity(items.len());
      for it in items {
        let cs = it.FmtValue.CStatus;
        if cs != PDH_CSTATUS_VALID_DATA && cs != PDH_CSTATUS_NEW_DATA {
          continue;
        }
        let name = it.szName.to_string().unwrap_or_default();
        out.push((name, it.FmtValue.Anonymous.doubleValue));
      }
      Ok(out)
    }
  }
}

/// `luid_0x00000000_0x0000C3F1`（26 字符），统一小写便于比较。
fn luid_key(name: &str) -> Option<String> {
  let i = name.find("luid_")?;
  name.get(i..i + 26).map(|s| s.to_ascii_lowercase())
}

fn pid_of(name: &str) -> Option<u32> {
  let rest = name.strip_prefix("pid_")?;
  rest.split('_').next()?.parse().ok()
}

fn engtype_of(name: &str) -> &str {
  name.rsplit_once("engtype_").map(|(_, t)| t).unwrap_or("")
}

/// PDH 引擎计数器里出现过的全部 LUID。
fn engine_luids(items: &[(String, f64)]) -> HashSet<String> {
  items
    .iter()
    .filter_map(|(name, _)| luid_key(name))
    .collect()
}

/// 按 LUID 汇总：先按引擎类型求和，再取各类型最大值（任务管理器口径）。
fn utilization_by_luid(items: &[(String, f64)], only_pid: Option<u32>) -> HashMap<String, f64> {
  let mut per_type: HashMap<(String, String), f64> = HashMap::new();
  for (name, v) in items {
    if let Some(p) = only_pid
      && pid_of(name) != Some(p)
    {
      continue;
    }
    if let Some(l) = luid_key(name) {
      *per_type
        .entry((l, engtype_of(name).to_string()))
        .or_default() += v;
    }
  }
  let mut out: HashMap<String, f64> = HashMap::new();
  for ((l, _), v) in per_type {
    let e = out.entry(l).or_default();
    *e = e.max(v.min(100.0));
  }
  out
}

struct DxgiAdapter {
  name: String,
  vendor_id: u32,
  luid: String,
  dedicated_total: u64,
  process_used: Probe<u64>,
}

fn dxgi_adapters() -> Result<Vec<DxgiAdapter>, String> {
  unsafe {
    let factory: IDXGIFactory1 =
      CreateDXGIFactory1().map_err(|e| format!("CreateDXGIFactory1 失败：{e}"))?;
    let mut out = Vec::new();
    let mut i = 0u32;
    loop {
      let adapter = match factory.EnumAdapters1(i) {
        Ok(a) => a,
        Err(e) if e.code() == DXGI_ERROR_NOT_FOUND => break,
        Err(e) => return Err(format!("EnumAdapters1({i}) 失败：{e}")),
      };
      i += 1;
      let desc = adapter
        .GetDesc1()
        .map_err(|e| format!("GetDesc1 失败：{e}"))?;
      if desc.Flags & (DXGI_ADAPTER_FLAG_SOFTWARE.0 as u32) != 0 {
        continue; // Microsoft Basic Render Driver 等软件适配器
      }
      let len = desc
        .Description
        .iter()
        .position(|&c| c == 0)
        .unwrap_or(desc.Description.len());
      let name = String::from_utf16_lossy(&desc.Description[..len]);
      let luid = format!(
        "luid_0x{:08x}_0x{:08x}",
        desc.AdapterLuid.HighPart as u32, desc.AdapterLuid.LowPart
      );
      let process_used = match adapter.cast::<IDXGIAdapter3>() {
        Ok(a3) => {
          let mut info = DXGI_QUERY_VIDEO_MEMORY_INFO::default();
          match a3.QueryVideoMemoryInfo(0, DXGI_MEMORY_SEGMENT_GROUP_LOCAL, &mut info) {
            Ok(()) => Probe::ok(info.CurrentUsage),
            Err(e) => Probe::unavailable(format!("QueryVideoMemoryInfo 失败：{e}")),
          }
        }
        Err(e) => Probe::unavailable(format!("IDXGIAdapter3 不可用：{e}")),
      };
      out.push(DxgiAdapter {
        name,
        vendor_id: desc.VendorId,
        luid,
        dedicated_total: desc.DedicatedVideoMemory as u64,
        process_used,
      });
    }
    Ok(out)
  }
}

pub struct GpuSampler {
  pdh: Result<Pdh, String>,
  pid: u32,
}

impl GpuSampler {
  pub fn new() -> Self {
    GpuSampler {
      pdh: Pdh::open(),
      pid: std::process::id(),
    }
  }

  pub fn sample(&mut self) -> Probe<Vec<GpuAdapter>> {
    let adapters = match dxgi_adapters() {
      Ok(a) => a,
      Err(e) => return Probe::unavailable(e),
    };
    if adapters.is_empty() {
      return Probe::unavailable("没有硬件 GPU 适配器（DXGI 只枚举到软件适配器）");
    }

    // PDH 一轮采集；结果按 LUID 归并
    type Maps = (
      Probe<HashSet<String>>,
      HashMap<String, f64>,
      HashMap<String, f64>,
      HashMap<String, f64>,
    );
    let (pdh_state, util_all, util_proc, mem_used): Maps = match &mut self.pdh {
      Err(e) => (
        Probe::unavailable(e.clone()),
        HashMap::new(),
        HashMap::new(),
        HashMap::new(),
      ),
      Ok(p) => {
        let st = unsafe { PdhCollectQueryData(p.query) };
        if st != 0 {
          (
            Probe::unavailable(format!("PdhCollectQueryData 失败：0x{st:08X}")),
            HashMap::new(),
            HashMap::new(),
            HashMap::new(),
          )
        } else {
          let mem = match Pdh::read_array(p.adapter_mem) {
            Ok(items) => {
              let mut m: HashMap<String, f64> = HashMap::new();
              for (name, v) in items {
                if let Some(l) = luid_key(&name) {
                  *m.entry(l).or_default() += v;
                }
              }
              m
            }
            Err(_) => HashMap::new(),
          };
          if !p.primed {
            p.primed = true;
            (Probe::Pending, HashMap::new(), HashMap::new(), mem)
          } else {
            match Pdh::read_array(p.engine) {
              Ok(items) => (
                Probe::ok(engine_luids(&items)),
                utilization_by_luid(&items, None),
                utilization_by_luid(&items, Some(self.pid)),
                mem,
              ),
              Err(e) => (Probe::unavailable(e), HashMap::new(), HashMap::new(), mem),
            }
          }
        }
      }
    };

    let util = |m: &HashMap<String, f64>, luid: &str| -> Probe<f64> {
      match &pdh_state {
        Probe::Pending => Probe::Pending,
        Probe::Unavailable { reason } => Probe::unavailable(reason.clone()),
        // 该卡在 PDH 中没有任何引擎实例：拿不到数据，不能当 0
        Probe::Ok { value: seen } if !seen.contains(luid) => {
          Probe::unavailable("PDH 中没有该适配器的引擎实例")
        }
        // 该卡有实例但筛选后为空（如本进程没有在这张卡上提交工作）= 真实为 0
        Probe::Ok { .. } => Probe::ok(m.get(luid).copied().unwrap_or(0.0)),
      }
    };

    let list = adapters
      .into_iter()
      .map(|a| {
        let dedicated_used = match (&self.pdh, mem_used.get(&a.luid)) {
          (Err(e), _) => Probe::unavailable(e.clone()),
          (Ok(_), Some(v)) => Probe::ok(*v as u64),
          (Ok(_), None) => Probe::unavailable("PDH 中没有该适配器的显存实例"),
        };
        GpuAdapter {
          utilization_pct: util(&util_all, &a.luid),
          process_utilization_pct: util(&util_proc, &a.luid),
          dedicated_used,
          process_dedicated_used: a.process_used,
          name: a.name,
          vendor_id: format!("0x{:04X}", a.vendor_id),
          luid: a.luid,
          dedicated_total: a.dedicated_total,
        }
      })
      .collect();
    Probe::ok(list)
  }
}
