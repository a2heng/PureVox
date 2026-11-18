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

//! Linux 防火墙放行提示（**只读探测 + 给可复制命令，不代为执行**）。
//!
//! Linux 的 ufw / firewalld 默认拒绝入站，而服务绑 `0.0.0.0`；本机
//! `https://127.0.0.1:<port>` 永远能开，手机却完全连不上。这里**不读规则内容、
//! 不提权、不执行任何命令**，只回答两件事：
//!
//! 1. 有没有可识别的防火墙（ufw / firewalld）在跑；
//! 2. 从世界可读的配置里能不能看出这个端口已放行（看不出就 `None`，不猜）。
//!
//! 界面据此给**两行可复制命令**：一行放行、一行回退，由用户自己决定是否执行。

/// 探测到的防火墙类型。
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Kind {
  Ufw,
  Firewalld,
}

impl Kind {
  pub fn name(self) -> &'static str {
    match self {
      Kind::Ufw => "ufw",
      Kind::Firewalld => "firewalld",
    }
  }
}

/// 探测结果。
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Status {
  pub kind: Kind,
  /// `Some(true)` = 看到放行规则；`Some(false)` = 没看到；`None` = 读不到 / 判断不了。
  pub allowed: Option<bool>,
}

#[cfg(target_os = "linux")]
#[path = "firewall_linux.rs"]
mod platform;

#[cfg(not(target_os = "linux"))]
#[path = "firewall_other.rs"]
mod platform;

/// 探测本机防火墙（没有可识别的防火墙时返回 `None`）。
pub fn status(port: u16) -> Option<Status> {
  platform::status(port)
}

/// 放行命令与回退命令（一行一条，可复制；**不执行**）。
pub fn commands(kind: Kind, port: u16) -> (String, String) {
  match kind {
    Kind::Ufw => (
      format!("sudo ufw allow {port}/tcp"),
      format!("sudo ufw delete allow {port}/tcp"),
    ),
    Kind::Firewalld => (
      format!("sudo firewall-cmd --permanent --add-port={port}/tcp && sudo firewall-cmd --reload"),
      format!(
        "sudo firewall-cmd --permanent --remove-port={port}/tcp && sudo firewall-cmd --reload"
      ),
    ),
  }
}

#[cfg(test)]
mod tests {
  use super::*;

  /// 两行命令必须成对、并且回退确实删的是同一端口（不能一边 allow 一边删别的端口）。
  #[test]
  fn 放行与回退命令成对() {
    for kind in [Kind::Ufw, Kind::Firewalld] {
      let (open, revert) = commands(kind, 59123);
      assert!(open.contains("59123"), "{kind:?}: {open}");
      assert!(revert.contains("59123"), "{kind:?}: {revert}");
      assert_ne!(open, revert);
      assert!(!open.contains('\n') && !revert.contains('\n'), "一行一条");
    }
    let (open, revert) = commands(Kind::Ufw, 59123);
    assert_eq!(open, "sudo ufw allow 59123/tcp");
    assert_eq!(revert, "sudo ufw delete allow 59123/tcp");
  }
}
