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

//! 服务端 mDNS 广告（NETWORK.md §4.1.11）。
//!
//! **为什么客户端不需要填地址**：局域网里就一台电脑，装了多个网卡之后「是哪一台」对用户
//! 是没意义的——他就是想连**这台正在跑的电脑**。所以客户端启动后自动发现即可，手动输入只
//! 作为发现失败时的兜底（换 Wi-Fi、组播被交换机挡了）。
//!
//! **广告跟着服务启停，不做常驻**。这是刻意的：常驻广告会让「电脑上明明没跑服务，手机却
//! 能搜到」发生，而连过去必然失败——那比搜不到更难排查。「服务没开 → 搜不到」是**预期
//! 行为**，界面上要写明，不要让人以为坏了。
//!
//! **TXT 只放 `v` 与 `proto`，不放 host**：mDNS 本来就是用来找 host 的（SRV 记录带地址），
//! 再放一遍是冗余，还会让两处不一致。`proto` 是给客户端判断协议版本用的——版本不符时直接
//! 告诉用户「客户端版本太旧」，而不是连上去再被服务端断开。
//!
//! 实例名带网卡 IP：同一台电脑有多张网卡时，每张各广播一条、实例名各不相同。这样「手机连的
//! 是哪张卡」在发现结果里就一目了然，省得用户回去猜。

use std::sync::Mutex;

/// mDNS 服务类型（DNS SRV 按 RFC 6763 约定小写）。
pub const SERVICE_TYPE: &str = "_purevox._tcp.local.";

/// 广告句柄。`ServiceDaemon` 自带一个后台线程，`shutdown()` 才真正退出；不关的话进程退出
/// 时会留下一个还在组播的后台线程（Windows 上调试表现是进程不退出）。
static REG: Mutex<Option<mdns_sd::ServiceDaemon>> = Mutex::new(None);

/// 注册广告。对每个可直连的网卡注册一条。**幂等**：重复调用先注销旧的。
///
/// `ips` 为空时什么都不做（没有网卡就没有可发现的服务，报错反而会让人以为程序坏了）。
pub fn advertise(ips: &[String], port: u16) -> Result<String, String> {
  unregister();
  if ips.is_empty() {
    return Ok("没有可直连的网卡，不广播".into());
  }

  let daemon = mdns_sd::ServiceDaemon::new().map_err(|e| format!("起 mDNS 守护线程失败：{e}"))?;
  let host = hostname();
  let proto = super::proto::PROTO.to_string();
  // 必须是 `&[(K, V)]`（切片），传数组 / `TxtProperties` 都不满足 mdns-sd 的 trait bound
  let props: Vec<(&str, &str)> = vec![("v", crate::version::VERSION), ("proto", &proto)];

  let mut n = 0usize;
  let mut first_err = String::new();
  for ip in ips {
    // 实例名带上 IP：多网卡时在手机上一眼能认出「连哪张卡」
    let name = format!("PureVox {ip}");
    // 一张网卡失败（例如它的 mDNS 被系统策略禁了）不该让其余网卡都不广播
    match mdns_sd::ServiceInfo::new(SERVICE_TYPE, &name, &host, ip, port, props.as_slice()) {
      Ok(info) => match daemon.register(info) {
        Ok(()) => n += 1,
        Err(e) => first_err = format!("{ip}：{e}"),
      },
      Err(e) => first_err = format!("{ip}：{e}"),
    }
  }

  *REG.lock().map_err(|_| "mDNS 句柄锁已中毒".to_string())? = Some(daemon);
  if n == 0 {
    Ok(if first_err.is_empty() {
      "mDNS 起来了但一张网卡都没广播出去（可能被系统防火墙挡了）".into()
    } else {
      format!("mDNS 起来了但一张网卡都没广播出去：{first_err}")
    })
  } else {
    Ok(format!("已广播 {SERVICE_TYPE}（{n} 张网卡，端口 {port}）"))
  }
}

/// 注销广告并关掉守护线程（服务停止时调）。
pub fn unregister() {
  let Ok(mut slot) = REG.lock() else { return };
  if let Some(d) = slot.take() {
    let _ = d.shutdown();
  }
}

/// 主机名。**必须以 `.local.` 结尾**——mDNS 的主机名一律属于 `.local.` 域，`ServiceInfo::new`
/// 会直接拒绝别的后缀（报 `Hostname must end with '.local.'`）。拿不到名字就用 `PureVox`，
/// 它在发现结果里只是个可读标签，不影响连接（真正定位靠 SRV 里的地址）。
fn hostname() -> String {
  let raw = std::env::var("COMPUTERNAME")
    .or_else(|_| std::env::var("HOSTNAME"))
    .ok()
    .map(|s| s.trim().to_string())
    .filter(|s| !s.is_empty())
    .unwrap_or_else(|| "PureVox".into());
  // 已经有 .local. 后缀就别重复加
  if raw.to_ascii_lowercase().ends_with(".local.") {
    raw
  } else {
    format!("{raw}.local.")
  }
}

#[cfg(test)]
mod tests {
  use super::*;

  #[test]
  fn 服务名符合_dns_sd_约定() {
    // 小写、下划线前缀、以 .local. 结尾 —— 客户端按这个字符串过滤，三处必须一致
    assert!(SERVICE_TYPE.starts_with("_"));
    assert!(SERVICE_TYPE.ends_with(".local."));
    assert_eq!(SERVICE_TYPE, "_purevox._tcp.local.");
    assert!(SERVICE_TYPE.chars().filter(|c| *c == '_').count() == 2);
  }

  /// 主机名**必须**带 `.local.`：mDNS 的主机名属于 `.local.` 域，`ServiceInfo::new`
  /// 会直接拒绝别的后缀。曾经漏了后缀，结果是每张网卡都注册失败、界面上只显示
  /// 「一张网卡都没广播出去」，而真实原因（主机名不合法）被吞在中间。
  #[test]
  fn 主机名以_local_结尾() {
    let h = hostname();
    assert!(h.to_ascii_lowercase().ends_with(".local."), "{h}");
    assert!(!h.to_ascii_lowercase().ends_with(".local..local."), "{h}");
    assert!(!h.contains(' '), "主机名不能有空格：{h}");
  }

  /// 用真实构造路径验证参数形状合法（跑一个 RFC 5737 保留段地址，不会真广播出去）。
  #[test]
  fn 能构造出合法的服务记录() {
    let daemon = mdns_sd::ServiceDaemon::new().expect("守护线程");
    let proto = super::super::proto::PROTO.to_string();
    let props: Vec<(&str, &str)> = vec![("v", crate::version::VERSION), ("proto", &proto)];
    let info = mdns_sd::ServiceInfo::new(
      SERVICE_TYPE,
      "PureVox 192.0.2.1",
      &hostname(),
      "192.0.2.1",
      59123,
      props.as_slice(),
    );
    assert!(info.is_ok(), "{:?}", info.err());
    let info = info.unwrap();
    // TXT 只放 v 与 proto，不放 host —— mDNS 本来就是找 host 的，放了是冗余且会有两处不一致。
    // `TxtProperty` 的 `key` 字段是私有的，只能靠 `Display` 看到键值对
    let txt = info.get_properties();
    assert_eq!(txt.len(), 2, "TXT 只该有两条：{txt:?}");
    let rendered: Vec<String> = txt.iter().map(|p| p.to_string()).collect();
    assert!(rendered.iter().any(|s| s.starts_with("v=")), "{rendered:?}");
    assert!(
      rendered.iter().any(|s| s.starts_with("proto=")),
      "{rendered:?}"
    );
    assert!(
      !rendered.iter().any(|s| s.starts_with("host=")),
      "TXT 不该放 host（mDNS 本来就是找 host 的）：{rendered:?}"
    );
    assert!(
      info.get_fullname().contains("_purevox._tcp"),
      "{}",
      info.get_fullname()
    );
    let _ = daemon.shutdown();
  }

  /// 没有网卡时**不能报错**：那会让「没连网」表现为「程序坏了」。
  #[test]
  fn 没有网卡时不报错() {
    assert!(advertise(&[], 59123).is_ok());
  }

  /// 幂等：连着调两次不能 panic 也不能泄漏线程（第二次会把第一组的守护线程关掉）。
  #[test]
  fn 重复广播是幂等的() {
    let r1 = advertise(&["192.0.2.1".into()], 59123);
    let r2 = advertise(&["192.0.2.1".into()], 59123);
    assert!(r1.is_ok() && r2.is_ok(), "{r1:?} / {r2:?}");
    unregister();
    // 关掉之后再调一次不会炸（槽位是 None）
    unregister();
  }
}
