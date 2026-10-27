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

//! Windows 网卡枚举：`GetAdaptersAddresses`。
//!
//! 用已在树里的 `windows` crate（只加 `Win32_NetworkManagement_IpHelper` /
//! `Win32_NetworkManagement_Ndis` 两个 feature），**不引新依赖、不新增 C 编译链**
//! —— 和 opus 运行时加载是同一套取舍（DEBUG.md §1.2）。

use std::net::{Ipv4Addr, UdpSocket};

use windows::Win32::Foundation::ERROR_BUFFER_OVERFLOW;
use windows::Win32::NetworkManagement::IpHelper::{
  GAA_FLAG_SKIP_ANYCAST, GAA_FLAG_SKIP_DNS_SERVER, GAA_FLAG_SKIP_MULTICAST, GetAdaptersAddresses,
  IP_ADAPTER_ADDRESSES_LH,
};
use windows::Win32::Networking::WinSock::{
  ADDRESS_FAMILY, AF_INET, SOCKADDR, SOCKADDR_IN, WSADATA, WSAStartup,
};

use super::NetIface;

/// `GetAdaptersAddresses` 是 **Winsock 函数，必须先 `WSAStartup`**。
///
/// 不调会拿到 `ERROR_INVALID_PARAMETER (87)`，枚举结果空得像一台没网卡的机器 —— 界面
/// 上子网下拉只剩「自动」一项，用户根本选不了别的网卡。而且**不能指望别人替我们初始化
/// 过**：本进程可能整个生命周期都不碰 socket（服务没启动、会话没跑），`WSAStartup`
/// 要等到 axum 建监听或 cpal 开流时才发生，那时机往往晚于第一次枚举。
///
/// `WSACleanup` 不调：Winsock 的引用计数由每次 `WSAStartup` 自增，漏掉反向调用只会让
/// 计数偏高（不影响功能），而多调一次会真的把别人已经建好的 socket 拆了。
fn ensure_winsock() {
  static ONCE: std::sync::Once = std::sync::Once::new();
  ONCE.call_once(|| {
    // SAFETY: 只往 `data`（本地栈变量）里写；返回码不用管——初始化真失败的话下面的
    // `GetAdaptersAddresses` 自己会报错，我们据此给空列表，界面上显示「不可用 + 原因」。
    unsafe {
      let mut data = WSADATA::default();
      let _ = WSAStartup(0x0202, &mut data); // 要求 2.2
    }
  });
}

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
  ensure_winsock();
  let dflt = default_ip_via_udp();
  let mut out = Vec::new();

  // 参数顺序：**第 1 个是地址族，第 2 个才是 flag**。写反了（拿缓冲区长度当前 1 个）会
  // 得到 `ERROR_INVALID_PARAMETER (87)`，而且不报别的错——就是一张空网卡列表，看起来
  // 像「这台机器没网卡」。
  let flags = GAA_FLAG_SKIP_ANYCAST | GAA_FLAG_SKIP_MULTICAST | GAA_FLAG_SKIP_DNS_SERVER;
  let mut size: u32 = 16 * 1024;
  let mut buf = vec![0u8; size as usize];
  let mut rc: u32 = unsafe {
    GetAdaptersAddresses(
      AF_INET.0 as u32,
      flags,
      None,
      Some(buf.as_mut_ptr() as *mut IP_ADAPTER_ADDRESSES_LH),
      &mut size,
    )
  };
  if rc == ERROR_BUFFER_OVERFLOW.0 {
    buf = vec![0u8; size as usize];
    rc = unsafe {
      GetAdaptersAddresses(
        AF_INET.0 as u32,
        flags,
        None,
        Some(buf.as_mut_ptr() as *mut IP_ADAPTER_ADDRESSES_LH),
        &mut size,
      )
    };
  }
  // 返回的是 u32 的 WIN32_ERROR，0 即成功（S_OK 是 i32 的 HRESULT，不能直接比）
  if rc != 0 {
    return out;
  }

  let mut cur = buf.as_ptr() as *mut IP_ADAPTER_ADDRESSES_LH;
  while !cur.is_null() {
    // SAFETY: 链表由 `GetAdaptersAddresses` 分配，遍历到 `Next == null` 为止。
    let a = unsafe { &*cur };
    let name = utf8z(a.AdapterName.0 as *const i8);
    let friendly = a.FriendlyName.as_ptr();
    let desc = if friendly.is_null() {
      name.clone()
    } else {
      utf16(friendly)
    };

    let mut ua = a.FirstUnicastAddress;
    while !ua.is_null() {
      // SAFETY: 同上，`Next` 走到 null。
      let u = unsafe { &*ua };
      ua = u.Next;
      if u.Address.lpSockaddr.is_null() {
        continue;
      }
      let sa = unsafe { &*u.Address.lpSockaddr };
      if sa.sa_family != ADDRESS_FAMILY(AF_INET.0) {
        continue;
      }
      // sockaddr_in 的 sin_addr 在结构体偏移 4 处（sa_family 占 2 字节，端口 2 字节）
      let sin = unsafe { &*(sa as *const SOCKADDR as *const SOCKADDR_IN) };
      // `S_addr` 是**按网络字节序躺在内存里的 u32**，不能直接喂 `Ipv4Addr::from`：
      // `from(u32)` 按大端解释，于是每台机器都会拿到反过来的地址
      // （172.26.38.56 → 56.38.26.172），而反过来的地址照样长得像一个 IP，连回环过滤都
      // 滤不掉。`to_ne_bytes()` 拿到的就是内存里那 4 个字节本身，两种字节序下都对。
      let b = unsafe { sin.sin_addr.S_un.S_addr }.to_ne_bytes();
      let ip = Ipv4Addr::new(b[0], b[1], b[2], b[3]).to_string();
      // 回环与链路本地手机都连不上，直接不列
      if ip.starts_with("127.") || ip.starts_with("169.254.") {
        continue;
      }
      out.push(NetIface {
        is_default: ip == dflt,
        ip,
        name: name.clone(),
        desc: desc.clone(),
      });
    }
    cur = a.Next;
  }
  out
}

fn utf8z(p: *const i8) -> String {
  if p.is_null() {
    return String::new();
  }
  unsafe { std::ffi::CStr::from_ptr(p) }
    .to_string_lossy()
    .into_owned()
}

fn utf16(p: *const u16) -> String {
  if p.is_null() {
    return String::new();
  }
  unsafe {
    let mut len = 0usize;
    while *p.add(len) != 0 {
      len += 1;
    }
    String::from_utf16_lossy(std::slice::from_raw_parts(p, len))
  }
}
