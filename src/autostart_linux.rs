// PureVox — AI 麦克风降噪工具
// Copyright (C) 2024-2026 a2heng <752848283@qq.com>
//
// PureVox is licensed under the GNU General Public License v3.0 or
// later (GPL-3.0-or-later).  See LICENSE for details.
// SPDX-License-Identifier: GPL-3.0-or-later

//! 开机自启（Linux）：XDG autostart 的 `.desktop` 文件（桌面环境通用）。

use std::path::PathBuf;

fn entry_path() -> Result<PathBuf, String> {
  let home =
    std::env::var_os("HOME").ok_or_else(|| "找不到 HOME，无法定位 autostart 目录".to_string())?;
  Ok(PathBuf::from(home).join(".config/autostart/purevox.desktop"))
}

pub fn set(on: bool) -> Result<(), String> {
  let path = entry_path()?;
  if !on {
    match std::fs::remove_file(&path) {
      Ok(()) => return Ok(()),
      Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(()),
      Err(e) => return Err(format!("删除 {} 失败：{e}", path.display())),
    }
  }
  let exe = std::env::current_exe().map_err(|e| format!("找不到当前程序路径：{e}"))?;
  if let Some(dir) = path.parent() {
    std::fs::create_dir_all(dir).map_err(|e| format!("创建 {} 失败：{e}", dir.display()))?;
  }
  let body = format!(
    "[Desktop Entry]\nType=Application\nName=PureVox\nExec=\"{}\"\nX-GNOME-Autostart-enabled=true\nTerminal=false\n",
    exe.display()
  );
  std::fs::write(&path, body).map_err(|e| format!("写入 {} 失败：{e}", path.display()))
}

pub fn is_enabled() -> bool {
  entry_path().is_ok_and(|p| p.is_file())
}
