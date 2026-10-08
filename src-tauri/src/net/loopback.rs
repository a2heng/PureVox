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

//! 网络回环测试（DESIGN.md §4.1）：起**真的** axum 服务，用**真的** WebSocket 客户端对接。
//!
//! 与单元测试的分工：单元测试（各模块 `mod tests`）验证纯逻辑——协议解析、缓冲边界、
//! 键码映射；这里验证**接线**：路由、握手、版本协商、订阅广播速率、远程输入开关真的会生效。
//!
//! 本模块只在 `cfg(test)` 下编译（不进产物）。

#[cfg(test)]
mod inner {
  use crate::net::hub::NetHub;
  use crate::net::opus_sys::{Decoder, Encoder, MAX_PACKET_BYTES};
  use crate::net::server::router;
  use futures_util::{SinkExt, StreamExt};
  use std::net::SocketAddr;
  use std::sync::Arc;
  use std::time::Duration;
  use tokio::net::TcpListener;
  use tokio_tungstenite::WebSocketStream;
  use tokio_tungstenite::tungstenite::Message;

  /// 起一个真实监听的服务（绑 127.0.0.1 随机端口，避免占用 59123）。
  async fn serve() -> SocketAddr {
    let hub = Arc::new(NetHub::new());
    let app = router(hub);
    let listener = TcpListener::bind("127.0.0.1:0").await.expect("绑定");
    let addr = listener.local_addr().expect("本地地址");
    tokio::spawn(async move {
      let _ = axum::serve(listener, app).await;
    });
    for _ in 0..50 {
      if tokio::net::TcpStream::connect(addr).await.is_ok() {
        return addr;
      }
      tokio::time::sleep(Duration::from_millis(20)).await;
    }
    panic!("服务未就绪");
  }

  /// 最小 HTTP GET（不用 reqwest：debug 依赖里没有，且这里只要状态与正文）。
  async fn http_get(addr: SocketAddr, path: &str) -> (u16, String) {
    use tokio::io::{AsyncReadExt, AsyncWriteExt};
    let mut s = tokio::net::TcpStream::connect(addr).await.expect("连接");
    let req = format!("GET {path} HTTP/1.1\r\nHost: {addr}\r\nConnection: close\r\n\r\n");
    s.write_all(req.as_bytes()).await.unwrap();
    let mut all = Vec::new();
    s.read_to_end(&mut all).await.ok();
    let text = String::from_utf8_lossy(&all).into_owned();
    let status = text
      .split_whitespace()
      .nth(1)
      .and_then(|c| c.parse().ok())
      .unwrap_or(0);
    (status, text)
  }

  /// 建一条 WebSocket 连接（用 maybe-tls 的具体类型别名，省去泛型噪声）。
  type Ws = WebSocketStream<tokio_tungstenite::MaybeTlsStream<tokio::net::TcpStream>>;

  async fn ws_connect(addr: SocketAddr) -> Ws {
    let url = format!("ws://{addr}/ws");
    let (s, _) = tokio_tungstenite::connect_async(url)
      .await
      .expect("WS 连接");
    s
  }

  async fn send_text(s: &mut Ws, t: &str) {
    s.send(Message::Text(t.into())).await.expect("发送");
  }

  /// 读到下一个文本帧为止，**容忍**随后连接被关闭（服务端拒绝握手时会先 err 再断）。
  async fn read_until_text(s: &mut Ws) -> String {
    let deadline = tokio::time::Instant::now() + Duration::from_secs(3);
    while tokio::time::Instant::now() < deadline {
      match tokio::time::timeout(Duration::from_secs(3), s.next()).await {
        Ok(Some(Ok(Message::Text(t)))) => return t.to_string(),
        Ok(Some(Ok(Message::Close(_)))) => panic!("未收到文本帧就被关闭"),
        Ok(Some(Ok(_))) => continue,
        Ok(Some(Err(_))) => panic!("未收到文本帧就出错断开"),
        Ok(None) => panic!("未收到文本帧就关闭"),
        Err(_) => panic!("等待文本帧超时"),
      }
    }
    panic!("未收到文本帧");
  }

  /// 收下一个文本控制帧（跳过二进制）。
  async fn next_text(s: &mut Ws) -> String {
    let deadline = tokio::time::Instant::now() + Duration::from_secs(3);
    while tokio::time::Instant::now() < deadline {
      match tokio::time::timeout(Duration::from_secs(3), s.next()).await {
        Ok(Some(Ok(Message::Text(t)))) => return t.to_string(),
        Ok(Some(Ok(_))) => continue,
        Ok(Some(Err(e))) => panic!("接收出错：{e}"),
        Ok(None) => panic!("连接已关闭"),
        Err(_) => panic!("等待文本帧超时"),
      }
    }
    panic!("未收到文本帧");
  }

  /// `/health` 自检：手机端与人都能看。
  #[tokio::test]
  async fn health_reports_service() {
    let addr = serve().await;
    let (status, body) = http_get(addr, "/health").await;
    assert_eq!(status, 200);
    assert!(body.contains("PureVox"), "{body}");
    assert!(body.contains(r#""port":59123"#), "{body}");
    // 远程输入必须默认关
    assert!(body.contains(r#""remote_input":false"#), "{body}");
  }

  /// 未知路径 404。
  #[tokio::test]
  async fn unknown_path_is_404() {
    let addr = serve().await;
    let (status, _) = http_get(addr, "/nope").await;
    assert_eq!(status, 404, "未知路径应 404");
  }

  /// 握手成功回 `ready`；协议版本不符回 `err`（DESIGN.md §4.1）。
  #[tokio::test]
  async fn handshake_and_version_negotiation() {
    let addr = serve().await;
    let mut s = ws_connect(addr).await;
    send_text(&mut s, r#"{"t":"hello","proto":1}"#).await;
    let ready = next_text(&mut s).await;
    assert!(ready.contains(r#""t":"ready""#), "{ready}");
    assert!(ready.contains(r#""codec":"opus""#), "{ready}");
    assert!(
      ready.contains(r#""frame_ms":10"#),
      "PC 侧出站必须是 10 ms：{ready}"
    );

    let mut s2 = ws_connect(addr).await;
    send_text(&mut s2, r#"{"t":"hello","proto":999}"#).await;
    // 服务端回 err 后立即断开：这里要读到 err 为止，不能把随后的关闭当异常
    let err = read_until_text(&mut s2).await;
    assert!(err.contains(r#""t":"err""#), "{err}");
    assert!(err.contains("协议版本不符"), "拒绝原因要明确：{err}");
  }

  /// 远程输入默认关：文本与按键都必须被拒，且原因可读（DESIGN.md §4.1）。
  #[tokio::test]
  async fn remote_input_refused_until_enabled() {
    let addr = serve().await;
    let mut s = ws_connect(addr).await;
    for msg in [
      r#"{"t":"text","s":"hi"}"#,
      r#"{"t":"key","code":29,"down":true}"#,
    ] {
      send_text(&mut s, msg).await;
      let err = next_text(&mut s).await;
      assert!(err.contains(r#""t":"err""#), "{err}");
      assert!(err.contains("远程输入未开启"), "拒绝原因要说明开关：{err}");
    }
  }

  /// ping/pong 回程。
  #[tokio::test]
  async fn ping_pong_roundtrip() {
    let addr = serve().await;
    let mut s = ws_connect(addr).await;
    send_text(&mut s, r#"{"t":"ping","id":4242}"#).await;
    let pong = next_text(&mut s).await;
    assert!(pong.contains(r#""t":"pong""#), "{pong}");
    assert!(pong.contains("4242"), "{pong}");
  }

  /// 订阅后按 10 ms 节拍收 Opus 包（1 s 约 100 包）。没有输出行时发静音包保时钟。
  #[tokio::test]
  async fn subscribe_streams_at_hop_rate() {
    let addr = serve().await;
    let mut s = ws_connect(addr).await;
    send_text(&mut s, r#"{"t":"sub"}"#).await;
    let ready = next_text(&mut s).await;
    assert!(ready.contains(r#""t":"ready""#), "订阅应回 ready：{ready}");

    // 放宽下限：本机 debug 构建 + 测试并行时单核被抢，tokio interval 会拖后。
    // 上限卡住：绝不能超频（超频 = 时钟跑快 = 手机端音频变调）。
    let mut packets = 0usize;
    let started = std::time::Instant::now();
    let window = Duration::from_millis(2000);
    while started.elapsed() < window {
      match tokio::time::timeout(Duration::from_millis(300), s.next()).await {
        Ok(Some(Ok(Message::Binary(_)))) => packets += 1,
        Ok(Some(Ok(_))) => continue,
        Ok(Some(Err(e))) => panic!("接收出错：{e}"),
        Ok(None) => panic!("连接被关闭"),
        Err(_) => {}
      }
    }
    let secs = started.elapsed().as_secs_f64();
    let rate = packets as f64 / secs;
    assert!(
      (80.0..=110.0).contains(&rate),
      "出站应为约 100 包/s（10 ms 节拍），实测 {rate:.1} 包/s（{packets}/{secs:.2}s）"
    );
  }

  /// 出站包必须是**真的 Opus**：用真解码器解开并能凑出 hop。
  #[tokio::test]
  async fn outbound_packets_are_decodable_opus() {
    let addr = serve().await;
    let mut s = ws_connect(addr).await;
    send_text(&mut s, r#"{"t":"sub"}"#).await;
    let _ = next_text(&mut s).await;

    let mut dec = Decoder::new(crate::audio::SAMPLE_RATE as i32).expect("解码器");
    // decode_packet 直接把样本交出来，这里自己累积并切 hop（与引擎侧同一做法）
    let mut acc: Vec<f32> = Vec::with_capacity(crate::audio::HOP * 4);
    let mut got_hops = 0usize;
    let mut peak = 0.0f32;
    // 注意：这里的服务用 serve() 内部自建的 hub，没有 remote_speaker 输出行，
    // 因此出站是**欠载静音包**——正好验证「欠载补静音、时钟不断」这条契约。
    let deadline = tokio::time::Instant::now() + Duration::from_millis(2000);
    while tokio::time::Instant::now() < deadline && got_hops < 20 {
      match tokio::time::timeout(Duration::from_millis(300), s.next()).await {
        Ok(Some(Ok(Message::Binary(b)))) => {
          let mut samples = Vec::new();
          dec
            .decode_packet(&b, 0, &mut samples)
            .expect("出站包应能解码");
          assert_eq!(samples.len(), crate::audio::HOP, "PC 侧每包应恰好一 hop");
          acc.extend_from_slice(&samples);
          while acc.len() >= crate::audio::HOP {
            for x in &acc[..crate::audio::HOP] {
              peak = peak.max(x.abs());
            }
            acc.drain(..crate::audio::HOP);
            got_hops += 1;
          }
        }
        Ok(Some(Ok(_))) => continue,
        Ok(Some(Err(e))) => panic!("接收出错：{e}"),
        Ok(None) => panic!("连接被关闭"),
        Err(_) => {}
      }
    }
    assert!(got_hops >= 20, "应解出至少 20 个 hop，实得 {got_hops}");
    // 没推信号时发的是静音包，解码后幅度应为 0（证明欠载补的是静音而不是噪声）
    assert!(peak < 1e-3, "无信号时应解出静音，峰值却 {peak}");
  }

  /// 入站：真编码器造包发过去，服务端应解出样本并喂给引擎 hop 网格。
  /// 这里不能直接断言 hub 计数（连接任务在别的线程），但要确认**包被接受**（没有 err、
  /// 连接不崩、且过一会儿仍能正常收发）。
  #[tokio::test]
  async fn inbound_opus_accepted_without_error() {
    let addr = serve().await;
    let mut s = ws_connect(addr).await;
    send_text(&mut s, r#"{"t":"hello","proto":1}"#).await;
    let _ = next_text(&mut s).await;

    let mut enc = Encoder::new(crate::audio::SAMPLE_RATE as i32, 32_000).expect("编码器");
    let mut pkt = [0u8; MAX_PACKET_BYTES];
    let mut frame = vec![0.0f32; crate::audio::HOP];
    for k in 0..10usize {
      for (i, x) in frame.iter_mut().enumerate() {
        *x = if k < 8 {
          0.3
            * (2.0 * std::f32::consts::PI * 1000.0 * ((k * crate::audio::HOP + i) as f32)
              / crate::audio::SAMPLE_RATE as f32)
              .sin()
        } else {
          0.0
        };
      }
      let n = enc.encode(&frame, &mut pkt).expect("编码");
      s.send(Message::Binary(pkt[..n].to_vec()))
        .await
        .expect("发送二进制");
      tokio::time::sleep(Duration::from_millis(10)).await;
    }

    // 发完仍应能正常 ping（说明收侧没崩）
    send_text(&mut s, r#"{"t":"ping","id":7}"#).await;
    let pong = next_text(&mut s).await;
    assert!(
      pong.contains(r#""t":"pong""#) && pong.contains('7'),
      "{pong}"
    );
  }

  /// 非法控制消息应回 err 而不是断连（客户端能自己看到原因）。
  #[tokio::test]
  async fn malformed_control_message_is_reported() {
    let addr = serve().await;
    let mut s = ws_connect(addr).await;
    send_text(&mut s, "{not json").await;
    let err = next_text(&mut s).await;
    assert!(err.contains(r#""t":"err""#), "{err}");
    // 缺字段（key 没有 down）也必须被拒：半条按键不能当完整事件注入
    send_text(&mut s, r#"{"t":"key","code":29}"#).await;
    let err2 = next_text(&mut s).await;
    assert!(err2.contains(r#""t":"err""#), "{err2}");
  }

  /// 开启远程输入后，`key` 不再被拒（这里不真注入：注入会打进当前前台窗口）。
  /// 只验证「开关状态被正确读取」——发一个未映射的键码（音量键），它应被静默忽略而非报错。
  #[tokio::test]
  async fn enabled_remote_input_accepts_keys() {
    let hub = Arc::new(NetHub::new());
    hub.set_remote_input(true);
    let app = router(hub.clone());
    let listener = TcpListener::bind("127.0.0.1:0").await.expect("绑定");
    let addr = listener.local_addr().unwrap();
    tokio::spawn(async move {
      let _ = axum::serve(listener, app).await;
    });
    tokio::time::sleep(Duration::from_millis(100)).await;

    let mut s = ws_connect(addr).await;
    // 未映射的音量键（KEYCODE_VOLUME_UP = 24）：静默忽略，不应回 err
    send_text(&mut s, r#"{"t":"key","code":24,"down":true}"#).await;
    send_text(&mut s, r#"{"t":"ping","id":11}"#).await;
    // 第一条收到的应该是 pong（说明 key 没产生 err）
    let msg = next_text(&mut s).await;
    assert!(
      msg.contains(r#""t":"pong""#),
      "未映射键应静默忽略，却收到：{msg}"
    );
  }
}
