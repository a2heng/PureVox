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

//! 节点注册表（DESIGN.md §3.5）：界面与 SessionPlan 只能通过它发现节点类型；
//! `create_stage` 是处理类组件的唯一实例化入口。
//!
//! 规格字段（label / kind / params）供界面与 SessionPlan 使用，随后续步骤接入。
#![allow(dead_code)]

use std::collections::BTreeMap;

use super::stage::Stage;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[allow(dead_code)] // 预留：输入/输出/可视化节点由后续步骤接入（DESIGN.md §4）
pub enum NodeKind {
  Input,
  Process,
  Output,
  Viz,
}

#[derive(Clone, Debug, PartialEq)]
#[allow(dead_code)] // 预留：数值/布尔参数由后续组件使用
pub enum ParamValue {
  Number(f64),
  Text(String),
  Bool(bool),
}

pub type Params = BTreeMap<String, ParamValue>;

#[derive(Clone, Copy, Debug)]
#[allow(dead_code)] // 预留
pub enum ParamKind {
  Number { lo: f64, hi: f64, default: f64, step: f64 },
  Text { default: &'static str },
  Bool { default: bool },
}

#[derive(Clone, Copy, Debug)]
pub struct ParamSpec {
  pub key: &'static str,
  pub label: &'static str,
  pub kind: ParamKind,
}

pub struct NodeSpec {
  pub ptype: &'static str,
  pub label: &'static str,
  pub kind: NodeKind,
  pub params: &'static [ParamSpec],
}

static SPECS: &[NodeSpec] = &[
  // 输入行（源）
  NodeSpec { ptype: "audio_input", label: "录音输入", kind: NodeKind::Input, params: &[] },
  NodeSpec { ptype: "tone", label: "测试音 1 kHz", kind: NodeKind::Input, params: &[] },
  NodeSpec {
    ptype: "echo_cancel",
    label: "AEC 回声消除",
    kind: NodeKind::Input,
    params: &[
      ParamSpec {
        key: "model",
        label: "模型",
        kind: ParamKind::Text { default: crate::infer::aec::MODEL_AEC },
      },
      // far/远端参考设备：`loopback` = 系统默认输出回环、`loopback:<渲染端点ID>` = 指定输出回环、
      // 其它 = 输入设备。校准探针自动送到被回环的输出（无需另选播放）。
      ParamSpec { key: "far_device", label: "远端设备", kind: ParamKind::Text { default: "" } },
      // 进 AEC 模型前的端侧增益（近端抬高听得清；远端与近端做电平匹配）
      ParamSpec {
        key: "mic_gain_db",
        label: "近端增益",
        kind: ParamKind::Number { lo: -40.0, hi: 40.0, default: 0.0, step: 1.0 },
      },
      ParamSpec {
        key: "far_gain_db",
        label: "远端增益",
        kind: ParamKind::Number { lo: -40.0, hi: 40.0, default: 0.0, step: 1.0 },
      },
      // 直通：跳过 AEC 直接过 mic（A/B 对比）
      ParamSpec { key: "bypass", label: "直通", kind: ParamKind::Bool { default: false } },
      ParamSpec {
        key: "far_delay_ms",
        label: "远端延时",
        kind: ParamKind::Number { lo: -1000.0, hi: 1000.0, default: 0.0, step: 10.0 },
      },
    ],
  },
  // 处理行
  NodeSpec {
    ptype: "denoise",
    label: "降噪",
    kind: NodeKind::Process,
    params: &[ParamSpec {
      key: "model",
      label: "模型",
      kind: ParamKind::Text { default: crate::infer::MODEL_DENOISE },
    }],
  },
  NodeSpec {
    ptype: "gain",
    label: "增益",
    kind: NodeKind::Process,
    params: &[ParamSpec {
      key: "gain_db",
      label: "增益(dB)",
      kind: ParamKind::Number { lo: -40.0, hi: 40.0, default: 0.0, step: 1.0 },
    }],
  },
  NodeSpec {
    ptype: "tse",
    label: "目标说话人 TSE",
    kind: NodeKind::Process,
    params: &[
      ParamSpec {
        key: "model",
        label: "模型",
        kind: ParamKind::Text { default: crate::infer::tse::MODEL_TSE },
      },
      // 留空 = 默认 ~/.purevox/tse_reference.wav
      ParamSpec { key: "reference", label: "参考语音", kind: ParamKind::Text { default: "" } },
    ],
  },
  // 输出行（汇）
  NodeSpec { ptype: "audio_output", label: "音频输出", kind: NodeKind::Output, params: &[] },
];

impl NodeKind {
  pub fn as_str(&self) -> &'static str {
    match self {
      NodeKind::Input => "input",
      NodeKind::Process => "process",
      NodeKind::Output => "output",
      NodeKind::Viz => "viz",
    }
  }
}

impl ParamSpec {
  pub fn kind_str(&self) -> &'static str {
    match self.kind {
      ParamKind::Number { .. } => "number",
      ParamKind::Text { .. } => "text",
      ParamKind::Bool { .. } => "bool",
    }
  }

  pub fn default_text(&self) -> String {
    match self.kind {
      ParamKind::Text { default } => default.to_string(),
      ParamKind::Number { default, .. } => default.to_string(),
      ParamKind::Bool { default } => default.to_string(),
    }
  }
}

pub fn all_specs() -> &'static [NodeSpec] {
  SPECS
}

pub fn get_spec(ptype: &str) -> Option<&'static NodeSpec> {
  SPECS.iter().find(|s| s.ptype == ptype)
}

/// 实例化处理类组件（输入/输出/可视化不是 Stage，由 L0 实现）。
pub fn create_stage(ptype: &str, params: &Params) -> Result<Box<dyn Stage>, String> {
  let spec = get_spec(ptype).ok_or_else(|| format!("未知节点类型 {ptype}"))?;
  match spec.ptype {
    "denoise" => {
      let model = match params.get("model") {
        Some(ParamValue::Text(t)) => t.clone(),
        _ => crate::infer::MODEL_DENOISE.to_string(),
      };
      Ok(Box::new(super::components::denoise::DenoiseStage::load(&model)?))
    }
    "tse" => {
      let model = match params.get("model") {
        Some(ParamValue::Text(t)) => t.clone(),
        _ => crate::infer::tse::MODEL_TSE.to_string(),
      };
      let reference = match params.get("reference") {
        Some(ParamValue::Text(t)) if !t.trim().is_empty() => Some(t.clone()),
        _ => None,
      };
      Ok(Box::new(super::components::tse::TseStage::load(&model, reference.as_deref())?))
    }
    "gain" => {
      let db = match params.get("gain_db") {
        Some(ParamValue::Number(n)) => *n,
        Some(ParamValue::Text(t)) => t.trim().parse().unwrap_or(0.0),
        _ => 0.0,
      };
      Ok(Box::new(super::components::gain::GainStage::load(db)))
    }
    other => Err(format!("节点类型 {other} 不是处理组件")),
  }
}
