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

//! 线上协议（DESIGN.md §4.1）。**协议号 3**（2026-10-27 重建实时链路）。
//!
//! ## 两条通道
//!
//! - **音频 = WebSocket 二进制帧**：8 字节帧头（`seq` + 发送方 µs 时间戳）+ 一个 Opus 包。
//!   帧头是 v3 新增的：没有它，收端不知道包是「晚到」还是「网络抖」，也算不出单向延迟
//!   （见 `net/rt.rs` 的 [`crate::net::rt::AudioHeader`]）。
//! - **控制 = WebSocket 文本帧**：JSON，`t` 判别。
//!
//! ## 为什么要 `sync` / `syncreply` / `offset`
//!
//! 「扬声器还是慢」这类问题，没有**实测单向延迟**就只能猜。这里用 NTP 的四时戳法：
//! 服务端发 `sync{t0}`，客户端回 `syncreply{t0,t1,t2}`，服务端拿到 `t3` 后算
//! `offset = ((t1-t0)+(t2-t3))/2`（客户端时钟 − 服务端时钟）与 `rtt`，保留**最小 RTT**
//! 那一个样本（排队最少、最接近真实偏移）。随后：
//! - 服务端用 `offset` + 入站帧头算「手机 → 电脑」单向延迟；
//! - 服务端把 `offset` 用 `offset{us}` 发给客户端，客户端用它 + 出站帧头算「电脑 → 手机」
//!   单向延迟，随 [`ClientMsg::St`] 报回来。
//!
//! 时间戳一律是 **u32 微秒**（71.6 分钟回绕），比较只走 `rt::us_diff`。

use serde::Deserialize;

/// 协议版本；不匹配时服务端回 `err` 并断开。
///
/// **v3 与 v1/v2 不兼容**：音频帧多了 8 字节头。老 APK 会收到「协议版本不符，请更新客户端」，
/// 这是刻意的 —— 半边新半边旧会解出噪声，比明确拒绝难排查得多。
pub const PROTO: i32 = 3;
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
  /// 退订电脑音频（网页版关掉「扬声器」）。
  #[serde(rename = "unsub")]
  Unsub,
  /// 手机输入法提交的文本（已组合好的字符串）。
  #[serde(rename = "text")]
  Text { s: String },
  /// 手机实体键：`code` = Android `KeyEvent.KEYCODE_*`，`down` = true 按下 / false 松开。
  #[serde(rename = "key")]
  Key { code: i32, down: bool },
  /// 客户端发起的往返时延测量；服务端原样回 `pong`。
  #[serde(rename = "ping")]
  Ping { id: u64 },
  /// 客户端自测的往返时延（ms），给调试接口看。
  #[serde(rename = "rtt")]
  Rtt { ms: f64 },
  /// 客户端侧播放 / 采集状态上报（2 Hz）。服务端不做任何控制，只为「看得见」。
  #[serde(rename = "st")]
  St {
    /// 播放环水位 / 目标（ms）
    #[serde(default)]
    level_ms: f64,
    #[serde(default)]
    target_ms: f64,
    /// 客户端自己补的欠载次数（累计）
    #[serde(default)]
    underruns: u64,
    /// Opus PLC 掩盖次数（= 数据流里真的缺包）
    #[serde(default)]
    concealed: u64,
    /// 到达太晚、错过播放时刻的包数
    #[serde(default)]
    late: u64,
    /// 客户端实测的**音频时钟偏差**（ppm，正 = 客户端比电脑快）；测不出来就不带
    #[serde(default)]
    clock_ppm: Option<f64>,
    /// 客户端实测的单向延迟（电脑发出 → 收到并开始播，ms）；没测出来就不带
    #[serde(default)]
    one_way_ms: Option<f64>,
  },
  /// 改接收方播放环深度（ms）。**任一侧都能改**：服务端落盘后广播 `target` 给所有客户端，
  /// 包括发起方自己（这样两端 UI 与环永远同一个值，不用各自猜）。
  #[serde(rename = "settarget")]
  SetTarget { ms: u32 },
  /// 对服务端 `sync` 的应答（NTP 四时戳）。三个时间戳都是**客户端时钟的 µs**（t0 原样带回）。
  #[serde(rename = "syncreply")]
  SyncReply { id: u64, t0: u32, t1: u32, t2: u32 },
}

/// 服务端 → 客户端。
#[derive(serde::Serialize, Debug)]
#[serde(tag = "t")]
pub enum ServerMsg {
  #[serde(rename = "ready")]
  Ready {
    proto: i32,
    /// 服务端往客户端发的音频格式。客户端按这个来准备解码 / 播放。
    codec: &'static str,
    rate: u32,
    channels: u32,
    /// 服务端出站帧长（ms）
    frame_ms: u32,
    /// 音频帧头长度（字节）。客户端按它切 Opus 载荷（v3 = 8）。
    header_bytes: u32,
    /// 客户端播放环深度（ms）。缓冲只在接收方，深度是**同一个值**：
    /// 由服务端下发，避免两端各拍一个数字（可手动调，见 [`ClientMsg::SetTarget`]）。
    target_ms: u32,
    clients: usize,
  },
  #[serde(rename = "pong")]
  Pong { id: u64 },
  /// 时钟同步探测：`t0` = 服务端发出时的 µs（客户端原样带回 `syncreply`）。
  #[serde(rename = "sync")]
  Sync { id: u64, t0: u32 },
  /// 服务端算出的时钟偏移（客户端 − 服务端，µs），客户端用它算下行单向延迟。
  #[serde(rename = "offset")]
  Offset { us: i64 },
  /// 环深度变了（手动调整，两侧同步）。收到就按这个值改自己的环与界面。
  #[serde(rename = "target")]
  Target { ms: u32 },
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
    let hello: ClientMsg = serde_json::from_str(r#"{"t":"hello","proto":3}"#).unwrap();
    assert!(matches!(hello, ClientMsg::Hello { proto: 3 }));
    assert!(matches!(
      serde_json::from_str::<ClientMsg>(r#"{"t":"sub"}"#).unwrap(),
      ClientMsg::Sub
    ));
    assert!(matches!(
      serde_json::from_str::<ClientMsg>(r#"{"t":"unsub"}"#).unwrap(),
      ClientMsg::Unsub
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

  /// `st` 的字段全部可选：客户端在哪一版加了新字段，老服务端也能收（反之亦然）。
  #[test]
  fn client_stats_tolerates_missing_fields() {
    let st: ClientMsg = serde_json::from_str(r#"{"t":"st"}"#).unwrap();
    match st {
      ClientMsg::St {
        level_ms,
        one_way_ms,
        ..
      } => {
        assert_eq!(level_ms, 0.0);
        assert!(one_way_ms.is_none());
      }
      _ => panic!("解析成了别的消息"),
    }
    let full: ClientMsg = serde_json::from_str(
      r#"{"t":"st","level_ms":21.5,"target_ms":20,"underruns":1,"concealed":0,"late":2,"one_way_ms":13.2,"clock_ppm":-27.5}"#,
    )
    .unwrap();
    match full {
      ClientMsg::St {
        level_ms,
        concealed,
        one_way_ms,
        ..
      } => {
        assert!((level_ms - 21.5).abs() < 1e-9);
        assert_eq!(concealed, 0);
        assert_eq!(one_way_ms, Some(13.2));
      }
      _ => panic!("解析成了别的消息"),
    }
  }

  #[test]
  fn parses_sync_reply() {
    let m: ClientMsg =
      serde_json::from_str(r#"{"t":"syncreply","id":9,"t0":100,"t1":105,"t2":107}"#).unwrap();
    match m {
      ClientMsg::SyncReply { id, t0, t1, t2 } => {
        assert_eq!((id, t0, t1, t2), (9, 100, 105, 107));
      }
      _ => panic!("解析成了别的消息"),
    }
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
      header_bytes: 8,
      target_ms: 20,
      clients: 1,
    };
    assert!(r.to_text().contains(r#""t":"ready""#));
    assert!(
      ServerMsg::Pong { id: 3 }
        .to_text()
        .contains(r#""t":"pong""#)
    );
    let sync = ServerMsg::Sync { id: 1, t0: 12345 }.to_text();
    assert!(
      sync.contains(r#""t":"sync""#) && sync.contains("12345"),
      "{sync}"
    );
    let off = ServerMsg::Offset { us: -1234 }.to_text();
    assert!(
      off.contains(r#""t":"offset""#) && off.contains("-1234"),
      "{off}"
    );
    assert!(
      ServerMsg::err("x").to_text().contains(r#""t":"err""#),
      "err 序列化异常"
    );
  }

  /// `ready` 必须告诉客户端帧头长度：客户端要靠它切载荷，猜错了就是满屏噪声。
  #[test]
  fn ready_declares_header_bytes() {
    let t = ServerMsg::Ready {
      proto: PROTO,
      codec: "opus",
      rate: 48000,
      channels: 1,
      frame_ms: 10,
      header_bytes: crate::net::rt::HDR_LEN as u32,
      target_ms: 20,
      clients: 0,
    }
    .to_text();
    assert!(t.contains(r#""header_bytes":8"#), "{t}");
  }
}
