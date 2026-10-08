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

//! Linux 输出回环采集（AEC 远端参考：扬声器 → far）。
//!
//! cpal 在 Linux 上只有 ALSA，没有「监听某个 sink」的概念；这里用 PulseAudio/PipeWire
//! 兼容层的 `parec` 监听输出的 **monitor**，得到该输出正在播放的音频（`float32le` 48 kHz
//! 单声道），再喂进与 cpal 采集完全相同的 [`capture::worker`]（重采样/计量/扇出一致）。
//! 时间原点由 far 网格按实时推进兜住（引擎侧，见 `engine/aec.rs`）。
//!
//! `output_id`：
//!   - `None` / 空串 → 系统默认输出的 monitor（`parec -d @DEFAULT_MONITOR@`）；
//!   - `Some(sink)` → 指定 PipeWire/Pulse sink 的 monitor（`<sink>.monitor`；已是
//!     `.monitor` 则原样）。
//!
//! 与 Windows `loopback.rs` 同一契约（`spawn` 返回 [`Capture`]），差异只在数据源。

use std::io::Read;
use std::process::{Command, Stdio};
use std::sync::atomic::{AtomicBool, Ordering::Relaxed};
use std::sync::{Arc, mpsc};

use rtrb::RingBuffer;

use super::capture::{self, CallbackStats, Capture, OPEN_TIMEOUT, SourceInfo};
use super::fanout::Fanout;
use crate::debug::SharedHub;

/// 采集采样率/格式（parec 直接给 48 kHz 单声道 f32，与引擎内部格式一致）。
const RATE: u32 = 48_000;

/// 打开输出回环并启动采集线程。
pub fn spawn(hub: SharedHub, output_id: Option<String>, tag: String) -> Result<Capture, String> {
  let stop = Arc::new(AtomicBool::new(false));
  let (tx, rx) = mpsc::channel::<Result<Arc<Fanout>, String>>();
  let stop2 = stop.clone();
  let name = tag.clone();
  let join = std::thread::Builder::new()
    .name(format!("loopback-{name}"))
    .spawn(move || run(hub, output_id, tag, stop2, tx))
    .map_err(|e| format!("创建回环采集线程失败：{e}"))?;
  match rx.recv_timeout(OPEN_TIMEOUT) {
    Ok(Ok(fanout)) => Ok(Capture {
      handle: super::WorkerHandle::new(stop, join),
      fanout,
    }),
    Ok(Err(e)) => {
      let _ = join.join();
      Err(e)
    }
    Err(_) => {
      stop.store(true, Relaxed);
      Err("打开回环超时（5 s）".into())
    }
  }
}

fn run(
  hub: SharedHub,
  output_id: Option<String>,
  tag: String,
  stop: Arc<AtomicBool>,
  tx: mpsc::Sender<Result<Arc<Fanout>, String>>,
) {
  let stats = Arc::new(CallbackStats::new());
  let (prod, cons) = RingBuffer::<f32>::new(RATE as usize); // 1 s 单声道
  let (fmt_tx, fmt_rx) = mpsc::channel::<Result<String, String>>();

  let stop2 = stop.clone();
  let stats2 = stats.clone();
  let pump = std::thread::Builder::new()
    .name("loopback-pump".into())
    .spawn(move || pump(output_id, prod, fmt_tx, stop2, stats2))
    .expect("spawn loopback pump");

  let name = match fmt_rx.recv_timeout(OPEN_TIMEOUT) {
    Ok(Ok(v)) => v,
    Ok(Err(e)) => {
      let _ = tx.send(Err(e));
      return;
    }
    Err(_) => {
      stop.store(true, Relaxed);
      let _ = tx.send(Err("打开回环超时（5 s）".into()));
      return;
    }
  };
  let info = SourceInfo {
    name,
    device_id: "loopback".into(),
    native_rate: RATE,
    channels: 1,
    sample_format: "f32".into(),
  };
  capture::worker(
    hub,
    tag,
    info,
    capture::WorkerSrc {
      cons,
      stats,
      keep: pump,
      stop,
    },
    tx,
  );
}

/// 启动 `parec` 监听 monitor，读 stdout（float32le）写进无锁环。目标经 `fmt_tx` 回传。
fn pump(
  output_id: Option<String>,
  mut prod: rtrb::Producer<f32>,
  fmt_tx: mpsc::Sender<Result<String, String>>,
  stop: Arc<AtomicBool>,
  stats: Arc<CallbackStats>,
) {
  let monitor = match resolve_monitor(output_id.as_deref()) {
    Ok(m) => m,
    Err(e) => {
      let _ = fmt_tx.send(Err(e));
      return;
    }
  };
  let name = if output_id.as_deref().filter(|s| !s.is_empty()).is_some() {
    format!("回环：{monitor}")
  } else {
    "回环：系统默认输出".to_string()
  };
  let child = Command::new("parec")
    .args([
      &format!("--device={monitor}"),
      "--format=float32le",
      &format!("--rate={RATE}"),
      "--channels=1",
    ])
    .stdin(Stdio::null())
    .stdout(Stdio::piped())
    .stderr(Stdio::null())
    .spawn();
  let mut child = match child {
    Ok(c) => c,
    Err(e) => {
      let _ = fmt_tx.send(Err(format!(
        "启动 parec 失败（需要 PipeWire/PulseAudio）：{e}"
      )));
      return;
    }
  };
  let mut out = match child.stdout.take() {
    Some(o) => o,
    None => {
      let _ = fmt_tx.send(Err("parec 未提供 stdout".into()));
      return;
    }
  };
  let _ = fmt_tx.send(Ok(name));

  // 4800 样本/块（100 ms）缓冲读；按小端 f32 逐样本推入环。
  let mut buf = [0u8; 4800 * 4];
  while !stop.load(Relaxed) {
    match out.read(&mut buf) {
      Ok(0) => break,
      Ok(n) => {
        stats.on_block((n / 4) as u32);
        let mut dropped = 0u64;
        for c in buf[..n].as_chunks::<4>().0 {
          let s = f32::from_le_bytes(*c);
          if prod.push(s).is_err() {
            dropped += 1;
          }
        }
        if dropped > 0 {
          stats.xruns.fetch_add(dropped, Relaxed);
        }
      }
      Err(_) => break,
    }
  }
  let _ = child.kill();
  let _ = child.wait();
}

/// `None`/空 → 默认 monitor；`Some(sink)` → `<sink>.monitor`（已是 monitor 则原样）。
fn resolve_monitor(output_id: Option<&str>) -> Result<String, String> {
  match output_id.filter(|s| !s.is_empty()) {
    None => Ok("@DEFAULT_MONITOR@".to_string()),
    Some(s) => {
      let s = s.strip_prefix("pulse:").unwrap_or(s);
      Ok(if s.ends_with(".monitor") {
        s.to_string()
      } else {
        format!("{s}.monitor")
      })
    }
  }
}
