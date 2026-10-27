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

//! 枚举本机可供局域网直连的网卡（NETWORK.md §4.1）。
//!
//! **为什么要枚举而不是只报一个地址**：原来的 `net_local_addr()` 用 UDP
//! `connect("8.8.8.8:80")` 的技巧，只能拿到**默认路由那张网卡**的地址。一台机器上
//! 同时有 Wi-Fi、有线、USB 共享、蓝牙共享、移动热点、有线共享、VPN、虚拟网卡时
//! （很常见），手机连的那张和默认路由那张经常不是同一张 —— 界面上打印的地址手机
//! 根本连不上，而且**没有任何办法换**。
//!
//! 所以这里列出全部非回环 IPv4，让界面上用下拉选；二维码与「复制」跟着选中的走。
//!
//! **过滤只按地址段**（回环 `127.` / 链路本地 `169.254.`），**不按网卡类型**：USB 共享、
//! 蓝牙共享、热点共享、VPN、虚拟网卡全都是能直连的可用网卡，按类型过滤反而会把
//! 有用的滤掉；`desc` 里带「VPN」「WSL」这类字样，用户自己认得出。
//!
//! 平台实现按 CONTRIBUTING.md 拆文件：`addrs_windows.rs` / `addrs_unix.rs` /
//! `addrs_other.rs`。都不引新依赖：Windows 用已在树里的 `windows` crate（只加 feature），
//! Unix 用已在树里的 `libc::getifaddrs`。

/// 一张网卡的一个 IPv4 地址。
#[derive(Clone, serde::Serialize)]
pub struct NetIface {
  /// 点分十进制地址，如 `192.168.1.5`
  pub ip: String,
  /// 网卡名（`WLAN` / `eth0` / `{GUID}`）
  pub name: String,
  /// 人类可读的网卡全名（Windows 的 `FriendlyName`；Unix 没有就回退到 name）
  pub desc: String,
  /// 是否是默认路由那张（界面默认选中它，保持原有行为）
  pub is_default: bool,
}

#[cfg(windows)]
#[path = "addrs_windows.rs"]
mod platform;

#[cfg(unix)]
#[path = "addrs_unix.rs"]
mod platform;

#[cfg(not(any(windows, unix)))]
#[path = "addrs_other.rs"]
mod platform;

/// 列出可直连的网卡：默认路由那张排第一，其余按 IP 排序（稳定，便于界面记住选择）。
pub fn list() -> Vec<NetIface> {
  let mut v = platform::enumerate();
  v.sort_by(|a, b| match (a.is_default, b.is_default) {
    (true, false) => std::cmp::Ordering::Less,
    (false, true) => std::cmp::Ordering::Greater,
    _ => a.ip.cmp(&b.ip),
  });
  v
}

/// 默认路由那张的地址（没选过、或选的那张已经不属于本机时用它）。
pub fn default_ip() -> String {
  list()
    .into_iter()
    .find(|n| n.is_default)
    .map(|n| n.ip)
    .unwrap_or_default()
}

#[cfg(test)]
mod tests {
  use super::*;

  #[test]
  fn 列表不含回环与链路本地() {
    for n in list() {
      assert!(!n.ip.starts_with("127."), "回环不该出现：{}", n.ip);
      assert!(!n.ip.starts_with("169.254."), "链路本地不该出现：{}", n.ip);
      assert!(!n.ip.is_empty());
    }
  }

  #[test]
  fn 默认项排第一() {
    let v = list();
    let any_default = v.iter().any(|n| n.is_default);
    if any_default && !v.is_empty() {
      assert!(v[0].is_default, "默认路由那张必须排第一");
    }
  }
}
