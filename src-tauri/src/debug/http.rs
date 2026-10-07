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
//! 请求量极小（人工 curl / 脚本轮询），用 std::net 单线程顺序处理即可。

use std::io::{BufRead, BufReader, Write};
use std::net::{Ipv4Addr, SocketAddr, TcpListener, TcpStream};
use std::time::Duration;

use serde_json::{Value, json};

use super::{DEBUG_HTTP_PORT, Probe, SharedHub};

const ENDPOINTS: [&str; 4] = ["/debug", "/debug/system", "/debug/audio", "/debug/devices"];

pub fn spawn(hub: SharedHub) {
  let addr = SocketAddr::from((Ipv4Addr::LOCALHOST, DEBUG_HTTP_PORT));
  let listener = match TcpListener::bind(addr) {
    Ok(l) => l,
    Err(e) => {
      let reason = format!("绑定 {addr} 失败：{e}");
      eprintln!("[PureVox][debug-http] {reason}");
      hub.set_http(Probe::unavailable(reason));
      return;
    }
  };
  hub.set_http(Probe::ok(format!("http://{addr}/debug")));
  std::thread::Builder::new()
    .name("debug-http".into())
    .spawn(move || {
      for stream in listener.incoming().flatten() {
        if let Err(e) = handle(&hub, stream) {
          eprintln!("[PureVox][debug-http] 请求处理失败：{e}");
        }
      }
    })
    .expect("spawn debug-http");
}

fn route(hub: &SharedHub, path: &str) -> (&'static str, Value) {
  let snap = hub.snapshot();
  let head = |key: &str, v: Value| {
    let mut m = serde_json::Map::new();
    m.insert("ts".into(), json!(snap.ts));
    m.insert("uptime_ms".into(), json!(snap.uptime_ms));
    m.insert(key.into(), v);
    Value::Object(m)
  };
  match path {
    "/debug" => ("200 OK", serde_json::to_value(&snap).unwrap_or(Value::Null)),
    "/debug/system" => ("200 OK", head("system", json!(snap.system))),
    "/debug/audio" => ("200 OK", head("audio", json!(snap.audio))),
    "/debug/devices" => ("200 OK", head("devices", json!(snap.devices))),
    _ => (
      "404 Not Found",
      json!({ "ts": snap.ts, "uptime_ms": snap.uptime_ms, "error": "未知端点", "endpoints": ENDPOINTS }),
    ),
  }
}

fn handle(hub: &SharedHub, mut stream: TcpStream) -> std::io::Result<()> {
  stream.set_read_timeout(Some(Duration::from_secs(2)))?;
  let mut reader = BufReader::new(stream.try_clone()?);
  let mut request_line = String::new();
  reader.read_line(&mut request_line)?;
  // 丢弃请求头
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
  let path = target
    .split('?')
    .next()
    .unwrap_or("/")
    .trim_end_matches('/');
  let path = if path.is_empty() { "/" } else { path };

  let (status, body) = if method == "GET" {
    route(hub, path)
  } else {
    (
      "405 Method Not Allowed",
      json!({ "error": "调试接口只读，仅支持 GET" }),
    )
  };
  let body = serde_json::to_string_pretty(&body).unwrap_or_else(|_| "{}".into());
  write!(
    stream,
    "HTTP/1.1 {status}\r\nContent-Type: application/json; charset=utf-8\r\nContent-Length: {}\r\nCache-Control: no-store\r\nConnection: close\r\n\r\n",
    body.len()
  )?;
  stream.write_all(body.as_bytes())?;
  stream.flush()
}
