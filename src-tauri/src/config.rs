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

//! 配置持久化：会话计划存 `~/.purevox/session.json`（DESIGN.md §5、§6.2 的目录约定）。

use std::path::PathBuf;

use crate::plan::Plan;

fn home_dir() -> Result<PathBuf, String> {
  std::env::var_os("USERPROFILE")
    .or_else(|| std::env::var_os("HOME"))
    .map(PathBuf::from)
    .ok_or_else(|| "找不到用户目录（USERPROFILE/HOME）".to_string())
}

pub fn config_dir() -> Result<PathBuf, String> {
  Ok(home_dir()?.join(".purevox"))
}

pub fn session_path() -> Result<PathBuf, String> {
  Ok(config_dir()?.join("session.json"))
}

/// 展开路径里的 `~`（用户目录）。
pub fn expand_path(p: &str) -> PathBuf {
  let p = p.trim();
  if let Some(rest) = p.strip_prefix("~/").or_else(|| p.strip_prefix("~\\")) {
    if let Ok(home) = home_dir() {
      return home.join(rest);
    }
  }
  PathBuf::from(p)
}

/// TSE 参考录音默认路径（`~/.purevox/tse_reference.wav`）。
pub fn default_tse_reference() -> PathBuf {
  config_dir()
    .map(|d| d.join("tse_reference.wav"))
    .unwrap_or_else(|_| PathBuf::from("tse_reference.wav"))
}

/// 读取已保存的计划；无文件或解析失败则返回默认计划。
pub fn load_plan() -> Plan {
  let Ok(path) = session_path() else { return Plan::default() };
  let Ok(text) = std::fs::read_to_string(&path) else { return Plan::default() };
  serde_json::from_str(&text).unwrap_or_else(|_| Plan::default())
}

pub fn save_plan(plan: &Plan) -> Result<(), String> {
  let dir = config_dir()?;
  std::fs::create_dir_all(&dir).map_err(|e| format!("创建配置目录失败：{e}"))?;
  let text = serde_json::to_string_pretty(plan).map_err(|e| format!("序列化失败：{e}"))?;
  std::fs::write(session_path()?, text).map_err(|e| format!("写配置失败：{e}"))
}
