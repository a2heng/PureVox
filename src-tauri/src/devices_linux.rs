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

//! Linux 的设备面：Pulse/PipeWire 的 source/sink（与 GNOME 声音设置、OBS 同源）。
//!
//! 输入一律走 `pulse:<source>`（`parec` 采集，PipeWire 混音，多个软件可同时用麦克风）；
//! 输出暂时仍映射到 ALSA `plughw`（需要设备时钟做 PI 伺服；输出被独占时仍可能 busy）。

use std::collections::HashMap;
use std::io::Read;
use std::process::{Command, Stdio};
use std::time::{Duration, Instant};

use serde_json::Value;

use super::{DeviceInfo, DeviceList, LoopbackTarget, NativeFormat};
use crate::debug::{Probe, now_ms};

/// Linux 的回环是 Pulse sink 的 monitor：**不接受裸 ALSA 设备 ID**。
pub(super) const LOOPBACK_ACCEPTS_DEVICE_ID: bool = false;

/// **OBS 同款设备面**：Linux 不遍历 ALSA hint PCM，而是用 PulseAudio/PipeWire 的
/// source/sink 列表（OBS 的 `plugins/linux-pulseaudio/pulse-input.c` 就是
/// `pa_context_get_source_info_list` + `i->description`；Windows 侧对应
/// `IMMDeviceEnumerator::EnumAudioEndpoints`）。这里用 `pactl -f json` 取同一份数据：
/// - 每台**真实设备**一条，不再出现 100+ 个 `null/dmix/speex/pulse/...` 插件 PCM；
/// - 原生格式直接取 Pulse 的 `sample_specification`，枚举期间**不打开任何 PCM**；
/// - **输入一律**用 `pulse:<name>`，由 `parec` 采集（`audio/loopback_linux.rs`
///   已有同一套工作循环）；这样不直接占 ALSA `plughw`，浏览器/OBS 可同时用麦克风；
/// - 输出暂时把 `alsa.card`/`alsa.device` 映射成 cpal 的 `alsa:plughw:CARD=…,DEV=…`
///   （输出需要设备时钟伺服；PipeWire 输出通路是后续工作）。
pub(super) fn enumerate_pulse() -> Option<DeviceList> {
  let t0 = Instant::now();
  let sources = pactl_json(&["-f", "json", "list", "sources"])?;
  let sinks = pactl_json(&["-f", "json", "list", "sinks"])?;
  if sources.is_empty() && sinks.is_empty() {
    return None;
  }
  let default_source = pactl_default("source");
  let default_sink = pactl_default("sink");
  // **系统同款名字**：GNOME 声音设置显示的是 PipeWire 的 `node.description`
  // （如「YUKUI D80 模拟立体声」「USB Audio Microphone」），而 `pactl -f json` 的
  // `description` 在 PipeWire 下常是 `(null)`；文本接口的 `Description:` 才是系统那份。
  // ——三只设备都叫「USB Audio」就是这么来的。
  // 系统声音面板的 `pa_source_info.description`（JSON 里常为 null，文本里有）。
  let source_desc = pactl_descriptions("sources");
  let sink_desc = pactl_descriptions("sinks");
  let mut errors = Vec::new();
  let mut best: HashMap<(String, String), (DeviceInfo, f64)> = HashMap::new();

  for (list, direction) in [(&sources, "input"), (&sinks, "output")] {
    for v in list {
      let name = v["name"].as_str().unwrap_or_default();
      if name.is_empty() {
        continue;
      }
      // 输入侧的 monitor source 是「某只输出的回环」，由 `loopback` 行单独处理，
      // 不混进普通麦克风列表（OBS 也把它们分在「音频输出采集」里）。
      if direction == "input" && (name.ends_with(".monitor") || !is_real_source(v)) {
        continue;
      }
      let Some(mut info) = pulse_device_info(v, direction, name) else {
        errors.push(format!(
          "跳过非 ALSA 输出设备 {name}（当前 cpal 输出后端只支持 ALSA 硬件）"
        ));
        continue;
      };
      if let Some(d) = if direction == "input" {
        source_desc.get(name)
      } else {
        sink_desc.get(name)
      } {
        if !d.is_empty() && d != "(null)" {
          info.name = d.clone();
        }
      }
      if direction == "input" {
        info.is_default = default_source.as_deref() == Some(name);
      } else {
        info.is_default = default_sink.as_deref() == Some(name);
      }
      let priority = v["properties"]["priority.session"]
        .as_str()
        .and_then(|s| s.parse::<f64>().ok())
        .unwrap_or(0.0);
      let key = (direction.to_string(), info.id.clone());
      match best.get(&key) {
        Some((old, old_pri)) if *old_pri >= priority && !info.is_default => {
          let _ = old;
        }
        _ => {
          best.insert(key, (info, priority));
        }
      }
    }
  }

  let mut devices: Vec<DeviceInfo> = best.into_values().map(|(d, _)| d).collect();

  // **PipeWire 共享输出**：Linux 的 ALSA `plughw` 是排他的（别的软件占着就会 busy）。
  // ALSA 的 `pipewire` PCM 走系统混音器，可以多路共存；把它作为「系统默认输出（共享）」
  // 提供给用户，默认指向系统 default sink。已经有人直连 ALSA 时仍可切回硬件项。
  if sinks
    .iter()
    .any(|v| v["driver"].as_str() == Some("PipeWire"))
  {
    let spec = default_sink
      .as_deref()
      .and_then(|n| sinks.iter().find(|v| v["name"].as_str() == Some(n)))
      .or_else(|| sinks.first())
      .and_then(|v| v["sample_specification"].as_str())
      .unwrap_or_default();
    for d in &mut devices {
      if d.direction == "output" {
        d.is_default = false;
      }
    }
    devices.push(DeviceInfo {
      id: "alsa:pipewire".to_string(),
      name: "系统默认输出（PipeWire 共享）".to_string(),
      host: "PulseAudio/PipeWire".to_string(),
      direction: "output",
      device_type: "output".to_string(),
      interface_type: "pipewire".to_string(),
      native: parse_spec(spec),
      is_default: true,
      selected: false,
      opened: false,
      refs: 0,
      mix_sources: Vec::new(),
    });
  }

  // 默认设备排最前（界面第一眼就能选）
  devices.sort_by_key(|d| (!d.is_default, d.direction, d.name.clone()));
  Some(DeviceList {
    enumerated_at: now_ms(),
    duration_ms: t0.elapsed().as_millis() as u64,
    hosts: vec!["PulseAudio/PipeWire".to_string()],
    devices,
    errors,
  })
}

/// Pulse 的 `monitor_of_sink` 为空/`n/a` 才是真输入；monitor source 是回环。
fn is_real_source(v: &Value) -> bool {
  match &v["monitor_source"] {
    Value::String(s) => s.is_empty() || s == "n/a",
    _ => true,
  }
}

/// 一条 Pulse source/sink → 一个 cpal 可打开的 `DeviceInfo`。
///
/// ALSA 设备映射到 `alsa:plughw:CARD=<card>,DEV=<device>`；非 ALSA 输入映射到
/// `pulse:<name>`（`capture::spawn` 走 parec）。非 ALSA 输出返回 None（见上）。
fn pulse_device_info(v: &Value, direction: &'static str, name: &str) -> Option<DeviceInfo> {
  let props = &v["properties"];
  let card = props["alsa.card"]
    .as_str()
    .or_else(|| props["api.alsa.pcm.card"].as_str());
  let device = props["alsa.device"]
    .as_str()
    .or_else(|| props["api.alsa.pcm.device"].as_str())
    .unwrap_or("0");
  // Linux 输入**一律用 Pulse source 名**：`capture::spawn` 走 `parec`，由 PipeWire/Pulse
  // 混音。直接开 ALSA `plughw` 是排他访问，浏览器/OBS 占着麦克风时必然 `busy`
  // （2026-10-25 用户实测）。输出暂时仍映射到 plughw（见 README/AGENTS 的待办）。
  let id = if direction == "input" {
    format!("pulse:{name}")
  } else {
    match card {
      Some(card) => format!("alsa:plughw:CARD={card},DEV={device}"),
      None => return None,
    }
  };
  let display = props["node.description"]
    .as_str()
    .filter(|s| !s.is_empty() && *s != "(null)")
    .or_else(|| {
      props["device.description"]
        .as_str()
        .filter(|s| !s.is_empty() && *s != "(null)")
    })
    .or_else(|| props["node.nick"].as_str())
    .or_else(|| v["description"].as_str())
    .filter(|s| !s.is_empty() && *s != "(null)")
    .unwrap_or(name);
  let native = parse_spec(v["sample_specification"].as_str().unwrap_or_default());
  Some(DeviceInfo {
    id,
    name: display.to_string(),
    host: "PulseAudio".to_string(),
    direction,
    device_type: direction.to_string(),
    interface_type: "pulse".to_string(),
    native,
    is_default: false,
    selected: false,
    opened: false,
    refs: 0,
    mix_sources: Vec::new(),
  })
}

/// `s16le 1ch 16000Hz` → NativeFormat（Pulse 报告的协商格式；不打开设备）。
fn parse_spec(spec: &str) -> Probe<NativeFormat> {
  let mut it = spec.split_whitespace();
  let fmt = it.next().unwrap_or_default();
  let ch = it
    .next()
    .and_then(|s| s.strip_suffix("ch"))
    .and_then(|s| s.parse::<u32>().ok())
    .unwrap_or(0);
  let rate = it
    .next()
    .and_then(|s| s.strip_suffix("Hz"))
    .and_then(|s| s.parse::<u32>().ok())
    .unwrap_or(0);
  if rate == 0 {
    return Probe::unavailable("Pulse 未报告采样格式（打开设备时读取）".to_string());
  }
  Probe::ok(NativeFormat {
    sample_rate: rate,
    channels: ch,
    sample_format: fmt.to_string(),
  })
}

/// 从 `pactl list sources|sinks` 文本里取 `Name` → `Description`。
/// 这是系统声音设置显示的同一份名字（`pa_source_info.description`），
/// `pactl -f json` 在 PipeWire 上会把 description 给成 `(null)`。
fn pactl_descriptions(kind: &str) -> HashMap<String, String> {
  let mut out = HashMap::new();
  let Some(text) = run_cmd("pactl", &["list", kind]) else {
    return out;
  };
  let mut name: Option<String> = None;
  let mut desc: Option<String> = None;
  fn flush(
    name: &mut Option<String>,
    desc: &mut Option<String>,
    out: &mut HashMap<String, String>,
  ) {
    if let (Some(n), Some(d)) = (name.take(), desc.take()) {
      out.insert(n, d);
    }
  }
  for line in text.lines() {
    let t = line.trim();
    if t.starts_with("Source #") || t.starts_with("Sink #") {
      flush(&mut name, &mut desc, &mut out);
      continue;
    }
    if let Some(v) = t.strip_prefix("Name:") {
      name = Some(v.trim().to_string());
    } else if let Some(v) = t.strip_prefix("Description:") {
      desc = Some(v.trim().to_string());
    }
  }
  flush(&mut name, &mut desc, &mut out);
  out
}

/// 从 `pactl list sources|sinks` 文本里取 `Name` → `Description`。
/// 这是系统声音设置显示的同一份名字（`pa_source_info.description`），JSON 输出拿不到。
fn pactl_json(args: &[&str]) -> Option<Vec<Value>> {
  let out = run_cmd("pactl", args)?;
  serde_json::from_str(&out).ok()
}

fn pactl_default(kind: &str) -> Option<String> {
  let arg = if kind == "source" {
    "get-default-source"
  } else {
    "get-default-sink"
  };
  if let Some(s) = run_cmd("pactl", &[arg]) {
    let t = s.trim();
    if !t.is_empty() {
      return Some(t.to_string());
    }
  }
  // 老版本 pactl 没有 get-default-*：退回 `pactl info`
  let info = run_cmd("pactl", &["info"])?;
  let key = if kind == "source" {
    "Default Source:"
  } else {
    "Default Sink:"
  };
  info
    .lines()
    .find_map(|l| l.trim().strip_prefix(key).map(|s| s.trim().to_string()))
}

/// 跑一个外部命令，**最多等 2 秒**（pactl 卡住时不能拖住设备刷新线程）。
fn run_cmd(cmd: &str, args: &[&str]) -> Option<String> {
  let mut child = Command::new(cmd)
    .env("LC_ALL", "C.UTF-8")
    .args(args)
    .stdin(Stdio::null())
    .stdout(Stdio::piped())
    .stderr(Stdio::null())
    .spawn()
    .ok()?;
  let mut stdout = child.stdout.take()?;
  let reader = std::thread::Builder::new()
    .name(format!("{cmd}-read"))
    .spawn(move || {
      let mut s = String::new();
      let _ = stdout.read_to_string(&mut s);
      s
    })
    .ok()?;
  let start = Instant::now();
  loop {
    match child.try_wait() {
      Ok(Some(_)) => break,
      Ok(None) if start.elapsed() < Duration::from_secs(2) => {
        std::thread::sleep(Duration::from_millis(20));
      }
      Ok(None) | Err(_) => {
        let _ = child.kill();
        let _ = child.wait();
        let _ = reader.join();
        return None;
      }
    }
  }
  reader.join().ok()
}

/// Linux 的 ALSA 枚举会把「插件 PCM」（null / 各种 rate converter / speex / jack / oss /
/// pipewire / pulse / upmix…）和同一张声卡的多种别名（`hw:` / `plughw:` / `sysdefault:` /
/// `front:`，外加按**名字**与按**序号**两套 `CARD=`）全列出来——本机一次枚举 108 条，
/// 选设备根本没法选。这里精简到「每个设备名一条」：
///   - 插件 PCM 一律丢弃（不是真实声卡）；
///   - 同一名字只留最可用的一条，优先级 `plughw`（自动格式转换）> `hdmi` > `hw` >
///     `sysdefault` > `front` > 其它；`default`（系统默认）始终保留。
pub(super) fn simplify(devices: Vec<DeviceInfo>) -> Vec<DeviceInfo> {
  // ALSA 插件 PCM 前缀（不是真实声卡）
  const PLUGIN: &[&str] = &[
    "null",
    "lavrate",
    "samplerate",
    "speexrate",
    "jack",
    "oss",
    "pipewire",
    "pulse",
    "speex",
    "upmix",
    "vdownmix",
    "dmix",
    "dsnoop",
    "usbstream",
    "iec958",
  ];
  fn rank(pcm: &str) -> Option<u32> {
    if pcm == "default" {
      return Some(0);
    }
    let kind = pcm.split(':').next().unwrap_or(pcm);
    if PLUGIN.contains(&kind) || kind.starts_with("surround") {
      return None;
    }
    Some(match kind {
      "plughw" => 1,
      "hdmi" => 2,
      "hw" => 3,
      "sysdefault" => 4,
      "front" => 5,
      _ => 6,
    })
  }
  let mut best: HashMap<(&'static str, String), usize> = HashMap::new();
  let mut best_rank: HashMap<(&'static str, String), u32> = HashMap::new();
  for (i, d) in devices.iter().enumerate() {
    let pcm = d.id.strip_prefix("alsa:").unwrap_or(&d.id);
    let Some(r) = rank(pcm) else { continue };
    let key = (d.direction, d.name.clone());
    if best_rank.get(&key).is_none_or(|&br| r < br) {
      best_rank.insert(key.clone(), r);
      best.insert(key, i);
    }
  }
  let mut idxs: Vec<usize> = best.into_values().collect();
  idxs.sort_unstable();
  idxs.into_iter().map(|i| devices[i].clone()).collect()
}

/// 可回环的 PipeWire/Pulse sinks（`pactl list sinks` 的 Name/Description）。
/// **必须 `LC_ALL=C`**：中文 locale 下 pactl 会输出「名称/描述」，按 `Name:` 解析会得到 0 个。
///
/// 入参是已枚举好的**设备**列表（Linux 的 sink 由 pactl 给，不在 cpal 列表里），保留是为了
/// 与其它平台同一签名；这里不用它，但仍**不该自己去枚举 cpal 设备**（那会和刷新抢驱动）。
pub(super) fn loopback_targets(_list: &DeviceList) -> Vec<LoopbackTarget> {
  let mut v = Vec::new();
  if let Ok(out) = std::process::Command::new("pactl")
    .env("LC_ALL", "C")
    .args(["list", "sinks"])
    .output()
  {
    let text = String::from_utf8_lossy(&out.stdout);
    let mut name: Option<String> = None;
    for line in text.lines() {
      let t = line.trim();
      if let Some(x) = t.strip_prefix("Name:") {
        name = Some(x.trim().to_string());
      } else if let Some(x) = t.strip_prefix("Description:")
        && let Some(n) = name.take()
      {
        v.push(LoopbackTarget {
          id: format!("loopback:{n}"),
          name: x.trim().to_string(),
        });
      }
    }
  }
  v
}
