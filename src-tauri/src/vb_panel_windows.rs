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

//! Windows：以管理员权限打开 VB-CABLE 控制面板（与旧实现 `open_virtual_cable_panel_win` 一致）。
//!
//! 控制面板必须提权（会写驱动设置）。用 `Start-Process -Verb RunAs` 触发 UAC，
//! 用户取消时 PowerShell 会返回错误，这里把路径与失败原因一起带回去，不静默。

pub fn open() -> Result<(), String> {
  let exe = r"C:\Program Files\VB\CABLE\VBCABLE_ControlPanel.exe";
  if !std::path::Path::new(exe).exists() {
    return Err("未找到 VB-CABLE 控制面板——请先安装 VB-CABLE 驱动".into());
  }
  // 单引号路径里的单引号要转义；这两条官方路径没有单引号，仍按规则处理
  let quoted = exe.replace('\'', "''");
  std::process::Command::new("powershell.exe")
    .args([
      "-NoProfile",
      "-WindowStyle",
      "Hidden",
      "-Command",
      &format!("Start-Process -Verb RunAs -FilePath '{quoted}'"),
    ])
    .spawn()
    .map(|_| ())
    .map_err(|e| format!("打开 VB-CABLE 控制面板失败（UAC 取消或权限不足）：{e}"))
}
