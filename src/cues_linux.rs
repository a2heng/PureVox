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

//! Linux 提示音播放：`paplay` 播自合成的 WAV 字节。
//!
//! 为什么是 `paplay`（而不是 `aplay` / `pw-play`）：它走**系统混音器**（PulseAudio 或
//! PipeWire 的 pulse 兼容层），与界面、其它软件同一条输出，不需要独占 ALSA；它也是
//! Debian 包 `pulseaudio-utils` 提供的东西（deb 的 `recommends` 里已经写了）。
//! `paplay` 只接受文件路径（`paplay -` 会去开一个名叫 `-` 的文件），所以落一个短命
//! 临时文件，播完即删。

use std::process::Command;
use std::sync::atomic::{AtomicU32, Ordering};

/// 临时文件序号：启动 / 停止提示音可能同时存在两份，不能共用同一个文件名。
static SEQ: AtomicU32 = AtomicU32::new(0);

pub(super) fn play_wav(wav: &[u8]) {
  let seq = SEQ.fetch_add(1, Ordering::Relaxed);
  let path = std::env::temp_dir().join(format!("purevox-cue-{}-{seq}.wav", std::process::id()));
  if let Err(e) = std::fs::write(&path, wav) {
    crate::debug::events::log("warn", "cue", format!("提示音临时文件写入失败：{e}"));
    return;
  }
  // 播完（paplay 自己退出）再删；播放在提示音线程里，最多几百毫秒，不挡主线程。
  let out = Command::new("paplay").arg(&path).status();
  let _ = std::fs::remove_file(&path);
  match out {
    Ok(s) if s.success() => {}
    Ok(s) => crate::debug::events::log(
      "warn",
      "cue",
      format!(
        "paplay 退出状态 {}（提示音没播出来）",
        s.code().unwrap_or(-1)
      ),
    ),
    Err(e) => crate::debug::events::log(
      "warn",
      "cue",
      format!("找不到 paplay（需 pulseaudio-utils / pipewire-pulse）：{e}"),
    ),
  }
}
