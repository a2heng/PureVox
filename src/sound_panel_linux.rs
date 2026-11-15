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

//! Linux：打开系统声音控制面板。优先 pavucontrol（Pulse/PipeWire），
//! 再回退 KDE / GNOME 设置（与旧实现 `open_sound_panel_posix` 同一顺序）。
//!
//! 只 `spawn` 不等待：面板会一直开到用户关掉，不能阻塞调用方。

pub fn open() -> Result<(), String> {
  let candidates: [(&str, &[&str]); 3] = [
    ("pavucontrol", &[]),
    ("systemsettings", &["kcm_pulseaudio"]),
    ("gnome-control-center", &["sound"]),
  ];
  for (cmd, args) in candidates {
    if std::process::Command::new(cmd).args(args).spawn().is_ok() {
      return Ok(());
    }
  }
  Err(
    "未找到声音控制面板（pavucontrol / systemsettings / gnome-control-center）；请先安装 pavucontrol"
      .into(),
  )
}
