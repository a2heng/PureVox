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

//! 本机 HTTP 调试接口：只绑 127.0.0.1、只读 JSON、只有 GET。
//!
//! 并发处理：每个连接一个短线程（调试口请求量小，但**一个慢客户端不能堵住其它人**；
//! 从前单线程顺序处理，一个不读响应的 curl 就能让面板/脚本一起超时）。
//!
//! 端点（AGENTS §1.2）：
//! - `/debug` 汇总（`?full=0` 去掉波形/频谱，给脚本轮询）
//! - `/debug/system` / `/debug/audio` / `/debug/devices`
//! - `/debug/logs?limit=N` 事件日志
//! - `/debug/metrics` 2 分钟指标历史
//! - `/debug/plan` 当前计划
//! - `/debug/calib` AEC 校准的结构化报告（与快照 `calib_report`、`calib_last.json` 同源）
//! - `/debug/bundle` 一键诊断包（快照 + 日志 + 指标）
//! - `/debug/ui` 调试面板页面本身（**给浏览器打开**；CSS/JS 同源静态资源随二进制内嵌，
//!   见 [`static_asset`]）。它只 `fetch('/debug')` 渲染，不经过 WebView2 / IPC。
//!
//! 为什么面板页面走这里而不是 Tauri 的第二个 WebView2 窗口：Tauri 3 alpha 在本机建第二个
//! WebView2 时**页面根本不会导航**（CDP 里目标停在 `about:blank`、`Chrome_WidgetWin_0`
//! 是 0×0，整窗纯白；2026-10-26 实测）。调试面板只是渲染 `/debug` 的只读数据，用系统浏览器
//! 开一个独立进程最省事，也不会把主界面拖下水。

use std::io::{BufRead, BufReader, Write};
use std::net::{Ipv4Addr, SocketAddr, TcpListener, TcpStream};
use std::time::Duration;

use serde_json::{Value, json};

use super::{DEBUG_HTTP_PORT, Probe, SharedHub, events, metrics};

const ENDPOINTS: [&str; 10] = [
  "/debug",
  "/debug/system",
  "/debug/audio",
  "/debug/devices",
  "/debug/logs",
  "/debug/metrics",
  "/debug/plan",
  "/debug/calib",
  "/debug/bundle",
  "/debug/ui",
];

/// 调试面板页面（浏览器版）的静态资源：**随二进制内嵌**，安装后不依赖磁盘上的 ui/ 目录。
///
/// 页面里用绝对路径引用（`/debug/ui/debug.js` …），所以不需要目录重定向：
/// `/debug/ui` 与 `/debug/ui/xxx` 都直接命中这张表。
fn static_asset(path: &str) -> Option<(&'static str, &'static [u8])> {
  const HTML: &[u8] = include_bytes!("../../ui/debug.html");
  const JS: &str = "text/javascript; charset=utf-8";
  const CSS: &str = "text/css; charset=utf-8";
  Some(match path {
    "/debug/ui" => ("text/html; charset=utf-8", HTML),
    "/debug/ui/debug.js" => (JS, include_bytes!("../../ui/debug.js")),
    "/debug/ui/debug.css" => (CSS, include_bytes!("../../ui/debug.css")),
    "/debug/ui/i18n.js" => (JS, include_bytes!("../../ui/i18n.js")),
    "/debug/ui/snapshot.js" => (JS, include_bytes!("../../ui/snapshot.js")),
    "/debug/ui/jsdebug.js" => (JS, include_bytes!("../../ui/jsdebug.js")),
    _ => return None,
  })
}

pub fn spawn(hub: SharedHub) {
  let addr = SocketAddr::from((Ipv4Addr::LOCALHOST, DEBUG_HTTP_PORT));
  let listener = match TcpListener::bind(addr) {
    Ok(l) => l,
    Err(e) => {
      let reason = format!("绑定 {addr} 失败：{e}");
      eprintln!("[PureVox][debug-http] {reason}");
      hub.set_http(Probe::unavailable(reason.clone()));
      events::log("error", "debug-http", reason);
      return;
    }
  };
  hub.set_http(Probe::ok(format!("http://{addr}/debug")));
  std::thread::Builder::new()
    .name("debug-http".into())
    .spawn(move || {
      let token = super::threads::ThreadToken::new("debug-http");
      for stream in listener.incoming().flatten() {
        token.beat();
        let hub = hub.clone();
        std::thread::spawn(move || {
          if let Err(e) = handle(&hub, stream) {
            // 客户端提前断开是常态，不写成 error；只记 debug 级
            events::log("debug", "debug-http", format!("请求处理结束：{e}"));
          }
        });
      }
    })
    .map_err(|e| {
      events::log("error", "debug-http", format!("调试接口线程起不来：{e}"));
    })
    .ok();
}

fn query_param<'a>(query: &'a str, key: &str) -> Option<&'a str> {
  query.split('&').find_map(|kv| {
    let (k, v) = kv.split_once('=')?;
    (k == key).then_some(v)
  })
}

fn route(hub: &SharedHub, path: &str, query: &str) -> (&'static str, Value) {
  let head = |v: Value, snap_ts: u64, uptime: u64| {
    let mut m = serde_json::Map::new();
    m.insert("ts".into(), json!(snap_ts));
    m.insert("uptime_ms".into(), json!(uptime));
    if let Value::Object(o) = v {
      m.extend(o);
      Value::Object(m)
    } else {
      m.insert("value".into(), v);
      Value::Object(m)
    }
  };
  match path {
    "/debug" => {
      let mut snap = hub.snapshot();
      if query_param(query, "full") == Some("0") {
        for s in &mut snap.audio.streams {
          s.waveform.clear();
          s.spectrum_db.clear();
        }
      }
      ("200 OK", serde_json::to_value(&snap).unwrap_or(Value::Null))
    }
    "/debug/system" => {
      let snap = hub.snapshot();
      (
        "200 OK",
        head(json!({ "system": snap.system }), snap.ts, snap.uptime_ms),
      )
    }
    "/debug/audio" => {
      let mut snap = hub.snapshot();
      if query_param(query, "full") == Some("0") {
        for s in &mut snap.audio.streams {
          s.waveform.clear();
          s.spectrum_db.clear();
        }
      }
      (
        "200 OK",
        head(json!({ "audio": snap.audio }), snap.ts, snap.uptime_ms),
      )
    }
    "/debug/devices" => {
      let snap = hub.snapshot();
      (
        "200 OK",
        head(json!({ "devices": snap.devices }), snap.ts, snap.uptime_ms),
      )
    }
    "/debug/logs" => {
      let limit = query_param(query, "limit")
        .and_then(|v| v.parse::<usize>().ok())
        .unwrap_or(500)
        .min(events::CAPACITY);
      let snap = hub.snapshot();
      (
        "200 OK",
        head(
          json!({ "events": events::recent(limit) }),
          snap.ts,
          snap.uptime_ms,
        ),
      )
    }
    "/debug/metrics" => {
      let snap = hub.snapshot();
      (
        "200 OK",
        head(
          json!({ "metrics": metrics::snapshot() }),
          snap.ts,
          snap.uptime_ms,
        ),
      )
    }
    "/debug/plan" => {
      let snap = hub.snapshot();
      (
        "200 OK",
        head(json!({ "plan": snap.plan }), snap.ts, snap.uptime_ms),
      )
    }
    "/debug/calib" => {
      let snap = hub.snapshot();
      (
        "200 OK",
        head(
          json!({ "calib": snap.calib_report }),
          snap.ts,
          snap.uptime_ms,
        ),
      )
    }
    "/debug/bundle" => {
      let snap = hub.snapshot();
      let bundle = json!({
        "bundle_version": 1,
        "ts": snap.ts,
        "uptime_ms": snap.uptime_ms,
        "snapshot": snap,
        "logs": events::recent(events::CAPACITY),
        "metrics": metrics::snapshot(),
      });
      ("200 OK", bundle)
    }

    _ => (
      "404 Not Found",
      json!({ "error": "未知端点", "endpoints": ENDPOINTS }),
    ),
  }
}

fn handle(hub: &SharedHub, mut stream: TcpStream) -> std::io::Result<()> {
  stream.set_read_timeout(Some(Duration::from_secs(2)))?;
  stream.set_write_timeout(Some(Duration::from_secs(5)))?;
  let mut reader = BufReader::new(stream.try_clone()?);
  let mut request_line = String::new();
  reader.read_line(&mut request_line)?;
  loop {
    let mut h = String::new();
    let n = reader.read_line(&mut h)?;
    if n == 0 || h == "\r\n" || h == "\n" {
      break;
    }
  }
  let mut parts = request_line.split_whitespace();
  let method = parts.next().unwrap_or("");
  let target = parts.next().unwrap_or("/");
  let (path, query) = target.split_once('?').unwrap_or((target, ""));
  let path = path.trim_end_matches('/');
  let path = if path.is_empty() { "/" } else { path };

  // 调试面板页面（浏览器版）：HTML / CSS / JS 都是内嵌静态资源，不走 JSON 通道。
  if method == "GET"
    && let Some((ctype, body)) = static_asset(path)
  {
    write!(
      stream,
      "HTTP/1.1 200 OK\r\nContent-Type: {ctype}\r\nContent-Length: {}\r\nCache-Control: no-store\r\nConnection: close\r\n\r\n",
      body.len()
    )?;
    stream.write_all(body)?;
    return stream.flush();
  }

  let (status, body) = if method == "GET" {
    route(hub, path, query)
  } else {
    (
      "405 Method Not Allowed",
      json!({ "error": "调试接口只读，仅支持 GET" }),
    )
  };
  let body = serde_json::to_string(&body).unwrap_or_else(|_| "{}".into());
  write!(
    stream,
    "HTTP/1.1 {status}\r\nContent-Type: application/json; charset=utf-8\r\nContent-Length: {}\r\nCache-Control: no-store\r\nConnection: close\r\n\r\n",
    body.len()
  )?;
  stream.write_all(body.as_bytes())?;
  stream.flush()
}
