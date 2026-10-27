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

//! WebSocket 服务（DESIGN.md §4.1）：一个端口承载三条能力。
//!
//! - `/` 网页版客户端（二维码指向这里，扫码即开）；`/ws` 音频 + 控制；`/health` 局域网自检
//! - 每连接一份 Opus 编解码器；断开即销毁
//! - 出站按**绝对时刻 10 ms 节拍**取 hop（与列工作线程同一套节拍纪律，DESIGN.md §3.3），
//!   欠载补静音包而不是停发（时钟不断，客户端不会累积延迟）

use std::sync::{Arc, Mutex};
use std::time::Duration;

use axum::Router;
use axum::extract::ws::{Message, WebSocket, WebSocketUpgrade};
use axum::response::IntoResponse;
use axum::routing::get;
use futures_util::{SinkExt, StreamExt};
use tokio::sync::mpsc;

use super::hub::{NetHub, OutboundSub};
use super::keymap;
use super::keys;
use super::opus_sys::{Decoder, Encoder, MAX_PACKET_BYTES};
use super::proto::{ClientMsg, ServerMsg};
use crate::audio::{HOP, SAMPLE_RATE};

/// 出站节拍 = 1 hop。
const TICK: Duration = Duration::from_millis(10);
/// 出站码率（bit/s）。
const TX_BITRATE: i32 = 32_000;
/// 连接空闲多久断开（秒）。
const IDLE_TIMEOUT: u64 = 60;

/// 路由表（跨平台）。`Arc<NetHub>` 作为 axum 状态。
pub fn router(hub: Arc<NetHub>, ca_pem: String) -> Router {
  Router::new()
    .route("/", get(web_page))
    .route("/ws", get(ws_entry))
    .route("/health", get(health))
    .route("/ca-cert", get(move || ca_cert_download(ca_pem.clone())))
    .route("/libopus-wasm.js", get(libopus_wasm_js))
    .route(
      "/generated/libopus.generated.mjs",
      get(libopus_wasm_generated),
    )
    .route("/report-caps", get(report_caps))
    .with_state(hub)
}

/// 浏览器端的 Opus **编解码器**（`libopus-wasm`，MIT，见 `LICENSE-THIRD-PARTY.txt`）。
///
/// **为什么不用浏览器内置的 WebCodecs**：`AudioEncoder` / `AudioDecoder` 只有 Chrome 94+ /
/// Safari 16.4+ 才有，实测有浏览器根本没有，于是麦克风与扬声器一起不可用。而 wasm 只要有
/// `WebAssembly`（所有现代浏览器）就能跑，**编解码还都在客户端、帧长按我们的 10 ms hop
/// 配置**，服务端 `opus.dll` 只管解码，两边对得上。
///
/// wasm 以 base64 内嵌在这个 `.mjs` 里（编译期产物），所以**只有一次 JS 请求**，没有第二个
/// `.wasm` 请求。路径必须是 `/generated/libopus.generated.mjs` —— `libopus-wasm.js` 内部写死
/// 了相对路径 `./generated/libopus.generated.mjs`，改路由就得同步改第三方产物，不如不动它。
const LIBOPUS_JS: &[u8] = include_bytes!("wasm/libopus-wasm.js");
const LIBOPUS_GENERATED: &[u8] = include_bytes!("wasm/generated/libopus.generated.mjs");

async fn libopus_wasm_js() -> impl IntoResponse {
  wasm_asset(LIBOPUS_JS)
}

async fn libopus_wasm_generated() -> impl IntoResponse {
  wasm_asset(LIBOPUS_GENERATED)
}

/// wasm 产物的响应头。
///
/// **必须给长缓存**：这两个文件是编译期产物，随版本变、运行期不变。手机上扫码会反复打开
/// 页面，没有缓存头就每次重下 377 KB；有了就第二次几乎瞬间。网页本身（`/`）反过来
/// **不能**缓存——它内嵌页面内容，改了就得立刻生效。
///
/// `no-cache`（而不是 `no-store`）意思是「可以存，但每次必须回服务器验证」：配合
/// `ETag` / `Last-Modified` 就能拿到 304，几 KB 的往返，远小于重下 377 KB。
fn wasm_asset(body: &'static [u8]) -> impl IntoResponse {
  use axum::http::header::{CACHE_CONTROL, CONTENT_TYPE, ETAG, HeaderValue};
  // 产物内容不变，用长度当 ETag 足够（变了长度必变；等长改动由发布新版本覆盖）
  let etag = format!("\"{}\"", body.len());
  (
    [
      (
        CONTENT_TYPE,
        HeaderValue::from_static("text/javascript; charset=utf-8"),
      ),
      (
        CACHE_CONTROL,
        HeaderValue::from_static("public, max-age=0, must-revalidate"),
      ),
      (
        ETAG,
        HeaderValue::from_str(&etag).unwrap_or_else(|_| HeaderValue::from_static("\"0\"")),
      ),
    ],
    body,
  )
}

/// 客户端能力上报。
///
/// 网页版浏览器缺什么只有它自己知道，而「麦克风用不了」有好几种完全不同的原因（不是安全
/// 上下文 / 没有 `AudioEncoder` / 浏览器不支持 Opus 编码 / 权限被拒），从服务端看一律表现
/// 为「没声音」，只能靠猜浏览器版本来分辨。让它自己报上来，`/debug` 的 `net` 段直接能读，
/// 这也正是 AGENTS.md §1「调试优先」要的东西：不要让人猜为什么用不了。
async fn report_caps(
  axum::extract::State(hub): axum::extract::State<Arc<NetHub>>,
  axum::extract::Query(q): axum::extract::Query<ClientCaps>,
) -> impl IntoResponse {
  let line = format!(
    "浏览器={}｜安全上下文={}｜采集={}｜编码器={}｜解码器={}｜Opus编码={}｜原因={}",
    q.ua, q.secure, q.mic, q.enc, q.dec, q.opus, q.why
  );
  hub.set_client_caps(line);
  axum::http::StatusCode::NO_CONTENT
}

/// 客户端能力上报的查询参（全用字符串，省一套自定义反序列化）。
#[derive(serde::Deserialize)]
struct ClientCaps {
  #[serde(default)]
  ua: String,
  #[serde(default)]
  secure: String,
  #[serde(default)]
  mic: String,
  #[serde(default)]
  enc: String,
  #[serde(default)]
  dec: String,
  #[serde(default)]
  opus: String,
  #[serde(default)]
  why: String,
}

/// CA 证书下载：手机装一次它，就再也不会看到「不安全」警告，也就**永久拿到安全上下文**
/// （`getUserMedia` 的前提）。
///
/// 为什么必须走 HTTPS 才能装：浏览器只在安全上下文里允许下载并保留用户证书——用
/// `http://` 连过来这一页本身就拿不到麦克风，正好是要修的问题。（另有一条命令行的路子，
/// 见 README；这里是给不方便用 adb 的人。）
async fn ca_cert_download(ca_pem: String) -> impl IntoResponse {
  use axum::http::header::{CONTENT_DISPOSITION, CONTENT_TYPE, HeaderValue};
  (
    [
      (
        CONTENT_TYPE,
        HeaderValue::from_static("application/x-x509-ca-cert"),
      ),
      (
        CONTENT_DISPOSITION,
        HeaderValue::from_static("attachment; filename=\"purevox-ca.crt\""),
      ),
    ],
    ca_pem,
  )
}

/// WebSocket 升级入口。
/// 网页版客户端（`GET /`）。二维码指向这里，扫码即开；它自己连 `/ws`，协议与 APK 同源。
async fn web_page() -> impl IntoResponse {
  use axum::http::header::{CACHE_CONTROL, CONTENT_TYPE, HeaderValue};
  (
    [
      (
        CONTENT_TYPE,
        HeaderValue::from_static("text/html; charset=utf-8"),
      ),
      // 页面**不许缓存**（它内嵌内容，改了要立刻生效）；而 `/libopus-wasm.js` 那些产物
      // 相反，要长缓存——见 `wasm_asset` 的注释
      (CACHE_CONTROL, HeaderValue::from_static("no-store")),
    ],
    super::webclient::PAGE,
  )
}

async fn ws_entry(
  axum::extract::State(hub): axum::extract::State<Arc<NetHub>>,
  ws: WebSocketUpgrade,
) -> impl IntoResponse {
  ws.on_upgrade(move |socket| handle(socket, hub))
}

/// 局域网自检：手机端与人都能看（不需要任何客户端）。
async fn health(axum::extract::State(hub): axum::extract::State<Arc<NetHub>>) -> impl IntoResponse {
  let s = hub.stats();
  (
    axum::http::StatusCode::OK,
    axum::Json(serde_json::json!({
      "app": "PureVox",
      "proto": super::proto::PROTO,
      "port": s.port,
      "clients": s.clients,
      "subs": s.subs,
      "rx_packets": s.rx_packets,
      "rx_dropped": s.rx_dropped,
      "rx_level": s.rx_level,
      "rx_underruns": s.rx_underruns,
      "tx_packets": s.tx_packets,
      "tx_dropped": s.tx_dropped,
      "tx_level": s.tx_level,
      "tx_subs": s.tx_subs,
      "remote_input": s.remote_input,
      "rtt_ms": s.rtt_ms,
      "last_error": s.last_error,
    })),
  )
}

/// 写 socket 的消息（音频包 / 控制文本共用一条出口，保证写顺序单一来源）。
enum Out {
  Packet(Vec<u8>),
  Text(String),
}

/// 一条连接：一个收任务 + 一个写任务。
async fn handle(socket: WebSocket, hub: Arc<NetHub>) {
  hub.client_connected();
  let (mut sink, mut stream) = socket.split();
  let (out_tx, mut out_rx) = mpsc::unbounded_channel::<Out>();
  // 订阅状态由收任务写、编码任务读
  let sub: Arc<Mutex<Option<(u64, OutboundSub)>>> = Arc::new(Mutex::new(None));
  // 没有订阅者时编码任务**睡觉**，不要空转 100 次/s（手机只连不用扬声器时的耗电大头）
  let sub_ready = Arc::new(tokio::sync::Notify::new());
  let (sub_out, out_o, hub_o, sub_wake) =
    (sub.clone(), out_tx.clone(), hub.clone(), sub_ready.clone());
  let wake_in_task = sub_wake.clone();

  // 出站：10 ms 绝对节拍 → 取 hop → Opus 编码
  let encoder_task = tokio::spawn(async move {
    let mut enc: Option<Encoder> = None;
    let mut hop = vec![0.0f32; HOP];
    let mut pkt = [0u8; MAX_PACKET_BYTES];
    // **绝对时刻节拍**（与列工作线程同一套纪律，DESIGN.md §3.3）：
    // 每轮对齐到 10 ms 网格再睡，而不是「睡 10 ms 减去本轮已用」。
    // 后者把 sleep 过冲逐轮累加，实测只产 ~64 包/s（正是本模块初版的实测值）。
    let period = Duration::from_nanos(10_000_000);
    let mut next = tokio::time::Instant::now() + period;
    loop {
      // 没有订阅者就等通知，不占 CPU / 唤醒次数
      let notified = wake_in_task.notified();
      if sub_out.lock().unwrap().is_none() {
        notified.await;
        next = tokio::time::Instant::now() + period;
        continue;
      }
      // 等到下一个网格时刻；落后超过一拍就重新对齐，不补发积压
      loop {
        let now = tokio::time::Instant::now();
        if next <= now {
          break;
        }
        tokio::time::sleep_until(next).await;
      }
      let now = tokio::time::Instant::now();
      if now.duration_since(next) > Duration::from_millis(200) {
        next = now + period;
      } else {
        next += period;
      }
      // 锁内不做 await
      let got = {
        let g = sub_out.lock().unwrap();
        g.as_ref().map(|(_, s)| s.take_hop(&mut hop))
      };
      let Some(got) = got else { continue };
      if !got {
        hop.iter_mut().for_each(|x| *x = 0.0);
      }
      if enc.is_none() {
        match Encoder::new(SAMPLE_RATE as i32, TX_BITRATE) {
          Ok(e) => enc = Some(e),
          Err(e) => {
            hub_o.set_last_error(format!("出站编码器：{e}"));
            return;
          }
        }
      }
      // SAFETY 前提：上面刚确保 enc 是 Some
      let e = enc.as_mut().expect("enc 已建");
      match e.encode(&hop, &mut pkt) {
        Ok(n) => {
          if out_o.send(Out::Packet(pkt[..n].to_vec())).is_err() {
            break;
          }
          hub_o.count_tx_packet();
        }
        Err(e) => hub_o.set_last_error(format!("出站编码：{e}")),
      }
    }
  });

  // 唯一写 socket 的任务
  let writer_task = tokio::spawn(async move {
    while let Some(o) = out_rx.recv().await {
      let r = match o {
        Out::Packet(p) => sink.send(Message::Binary(p.into())).await,
        Out::Text(t) => sink.send(Message::Text(t.into())).await,
      };
      if r.is_err() {
        break;
      }
    }
  });

  // 收：音频包 → 入站队列；文本 → 控制
  let mut dec = Decoder::new(SAMPLE_RATE as i32).ok();
  let mut samples: Vec<f32> = Vec::with_capacity(SAMPLE_RATE as usize / 50);
  let mut last_rx = tokio::time::Instant::now();

  while let Some(msg) = stream.next().await {
    match msg {
      Ok(Message::Binary(b)) => {
        last_rx = tokio::time::Instant::now();
        if let Some(d) = dec.as_mut() {
          match d.decode_packet(&b, 0, &mut samples) {
            Ok(_) => hub.push_rx(&samples),
            Err(e) => hub.set_last_error(format!("入站解码：{e}")),
          }
        }
      }
      Ok(Message::Text(t)) => {
        last_rx = tokio::time::Instant::now();
        let mut reply: Option<ServerMsg> = None;
        match serde_json::from_str::<ClientMsg>(&t) {
          Err(_) => reply = Some(ServerMsg::err("控制消息格式不对")),
          Ok(ClientMsg::Hello { proto }) => {
            if proto != super::proto::PROTO {
              let _ = out_tx.send(Out::Text(
                ServerMsg::err(format!(
                  "协议版本不符：服务端 {}，客户端 {proto}",
                  super::proto::PROTO
                ))
                .to_text(),
              ));
              break;
            }
            reply = Some(ready(&hub));
          }
          Ok(ClientMsg::Sub) => {
            // 重新订阅（例如网页版关掉扬声器又打开）必须**先退掉旧的**：
            // 否则出站列表里会多留一条没人消费的队列，水位顶格、白占 CPU。
            if let Some((old_id, _)) = sub.lock().unwrap().take() {
              hub.unsubscribe_tx(old_id);
              super::pacer_release();
            }
            let (id, s) = hub.subscribe_tx();
            *sub.lock().unwrap() = Some((id, s));
            // 只有真的有订阅者时才把 Windows 定时器精度提到 1 ms（见 net/mod.rs PACER）
            super::pacer_acquire();
            sub_wake.notify_one();
            reply = Some(ready(&hub));
          }
          Ok(ClientMsg::Unsub) => {
            if let Some((id, _)) = sub.lock().unwrap().take() {
              hub.unsubscribe_tx(id);
              super::pacer_release();
            }
            reply = Some(ready(&hub));
          }
          Ok(ClientMsg::Text { s }) => {
            if !hub.remote_input() {
              reply = Some(ServerMsg::err("远程输入未开启（界面开关）"));
            } else {
              match keys::inject_text(&s) {
                Ok(n) => hub.count_chars(n),
                Err(e) => reply = Some(ServerMsg::err(format!("文本注入失败：{e}"))),
              }
            }
          }
          Ok(ClientMsg::Key { code, down }) => {
            if !hub.remote_input() {
              reply = Some(ServerMsg::err("远程输入未开启（界面开关）"));
            } else {
              match keymap::scan(code) {
                Some(sc) => match keys::inject_key(sc, down) {
                  Ok(()) => hub.count_key(true),
                  Err(e) => {
                    hub.count_key(false);
                    reply = Some(ServerMsg::err(format!("按键注入失败：{e}")));
                  }
                },
                None => hub.count_key(false),
              }
            }
          }
          Ok(ClientMsg::Ping { id }) => reply = Some(ServerMsg::Pong { id }),
          Ok(ClientMsg::Rtt { ms }) => hub.set_rtt(ms),
        }
        if let Some(r) = reply {
          let _ = out_tx.send(Out::Text(r.to_text()));
        }
      }
      Ok(_) => {}
      Err(_) => break,
    }
    if last_rx.elapsed() > Duration::from_secs(IDLE_TIMEOUT) {
      hub.set_last_error("客户端空闲超时，已断开");
      break;
    }
  }

  // 收尾：退订；**让写任务自然结束**（先丢掉发送端让通道关闭，再等它把队列里
  // 已排队的 err 真正冲出去）—— 直接 abort 会把「协议版本不符」这类 err 吞掉。
  if let Some((id, _)) = sub.lock().unwrap().take() {
    hub.unsubscribe_tx(id);
    super::pacer_release();
  }
  encoder_task.abort();
  drop(out_tx);
  let _ = tokio::time::timeout(Duration::from_millis(500), writer_task).await;
  hub.client_disconnected();
}

/// `ready` 消息（握手与订阅都回它，客户端据此确认可用）。
fn ready(hub: &NetHub) -> ServerMsg {
  ServerMsg::Ready {
    proto: super::proto::PROTO,
    codec: "opus",
    rate: SAMPLE_RATE,
    channels: 1,
    frame_ms: TICK.as_millis() as u32,
    clients: hub.stats().clients,
  }
}
#[cfg(test)]
mod tests {
  use super::*;
  use crate::net::opus_sys::{Decoder, Encoder, MAX_PACKET_BYTES};

  /// 端到端（不经真实网络）：PC 编码器 → 解码器 → 入站队列 → hop。
  /// 验证「一包 = 一 hop」这条契约在真实 libopus 上成立，且 hop 边界不丢样本。
  #[test]
  fn pc_to_engine_hop_contract() {
    let hub = NetHub::new();
    let mut enc = Encoder::new(SAMPLE_RATE as i32, 32_000).expect("编码器");
    let mut dec = Decoder::new(SAMPLE_RATE as i32).expect("解码器");
    let mut pkt = [0u8; MAX_PACKET_BYTES];
    let mut frame = vec![0.0f32; HOP];
    let mut got = [0.0f32; HOP];
    for k in 0..12usize {
      for (i, x) in frame.iter_mut().enumerate() {
        *x = if k < 8 {
          0.3
            * (2.0 * std::f32::consts::PI * 1000.0 * ((k * HOP + i) as f32) / SAMPLE_RATE as f32)
              .sin()
        } else {
          0.0
        };
      }
      let n = enc.encode(&frame, &mut pkt).expect("编码");
      let mut samples = Vec::with_capacity(SAMPLE_RATE as usize / 50);
      dec.decode_packet(&pkt[..n], 0, &mut samples).expect("解码");
      // 契约：PC 侧发 10 ms 包 → 收端恰好解出 480 样本（一个 hop）
      assert_eq!(samples.len(), HOP, "第 {k} 包不是一 hop");
      hub.push_rx(&samples);
      hub.take_rx_hop(&mut got);
    }
    let st = hub.stats();
    assert_eq!(st.rx_packets, 12);
    assert_eq!(st.rx_hops, 12, "应每个包取走一个 hop");
    assert_eq!(st.rx_underruns, 0, "一包一 hop 时不应欠载");
    assert_eq!(st.rx_dropped, 0, "水位远低于 80 ms 上限，不应丢样本");
    assert_eq!(hub.rx_level(), 0, "取完应为空");
  }

  /// 出站：推 hop → 订阅者能取到；欠载返回 false（发静音包保时钟）。
  #[test]
  fn outbound_tap_and_underrun() {
    let hub = NetHub::new();
    let (id, sub) = hub.subscribe_tx();
    assert_eq!(hub.sub_count(), 1);
    let mut out = [0.0f32; HOP];
    assert!(!sub.take_hop(&mut out), "没有数据时应欠载（发静音）");
    hub.push_tx_hop(&[0.25; HOP]);
    assert!(sub.take_hop(&mut out));
    assert!(out.iter().all(|x| *x == 0.25));
    hub.unsubscribe_tx(id);
    assert_eq!(hub.sub_count(), 0);
  }

  /// 出站节拍必须是 10 ms（= 1 hop），否则出站采样率会偏。
  #[test]
  fn tick_is_one_hop() {
    assert_eq!(TICK, Duration::from_millis(10));
    // 注：HOP 是样本数（480）、TICK 是时长（10 ms），两者不能直接相等；换算后必须相等
    assert_eq!(TICK.as_secs_f64() * SAMPLE_RATE as f64, HOP as f64);
    assert_eq!(SAMPLE_RATE as usize / 100, HOP, "hop 应由采样率派生");
  }

  /// 路由表必须能构造（axum 内部校验路由不冲突）。
  #[test]
  fn router_builds() {
    // CA PEM 只是被 `/ca-cert` 的闭包捕获，构造路由时不解析它，所以这里给空串即可
    let _ = router(Arc::new(NetHub::new()), String::new());
  }
}
