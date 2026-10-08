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

//! 线上控制消息（DESIGN.md §4.1）。只增不改：字段加了老的客户端还能用。
//!
//! 音频走 WebSocket 二进制帧（一个 Opus 包一条消息），控制走文本帧（JSON）。

use serde::Deserialize;

/// 协议版本；不匹配时服务端回 `err` 并断开。
pub const PROTO: i32 = 1;
/// 网络服务端口（沿用旧实现的 59123）。
pub const PORT: u16 = 59123;

/// 客户端 → 服务端。
#[derive(Deserialize, Debug)]
#[serde(tag = "t")]
pub enum ClientMsg {
  /// 握手。`proto` 必须等于 [`PROTO`]。
  #[serde(rename = "hello")]
  Hello { proto: i32 },
  /// 订阅电脑音频（电脑当手机音箱）。
  #[serde(rename = "sub")]
  Sub,
  /// 手机输入法提交的文本（已组合好的字符串）。
  #[serde(rename = "text")]
  Text { s: String },
  /// 手机实体键：`code` = Android `KeyEvent.KEYCODE_*`，`down` = true 按下 / false 松开。
  #[serde(rename = "key")]
  Key { code: i32, down: bool },
  /// 往返时延测量；服务端原样回 `pong`。
  #[serde(rename = "ping")]
  Ping { id: u64 },
  /// 客户端自测的往返时延（ms）：`ping`/`pong` 的时差由客户端算好后上报，
  /// 这样调试接口里能看到 RTT，服务端不需要自己计时。
  #[serde(rename = "rtt")]
  Rtt { ms: f64 },
}

/// 服务端 → 客户端。
#[derive(serde::Serialize, Debug)]
#[serde(tag = "t")]
pub enum ServerMsg {
  #[serde(rename = "ready")]
  Ready {
    proto: i32,
    codec: &'static str,
    rate: u32,
    channels: u32,
    /// 服务端出站帧长（ms）；客户端自己的发包帧长可不同（接收端按样本累积）
    frame_ms: u32,
    clients: usize,
  },
  #[serde(rename = "pong")]
  Pong { id: u64 },
  #[serde(rename = "err")]
  Err { msg: String },
}

impl ServerMsg {
  pub fn err(msg: impl Into<String>) -> Self {
    ServerMsg::Err { msg: msg.into() }
  }

  pub fn to_text(&self) -> String {
    // 只有内部固定形状的结构，序列化不会失败
    serde_json::to_string(self).unwrap_or_else(|_| r#"{"t":"err","msg":"序列化失败"}"#.to_string())
  }
}

#[cfg(test)]
mod tests {
  use super::*;

  #[test]
  fn parses_every_client_message() {
    let hello: ClientMsg = serde_json::from_str(r#"{"t":"hello","proto":1}"#).unwrap();
    assert!(matches!(hello, ClientMsg::Hello { proto: 1 }));
    assert!(matches!(
      serde_json::from_str::<ClientMsg>(r#"{"t":"sub"}"#).unwrap(),
      ClientMsg::Sub
    ));
    let text: ClientMsg = serde_json::from_str(r#"{"t":"text","s":"你好 world"}"#).unwrap();
    match text {
      ClientMsg::Text { s } => assert_eq!(s, "你好 world"),
      _ => panic!("解析成了别的消息"),
    }
    let key: ClientMsg = serde_json::from_str(r#"{"t":"key","code":29,"down":true}"#).unwrap();
    match key {
      ClientMsg::Key { code, down } => {
        assert_eq!(code, 29);
        assert!(down);
      }
      _ => panic!("解析成了别的消息"),
    }
    assert!(matches!(
      serde_json::from_str::<ClientMsg>(r#"{"t":"ping","id":7}"#).unwrap(),
      ClientMsg::Ping { id: 7 }
    ));
    assert!(matches!(
      serde_json::from_str::<ClientMsg>(r#"{"t":"rtt","ms":12.5}"#).unwrap(),
      ClientMsg::Rtt { ms } if (ms - 12.5).abs() < 1e-9
    ));
  }

  #[test]
  fn rejects_unknown_and_missing_fields() {
    assert!(serde_json::from_str::<ClientMsg>(r#"{"t":"nope"}"#).is_err());
    // 缺 down 必须报错：宁可丢掉也不能把半条按键当完整事件注入
    assert!(serde_json::from_str::<ClientMsg>(r#"{"t":"key","code":29}"#).is_err());
  }

  #[test]
  fn server_messages_are_tagged() {
    let r = ServerMsg::Ready {
      proto: PROTO,
      codec: "opus",
      rate: 48000,
      channels: 1,
      frame_ms: 10,
      clients: 1,
    };
    assert!(r.to_text().contains(r#""t":"ready""#));
    assert!(
      ServerMsg::Pong { id: 3 }
        .to_text()
        .contains(r#""t":"pong""#)
    );
    assert!(
      ServerMsg::err("x").to_text().contains(r#""t":"err""#),
      "err 序列化异常"
    );
  }
}
