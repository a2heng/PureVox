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

//! Unix（Linux / macOS）网卡枚举：`libc::getifaddrs`。
//!
//! `libc` 已经在树里（给 `opus_sys` 运行时 `dlopen` 用的），**不引新依赖**。

use std::net::{Ipv4Addr, UdpSocket};

use super::NetIface;

/// 默认路由那张网卡：UDP 连一个公网地址，取本地地址 —— 不发包也不需要真的连上。
fn default_ip_via_udp() -> String {
  for target in ["8.8.8.8:80", "1.1.1.1:80"] {
    if let Ok(s) = UdpSocket::bind("0.0.0.0:0")
      && s.connect(target).is_ok()
      && let Ok(a) = s.local_addr()
    {
      return a.ip().to_string();
    }
  }
  String::new()
}

pub fn enumerate() -> Vec<NetIface> {
  let dflt = default_ip_via_udp();
  let mut out = Vec::new();
  let mut head: *mut libc::ifaddrs = std::ptr::null_mut();
  // SAFETY: `getifaddrs` 成功时 `head` 指向链表；`freeifaddrs` 收尾。失败时不动 head。
  if unsafe { libc::getifaddrs(&mut head) } != 0 || head.is_null() {
    return out;
  }
  unsafe {
    let mut cur = head;
    while !cur.is_null() {
      let ifa = &*cur;
      cur = ifa.ifa_next;
      if ifa.ifa_addr.is_null() {
        continue;
      }
      let fam = (*ifa.ifa_addr).sa_family as i32;
      if fam != libc::AF_INET {
        continue;
      }
      if ifa.ifa_flags as i32 & libc::IFF_LOOPBACK != 0 {
        continue;
      }
      // 两处坑，Windows 上编译不到（这边是 `addrs_windows.rs`），只有跑 Linux/macOS 才暴露：
      // 1. `ifa_addr` 在 Linux 是 `*mut sockaddr`、macOS 是 `*const sockaddr`，而指针**不能**
      //    一步 `as` 成另一种结构体（rustc E0605：as 只允许原始类型之间）——必须走 `.cast()`。
      // 2. `sin_addr.s_addr` 是**网络字节序**存进内存的 32 位；`Ipv4Addr::from(u32)` 按
      //    大端解释它，在小端机上等于把地址四段倒过来（192.168.1.5 会读成 5.1.168.192）。
      //    `to_ne_bytes()` 直接按内存顺序取字节才是对的。
      let sin = &*ifa.ifa_addr.cast::<libc::sockaddr_in>();
      let ip = Ipv4Addr::from(sin.sin_addr.s_addr.to_ne_bytes()).to_string();
      if ip.starts_with("127.") || ip.starts_with("169.254.") {
        continue;
      }
      let mut name = String::new();
      if !ifa.ifa_name.is_null() {
        name =
          String::from_utf8_lossy(std::ffi::CStr::from_ptr(ifa.ifa_name).to_bytes()).into_owned();
      }
      out.push(NetIface {
        is_default: ip == dflt,
        ipv6: Vec::new(),
        ip,
        desc: name.clone(),
        name,
      });
    }
    libc::freeifaddrs(head);
  }
  out
}
