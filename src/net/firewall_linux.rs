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

//! Linux 探测实现：只读世界可读配置，不读规则正文、不提权。
//!
//! ufw 的 `/etc/ufw/user.rules` 通常是 `0640 root:root`，普通用户读不到 ——
//! 那就返回 `None`（界面写「需要 root 才能确认」），并照常给出可复制命令。
//! 能读到也只看「有没有这个端口的 ACCEPT」，不把规则内容带出本模块。

use std::path::Path;
use std::process::Command;
use std::sync::Mutex;
use std::time::{Duration, Instant};

use super::{Kind, Status};

static CACHE: Mutex<Option<(Instant, u16, Option<Status>)>> = Mutex::new(None);
const TTL: Duration = Duration::from_secs(5);

pub fn status(port: u16) -> Option<Status> {
  if let Ok(g) = CACHE.lock()
    && let Some((at, p, s)) = *g
    && p == port
    && at.elapsed() < TTL
  {
    return s;
  }
  let s = detect(port);
  if let Ok(mut g) = CACHE.lock() {
    *g = Some((Instant::now(), port, s));
  }
  s
}

fn detect(port: u16) -> Option<Status> {
  if ufw_enabled() {
    let allowed = if ufw_default_input_policy() == Some("ACCEPT") {
      Some(true)
    } else {
      ufw_port_allowed(port)
    };
    return Some(Status {
      kind: Kind::Ufw,
      allowed,
    });
  }
  if firewalld_active() {
    return Some(Status {
      kind: Kind::Firewalld,
      allowed: firewalld_port_allowed(port),
    });
  }
  None
}

fn ufw_enabled() -> bool {
  let Ok(t) = std::fs::read_to_string("/etc/ufw/ufw.conf") else {
    return false;
  };
  t.lines()
    .any(|l| l.trim().eq_ignore_ascii_case("ENABLED=yes"))
}

fn ufw_default_input_policy() -> Option<&'static str> {
  let t = std::fs::read_to_string("/etc/default/ufw").ok()?;
  for l in t.lines() {
    let Some(v) = l.trim().strip_prefix("DEFAULT_INPUT_POLICY=") else {
      continue;
    };
    return match v.trim().trim_matches('"').trim_matches('\'') {
      "ACCEPT" => Some("ACCEPT"),
      "DROP" => Some("DROP"),
      "REJECT" => Some("REJECT"),
      _ => None,
    };
  }
  None
}

/// `/etc/ufw/user.rules` 里有没有 `--dport <port>` + `-j ACCEPT`。读不到 → `None`。
fn ufw_port_allowed(port: u16) -> Option<bool> {
  let t = std::fs::read_to_string("/etc/ufw/user.rules").ok()?;
  let dport = format!("--dport {port}");
  Some(
    t.lines()
      .any(|l| l.contains(&dport) && l.contains("-j ACCEPT")),
  )
}

fn firewalld_active() -> bool {
  Command::new("systemctl")
    .args(["is-active", "--quiet", "firewalld"])
    .status()
    .map(|s| s.success())
    .unwrap_or(false)
}

/// 永久区 + 运行时区（`/etc/firewalld/zones`、`/run/firewalld/zones`）里有没有该端口。
fn firewalld_port_allowed(port: u16) -> Option<bool> {
  let mut readable = false;
  let needle = format!("port=\"{port}\"");
  for dir in ["/etc/firewalld/zones", "/run/firewalld/zones"] {
    let Ok(rd) = std::fs::read_dir(Path::new(dir)) else {
      continue;
    };
    for e in rd.flatten() {
      let p = e.path();
      if p.extension().and_then(|s| s.to_str()) != Some("xml") {
        continue;
      }
      let Ok(t) = std::fs::read_to_string(&p) else {
        continue;
      };
      readable = true;
      if t
        .lines()
        .any(|l| l.contains(&needle) && l.contains("protocol=\"tcp\""))
      {
        return Some(true);
      }
    }
  }
  readable.then_some(false)
}
