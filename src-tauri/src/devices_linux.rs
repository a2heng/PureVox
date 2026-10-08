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

//! Linux 的设备面：ALSA 枚举精简 + 可回环的 PipeWire/Pulse sinks。

use std::collections::HashMap;

use super::{DeviceInfo, LoopbackTarget};

/// Linux 的回环是 Pulse sink 的 monitor：**不接受裸 ALSA 设备 ID**。
pub(super) const LOOPBACK_ACCEPTS_DEVICE_ID: bool = false;

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
pub(super) fn loopback_targets() -> Vec<LoopbackTarget> {
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
