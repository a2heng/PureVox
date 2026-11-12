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

//! 会话计划（ARCHITECTURE.md §5）：配置 → 可启动描述，纯数据 + 校验，不做 I/O。
//!
//! **Session V2：节点 + 连线。** 一列仍然是有序的行，但行与行之间不再是「隐式上一条」，
//! 每条连线都记在目标的 `inputs` 里。节点是节点，线是线：断开一条线只删这条线，
//! 节点、端口、设备选择都保留。

use std::collections::{BTreeMap, HashMap, HashSet};
use std::sync::Arc;

use serde::{Deserialize, Serialize};

/// 计划版本：0 = 旧的「列内顺序链」，2 = 节点 + 连线。
pub const VERSION: u32 = 3;

#[derive(Serialize, Deserialize, Clone, Copy, Debug, PartialEq, Eq)]
#[serde(rename_all = "lowercase")]
pub enum RowKind {
  Input,
  Process,
  Output,
}

/// 一条入线：`from` 节点的第 `from_port` 路输出接到本节点的第 `to_port` 路输入。
/// **端口 = 通道**（AEC 的 0/1 是近端/远端，不是 L/R）。
#[derive(Serialize, Deserialize, Clone, Debug, PartialEq, Eq)]
pub struct PortInput {
  pub from: u32,
  #[serde(default)]
  pub from_port: u8,
  #[serde(default)]
  pub to_port: u8,
}

#[derive(Serialize, Deserialize, Clone, Debug)]
pub struct RowSpec {
  pub kind: RowKind,
  /// 节点类型（注册表 ptype）：input 用 audio_input，process 用 denoise，output 用 audio_output
  pub ptype: String,
  /// 输入/输出行绑定的设备 ID（cpal 稳定 ID）
  #[serde(default)]
  pub device: Option<String>,
  /// 选设备时的**显示名**（只给界面/诊断看，**不参与设备匹配**）。
  #[serde(default, skip_serializing_if = "Option::is_none")]
  pub device_name: Option<String>,
  #[serde(default = "default_true")]
  pub enabled: bool,
  #[serde(default)]
  pub params: BTreeMap<String, String>,
  /// 稳定节点 ID（连线引用它；删除/重排不改变 ID）。
  #[serde(default)]
  pub id: u32,
  /// 画板位置（逻辑像素，左上角）。
  #[serde(default)]
  pub x: f32,
  #[serde(default)]
  pub y: f32,
  /// 音频通道数；端口 = 通道（AEC 固定 1，两个语义端口）。
  #[serde(default = "default_channels")]
  pub channels: u32,
  /// 节点输入微调（dB，热参数）。所有节点都有（Element 的 node input gain）。
  #[serde(default = "default_gain")]
  pub input_gain_db: String,
  /// 入线。空 = 这个节点还没接输入（不是错误，节点仍然存在）。
  #[serde(default)]
  pub inputs: Vec<PortInput>,
}

fn default_channels() -> u32 {
  1
}

fn default_gain() -> String {
  "0.0".into()
}

impl Default for RowSpec {
  fn default() -> Self {
    RowSpec {
      kind: RowKind::Process,
      ptype: String::new(),
      device: None,
      device_name: None,
      enabled: true,
      params: BTreeMap::new(),
      channels: 1,
      input_gain_db: "0.0".into(),
      id: 0,
      x: 0.0,
      y: 0.0,
      inputs: Vec::new(),
    }
  }
}

#[derive(Serialize, Deserialize, Clone, Debug)]
pub struct ColumnSpec {
  pub rows: Vec<RowSpec>,
  #[serde(default = "default_true")]
  pub enabled: bool,
}

#[derive(Serialize, Deserialize, Clone, Debug)]
pub struct Plan {
  /// 0（旧文件）= 列内顺序链；[`VERSION`] = 节点 + 连线。
  #[serde(default)]
  pub v: u32,
  pub columns: Vec<ColumnSpec>,
}

fn default_true() -> bool {
  true
}

impl Default for Plan {
  fn default() -> Self {
    // 默认：一个输入节点 + 一个输出节点，线接好（界面会填设备）。
    let input = RowSpec {
      kind: RowKind::Input,
      ptype: "audio_input".into(),
      ..Default::default()
    };
    let output = RowSpec {
      kind: RowKind::Output,
      ptype: "audio_output".into(),
      ..Default::default()
    };
    let mut plan = Plan {
      v: VERSION,
      columns: vec![ColumnSpec {
        enabled: true,
        rows: vec![input, output],
      }],
    };
    plan.normalize();
    plan
  }
}

// ── 端口 ──────────────────────────────────────────────────────────────

/// 输入端口数。端口 = 通道；AEC 例外（近端/远端两个语义端口，固定单通道）。
pub fn input_ports(ptype: &str, channels: u32) -> usize {
  match ptype {
    "echo_cancel" => 2,
    "audio_input" | "remote_mic" | "loopback" | "tone" | "app_capture" => 0,
    // 媒体源（音效板 / 播放器）：声音由 Stage 自己生成，不需要入线。
    // 引擎仍按 Process 行执行（带着 media_slot 跑），但画板上它们是源节点。
    "soundpad" | "player" => 0,
    "audio_output" | "remote_speaker" => channels.max(1) as usize,
    _ => channels.max(1) as usize,
  }
}

pub fn output_ports(ptype: &str, channels: u32) -> usize {
  match ptype {
    "audio_output" | "remote_speaker" => 0,
    "echo_cancel" => 1,
    _ => channels.max(1) as usize,
  }
}

/// 该节点允许的通道数上限（UI / 校验共用）。
pub fn max_channels(ptype: &str) -> u32 {
  match ptype {
    "echo_cancel" | "tse" | "remote_mic" | "tone" | "remote_speaker" | "soundpad" | "player"
    | "viz" | "app_capture" => 1,
    "denoise" => 2,
    "audio_input" | "loopback" | "audio_output" => 16,
    _ => 8,
  }
}

fn is_source(ptype: &str) -> bool {
  input_ports(ptype, 1) == 0
}

// ── 节点查找 / 编辑 ───────────────────────────────────────────────────

impl Plan {
  pub fn nodes(&self) -> impl Iterator<Item = &RowSpec> {
    self.columns.iter().flat_map(|c| c.rows.iter())
  }

  pub fn node(&self, id: u32) -> Option<&RowSpec> {
    self.nodes().find(|n| n.id == id)
  }

  pub fn node_mut(&mut self, id: u32) -> Option<&mut RowSpec> {
    self
      .columns
      .iter_mut()
      .flat_map(|c| c.rows.iter_mut())
      .find(|n| n.id == id)
  }

  pub fn position(&self, id: u32) -> Option<(usize, usize)> {
    for (ci, c) in self.columns.iter().enumerate() {
      if let Some(ri) = c.rows.iter().position(|r| r.id == id) {
        return Some((ci, ri));
      }
    }
    None
  }

  pub fn next_id(&self) -> u32 {
    self.nodes().map(|n| n.id).max().unwrap_or(0) + 1
  }

  /// 迁移 / 修复：补 ID、补位置、旧文件补顺序链、去掉历史自动插入的 0 dB 增益，
  /// 再按连通分量把节点归到执行列（列对用户不可见，只决定哪一组节点在同一个线程里跑）。
  pub fn normalize(&mut self) -> bool {
    let mut changed = false;
    // v1 → v2：列内顺序链 → 节点 + 连线（含 AEC 拆分、占位增益旁路）。
    if self.v < 2 {
      let mut next = self.nodes().map(|n| n.id).max().unwrap_or(0) + 1;
      let mut prev: Option<u32> = None;
      for (ci, c) in self.columns.iter_mut().enumerate() {
        for (ri, r) in c.rows.iter_mut().enumerate() {
          if r.id == 0 {
            r.id = next;
            next += 1;
          }
          if r.x == 0.0 && r.y == 0.0 {
            // 旧链是纵向的；Session V2 默认横向铺开，读起来像 Element。
            r.x = 48.0 + ri as f32 * 300.0;
            r.y = 64.0 + ci as f32 * 120.0;
          }
          if r.inputs.is_empty()
            && r.kind != RowKind::Input
            && !is_source(&r.ptype)
            && let Some(p) = prev
          {
            r.inputs.push(PortInput {
              from: p,
              from_port: 0,
              to_port: 0,
            });
          }
          prev = Some(r.id);
        }
      }
      // 旧 AEC 是「一个输入行 = mic + far_device」；V2 拆成两个源 + 一个两输入 AEC 节点。
      let mut next = self.nodes().map(|n| n.id).max().unwrap_or(0) + 1;
      for c in &mut self.columns {
        let mut rows: Vec<RowSpec> = Vec::new();
        for mut r in std::mem::take(&mut c.rows) {
          if r.ptype == "echo_cancel" && r.kind == RowKind::Input {
            let old_id = r.id;
            let far = r.params.get("far_device").cloned().unwrap_or_default();
            let near = RowSpec {
              id: next,
              kind: RowKind::Input,
              ptype: "audio_input".into(),
              device: r.device.clone(),
              device_name: r.device_name.clone(),
              ..Default::default()
            };
            next += 1;
            let near_id = near.id;
            rows.push(near);
            let mut ins = vec![PortInput {
              from: near_id,
              from_port: 0,
              to_port: 0,
            }];
            if !far.is_empty() {
              let loopback = far.starts_with("loopback");
              let far_node = RowSpec {
                id: next,
                kind: RowKind::Input,
                ptype: if loopback {
                  "loopback".into()
                } else {
                  "audio_input".into()
                },
                device: Some(if loopback { "loopback".into() } else { far }),
                device_name: Some(if loopback {
                  "系统声音（默认输出）".into()
                } else {
                  String::new()
                }),
                ..Default::default()
              };
              next += 1;
              ins.push(PortInput {
                from: far_node.id,
                from_port: 0,
                to_port: 1,
              });
              rows.push(far_node);
            }
            r.id = old_id;
            r.kind = RowKind::Process;
            r.ptype = "echo_cancel".into();
            r.device = None;
            r.device_name = None;
            r.inputs = ins;
            r.params.remove("far_device");
            rows.push(r);
          } else {
            rows.push(r);
          }
        }
        c.rows = rows;
      }

      // 旧版每个新列都会自动塞一个 0 dB 增益；Session V2 不再要它。
      // 删之前把它的下游接到它的上游，否则整条链会断。
      for c in &mut self.columns {
        let mut bypass: Vec<(u32, Option<u32>)> = Vec::new();
        for r in &c.rows {
          if r.kind == RowKind::Process
            && r.ptype == "gain"
            && r.params.len() == 1
            && r.params.get("gain_db").map(|v| v == "0.0").unwrap_or(false)
          {
            bypass.push((r.id, r.inputs.first().map(|e| e.from)));
          }
        }
        if bypass.is_empty() {
          continue;
        }
        for r in &mut c.rows {
          for e in &mut r.inputs {
            if let Some((_, Some(src))) = bypass.iter().find(|(id, _)| *id == e.from) {
              e.from = *src;
            }
          }
        }
        c.rows.retain(|r| !bypass.iter().any(|(id, _)| *id == r.id));
      }
      // 删掉占位增益 / 拆完 AEC 后按深度重新铺开，避免把节点推到视口外。
      self.layout_by_depth();
      self.v = 2;
      changed = true;
    }
    // v2 → v3：端子改成上下排布，信号改为自上而下；旧坐标按深度重排一次。
    if self.v == 2 {
      self.layout_by_depth();
      self.v = 3;
      changed = true;
    }

    // 补 ID / 补位置（按当前列顺序给默认值）。同一份计划里 ID 必须唯一：
    // 读到手改 / 旧版合并坏掉的文件时，重复 ID 会重新编号。
    let mut seen = HashSet::new();
    let mut next = self.nodes().map(|n| n.id).max().unwrap_or(0) + 1;
    for (ci, c) in self.columns.iter_mut().enumerate() {
      for (ri, r) in c.rows.iter_mut().enumerate() {
        if r.id == 0 || !seen.insert(r.id) {
          r.id = next;
          next += 1;
          changed = true;
        }
        // 旧的 loopback「音量」/ 输出「发送增益」统一并进节点输入微调：
        // 图里只留一个增益控制，引擎也只读 `input_gain_db`。
        if let Some(v) = r.params.remove("volume_db") {
          let a = r.input_gain_db.trim().parse::<f64>().unwrap_or(0.0);
          let b = v.trim().parse::<f64>().unwrap_or(0.0);
          r.input_gain_db = format!("{:.1}", a + b);
          changed = true;
        }
        // 网络麦克风的 AEC 已解耦为独立节点（2026-11-12）。旧计划里残留的
        // `aec` / 远端参数没有第二条入口，直接清掉；要消回声就把 AEC 节点接在这条支路上。
        if r.ptype == "remote_mic" {
          for k in [
            "aec",
            "far_device",
            "mic_gain_db",
            "far_gain_db",
            "bypass",
            "far_delay_ms",
          ] {
            if r.params.remove(k).is_some() {
              changed = true;
            }
          }
        }
        if r.x == 0.0 && r.y == 0.0 {
          // 端子在上/下 => 同一条链自上而下；同一深度的分支横向排开。
          r.x = 64.0 + ci as f32 * 272.0;
          r.y = 48.0 + ri as f32 * 88.0;
          changed = true;
        }
      }
    }

    // 丢掉指向不存在节点 / 没有输出 / 端口越界的入线。
    let existing: HashSet<u32> = self.nodes().map(|n| n.id).collect();
    let outs: HashMap<u32, usize> = self
      .nodes()
      .map(|n| (n.id, output_ports(&n.ptype, n.channels)))
      .collect();
    for c in &mut self.columns {
      for r in &mut c.rows {
        let before = r.inputs.len();
        let own = input_ports(&r.ptype, r.channels);
        r.inputs.retain(|e| {
          let src_outs = outs.get(&e.from).copied().unwrap_or(0);
          (e.to_port as usize) < own
            && (e.from_port as usize) < src_outs
            && existing.contains(&e.from)
            && src_outs > 0
        });
        let mut keys = HashSet::new();
        r.inputs
          .retain(|e| keys.insert((e.from, e.from_port, e.to_port)));
        if r.inputs.len() != before {
          changed = true;
        }
      }
    }

    changed |= self.regroup();
    changed
  }

  /// 按「到源节点的最长路径」分列排版；只改 x/y，不改连线。
  pub fn layout_by_depth(&mut self) {
    let ids: Vec<u32> = self.nodes().map(|n| n.id).collect();
    let mut depth: HashMap<u32, usize> = HashMap::new();
    for _ in 0..ids.len() {
      let mut changed = false;
      for id in &ids {
        let Some(n) = self.node(*id) else { continue };
        let d = n
          .inputs
          .iter()
          .filter_map(|e| depth.get(&e.from).copied())
          .max()
          .map(|x| x + 1)
          .unwrap_or(0);
        if depth.get(id).copied() != Some(d) {
          depth.insert(*id, d);
          changed = true;
        }
      }
      if !changed {
        break;
      }
    }
    let mut per: HashMap<usize, usize> = HashMap::new();
    for id in ids {
      let d = depth.get(&id).copied().unwrap_or(0);
      let row = per.entry(d).or_insert(0);
      if let Some(n) = self.node_mut(id) {
        // 端子在上/下 => 信号自上而下流：深度走 y，分支走 x。
        // 卡片 240 宽 / 36 高：留 32 px 横向、52 px 纵向走线，整理后不会「飞出去」。
        n.x = 48.0 + *row as f32 * 272.0;
        n.y = 64.0 + d as f32 * 88.0;
      }
      *row += 1;
    }
  }

  /// 按弱连通分量重新归列，并在每列里拓扑排序。已合法时不动（返回 false）。
  pub fn regroup(&mut self) -> bool {
    let mut all: Vec<RowSpec> = Vec::new();
    for c in &mut self.columns {
      all.append(&mut c.rows);
    }
    if all.is_empty() {
      return !self.columns.is_empty();
    }
    let ids: HashSet<u32> = all.iter().map(|n| n.id).collect();
    let index: HashMap<u32, usize> = all.iter().enumerate().map(|(i, n)| (n.id, i)).collect();
    // 并查集
    let mut parent: Vec<usize> = (0..all.len()).collect();
    fn find(p: &mut [usize], mut x: usize) -> usize {
      while p[x] != x {
        p[x] = p[p[x]];
        x = p[x];
      }
      x
    }
    for (i, n) in all.iter().enumerate() {
      for e in &n.inputs {
        if let Some(&j) = index.get(&e.from) {
          let (a, b) = (find(&mut parent, i), find(&mut parent, j));
          if a != b {
            parent[a] = b;
          }
        }
      }
    }
    // 保留首次出现的顺序；孤立节点也各成一列
    let mut order: Vec<usize> = Vec::new();
    let mut root_of: HashMap<usize, usize> = HashMap::new();
    for i in 0..all.len() {
      let r = find(&mut parent, i);
      if root_of.insert(r, i).is_none() {
        order.push(r);
      }
    }
    let mut groups: Vec<Vec<RowSpec>> = Vec::new();
    for r in &order {
      let mut g = Vec::new();
      for (i, n) in all.iter().enumerate() {
        if find(&mut parent, i) == *r {
          g.push(n.clone());
        }
      }
      if !topo_sort(&mut g) {
        g.sort_by_key(|n| n.id);
      }
      groups.push(g);
    }

    // 和现有列完全一致就不动。
    let same = groups.len() == self.columns.len()
      && groups.iter().zip(self.columns.iter()).all(|(g, c)| {
        g.len() == c.rows.len() && g.iter().zip(c.rows.iter()).all(|(a, b)| a.id == b.id)
      });
    if same {
      return false;
    }
    self.columns = groups
      .into_iter()
      .map(|rows| ColumnSpec {
        rows,
        enabled: true,
      })
      .collect();
    let _ = ids;
    true
  }

  /// 把一条线接到 `to` 节点的 `to_port`。跨列会自动并进同一个执行分组；
  /// 会形成环则拒绝。
  pub fn connect(&mut self, from: u32, from_port: u8, to: u32, to_port: u8) -> Result<(), String> {
    if from == to {
      return Err("不能把节点接回自己".into());
    }
    let Some(target) = self.node(to) else {
      return Err("目标节点不存在".into());
    };
    let ports = input_ports(&target.ptype, target.channels);
    if ports == 0 {
      return Err("这个节点没有输入端口".into());
    }
    if (to_port as usize) >= ports {
      return Err(format!("这个节点只有 {ports} 个输入端口"));
    }
    let Some(src) = self.node(from) else {
      return Err("来源节点不存在".into());
    };
    let outs = output_ports(&src.ptype, src.channels);
    if outs == 0 {
      return Err("这个节点没有输出端口".into());
    }
    if (from_port as usize) >= outs {
      return Err(format!("这个节点只有 {outs} 个输出端口"));
    }
    if self.reaches(from, to) {
      return Err("这条线会形成环".into());
    }
    let Some(r) = self.node_mut(to) else {
      return Err("目标节点不存在".into());
    };
    r.inputs.push(PortInput {
      from,
      from_port,
      to_port,
    });
    r.inputs.sort_by_key(|e| (e.to_port, e.from, e.from_port));
    self.regroup();
    Ok(())
  }

  /// 断开一条线；**节点保留**。
  pub fn disconnect(&mut self, from: u32, from_port: u8, to: u32, to_port: u8) {
    if let Some(r) = self.node_mut(to) {
      r.inputs
        .retain(|e| !(e.from == from && e.from_port == from_port && e.to_port == to_port));
    }
  }

  /// 删除节点（连同指向它的线）。这是唯一会删点的操作。
  pub fn remove_node(&mut self, id: u32) {
    for c in &mut self.columns {
      c.rows.retain(|r| r.id != id);
    }
    for n in self.nodes_mut_vec() {
      n.inputs.retain(|e| e.from != id);
    }
    self.columns.retain(|c| !c.rows.is_empty());
  }

  fn nodes_mut_vec(&mut self) -> Vec<&mut RowSpec> {
    self
      .columns
      .iter_mut()
      .flat_map(|c| c.rows.iter_mut())
      .collect()
  }

  fn reaches(&self, start: u32, goal: u32) -> bool {
    let mut stack = vec![start];
    let mut seen = HashSet::new();
    while let Some(id) = stack.pop() {
      if id == goal {
        return true;
      }
      if !seen.insert(id) {
        continue;
      }
      if let Some(n) = self.node(id) {
        for e in &n.inputs {
          stack.push(e.from);
        }
      }
    }
    false
  }
}

/// 列内按入线排序（Kahn）。返回 false = 有环。
pub fn topo_sort(rows: &mut Vec<RowSpec>) -> bool {
  let ids: HashMap<u32, usize> = rows.iter().enumerate().map(|(i, r)| (r.id, i)).collect();
  let mut indeg = vec![0usize; rows.len()];
  let mut outs: Vec<Vec<usize>> = vec![Vec::new(); rows.len()];
  for (i, r) in rows.iter().enumerate() {
    for e in &r.inputs {
      if let Some(&j) = ids.get(&e.from) {
        indeg[i] += 1;
        outs[j].push(i);
      }
    }
  }
  let mut queue: std::collections::VecDeque<usize> =
    (0..rows.len()).filter(|i| indeg[*i] == 0).collect();
  let mut order = Vec::with_capacity(rows.len());
  while let Some(i) = queue.pop_front() {
    order.push(i);
    for &o in &outs[i] {
      indeg[o] -= 1;
      if indeg[o] == 0 {
        queue.push_back(o);
      }
    }
  }
  if order.len() != rows.len() {
    return false;
  }
  let mut old: Vec<Option<RowSpec>> = rows.drain(..).map(Some).collect();
  for i in order {
    if let Some(r) = old[i].take() {
      rows.push(r);
    }
  }
  true
}

/// 计划里所有节点的稳定顺序（拓扑安全的列顺序）。
pub fn execution_columns(plan: &Plan) -> Vec<Vec<u32>> {
  plan
    .columns
    .iter()
    .map(|c| c.rows.iter().map(|r| r.id).collect())
    .collect()
}

/// 结构校验：环、端口、断开的必要节点。
pub fn validate(plan: &Plan) -> Vec<String> {
  let mut problems = Vec::new();
  if plan.columns.is_empty() {
    problems.push("画板是空的".into());
    return problems;
  }
  let ids: HashSet<u32> = plan.nodes().map(|n| n.id).collect();
  for c in &plan.columns {
    let mut local = c.rows.clone();
    if !topo_sort(&mut local) {
      problems.push("有环：连线绕回来了，无法执行".into());
    }
  }
  for n in plan.nodes() {
    if !ids.contains(&n.id) {
      problems.push(format!("节点 {} 不在计划里", n.id));
    }
    for e in &n.inputs {
      if !ids.contains(&e.from) {
        problems.push(format!("节点 {} 的输入指向不存在的节点 {}", n.id, e.from));
      }
      if (e.to_port as usize) >= input_ports(&n.ptype, n.channels) {
        problems.push(format!("节点 {} 的输入端口 {} 越界", n.id, e.to_port));
      }
    }
    if n.kind != RowKind::Input && !is_source(&n.ptype) && n.inputs.is_empty() && n.enabled {
      problems.push(format!("{} 还没有接输入（会输出静音）", n.ptype));
    }
  }
  problems
}

/// 计划里所有 `soundpad` 行的音效热键（`pads` 参数 JSON 的 `hotkey` 字段），
/// 逐条转成「规范串 → 触发该音效」的热键绑定。
pub fn pad_hotkeys(plan: &Plan, hub: crate::debug::SharedHub) -> Vec<crate::hotkey::Binding> {
  let mut out = Vec::new();
  for (ci, c) in plan.columns.iter().enumerate() {
    for (ri, r) in c.rows.iter().enumerate() {
      if r.kind != RowKind::Process || r.ptype != "soundpad" || !r.enabled {
        continue;
      }
      let Some(raw) = r.params.get("pads") else {
        continue;
      };
      let pads: Vec<crate::engine::components::soundpad::PadCfg> = match serde_json::from_str(raw) {
        Ok(p) => p,
        Err(_) => continue, // 列表坏了不该拖垮热键（会话那边会报参数问题）
      };
      for (i, pad) in pads.iter().enumerate() {
        let spec = crate::hotkey::normalize_spec(&pad.hotkey);
        if spec.is_empty() {
          continue;
        }
        let hub2 = hub.clone();
        out.push((
          spec,
          Arc::new(move || {
            // `slot()` 返回 `Option<Arc<MediaSlot>>`，`Option::inspect` 对 Some 调闭包，
            // 于是 `s.send(..)` 自动解引用到 MediaSlot——会话没运行时这里就是 no-op。
            crate::engine::media::hub()
              .slot(ci, ri)
              .inspect(|s| s.send(crate::engine::media::MediaCmd::SoundpadPlay(i)));
            hub2.push_ui(
              "info",
              format!("音效热键：第 {} 列第 {} 行 第 {} 个", ci + 1, ri + 1, i + 1),
            );
          }) as Arc<dyn Fn() + Send + Sync>,
        ));
      }
    }
  }
  out
}

/// 供 UI 使用：一个节点在画板上的默认尺寸（布局/碰撞用）。
/// 卡片**主体**尺寸（不含上下端子排；UI 再按端口数加上端子高度）。
pub fn node_size(kind: RowKind, ptype: &str, _channels: u32) -> (f32, f32) {
  // 所有卡片同宽：端口固定在左侧固定槽位，换卡片也能一眼知道第 N 路的点在哪。
  match (kind, ptype) {
    (RowKind::Input, _) | (RowKind::Output, _) | (_, "echo_cancel") => (240.0, 36.0),
    (_, "eq31") => (240.0, 36.0),
    _ => (240.0, 36.0),
  }
}

/// Arc 是为了兼容旧测试里对 `Arc<Plan>` 的使用。
pub type SharedPlan = Arc<Plan>;

#[cfg(test)]
mod tests {
  use super::*;

  #[test]
  fn normalize_strips_legacy_network_aec_params() {
    let mut row = RowSpec {
      kind: RowKind::Input,
      ptype: "remote_mic".into(),
      ..Default::default()
    };
    row.params.insert("aec".into(), "1".into());
    row.params.insert("far_device".into(), "loopback".into());
    row.params.insert("mic_gain_db".into(), "2.0".into());
    let mut plan = Plan {
      v: VERSION,
      columns: vec![ColumnSpec {
        rows: vec![row],
        enabled: true,
      }],
    };
    assert!(plan.normalize());
    let node = &plan.columns[0].rows[0];
    assert!(!node.params.contains_key("aec"));
    assert!(!node.params.contains_key("far_device"));
    assert!(!node.params.contains_key("mic_gain_db"));
  }

  /// 端口 = 通道：多通道处理节点的输入/输出端口数都跟着 `channels` 走。
  /// 立体声降噪必须有 2 个输出端口，否则界面连不出第二条通道。
  #[test]
  fn ports_follow_channels() {
    assert_eq!(input_ports("denoise", 2), 2);
    assert_eq!(output_ports("denoise", 2), 2);
    assert_eq!(input_ports("gain", 4), 4);
    assert_eq!(output_ports("gain", 4), 4);
    assert_eq!(input_ports("audio_output", 2), 2);
    assert_eq!(output_ports("audio_output", 2), 0);
    // AEC 例外：两个语义端口、一个输出，通道数固定 1
    assert_eq!(input_ports("echo_cancel", 1), 2);
    assert_eq!(output_ports("echo_cancel", 1), 1);
  }

  /// 立体声连线（L→L、R→R）能建立，越界端口必须被拒绝。
  #[test]
  fn stereo_edges_are_validated_by_port() {
    let mut plan = Plan {
      v: VERSION,
      columns: vec![ColumnSpec {
        enabled: true,
        rows: Vec::new(),
      }],
    };
    plan.columns[0].rows.push(RowSpec {
      id: 1,
      kind: RowKind::Input,
      ptype: "loopback".into(),
      channels: 2,
      ..Default::default()
    });
    plan.columns[0].rows.push(RowSpec {
      id: 2,
      kind: RowKind::Output,
      ptype: "audio_output".into(),
      channels: 2,
      ..Default::default()
    });
    plan.normalize();
    assert!(plan.connect(1, 0, 2, 0).is_ok());
    assert!(plan.connect(1, 1, 2, 1).is_ok());
    assert!(plan.connect(1, 2, 2, 0).is_err(), "来源没有第 3 个端口");
    assert!(plan.connect(1, 0, 2, 2).is_err(), "目标没有第 3 个端口");
  }
}
