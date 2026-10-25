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

use std::time::Instant;

use cpal::traits::{DeviceTrait, HostTrait};
use serde::Serialize;

use crate::debug::{Probe, SharedHub, now_ms};

// 平台设备面实现分文件（AGENTS.md §4）。
#[cfg(windows)]
#[path = "devices_windows.rs"]
mod plat;
#[cfg(target_os = "linux")]
#[path = "devices_linux.rs"]
mod plat;
#[cfg(not(any(windows, target_os = "linux")))]
#[path = "devices_other.rs"]
mod plat;

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
  /// 设备池里这一路的**引用数**（几个逻辑路在用；0 = 池没持有）。见 DESIGN.md §6.5。
  pub refs: usize,
  /// 输出设备：这一条混音上的**逐源**状态（谁在出声、谁在丢；输入设备恒空）
  pub mix_sources: Vec<crate::audio::mixer::SourceStat>,
}

#[derive(Clone, Debug, Serialize)]
pub struct NativeFormat {
  pub sample_rate: u32,
  pub channels: u32,
  pub sample_format: String,
}

#[derive(Clone, Debug, Serialize, Default)]
pub struct DeviceList {
  pub enumerated_at: u64,
  pub duration_ms: u64,
  pub hosts: Vec<String>,
  pub devices: Vec<DeviceInfo>,
  /// 枚举过程中跳过的设备/接口及原因
  pub errors: Vec<String>,
}

/// AEC 远端可选的「回环目标」：某个输出设备/sink 的 monitor。
/// `id` 形如 `loopback:<sink>`，与 `engine::aec::far_loopback_id` 的约定一致。
#[derive(Clone, Debug, Serialize)]
pub struct LoopbackTarget {
  pub id: String,
  pub name: String,
}

/// 枚举结果的对外状态（进快照，供界面显示「正在刷新 / 上次刷新耗时 / 上次失败原因」）。
///
/// **必须有它**：刷新设备是用户主动点的动作，静默失败（或被静默跳过）会让人以为程序
/// 卡住了。以前 `REFRESHING` 一旦被 panic 卡住就再也刷不了，而界面上什么提示都没有。
#[derive(Clone, Debug, Serialize)]
pub struct RefreshState {
  /// 此刻是否有枚举在跑
  pub refreshing: bool,
  /// 最近一次枚举的 Unix 毫秒（0 = 从未成功）
  pub last_ok_ms: u64,
  /// 最近一次枚举耗时（毫秒）
  pub last_ms: u64,
  /// 最近一次失败原因（Rust 诊断文本；成功时为空）
  pub last_error: String,
}

static REFRESH: std::sync::Mutex<RefreshState> = std::sync::Mutex::new(RefreshState {
  refreshing: false,
  last_ok_ms: 0,
  last_ms: 0,
  last_error: String::new(),
});

/// 枚举进程内**串行且带缓存**。
///
/// 为什么要缓存：① 枚举会打到 WASAPI / ALSA，跟正在跑的会话并发枚举互相拖慢，界面点
/// 刷新时就「感觉卡住」；② `loopback_targets()`（界面初始化时调）原先自己又跑一遍
/// **完整枚举**，等于同一时刻两条枚举在抢驱动。
static ENUM_CACHE: std::sync::Mutex<Option<DeviceList>> = std::sync::Mutex::new(None);

/// 当前刷新状态（界面与快照同源）。
pub fn refresh_state() -> RefreshState {
  REFRESH.lock().unwrap().clone()
}

/// 拿一份设备列表：**绝不在调用线程里枚举**。
///
/// 这条以前是 `else { enumerate() }` —— 而 `loopback_targets()` 在界面初始化时被同步
/// 命令调用，于是整条 UI 主线程会替我们跑一遍完整 cpal 枚举（Linux 108 条 ALSA、
/// Windows WASAPI 都能到秒级）。用户看到的就是「只有菜单栏、第一列迟迟不出来」，
/// 系统还会弹「程序无响应：等待 / 强制退出」。
///
/// 现在的语义：有缓存就用（哪怕略旧），没有就返回空列表；真正的枚举只发生在
/// `spawn_refresh` 的后台线程里。界面拿到空列表会先渲染，刷新完成后再补。
fn cached_enumerate() -> DeviceList {
  if let Ok(g) = ENUM_CACHE.lock()
    && let Some(list) = g.as_ref()
  {
    return list.clone();
  }
  DeviceList::default()
}

/// 列出可回环的输出（不含「系统默认输出」那条，默认项由界面加并翻译）。
///   - Linux：PipeWire/Pulse 的 sinks（`pactl list sinks` 的 Name/Description）；
///   - Windows：cpal 的输出设备（`loopback:<wasapi 端点 ID>`）；
///   - 其它平台：空（AEC 回环不可用）。
pub fn loopback_targets() -> Vec<LoopbackTarget> {
  plat::loopback_targets(&cached_enumerate())
}

/// 解析设备显示名：优先用缓存里的枚举结果（Linux 的 Pulse 列表包含 `pulse:` 设备），
/// 缓存没有才回退 cpal `find`（会枚举 ALSA，但这一步在后台线程，不在 UI）。
pub fn display_name(device_id: &str, input: bool) -> Result<String, String> {
  if let Some(name) = device_id.strip_prefix("pulse:") {
    // parec 直接认 Pulse source 名；显示名从缓存拿不到时退回源名
    if let Ok(g) = ENUM_CACHE.lock()
      && let Some(list) = g.as_ref()
      && let Some(d) = list
        .devices
        .iter()
        .find(|d| d.id == device_id && d.direction == if input { "input" } else { "output" })
    {
      return Ok(d.name.clone());
    }
    return Ok(name.to_string());
  }
  if let Ok(g) = ENUM_CACHE.lock()
    && let Some(list) = g.as_ref()
    && let Some(d) = list
      .devices
      .iter()
      .find(|d| d.id == device_id && d.direction == if input { "input" } else { "output" })
  {
    return Ok(d.name.clone());
  }
  find(device_id, input).map(|(_, n)| n)
}

/// 按设备 ID 查找设备（输入/输出共用的唯一查找入口），返回设备与显示名。
pub fn find(device_id: &str, input: bool) -> Result<(cpal::Device, String), String> {
  for host_id in cpal::available_hosts() {
    let Ok(host) = cpal::host_from_id(host_id) else {
      continue;
    };
    let Ok(devs) = host.devices() else { continue };
    for d in devs {
      if d.id().map(|i| i.to_string()).ok().as_deref() != Some(device_id) {
        continue;
      }
      if (input && !d.supports_input()) || (!input && !d.supports_output()) {
        continue;
      }
      let name = d
        .description()
        .map(|x| x.name().to_string())
        .unwrap_or_else(|_| d.to_string());
      return Ok((d, name));
    }
  }
  let dir = if input { "输入" } else { "输出" };
  Err(format!(
    "找不到{dir}设备 {device_id}（可能已拔出，请刷新设备列表）"
  ))
}

/// 系统默认输出设备的 ID（cpal 稳定 ID）；找不到返回 None。
pub fn default_output_id() -> Option<String> {
  let host = cpal::default_host();
  let dev = host.default_output_device()?;
  dev.id().ok().map(|i| i.to_string())
}

/// 枚举繁忙守卫：**析构时必定解锁**。
///
/// 以前解锁写在正常路径末尾——枚举线程一旦 panic（或线程起不来时 `.expect` 直接崩），
/// 标志就永久卡住，**此后所有刷新都被静默跳过**，只能重启程序。放进 `Drop` 才保证
/// 「异常也解锁」。
struct BusyGuard;

impl Drop for BusyGuard {
  fn drop(&mut self) {
    REFRESH.lock().unwrap().refreshing = false;
  }
}

/// 后台刷新设备列表。返回一句给界面**直接显示**的话——不做静默跳过：
/// 正在刷就如实说正在刷，线程起不来就把错误返回。
pub fn spawn_refresh(hub: SharedHub) -> String {
  {
    let mut g = REFRESH.lock().unwrap();
    if g.refreshing {
      return "正在刷新设备，请稍候".to_string();
    }
    g.refreshing = true;
    g.last_error.clear();
  }
  let hub2 = hub.clone();
  let r = std::thread::Builder::new()
    .name("device-enum".into())
    .spawn(move || {
      // 守卫必须在**线程内**：放在 spawn 外层时，`spawn_refresh` 一返回就会把
      // `refreshing` 置回 false（从前的 bug：刷新状态永远显示「没在刷」，
      // 而且第二次刷新会和第一条枚举并发抢驱动）。
      let _busy = BusyGuard;
      let t0 = Instant::now();
      // catch_unwind：枚举要碰到用户机器的驱动状态，任何 panic 都不该带走整个程序，
      // 更不该把刷新锁死——记原因、报不可用，界面照常显示。
      match std::panic::catch_unwind(std::panic::AssertUnwindSafe(enumerate)) {
        Ok(list) => {
          let ms = t0.elapsed().as_millis() as u64;
          hub2.set_devices(Probe::ok(list.clone()));
          *ENUM_CACHE.lock().unwrap() = Some(list);
          let mut g = REFRESH.lock().unwrap();
          g.last_ms = ms;
          g.last_ok_ms = now_ms();
          g.last_error.clear();
        }
        Err(_) => {
          hub2.set_devices(Probe::unavailable(
            "枚举设备时发生内部错误，已跳过本次刷新".to_string(),
          ));
          REFRESH.lock().unwrap().last_error = "枚举设备时发生内部错误".to_string();
        }
      }
    });
  match r {
    Ok(_) => "正在枚举设备…".to_string(),
    Err(e) => {
      let msg = format!("刷新设备失败：起不来枚举线程（{e}）");
      REFRESH.lock().unwrap().last_error = msg.clone();
      hub.set_devices(Probe::unavailable(msg.clone()));
      msg
    }
  }
}

fn native_of(r: Result<cpal::SupportedStreamConfig, cpal::Error>) -> Probe<NativeFormat> {
  match r {
    Ok(c) => Probe::ok(NativeFormat {
      sample_rate: c.sample_rate(),
      channels: c.channels() as u32,
      sample_format: c.sample_format().to_string(),
    }),
    Err(e) => Probe::unavailable(format!("读取默认格式失败：{e}")),
  }
}

/// 枚举入口：Linux 优先用 PulseAudio/PipeWire 的设备面（OBS 同款），失败才回退 cpal/ALSA。
fn enumerate() -> DeviceList {
  #[cfg(target_os = "linux")]
  if let Some(list) = plat::enumerate_pulse() {
    return list;
  }
  enumerate_cpal()
}

/// cpal 兜底枚举（Linux 上会把 ALSA 插件 PCM 全列出来，**只在 pactl 不可用时**用）。
fn enumerate_cpal() -> DeviceList {
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
        Ok(d) => (
          d.name().to_string(),
          d.device_type().to_string(),
          d.interface_type().to_string(),
        ),
        Err(_) => (dev.to_string(), "未知".into(), "未知".into()),
      };
      let mut push = |direction: &'static str, native: Probe<NativeFormat>, is_default: bool| {
        devices.push(DeviceInfo {
          refs: 0,
          mix_sources: Vec::new(),
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
        push(
          "input",
          native_of(dev.default_input_config()),
          default_in.as_ref() == Some(&id),
        );
      }
      if dev.supports_output() {
        push(
          "output",
          native_of(dev.default_output_config()),
          default_out.as_ref() == Some(&id),
        );
      }
    }
  }

  DeviceList {
    enumerated_at: now_ms(),
    duration_ms: t0.elapsed().as_millis() as u64,
    hosts,
    devices: simplify(devices),
    errors,
  }
}

/// 平台设备面精简（实现见 `devices_linux.rs` / `devices_windows.rs` / `devices_other.rs`）。
fn simplify(devices: Vec<DeviceInfo>) -> Vec<DeviceInfo> {
  plat::simplify(devices)
}

/// AEC 远端是否接受「裸输出设备 ID」当回环目标：Windows 的 WASAPI 端点可以直接回环，
/// 所以 `far_device` 填一个输出设备 ID 就行；Linux 的回环是 Pulse sink 的 monitor，
/// 只认 `loopback` / `loopback:<sink>`，填 ALSA 设备 ID 会拼出无效的 `<id>.monitor`。
pub fn loopback_accepts_device_id() -> bool {
  plat::LOOPBACK_ACCEPTS_DEVICE_ID
}

#[cfg(test)]
mod tests {
  use super::*;

  /// 刷新**不会被永久卡住**：`BusyGuard` 析构必解锁。
  ///
  /// 这正是原来的 bug：`REFRESHING` 只在正常路径末尾清零，枚举线程一旦 panic（或线程
  /// 起不来时 `.expect` 直接崩），标志永久停在 true，之后所有刷新被静默跳过，只能重启。
  #[test]
  fn busy_guard_always_unlocks_even_on_panic() {
    REFRESH.lock().unwrap().refreshing = false;
    let r = std::panic::catch_unwind(|| {
      REFRESH.lock().unwrap().refreshing = true;
      let _g = BusyGuard;
      panic!("模拟枚举线程 panic");
    });
    assert!(r.is_err(), "先确认确实 panic 了");
    assert!(
      !refresh_state().refreshing,
      "守卫析构后必须解锁，否则刷新被永久静默跳过"
    );
  }

  #[test]
  fn refresh_state_roundtrip_and_serializes() {
    REFRESH.lock().unwrap().refreshing = false;
    REFRESH.lock().unwrap().last_error = "".into();
    let j = serde_json::to_string(&refresh_state()).unwrap();
    for k in ["refreshing", "last_ok_ms", "last_ms", "last_error"] {
      assert!(j.contains(k), "刷新状态字段 {k} 必须进快照：{j}");
    }
  }

  /// 连续两次刷新都要有明确回应（第二次可以是「正在刷新」，但**不能无声无息**）。
  #[test]
  fn spawn_refresh_always_returns_a_message() {
    let hub = crate::debug::DebugHub::new();
    REFRESH.lock().unwrap().refreshing = false;
    let first = spawn_refresh(hub.clone());
    assert!(!first.is_empty(), "刷新必须回一句话给界面");
    // 第二次：要么也开始刷（回「正在枚举」），要么如实说「正在刷新」
    let second = spawn_refresh(hub);
    assert!(!second.is_empty(), "并发刷新也必须回话，不能静默跳过");
  }
}
