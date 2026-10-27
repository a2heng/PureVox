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

//! 网络回环测试（NETWORK.md §4.1）：起**真的** axum 服务，用**真的** WebSocket 客户端对接。
//!
//! 与单元测试的分工：单元测试（各模块 `mod tests`）验证纯逻辑——协议解析、缓冲边界、
//! 键码映射；这里验证**接线**：路由、握手、版本协商、订阅广播速率、远程输入开关真的会生效。
//!
//! 本模块只在 `cfg(test)` 下编译（不进产物）。

#[cfg(test)]
mod inner {
  use crate::net::hub::NetHub;
  use crate::net::opus_sys::{Decoder, Encoder, MAX_PACKET_BYTES};
  use crate::net::rt::AudioHeader;
  use crate::net::server::router;
  use futures_util::{SinkExt, StreamExt};
  use std::net::SocketAddr;
  use std::sync::Arc;
  use std::time::Duration;
  use tokio::net::TcpListener;
  use tokio_tungstenite::WebSocketStream;
  use tokio_tungstenite::tungstenite::Message;

  /// 起一个真实监听的服务（绑 127.0.0.1 随机端口，避免占用 59123）。
  ///
  /// 返回中枢句柄：出站是**推来即发**（没有定时器），所以「有音频发出去」必须由测试自己
  /// 调 `push_tx_hop` 来模拟引擎那一列。
  async fn serve() -> (SocketAddr, Arc<NetHub>) {
    let hub = Arc::new(NetHub::new());
    let app = router(hub.clone(), String::new());
    let listener = TcpListener::bind("127.0.0.1:0").await.expect("绑定");
    let addr = listener.local_addr().expect("本地地址");
    tokio::spawn(async move {
      let _ = axum::serve(listener, app).await;
    });
    for _ in 0..50 {
      if tokio::net::TcpStream::connect(addr).await.is_ok() {
        return (addr, hub);
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

  /// 收下一个文本控制帧（跳过二进制，以及后台的 `sync` / `offset`）。
  ///
  /// 时钟同步是**周期性的后台流量**（每 2 s 一条），任何「下一条文本应该是什么」的
  /// 断言都不该被它打乱；专门测同步的用例用 [`next_raw_text`]。
  async fn next_text(s: &mut Ws) -> String {
    loop {
      let t = next_raw_text(s).await;
      if t.contains(r#""t":"sync""#) || t.contains(r#""t":"offset""#) {
        continue;
      }
      return t;
    }
  }

  /// 收下一个文本控制帧（不过滤任何消息）。
  async fn next_raw_text(s: &mut Ws) -> String {
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
    let (addr, _hub) = serve().await;
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
    let (addr, _hub) = serve().await;
    let (status, _) = http_get(addr, "/nope").await;
    assert_eq!(status, 404, "未知路径应 404");
  }

  /// 握手成功回 `ready`；协议版本不符回 `err`（NETWORK.md §4.1）。
  #[tokio::test]
  async fn handshake_and_version_negotiation() {
    let (addr, _hub) = serve().await;
    let mut s = ws_connect(addr).await;
    send_text(&mut s, r#"{"t":"hello","proto":3}"#).await;
    let ready = next_text(&mut s).await;
    assert!(ready.contains(r#""t":"ready""#), "{ready}");
    assert!(ready.contains(r#""codec":"opus""#), "{ready}");
    assert!(
      ready.contains(r#""header_bytes":8"#),
      "v3 必须声明帧头长度：{ready}"
    );
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

  /// 远程输入默认关：文本与按键都必须被拒，且原因可读（NETWORK.md §4.1）。
  #[tokio::test]
  async fn remote_input_refused_until_enabled() {
    let (addr, _hub) = serve().await;
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
    let (addr, _hub) = serve().await;
    let mut s = ws_connect(addr).await;
    send_text(&mut s, r#"{"t":"ping","id":4242}"#).await;
    let pong = next_text(&mut s).await;
    assert!(pong.contains(r#""t":"pong""#), "{pong}");
    assert!(pong.contains("4242"), "{pong}");
  }

  /// 订阅之后：**引擎推一个 hop，连接立刻发一个包**（没有 10 ms 定时器、没有缓冲）。
  ///
  /// 旧实现是「定时器每 10 ms 从队列取一格」，那正是出站多出来的 20~50 ms 延迟的来源；
  /// 现在缓冲只在接收方。这里按真实节奏推（10 ms 一个），验证「推多少发多少」。
  #[tokio::test]
  async fn outbound_sends_immediately_per_hop() {
    let (addr, hub) = serve().await;
    let mut s = ws_connect(addr).await;
    send_text(&mut s, r#"{"t":"sub"}"#).await;
    let ready = next_text(&mut s).await;
    assert!(ready.contains(r#""t":"ready""#), "订阅应回 ready：{ready}");

    let started = std::time::Instant::now();
    // 模拟列工作线程：10 ms 一拍，共 50 拍（= 0.5 s 的音频）
    let producer = hub.clone();
    tokio::spawn(async move {
      let mut tick = tokio::time::interval(Duration::from_millis(10));
      for _ in 0..50 {
        tick.tick().await;
        producer.push_tx_hop(&[0.1; crate::audio::HOP]);
      }
    });

    let mut packets = 0usize;
    while packets < 50 && started.elapsed() < Duration::from_secs(3) {
      match tokio::time::timeout(Duration::from_millis(500), s.next()).await {
        Ok(Some(Ok(Message::Binary(_)))) => packets += 1,
        Ok(Some(Ok(_))) => continue,
        Ok(Some(Err(e))) => panic!("接收出错：{e}"),
        Ok(None) => panic!("连接被关闭"),
        Err(_) => {}
      }
    }
    assert_eq!(packets, 50, "推 50 个 hop 必须发 50 个包（实测 {packets}）");
    assert!(
      started.elapsed() < Duration::from_millis(1200),
      "推来即发：不该比音频本身的时间轴慢（{:?}）",
      started.elapsed()
    );
  }

  /// 出站包必须是**真的 Opus**：8 字节帧头 + 可解码的载荷，且序号连续。
  #[tokio::test]
  async fn outbound_packets_are_decodable_opus() {
    let (addr, hub) = serve().await;
    let mut s = ws_connect(addr).await;
    send_text(&mut s, r#"{"t":"sub"}"#).await;
    let _ = next_text(&mut s).await;

    let mut dec = Decoder::new(crate::audio::SAMPLE_RATE as i32).expect("解码器");
    // 取一包（跳过非二进制帧）
    async fn next_packet(s: &mut Ws) -> Vec<u8> {
      let deadline = std::time::Instant::now() + Duration::from_secs(3);
      loop {
        assert!(std::time::Instant::now() < deadline, "等出站包超时");
        match tokio::time::timeout(Duration::from_millis(500), s.next()).await {
          Ok(Some(Ok(Message::Binary(b)))) => return b.to_vec(),
          Ok(Some(Ok(_))) => continue,
          Ok(Some(Err(e))) => panic!("接收出错：{e}"),
          Ok(None) => panic!("连接被关闭"),
          Err(_) => {}
        }
      }
    }

    // ① 一个 1 kHz 正弦 hop：帧头 8 字节 + 可解码载荷 + 峰值对得上
    let amp = 0.25f32;
    let tone: Vec<f32> = (0..crate::audio::HOP)
      .map(|i| {
        amp
          * (2.0 * std::f32::consts::PI * 1000.0 * i as f32 / crate::audio::SAMPLE_RATE as f32)
            .sin()
      })
      .collect();
    hub.push_tx_hop(&tone);
    let b = next_packet(&mut s).await;
    let hdr = AudioHeader::parse(&b[..]).expect("v3 出站包必须带 8 字节帧头");
    let mut samples = Vec::new();
    dec
      .decode_packet(&b[8..], 0, &mut samples)
      .expect("出站包应能解码");
    assert_eq!(samples.len(), crate::audio::HOP, "PC 侧每包应恰好一 hop");
    let peak = samples.iter().fold(0.0f32, |m, x| m.max(x.abs()));
    assert!(
      (peak - amp).abs() < 0.05,
      "有信号时解出峰值应约 {amp}，实测 {peak}"
    );

    // ② 连续静音：Opus 是**有状态**的（重叠相加的尾巴会拖一帧），所以静音要几帧之后才真的静。
    //    这里推 20 个静音 hop，断言最后一包安静，同时验证序号连续（丢/重都不行）。
    let mut seqs = vec![hdr.seq];
    let mut last_peak = 1.0f32;
    for _ in 0..20 {
      hub.push_tx_hop(&[0.0; crate::audio::HOP]);
      let b = next_packet(&mut s).await;
      let h = AudioHeader::parse(&b[..]).expect("帧头");
      seqs.push(h.seq);
      let mut out = Vec::new();
      dec.decode_packet(&b[8..], 0, &mut out).expect("可解码");
      last_peak = out.iter().fold(0.0f32, |m, x| m.max(x.abs()));
    }
    assert!(
      last_peak < 5e-3,
      "连续静音后应解出静音，实测峰值 {last_peak}"
    );
    for w in seqs.windows(2) {
      assert_eq!(w[1], w[0].wrapping_add(1), "出站包序号必须连续：{seqs:?}");
    }
    assert!(hub.stats().tx_dropped == 0, "推来即发不该丢 hop");
    assert_eq!(
      seqs[1],
      seqs[0].wrapping_add(1),
      "出站包序号必须连续：{seqs:?}"
    );
  }

  /// 入站：真编码器造包发过去，服务端应解出样本并喂给引擎 hop 网格。
  /// 这里不能直接断言 hub 计数（连接任务在别的线程），但要确认**包被接受**（没有 err、
  /// 连接不崩、且过一会儿仍能正常收发）。
  #[tokio::test]
  async fn inbound_opus_accepted_without_error() {
    let (addr, _hub) = serve().await;
    let mut s = ws_connect(addr).await;
    send_text(&mut s, r#"{"t":"hello","proto":3}"#).await;
    let _ = next_text(&mut s).await;

    let mut enc = Encoder::new(crate::audio::SAMPLE_RATE as i32, 32_000).expect("编码器");
    let mut pkt = [0u8; MAX_PACKET_BYTES];
    let mut wire = [0u8; 8 + MAX_PACKET_BYTES];
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
      AudioHeader {
        seq: k as u32,
        ts_us: crate::net::rt::now_us32(),
      }
      .write(&mut wire);
      wire[8..8 + n].copy_from_slice(&pkt[..n]);
      s.send(Message::Binary(wire[..8 + n].to_vec()))
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

  /// 时钟同步：服务端发 `sync{t0}`，客户端回 `syncreply`，服务端回一个 `offset`。
  /// 这里模拟「客户端时钟比服务端慢 5 ms」，断言算出来是 −5000 µs（客户端 − 服务端）。
  #[tokio::test]
  async fn clock_sync_round_trip() {
    let (addr, _hub) = serve().await;
    let mut s = ws_connect(addr).await;
    // 等到第一条 sync（2 s 一轮，建连后马上就有）
    let sync = loop {
      let t = next_raw_text(&mut s).await;
      if t.contains(r#""t":"sync""#) {
        break t;
      }
    };
    let v: serde_json::Value = serde_json::from_str(&sync).expect("sync 是 JSON");
    let id = v["id"].as_u64().expect("id");
    let t0 = v["t0"].as_u64().expect("t0") as u32;
    // 客户端时钟慢 5 ms：t1 = t0 - 5000（收到），t2 = t1（立刻回）
    let t1 = t0.wrapping_sub(5_000);
    let reply = format!(r#"{{"t":"syncreply","id":{id},"t0":{t0},"t1":{t1},"t2":{t1}}}"#);
    send_text(&mut s, &reply).await;
    let off = loop {
      let t = next_raw_text(&mut s).await;
      if t.contains(r#""t":"offset""#) {
        break t;
      }
    };
    let v: serde_json::Value = serde_json::from_str(&off).expect("offset 是 JSON");
    let us = v["us"].as_i64().expect("us");
    assert!(
      (us + 5_000).abs() < 1_500,
      "偏移应约 −5000 µs（客户端时钟慢 5 ms），实测 {us}"
    );
  }

  /// 非法控制消息应回 err 而不是断连（客户端能自己看到原因）。
  #[tokio::test]
  async fn malformed_control_message_is_reported() {
    let (addr, _hub) = serve().await;
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
    let app = router(hub.clone(), String::new());
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
