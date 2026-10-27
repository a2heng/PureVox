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
//! - **出站推来即发**：引擎每出一个 hop，这里的发送任务立刻编码发走 —— 服务端**没有定时器、
//!   没有队列**（缓冲只在接收方，DESIGN.md §4.1.2/§4.1.4）。没有信号就没有包：
//!   客户端的 AudioTrack 自己就是时钟，不需要服务端发静音包「保节拍」。
//!
//! ## v3 新增的两件东西
//!
//! 1. **音频帧头 8 字节**（`seq` + 发送方 µs）——收端据此判丢包 / 乱序、算单向延迟；
//!    丢包用 Opus 的 PLC 补（`Decoder::decode_plc`），**不是**在缓冲里补静音。
//! 2. **时钟同步 `sync` / `syncreply` / `offset`**——NTP 四时戳，保留最小 RTT 样本，
//!    于是「单向延迟」是**量出来的**，不是猜的（AGENTS §1.4）。

use std::sync::{Arc, Mutex};
use std::time::Duration;

use axum::Router;
use axum::extract::ws::{Message, WebSocket, WebSocketUpgrade};
use axum::response::IntoResponse;
use axum::routing::get;
use futures_util::{SinkExt, StreamExt};
use tokio::sync::mpsc;

use super::hub::{ClientReport, NetHub, OutboundSub};
use super::keymap;
use super::keys;
use super::opus_sys::{Decoder, Encoder, MAX_PACKET_BYTES};
use super::proto::{ClientMsg, ServerMsg};
use super::rt::{AudioHeader, HDR_LEN, us_diff};
use crate::audio::{HOP, SAMPLE_RATE};

/// 一帧的时长（ms），由 hop 派生（§3：10 ms 网格）。
const FRAME_MS: u32 = (HOP as u32 * 1000) / SAMPLE_RATE;
/// 出站码率（bit/s）。
const TX_BITRATE: i32 = 32_000;
/// 连接空闲多久断开（秒）。
const IDLE_TIMEOUT: u64 = 60;
/// 时钟同步的探测间隔。
const SYNC_PERIOD: Duration = Duration::from_secs(2);
/// 时钟偏移**变化超过**这个量才重发 `offset`（避免每 2 s 一条冗余控制帧）。
const OFFSET_RESEND_US: i64 = 2_000;

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
fn wasm_asset(body: &'static [u8]) -> impl IntoResponse {
  use axum::http::header::{CACHE_CONTROL, CONTENT_TYPE, ETAG, HeaderValue};
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

/// 客户端能力上报（网页版自己说清「为什么用不了麦克风」）。
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

/// CA 证书下载：手机装一次它，就再也不会看到「不安全」警告，也就**永久拿到安全上下文**。
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

/// 网页版客户端（`GET /`）。
async fn web_page() -> impl IntoResponse {
  use axum::http::header::{CACHE_CONTROL, CONTENT_TYPE, HeaderValue};
  (
    [
      (
        CONTENT_TYPE,
        HeaderValue::from_static("text/html; charset=utf-8"),
      ),
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
      "header_bytes": HDR_LEN,
      "port": s.port,
      "clients": s.clients,
      "subs": s.subs,
      "rx_packets": s.rx_packets,
      "rx_dropped": s.rx_dropped,
      "rx_level": s.rx_level,
      "rx_underruns": s.rx_underruns,
      "rx_plc": s.rx_plc,
      "rx_trims": s.rx_trims,
      "rx_late": s.rx_late,
      "rx_lost": s.rx_lost,
      "rx_latency_ms": s.rx_latency_ms,
      "rx_jitter_ms": s.rx_jitter_ms,
      "rx_max_gap_ms": s.rx_max_gap_ms,
      "rx_target_ms": s.rx_target_ms,
      "tx_packets": s.tx_packets,
      "tx_dropped": s.tx_dropped,
      "tx_level": s.tx_level,
      "tx_subs": s.tx_subs,
      "client": s.client,
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

/// 一条连接：一个收任务 + 一个出站编码任务 + 一个时钟同步任务 + 一个写任务。
async fn handle(socket: WebSocket, hub: Arc<NetHub>) {
  hub.client_connected();
  let (mut sink, mut stream) = socket.split();
  let (out_tx, mut out_rx) = mpsc::unbounded_channel::<Out>();
  // 订阅状态：收任务把句柄交给发送任务（`None` = 退订）
  let mut cur_sub: Option<u64> = None;

  // ---- 出站：**推来即发**（引擎每出一 hop 就发一包，不等定时器，不建缓冲）----
  //
  // 为什么不在服务端做定时器 + 队列：那等于在「引擎时钟」和「定时器时钟」之间再插一级
  // 缓冲，延迟白送、还要用伺服去追两套时钟。缓冲只需要在**接收方**（手机的播放环）：
  // 网络抖动和手机晶振差都在那里被吸收（DESIGN.md §4.1）。
  let (sub_tx, mut sub_rx) = mpsc::channel::<Option<OutboundSub>>(4);
  let (out_o, hub_o) = (out_tx.clone(), hub.clone());
  let send_task = tokio::spawn(async move {
    let mut cur: Option<OutboundSub> = None;
    let mut enc: Option<Encoder> = None;
    let mut payload = [0u8; MAX_PACKET_BYTES];
    let mut hdr_bytes = [0u8; HDR_LEN];
    let mut seq: u32 = 0;
    loop {
      if cur.is_none() {
        match sub_rx.recv().await {
          Some(Some(s)) => cur = Some(s),
          Some(None) => continue,
          None => break,
        }
        continue;
      }
      tokio::select! {
        m = sub_rx.recv() => match m {
          Some(Some(s)) => cur = Some(s),
          Some(None) => cur = None,
          None => break,
        },
        hop = cur.as_mut().expect("上面已确保 Some").recv() => {
          let Some(hop) = hop else { cur = None; continue };
          if enc.is_none() {
            match Encoder::new(SAMPLE_RATE as i32, TX_BITRATE) {
              Ok(e) => enc = Some(e),
              Err(e) => {
                hub_o.set_last_error(format!("出站编码器：{e}"));
                return;
              }
            }
          }
          let e = enc.as_mut().expect("enc 已建");
          match e.encode(&hop, &mut payload) {
            Ok(n) => {
              AudioHeader { seq, ts_us: super::rt::now_us32() }.write(&mut hdr_bytes);
              seq = seq.wrapping_add(1);
              let mut msg = Vec::with_capacity(HDR_LEN + n);
              msg.extend_from_slice(&hdr_bytes);
              msg.extend_from_slice(&payload[..n]);
              if out_o.send(Out::Packet(msg)).is_err() {
                break;
              }
              hub_o.count_tx_packet();
              if let Some(s) = cur.as_ref() {
                s.note_sent();
              }
            }
            Err(e) => hub_o.set_last_error(format!("出站编码：{e}")),
          }
        }
      }
    }
  });

  // ---- 环深度广播：任一侧改了，所有客户端立刻收到 `target`（含发起方）----
  let (tgt_out, mut tgt_rx) = (out_tx.clone(), hub.subscribe_target());
  let target_task = tokio::spawn(async move {
    while let Ok(ms) = tgt_rx.recv().await {
      if tgt_out
        .send(Out::Text(ServerMsg::Target { ms }.to_text()))
        .is_err()
      {
        return;
      }
    }
  });

  // ---- 时钟同步：每 2 s 一个 `sync`；应答在收任务里算（见 `SyncState`）----
  let sync = Arc::new(Mutex::new(SyncState::default()));
  let (sync_out, sync_state) = (out_tx.clone(), sync.clone());
  let sync_task = tokio::spawn(async move {
    let mut id: u64 = 0;
    // 第一次探测**延后 300 ms**：握手/订阅的应答必须先到，客户端不该在建连瞬间
    // 就收到一条它还没准备好的 `sync`（也让单测里的「下一条文本是 ready」保持成立）。
    tokio::time::sleep(Duration::from_millis(300)).await;
    let mut tick = tokio::time::interval(SYNC_PERIOD);
    tick.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Delay);
    loop {
      tick.tick().await;
      let t0 = super::rt::now_us32();
      sync_state.lock().unwrap().pending = Some((id, t0));
      if sync_out
        .send(Out::Text(ServerMsg::Sync { id, t0 }.to_text()))
        .is_err()
      {
        return;
      }
      id = id.wrapping_add(1);
    }
  });

  // 唯一写 socket 的任务
  let writer_hub = hub.clone();
  let writer_task = tokio::spawn(async move {
    while let Some(o) = out_rx.recv().await {
      let r = match o {
        Out::Packet(p) => {
          // 发包间隔在这里量：**只观测**（理想 10.0 ms；<5 ms = 一拍连发两包）
          writer_hub.note_tx_sent();
          sink.send(Message::Binary(p.into())).await
        }
        Out::Text(t) => sink.send(Message::Text(t.into())).await,
      };
      if r.is_err() {
        break;
      }
    }
  });

  // ---- 收：音频包 → 解码（缺包先补 PLC）→ 入站缓冲；文本 → 控制 ----
  let mut dec = Decoder::new(SAMPLE_RATE as i32).ok();
  let mut samples: Vec<f32> = Vec::with_capacity(SAMPLE_RATE as usize / 50);
  let mut plc: Vec<f32> = Vec::with_capacity(SAMPLE_RATE as usize / 50);
  let mut expected_seq: Option<u32> = None;
  let mut last_rx = tokio::time::Instant::now();

  while let Some(msg) = stream.next().await {
    match msg {
      Ok(Message::Binary(b)) => {
        last_rx = tokio::time::Instant::now();
        let Some(hdr) = AudioHeader::parse(&b[..]) else {
          hub.set_last_error(format!("音频包太短（{} 字节，帧头要 {HDR_LEN}）", b.len()));
          continue;
        };
        if let Some(d) = dec.as_mut() {
          // 序号不连续 = 真的丢了包：先用解码器 PLC 补上缺的那几格再解这一包，
          // 这样入站缓冲里的时间轴是连续的（补静音会让后面所有样本错位）。
          if let Some(exp) = expected_seq {
            let gap = hdr.seq.wrapping_sub(exp);
            if gap > 0 && gap < 100 {
              for _ in 0..gap {
                if let Ok(n) = d.decode_plc(&mut plc) {
                  hub.push_rx(&plc[..n]);
                  hub.note_rx_plc();
                }
              }
              hub.note_rx_lost(gap as u64);
            }
          }
          match d.decode_packet(&b[HDR_LEN..], 0, &mut samples) {
            Ok(_) => {
              // 单向延迟（有偏移估计时才算）：`ts` 是手机时钟，加上偏移换到电脑时钟
              if let Some(off) = hub.clock_offset() {
                let us = us_diff(super::rt::now_us32(), hdr.ts_us) + off;
                // 只信 0~5 s 的合理值：时钟偏移估计偶尔会跳，别把统计带飞
                if (0..5_000_000).contains(&us) {
                  hub.note_rx_latency(us);
                }
              }
              hub.push_rx(&samples);
              expected_seq = Some(hdr.seq.wrapping_add(1));
            }
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
                  "协议版本不符：服务端 {}，客户端 {proto}（请更新客户端）",
                  super::proto::PROTO
                ))
                .to_text(),
              ));
              break;
            }
            reply = Some(ready(&hub));
          }
          Ok(ClientMsg::Sub) => {
            // 重新订阅必须先退旧的，否则出站列表里多留一条没人消费的通道
            if let Some(old) = cur_sub.take() {
              hub.unsubscribe_tx(old);
            }
            let (id, s) = hub.subscribe_tx();
            cur_sub = Some(id);
            let _ = sub_tx.try_send(Some(s));
            reply = Some(ready(&hub));
          }
          Ok(ClientMsg::Unsub) => {
            if let Some(id) = cur_sub.take() {
              hub.unsubscribe_tx(id);
            }
            let _ = sub_tx.try_send(None);
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
          // 改环深度：落盘 + 广播给**所有**客户端（含发起方）—— 两侧 UI 与环永远同一个值。
          // 只走 `net::set_ring_ms` 这一条路（它内部才 `hub.set_ring_ms` 并广播一次），
          // 否则同一改动会广播两遍（客户端日志里会出现两条「电脑把抖动缓冲改成 X」）。
          Ok(ClientMsg::SetTarget { ms }) => {
            crate::net::set_ring_ms(ms);
          }
          Ok(ClientMsg::St {
            level_ms,
            target_ms,
            underruns,
            concealed,
            late,
            clock_ppm,
            one_way_ms,
          }) => {
            hub.set_client_report(ClientReport {
              level_ms: level_ms as f32,
              target_ms: target_ms as f32,
              underruns,
              concealed,
              late,
              one_way_ms,
              clock_ppm,
              at_ms: super::rt::now_us32() as u64 / 1000,
            });
          }
          Ok(ClientMsg::SyncReply { id, t0, t1, t2 }) => {
            // 只认「正在等的那一轮」的应答：乱序 / 重传的旧应答会把偏移算歪
            let pending = sync.lock().unwrap().pending.map(|(pid, _)| pid);
            if pending != Some(id) {
              continue;
            }
            // NTP 四时戳：t3 = 现在（服务端时钟）
            let t3 = super::rt::now_us32();
            let rtt = us_diff(t3, t0) - us_diff(t2, t1);
            let off = (us_diff(t1, t0) + us_diff(t2, t3)) / 2;
            if (0..500_000).contains(&rtt) {
              let mut g = sync.lock().unwrap();
              // 保留**最小 RTT** 样本（排队最少 = 最接近真实偏移）；
              // 只有明显更好（>1 ms）才替换，避免拥塞抖动把偏移带着来回跳。
              let better = match g.best {
                Some((b, _)) => rtt < b - 1_000,
                None => true,
              };
              if better {
                g.best = Some((rtt, off));
              }
              let (rtt_best, off_best) = g.best.unwrap_or((rtt, off));
              hub.set_clock_offset(off_best);
              hub.set_rtt(rtt_best as f64 / 1000.0);
              // 偏移变了才重发：客户端要用它算下行单向延迟
              if g
                .last_sent
                .is_none_or(|prev| (prev - off_best).abs() > OFFSET_RESEND_US)
              {
                g.last_sent = Some(off_best);
                let _ = out_tx.send(Out::Text(ServerMsg::Offset { us: off_best }.to_text()));
              }
            }
          }
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

  // 收尾：退订；让写任务自然结束（先 drop 发送端再等它把排队的 err 冲出去）
  if let Some(id) = cur_sub.take() {
    hub.unsubscribe_tx(id);
  }
  drop(sub_tx);
  let _ = tokio::time::timeout(Duration::from_millis(200), send_task).await;
  sync_task.abort();
  target_task.abort();
  drop(out_tx);
  let _ = tokio::time::timeout(Duration::from_millis(500), writer_task).await;
  hub.client_disconnected();
}

/// 一条连接的时钟同步状态。**逐连接**：多客户端各自的偏移不同，不能共用一个全局值。
#[derive(Default)]
struct SyncState {
  /// 已发出、等应答的 `(id, t0)`（只用于日志语义；应答里带着 t0，不依赖它）
  pending: Option<(u64, u32)>,
  /// 目前最好的样本（最小 RTT）：`(rtt_us, offset_us)`
  best: Option<(i64, i64)>,
  /// 上次发给客户端的偏移（变了才重发）
  last_sent: Option<i64>,
}

/// `ready` 消息（握手与订阅都回它，客户端据此确认可用）。
fn ready(hub: &NetHub) -> ServerMsg {
  ServerMsg::Ready {
    proto: super::proto::PROTO,
    codec: "opus",
    rate: SAMPLE_RATE,
    header_bytes: HDR_LEN as u32,
    // 客户端播放环的建议目标：出站没有服务端缓冲了，缓冲只在接收方（见 hub 的 OutboundSub）
    target_ms: crate::net::rt::RING_MS as u32,
    channels: 1,
    frame_ms: FRAME_MS,
    clients: hub.stats().clients,
  }
}

#[cfg(test)]
mod tests {
  use super::*;
  use crate::net::opus_sys::{Decoder, Encoder, MAX_PACKET_BYTES};
  use crate::net::rt::AudioHeader;

  /// 端到端（不经真实网络）：PC 编码器 → 解码器 → 入站缓冲 → hop。
  /// 验证「一包 = 一 hop」这条契约在真实 libopus 上成立，且带帧头时载荷偏移正确。
  #[test]
  fn pc_to_engine_hop_contract() {
    let hub = NetHub::new();
    let mut enc = Encoder::new(SAMPLE_RATE as i32, 32_000).expect("编码器");
    let mut dec = Decoder::new(SAMPLE_RATE as i32).expect("解码器");
    let mut pkt = [0u8; HDR_LEN + MAX_PACKET_BYTES];
    let mut payload = [0u8; MAX_PACKET_BYTES];
    let mut frame = vec![0.0f32; HOP];
    let mut got = [0.0f32; HOP];
    // 预热到固定深度（20 ms = 2 格）
    for _ in 0..2 {
      hub.push_rx(&[0.1; HOP]);
    }
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
      let n = enc.encode(&frame, &mut payload).expect("编码");
      pkt[HDR_LEN..HDR_LEN + n].copy_from_slice(&payload[..n]);
      AudioHeader {
        seq: k as u32,
        ts_us: crate::net::rt::now_us32(),
      }
      .write(&mut pkt);
      // 契约：帧头 8 字节 + 载荷，收端按 header_bytes 切
      let hdr = AudioHeader::parse(&pkt[..]).expect("帧头");
      assert_eq!(hdr.seq, k as u32);
      let mut samples = Vec::with_capacity(SAMPLE_RATE as usize / 50);
      dec
        .decode_packet(&pkt[HDR_LEN..HDR_LEN + n], 0, &mut samples)
        .expect("解码");
      assert_eq!(samples.len(), HOP, "第 {k} 包不是一 hop");
      hub.push_rx(&samples);
      hub.take_rx_hop(&mut got);
    }
    let st = hub.stats();
    // 12 个真实包 + 2 个预热带（`push_rx` 每调一次就记一个包）
    assert_eq!(st.rx_packets, 14);
    // 新策略：超过实时性上限就丢最旧的 —— 只要求水位有界、包数正确
    assert!(
      st.rx_level_ms <= 20.0 + 10.0,
      "水位必须有界：{} ms",
      st.rx_level_ms
    );
    assert!(st.rx_hops >= 10, "应取走大部分 hop：{}", st.rx_hops);
  }

  /// 出站：引擎推一 hop，连接任务立刻拿到（没有定时器、没有缓冲）。
  #[tokio::test]
  async fn outbound_hands_hop_over_immediately() {
    let hub = NetHub::new();
    let (id, mut sub) = hub.subscribe_tx();
    hub.push_tx_hop(&[0.25; HOP]);
    let hop = tokio::time::timeout(Duration::from_millis(50), sub.recv())
      .await
      .expect("推来即发，不该等 10 ms 定时器")
      .expect("订阅还在");
    assert!(hop.iter().all(|x| (*x - 0.25).abs() < 1e-6));
    hub.unsubscribe_tx(id);
    assert!(sub.recv().await.is_none(), "退订后 recv 应结束");
  }

  /// 一帧 = 10 ms = 1 hop，由采样率派生（§3 的 10 ms 网格）。
  #[test]
  fn frame_is_one_hop() {
    assert_eq!(FRAME_MS, 10);
    assert_eq!(FRAME_MS as f64 / 1000.0 * SAMPLE_RATE as f64, HOP as f64);
    assert_eq!(SAMPLE_RATE as usize / 100, HOP, "hop 应由采样率派生");
  }

  /// 路由表必须能构造（axum 内部校验路由不冲突）。
  #[test]
  fn router_builds() {
    let _ = router(Arc::new(NetHub::new()), String::new());
  }

  /// NTP 四时戳的算法：同一台机器上（两个时钟相同）偏移必须是 0，RTT 必须等于往返时间。
  #[test]
  fn ntp_offset_math() {
    // 单向各 100 µs、客户端时钟比服务端**快 1000 µs**：
    //   服务端 t0 = 0 发出 → 客户端在服务端时刻 100 收到 = 客户端 1100（t1）
    //   客户端 1105（t2）回 → 服务端 205（t3）收到
    let (t0, t1, t2, t3) = (0u32, 1100u32, 1105u32, 205u32);
    let rtt = us_diff(t3, t0) - us_diff(t2, t1);
    let off = (us_diff(t1, t0) + us_diff(t2, t3)) / 2;
    assert_eq!(rtt, 200, "RTT = (t3-t0) - (t2-t1)");
    assert_eq!(off, 1000, "偏移 = 客户端 − 服务端");
    // 同一时钟（无偏移）时 offset 必须为 0，否则会把延迟算歪
    let (t0, t1, t2, t3) = (1000u32, 1500u32, 1500u32, 2000u32);
    let off = (us_diff(t1, t0) + us_diff(t2, t3)) / 2;
    assert_eq!(off, 0);
  }
}
