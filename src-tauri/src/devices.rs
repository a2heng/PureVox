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

//! 音频设备枚举（cpal）。刷新单一入口：[`spawn_refresh`]，后台线程执行，
//! 结果写入 DebugHub；UI 线程禁止同步枚举。

use std::sync::atomic::{AtomicBool, Ordering};
use std::time::Instant;

use cpal::traits::{DeviceTrait, HostTrait};
use serde::Serialize;

use crate::debug::{now_ms, Probe, SharedHub};

#[derive(Clone, Debug, Serialize)]
pub struct DeviceInfo {
  /// cpal 稳定设备 ID（`<host>:<后端 ID>`）
  pub id: String,
  pub name: String,
  /// 音频接口（cpal host，如 WASAPI）
  pub host: String,
  pub direction: &'static str,
  pub device_type: String,
  pub interface_type: String,
  /// 设备原生（共享模式默认）格式；读取失败时为不可用 + 原因
  pub native: Probe<NativeFormat>,
  pub is_default: bool,
  /// 设备选择尚未实现，恒为 false
  pub selected: bool,
  /// 由 DebugHub::snapshot 按当前活动流推出（枚举时恒为 false）
  pub opened: bool,
}

#[derive(Clone, Debug, Serialize)]
pub struct NativeFormat {
  pub sample_rate: u32,
  pub channels: u32,
  pub sample_format: String,
}

#[derive(Clone, Debug, Serialize)]
pub struct DeviceList {
  pub enumerated_at: u64,
  pub duration_ms: u64,
  pub hosts: Vec<String>,
  pub devices: Vec<DeviceInfo>,
  /// 枚举过程中跳过的设备/接口及原因
  pub errors: Vec<String>,
}

static REFRESHING: AtomicBool = AtomicBool::new(false);

/// 按设备 ID 查找设备（输入/输出共用的唯一查找入口），返回设备与显示名。
pub fn find(device_id: &str, input: bool) -> Result<(cpal::Device, String), String> {
  for host_id in cpal::available_hosts() {
    let Ok(host) = cpal::host_from_id(host_id) else { continue };
    let Ok(devs) = host.devices() else { continue };
    for d in devs {
      if d.id().map(|i| i.to_string()).ok().as_deref() != Some(device_id) {
        continue;
      }
      if (input && !d.supports_input()) || (!input && !d.supports_output()) {
        continue;
      }
      let name = d.description().map(|x| x.name().to_string()).unwrap_or_else(|_| d.to_string());
      return Ok((d, name));
    }
  }
  let dir = if input { "输入" } else { "输出" };
  Err(format!("找不到{dir}设备 {device_id}（可能已拔出，请刷新设备列表）"))
}

/// 后台刷新设备列表；已有刷新在进行时直接返回。
pub fn spawn_refresh(hub: SharedHub) {
  if REFRESHING.swap(true, Ordering::SeqCst) {
    return;
  }
  std::thread::Builder::new()
    .name("device-enum".into())
    .spawn(move || {
      hub.set_devices(Probe::ok(enumerate()));
      REFRESHING.store(false, Ordering::SeqCst);
    })
    .expect("spawn device-enum");
}

fn native_of(
  r: Result<cpal::SupportedStreamConfig, cpal::Error>,
) -> Probe<NativeFormat> {
  match r {
    Ok(c) => Probe::ok(NativeFormat {
      sample_rate: c.sample_rate(),
      channels: c.channels() as u32,
      sample_format: c.sample_format().to_string(),
    }),
    Err(e) => Probe::unavailable(format!("读取默认格式失败：{e}")),
  }
}

fn enumerate() -> DeviceList {
  let t0 = Instant::now();
  let mut hosts = Vec::new();
  let mut devices = Vec::new();
  let mut errors = Vec::new();

  for host_id in cpal::available_hosts() {
    let host_name = host_id.name().to_string();
    let host = match cpal::host_from_id(host_id) {
      Ok(h) => h,
      Err(e) => {
        errors.push(format!("{host_name}: 打开接口失败：{e}"));
        continue;
      }
    };
    hosts.push(host_name.clone());
    let default_in = host.default_input_device().and_then(|d| d.id().ok());
    let default_out = host.default_output_device().and_then(|d| d.id().ok());
    let list = match host.devices() {
      Ok(l) => l,
      Err(e) => {
        errors.push(format!("{host_name}: 枚举设备失败：{e}"));
        continue;
      }
    };
    for dev in list {
      let id = match dev.id() {
        Ok(id) => id,
        Err(e) => {
          errors.push(format!("{host_name}: 设备 {dev} 无法取得 ID：{e}"));
          continue;
        }
      };
      let (name, device_type, interface_type) = match dev.description() {
        Ok(d) => (d.name().to_string(), d.device_type().to_string(), d.interface_type().to_string()),
        Err(_) => (dev.to_string(), "未知".into(), "未知".into()),
      };
      let mut push = |direction: &'static str, native: Probe<NativeFormat>, is_default: bool| {
        devices.push(DeviceInfo {
          id: id.to_string(),
          name: name.clone(),
          host: host_name.clone(),
          direction,
          device_type: device_type.clone(),
          interface_type: interface_type.clone(),
          native,
          is_default,
          selected: false,
          opened: false,
        });
      };
      if dev.supports_input() {
        push("input", native_of(dev.default_input_config()), default_in.as_ref() == Some(&id));
      }
      if dev.supports_output() {
        push("output", native_of(dev.default_output_config()), default_out.as_ref() == Some(&id));
      }
    }
  }

  DeviceList {
    enumerated_at: now_ms(),
    duration_ms: t0.elapsed().as_millis() as u64,
    hosts,
    devices,
    errors,
  }
}
