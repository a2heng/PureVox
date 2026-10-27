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

//! 网络子系统（DESIGN.md §4.1）：手机 ⇄ 电脑。
//!
//! 三条互不依赖的能力，共用一个 WebSocket 服务：
//! 1. `remote_mic` 输入行 —— 网络麦克风：手机麦克风当电脑麦克风（进引擎全链，可降噪）
//! 2. `remote_speaker` 输出行 —— 网络扬声器：手机当电脑音箱
//! 3. 远程输入 —— 手机输入法打字（`text`）+ 手机实体键当全尺寸键盘（`key`），默认关闭
//!
//! **平台**：传输与编解码跨平台（axum + tokio + 运行时加载的 libopus）；
//! 只有关键词注入是平台相关（Windows `SendInput`，其余平台明确报不可用，见 `keys`）。

pub mod addrs;
/// 网络回环测试（只 `cfg(test)` 编译，不进产物）。
#[cfg(test)]
mod diag;
pub mod hub;
pub mod keymap;
pub mod keys;
#[cfg(test)]
mod loopback;
pub mod mdns;
pub mod opus_sys;
pub mod proto;
pub mod rt;
pub mod server;
pub mod tls;
pub mod webclient;

use std::sync::{Arc, Mutex, OnceLock};

pub use hub::NetHub;

static HUB: OnceLock<Arc<NetHub>> = OnceLock::new();

/// 全局网络中枢（与测试音同款全局单例：只有一个入站源与一组出站订阅）。
pub fn hub() -> Arc<NetHub> {
  HUB.get_or_init(|| Arc::new(NetHub::new())).clone()
}

/// 服务监听地址：绑**所有**网卡的 59123（DESIGN.md §4.1）。
///
/// 这是用户可见功能，必须让手机能连；与 AGENTS §1.2 只绑 127.0.0.1 的本机调试接口
/// 是两回事。远程输入另有独立开关（默认关）。
const BIND_ADDR: [u8; 4] = [0, 0, 0, 0];

/// 监听任务句柄。**必须留着**：以前 `stop()` 只清状态位，监听任务还在跑，于是
/// 「停止服务」根本不生效，改端口后重启也必然 `address in use`——界面按钮看着能用，
/// 其实什么也没做。重启服务要能真的换掉监听，就靠 abort 这个任务。
static TASK: Mutex<Option<tauri::async_runtime::JoinHandle<()>>> = Mutex::new(None);

/// 启动服务（幂等）。返回绑定结果说明；端口占用等失败原因进调试接口，不静默失败。
pub fn start() -> Result<String, String> {
  let h = hub();
  if h.is_running() {
    return Ok(format!("已在运行（端口 {}）", h.port()));
  }
  // 缺 opus.dll 时先报出来：网络功能没有编解码器就是死的
  opus_sys::probe()?;
  // 上一次没走 stop()（比如启动后崩过）时先收干净，否则端口还占着
  stop();

  // 证书的 SAN 要盖住**当前**全部可直连网卡。漏了某一张的话，手机报的是「证书域名不
  // 匹配」——那是用户完全看不懂、也没法自己解决的报错（详见 `tls` 模块的注释）
  let ips: Vec<String> = addrs::list().into_iter().map(|n| n.ip).collect();
  let tls = tls::load_or_create(&ips).map_err(|e| format!("准备 HTTPS 证书失败：{e}"))?;
  let ca_pem = tls.ca_pem.clone();
  let ca_replaced = tls.ca_replaced;
  let acceptor = tokio_rustls::TlsAcceptor::from(std::sync::Arc::new(tls.config));

  let app = server::router(h.clone(), ca_pem.clone());
  let port = h.port();
  let addr = std::net::SocketAddr::from((BIND_ADDR, port));
  let hh = h.clone();
  let handle = tauri::async_runtime::spawn(async move {
    match tokio::net::TcpListener::bind(addr).await {
      Ok(listener) => {
        hh.set_running(true);
        hh.set_ca_pem(ca_pem);
        // 广告跟着服务起停：服务没开就搜不到，这是**预期行为**（界面要写明），
        // 总比「搜到了却连不上」好排查
        match mdns::advertise(&ips, port) {
          Ok(m) => hh.set_last_error(m),
          Err(e) => hh.set_last_error(format!("mDNS 广播失败：{e}")),
        }
        loop {
          // 单次 accept 失败（连接被 RST、fd 紧张）不该拖垮整个服务
          let Ok((tcp, _peer)) = listener.accept().await else {
            continue;
          };
          let acceptor = acceptor.clone();
          let app = app.clone();
          let hh = hh.clone();
          tauri::async_runtime::spawn(async move {
            // 握手失败只影响这一个客户端（多半是有人拿 http:// 探了一下这个端口）
            if let Ok(stream) = acceptor.accept(tcp).await
              && let Err(e) = serve_conn(stream, app).await
            {
              hh.set_last_error(format!("连接出错：{e}"));
            }
          });
        }
      }
      Err(e) => {
        hh.set_last_error(format!("端口 {} 绑定失败：{e}", port));
        hh.set_running(false);
      }
    }
  });
  *TASK.lock().map_err(|_| "服务锁已中毒".to_string())? = Some(handle);
  Ok(if ca_replaced {
    format!(
      "已启动，HTTPS 监听 {}:{}（CA 换过了，手机上装过的旧证书需要重装一次）",
      addr.ip(),
      port
    )
  } else {
    format!(
      "已启动，HTTPS 监听 {}:{}（局域网可达；首次访问需信任证书，手机上装一次 CA 就永久免警告）",
      addr.ip(),
      port
    )
  })
}

/// 单个 TLS 连接：HTTP/1 与 HTTP/2 自动协商，并**开启协议升级**（`/ws` 的 WebSocket 靠它）。
///
/// 不能用 `axum::serve`：它的 `serve` 要求实现了 axum 的 `Listener` trait，
/// `tokio_rustls::server::TlsStream` 没有实现，只能走 hyper-util 的 auto builder ——
/// 后者同样支持升级。
async fn serve_conn(
  stream: tokio_rustls::server::TlsStream<tokio::net::TcpStream>,
  app: axum::Router,
) -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
  // 用 hyper-util 自带的适配器，别自己写：
  // - `TokioIo` 把 tokio 的 `AsyncRead/AsyncWrite` 翻译成 hyper 的 `rt::Read/Write`
  // - `TowerToHyperService` 把 axum 的 `tower::Service` 翻译成 hyper 的 `service::Service`
  // - `TokioExecutor` spawn 到当前 tokio runtime，而当前 runtime 就是 Tauri 那个，
  //   所以连接任务天然跟着应用生命周期走，不必自己再造执行器
  hyper_util::server::conn::auto::Builder::new(hyper_util::rt::TokioExecutor::new())
    .serve_connection_with_upgrades(
      hyper_util::rt::TokioIo::new(stream),
      hyper_util::service::TowerToHyperService::new(app),
    )
    .await
}

/// 停服务（幂等）：真的 abort 掉监听任务，不只是清状态位。
pub fn stop() {
  let h = hub();
  h.set_remote_input(false);
  h.set_running(false);
  // 广告必须跟着停：常驻广播会让「明明没跑服务却搜得到」，连过去必然失败，比搜不到更难排查
  mdns::unregister();
  // 锁中毒时不能 panic：停服务是在退出路径上调用的，宁可漏停也不能崩。
  if let Ok(mut slot) = TASK.lock()
    && let Some(handle) = slot.take()
  {
    handle.abort();
  }
}

/// 重启服务：先真的停，再起。端口占用 / 改端口后用它生效。
pub fn restart() -> Result<String, String> {
  stop();
  start()
}

/// 启动时把上次选定的端口读进 hub。**必须在第一次 `start()` 之前调**，
/// 否则用户填的端口每次启动都白填（hub 默认 59123）。
pub fn load_settings() {
  let p = crate::config::load_settings().net_port;
  if p != 0 {
    hub().set_port(p);
  }
}

/// 要广告给客户端的地址：卡片上选的那张；没选、**或那张已经不属于本机了**（换了 Wi-Fi、
/// 拔了 USB 共享、DHCP 换了地址）就用默认路由那张。
///
/// 回落是刻意的：死地址比"不是用户想要的那张"更难排查 —— 界面显示一个连不上的 IP，
/// 用户只会怀疑服务坏了。所以先验明这张卡现在真的还在，再拿它去广告。
pub fn advertised_ip() -> String {
  let list = addrs::list();
  pick_advertised_ip(
    &crate::config::load_settings().net_ip,
    &list,
    &addrs::default_ip(),
  )
}

/// 挑广告地址的**纯函数**部分（`advertised_ip` 那一半要读设置、要摸网卡，不适合直接测）。
///
/// 选中项必须在**当前的**网卡列表里才作数：选过的网卡可能被拔掉（USB 共享、蓝牙共享、
/// 移动热点），DHCP 也可能换地址。报一个已经不属于本机的 IP 比报默认路由那张更难排查 ——
/// 用户只会怀疑服务坏了。
fn pick_advertised_ip(want: &str, list: &[addrs::NetIface], dflt: &str) -> String {
  if !want.is_empty() && list.iter().any(|n| n.ip == want) {
    want.to_string()
  } else {
    dflt.to_string()
  }
}

/// 记下选中的广告地址并落盘。空串 = 回到自动（默认路由那张）。
pub fn set_advertised_ip(ip: &str) -> Result<(), String> {
  let mut s = crate::config::load_settings();
  s.net_ip = ip.trim().to_string();
  crate::config::save_settings(&s)
}

/// 记下新端口并落盘。**不重启** —— 调用方（`net_set_port` 命令）决定要不要重启。
pub fn set_port(p: u16) -> Result<(), String> {
  let mut s = crate::config::load_settings();
  s.net_port = p;
  crate::config::save_settings(&s)
}

/// 服务状态文字（界面与调试接口共用）。
pub fn status() -> String {
  let h = hub();
  let s = h.stats();
  let codec = match &s.codec {
    crate::debug::Probe::Ok { value } => value.clone(),
    crate::debug::Probe::Unavailable { reason } => format!("编解码器不可用：{reason}"),
    crate::debug::Probe::Pending => "编解码器检测中".to_string(),
  };
  let input = match keys::backend() {
    Ok(b) => format!(
      "远程输入 {b}（{} 键）：{}",
      keymap::mapped_count(),
      if s.remote_input { "已开" } else { "关" }
    ),
    Err(e) => format!("远程输入不可用（{e}）"),
  };
  let ms = |v: usize| v as f64 / crate::audio::SAMPLE_RATE as f64 * 1000.0;
  format!(
    "{}：端口 {}，客户端 {}，订阅音频 {}，入站水位 {:.0}/{:.0} ms，出站已发 {} 包（待发 {:.0} ms）；{input}；{codec}",
    // 就绪与否看 `is_running()`，不能看 `port > 0` —— 端口是**下次要绑的**那个值，
    // 恒为正数，服务没开时也一样（端口改成可编辑之后这个判据就废了）
    if h.is_running() {
      "服务就绪"
    } else {
      "服务未启动"
    },
    s.port,
    s.clients,
    s.subs,
    s.rx_level_ms,
    s.rx_target_ms,
    s.tx_packets,
    ms(s.tx_level),
  )
}

#[cfg(test)]
mod tests {
  use super::*;

  #[test]
  fn hub_is_singleton() {
    let a = hub();
    let b = hub();
    assert!(Arc::ptr_eq(&a, &b));
  }

  #[test]
  fn status_mentions_port_and_codec() {
    let s = status();
    assert!(s.contains("端口"), "{s}");
    assert!(s.contains("opus") || s.contains("编解码器"), "{s}");
  }

  #[test]
  fn remote_input_defaults_closed() {
    assert!(!hub().remote_input());
  }

  /// 端口改成可编辑之后 `stats().port` **恒为正数**（它是「下次要绑的」那个值，
  /// 服务没开时也是 59123）。所以就绪判据必须换成 `is_running()`，否则界面永远显示
  /// 「服务就绪」，启动 / 停止 两个按钮的禁用状态也跟着全错。
  #[test]
  fn 就绪与否看运行状态不只看端口() {
    let s = status();
    assert_eq!(
      s.starts_with("服务就绪"),
      hub().is_running(),
      "状态串与 is_running() 不一致：{s}"
    );
  }

  #[test]
  fn 端口可改且回落默认() {
    let orig = hub().port();
    hub().set_port(59124);
    assert_eq!(hub().port(), 59124);
    assert!(status().contains("59124"), "{}", status());
    hub().set_port(orig);
  }

  /// 广告地址必须**先验明这张卡现在还在**再拿去广告：选过的网卡可能被拔掉（USB 共享、
  /// 蓝牙共享）、DHCP 也可能换了地址。报一个已经不属于本机的 IP 比报默认路由那张更难
  /// 排查 —— 用户只会怀疑服务坏了。
  ///
  /// 只测纯函数那一半：读真设置 / 摸真网卡的 `advertised_ip()` 不在测试里跑（会写用户
  /// 的 `settings.json`）。
  #[test]
  fn 失效的选定地址回落默认() {
    let list = vec![
      addrs::NetIface {
        ip: "192.168.1.5".into(),
        name: "WLAN".into(),
        desc: "WLAN".into(),
        is_default: true,
      },
      addrs::NetIface {
        ip: "192.168.7.1".into(),
        name: "以太网 4".into(),
        desc: "USB 以太网".into(),
        is_default: false,
      },
    ];
    // 选中的还在 → 用它
    assert_eq!(
      pick_advertised_ip("192.168.7.1", &list, "192.168.1.5"),
      "192.168.7.1"
    );
    // 选中的已经拔了 → 回落默认路由，不能拿死地址去广告
    assert_eq!(
      pick_advertised_ip("192.168.9.9", &list, "192.168.1.5"),
      "192.168.1.5"
    );
    // 没选过 → 默认路由
    assert_eq!(pick_advertised_ip("", &list, "192.168.1.5"), "192.168.1.5");
    // 一张网卡都没有时给空串，不给伪造的 0 / 空地址（界面据此显示「不可用 + 原因」）
    assert_eq!(pick_advertised_ip("", &[], ""), "");
  }
}
