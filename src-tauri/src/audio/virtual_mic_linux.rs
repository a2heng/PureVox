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

//! Linux 虚拟麦克风（PipeWire）实现。架构说明见 [`super`]（`virtual_mic.rs`）。
//!
//! 设备名与显示名**与 legacy 逐字一致**（改了会破坏别的软件里已保存的设备选择）。

use std::process::Command;
use std::time::Duration;

use super::Status;

const SINK: &str = "purevox_out";
const SOURCE: &str = "purevox_out.monitor";
const MIC: &str = "purevox_mic";
const SINK_LABEL: &str = "PureVox out";
const MIC_LABEL: &str = "PureVox mic";

/// `command -v <cmd>` 是否成功。
fn have(cmd: &str) -> bool {
  Command::new("sh")
    .arg("-c")
    .arg(format!("command -v {cmd}"))
    .output()
    .map(|o| o.status.success())
    .unwrap_or(false)
}

/// PipeWire 中指定 node.name 的本地 object id；不存在返回 None。
fn node_id(name: &str) -> Option<u64> {
  let out = Command::new("pw-cli").args(["ls", "Node"]).output().ok()?;
  let text = String::from_utf8_lossy(&out.stdout);
  let mut cur: Option<u64> = None;
  for line in text.lines() {
    let t = line.trim();
    if t.starts_with("id ") && t.contains("Node") {
      cur = t
        .split_whitespace()
        .nth(1)
        .and_then(|s| s.trim_end_matches(',').parse().ok());
    }
    if t.contains(name) {
      return cur;
    }
  }
  None
}

fn stderr(out: &std::process::Output) -> String {
  let e = String::from_utf8_lossy(&out.stderr).trim().to_string();
  if e.is_empty() {
    format!("退出码 {}", out.status.code().unwrap_or(-1))
  } else {
    e
  }
}

pub fn status() -> Result<Status, String> {
  if !have("pw-cli") {
    return Err("找不到 pw-cli：虚拟麦克风需要 PipeWire".into());
  }
  Ok(Status {
    sink: node_id(SINK).is_some(),
    source: node_id(MIC).is_some(),
    pw_cli: true,
    pactl: have("pactl"),
  })
}

pub fn create() -> Result<Status, String> {
  if !have("pw-cli") {
    return Err("找不到 pw-cli：虚拟麦克风需要 PipeWire".into());
  }
  if node_id(SINK).is_none() {
    let spec = format!(
      "{{ factory.name=support.null-audio-sink node.name={} media.class=Audio/Sink \
       object.linger=true audio.position=[MONO] monitor.mode=disabled node.description=\"{}\" }}",
      SINK, SINK_LABEL
    );
    let out = Command::new("pw-cli")
      .args(["create-node", "adapter", &spec])
      .output()
      .map_err(|e| format!("执行 pw-cli 失败：{e}"))?;
    if !out.status.success() {
      return Err(format!("创建虚拟 sink 失败：{}", stderr(&out)));
    }
    std::thread::sleep(Duration::from_millis(500));
  }
  if node_id(SINK).is_none() {
    return Err("虚拟 sink 创建后未就绪".into());
  }

  // 真源：无 pactl 时只保留 monitor 出口（不算失败）。
  if node_id(MIC).is_none() && have("pactl") {
    let out = Command::new("pactl")
      .args([
        "load-module",
        "module-remap-source",
        &format!("master={SOURCE}"),
        &format!("source_name={MIC}"),
        "channel_map=mono",
        &format!("source_properties=device.description={MIC_LABEL}"),
      ])
      .output()
      .map_err(|e| format!("执行 pactl 失败：{e}"))?;
    if !out.status.success() {
      return Err(format!("创建虚拟麦克风真源失败：{}", stderr(&out)));
    }
    std::thread::sleep(Duration::from_millis(600));
    if node_id(MIC).is_none() {
      return Err("虚拟麦克风真源创建后未就绪".into());
    }
  }
  status()
}

pub fn remove() -> Result<Status, String> {
  // 先卸 remap 模块（按 pactl 模块列表定位）。
  if have("pactl")
    && let Ok(out) = Command::new("pactl")
      .args(["list", "short", "modules"])
      .output()
  {
    let text = String::from_utf8_lossy(&out.stdout);
    for line in text.lines() {
      if line.contains(MIC)
        && let Some(id) = line.split_whitespace().next()
      {
        let _ = Command::new("pactl").args(["unload-module", id]).output();
        break;
      }
    }
  }
  // 防御：模块卸载后仍未消失的节点直接 destroy；最后销毁 sink。
  for name in [MIC, SINK] {
    if let Some(id) = node_id(name) {
      let _ = Command::new("pw-cli")
        .args(["destroy", &id.to_string()])
        .output();
    }
  }
  status()
}
