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

//! 会话计划（DESIGN.md §5）：配置 → 可启动描述，纯数据 + 校验，不做 I/O。
//!
//! 一列 = 有序的行列表；首行必须是输入、末行必须是输出，中间可任意增删/拖动。

use std::collections::BTreeMap;

use serde::{Deserialize, Serialize};

#[derive(Serialize, Deserialize, Clone, Copy, Debug, PartialEq, Eq)]
#[serde(rename_all = "lowercase")]
pub enum RowKind {
  Input,
  Process,
  Output,
}

#[derive(Serialize, Deserialize, Clone, Debug)]
pub struct RowSpec {
  pub kind: RowKind,
  /// 节点类型（注册表 ptype）：input 用 audio_input，process 用 denoise，output 用 audio_output
  pub ptype: String,
  /// 输入/输出行绑定的设备 ID（cpal 稳定 ID）
  #[serde(default)]
  pub device: Option<String>,
  #[serde(default = "default_true")]
  pub enabled: bool,
  #[serde(default)]
  pub params: BTreeMap<String, String>,
}

fn default_true() -> bool {
  true
}

#[derive(Serialize, Deserialize, Clone, Debug)]
pub struct ColumnSpec {
  pub rows: Vec<RowSpec>,
}

#[derive(Serialize, Deserialize, Clone, Debug)]
pub struct Plan {
  pub columns: Vec<ColumnSpec>,
}

impl Default for Plan {
  fn default() -> Self {
    // 默认一列：一个空输入行 + 一个空输出行（由界面填设备）
    Plan {
      columns: vec![ColumnSpec {
        rows: vec![
          RowSpec { kind: RowKind::Input, ptype: "audio_input".into(), device: None, enabled: true, params: BTreeMap::new() },
          RowSpec { kind: RowKind::Output, ptype: "audio_output".into(), device: None, enabled: true, params: BTreeMap::new() },
        ],
      }],
    }
  }
}

/// 结构校验（非阻断项只提示）：每列非空、首行输入、末行输出。
pub fn validate(plan: &Plan) -> Vec<String> {
  let mut problems = Vec::new();
  if plan.columns.is_empty() {
    problems.push("没有任何列".into());
  }
  for (i, c) in plan.columns.iter().enumerate() {
    let label = format!("第 {} 列", i + 1);
    if c.rows.is_empty() {
      problems.push(format!("{label} 是空的"));
      continue;
    }
    if c.rows[0].kind != RowKind::Input {
      problems.push(format!("{label} 的第一行必须是输入"));
    }
    if c.rows[c.rows.len() - 1].kind != RowKind::Output {
      problems.push(format!("{label} 的最后一行必须是输出"));
    }
  }
  problems
}
