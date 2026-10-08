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

//! Linux：校准探针经 Pulse/PipeWire 直送指定 **sink**（`pacat` 读 `float32le` stdin）。
//!
//! AEC 的远端回环监听的是某只 sink 的 monitor；探针必须播到**同一只 sink**，far 才收得到。
//! cpal 在 Linux 只有 ALSA，打不开 Pulse 的 sink 名，所以这里走 `pacat`（`@DEFAULT_SINK@` = 默认）。

use std::io::Write;
use std::process::{Command, Stdio};

/// Linux 可以把探针直送指定 sink。
pub const NATIVE: bool = true;

/// 把单声道 48 kHz f32 探针播到 `target`（None/空 = 默认输出 sink）。
pub fn play(pcm: &[f32], target: Option<&str>) -> Result<(), String> {
  let dev = target.filter(|s| !s.is_empty()).unwrap_or("@DEFAULT_SINK@");
  let mut child = Command::new("pacat")
    .env("LC_ALL", "C")
    .args([
      "--playback",
      &format!("--device={dev}"),
      "--format=float32le",
      "--rate=48000",
      "--channels=1",
      // 与 far 采集同样压低缓冲：探针要立刻出现在 far 的采集里
      "--latency-msec=100",
      "--process-time-msec=20",
    ])
    .stdin(Stdio::piped())
    .stdout(Stdio::null())
    .stderr(Stdio::null())
    .spawn()
    .map_err(|e| format!("启动 pacat 失败（需要 PulseAudio/PipeWire）：{e}"))?;
  {
    let mut stdin = child.stdin.take().ok_or("pacat 未提供 stdin")?;
    let mut bytes = Vec::with_capacity(pcm.len() * 4);
    for x in pcm {
      bytes.extend_from_slice(&x.to_le_bytes());
    }
    stdin
      .write_all(&bytes)
      .map_err(|e| format!("写 pacat 失败：{e}"))?;
  } // drop(stdin) → pacat 播完缓冲即退出
  let st = child.wait().map_err(|e| format!("等待 pacat 失败：{e}"))?;
  if st.success() {
    Ok(())
  } else {
    Err(format!("pacat 退出码 {}", st.code().unwrap_or(-1)))
  }
}
