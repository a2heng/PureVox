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
//! 1. `remote_mic` 输入行 —— 手机麦克风当电脑麦克风（进引擎全链，可降噪）
//! 2. `remote_speaker` 输出行 —— 手机扬声器当电脑音箱
//! 3. 远程输入 —— 手机输入法打字（`text`）+ 手机实体键当全尺寸键盘（`key`），默认关闭
//!
//! **平台**：传输与编解码跨平台（axum + tokio + 运行时加载的 libopus）；
//! 只有关键词注入是平台相关（Windows `SendInput`，其余平台明确报不可用，见 `keys`）。

pub mod hub;
pub mod keymap;
pub mod keys;
pub mod opus_sys;

/// 网络回环测试（只 `cfg(test)` 编译，不进产物）。
#[cfg(test)]
mod diag;
#[cfg(test)]
mod loopback;
pub mod proto;
pub mod server;

use std::sync::{Arc, OnceLock};

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

/// 启动服务（幂等）。返回绑定结果说明；端口占用等失败原因进调试接口，不静默失败。
pub fn start() -> Result<String, String> {
  let h = hub();
  if h.is_running() {
    return Ok(format!("已在运行（端口 {}）", proto::PORT));
  }
  // 缺 opus.dll 时先报出来：网络功能没有编解码器就是死的
  opus_sys::probe()?;
  let app = server::router(h.clone());
  let addr = std::net::SocketAddr::from((BIND_ADDR, proto::PORT));
  let hh = h.clone();
  tauri::async_runtime::spawn(async move {
    match tokio::net::TcpListener::bind(addr).await {
      Ok(listener) => {
        hh.set_running(true);
        if let Err(e) = axum::serve(listener, app).await {
          hh.set_last_error(format!("服务退出：{e}"));
          hh.set_running(false);
        }
      }
      Err(e) => {
        hh.set_last_error(format!("端口 {} 绑定失败：{e}", proto::PORT));
        hh.set_running(false);
      }
    }
  });
  Ok(format!(
    "已启动，监听 {}:{}（局域网可达）",
    addr.ip(),
    proto::PORT
  ))
}

/// 停服务（幂等）。
pub fn stop() {
  // 监听任务在进程内以 abort 方式结束由 main 的退出流程处理；这里只清状态
  let h = hub();
  h.set_remote_input(false);
  h.set_running(false);
}

/// 服务状态文字（界面与调试接口共用）。
pub fn status() -> String {
  let s = hub().stats();
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
    "{}：端口 {}，客户端 {}，订阅音频 {}，入站 {:.0} ms，出站已发 {} 包（水位 {:.0} ms）；{input}；{codec}",
    if s.port > 0 {
      "服务就绪"
    } else {
      "服务未启动"
    },
    s.port,
    s.clients,
    s.subs,
    ms(s.rx_level),
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
}
