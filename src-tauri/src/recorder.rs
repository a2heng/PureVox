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

//! TSE 参考录音（DESIGN.md §7）：在列工作线程里对「降噪后、TSE 前」的信号取样，
//! 录满 N 秒后做**音量归一化**（RMS 到 -20 dBFS，峰值不削顶）再写 48 kHz 单声道 WAV。
//!
//! 采样点由计划决定：优先第一个 TSE 行的**输入**（降噪后）；没有 TSE 行则取最后一个
//! 降噪行的**输出**。进度与结果发布到调试快照的 `recorder` 字段。

use std::path::{Path, PathBuf};
use std::sync::Mutex;
use std::time::Instant;

use crate::audio::SAMPLE_RATE;
use crate::debug::{Probe, SharedHub};
use crate::plan::Plan;

/// 采样相位：处理行「处理前」= 上游输出；「处理后」= 本行输出。
#[derive(Clone, Copy, PartialEq, Eq)]
pub enum Phase {
  Pre,
  Post,
}

enum State {
  Idle,
  Recording {
    target: (usize, usize, Phase),
    buf: Vec<f32>,
    needed: usize,
    path: PathBuf,
    last_pub: Instant,
  },
  Done,
  Failed,
}

pub struct RecorderHub {
  state: Mutex<State>,
  hub: SharedHub,
}

impl RecorderHub {
  pub fn new(hub: SharedHub) -> Self {
    RecorderHub { state: Mutex::new(State::Idle), hub }
  }

  /// 是否正在录制（列工作线程每轮查一次，避免每行都锁）。
  pub fn active(&self) -> bool {
    matches!(*self.state.lock().unwrap(), State::Recording { .. })
  }

  /// 开始录制：`seconds` 秒，写 `path`。目标从计划解析。
  pub fn start(&self, plan: &Plan, seconds: f64, path: PathBuf) -> Result<(), String> {
    let target = resolve_target(plan)
      .ok_or_else(|| "计划里没有 TSE 行，也没有降噪行；请先加一个再录参考".to_string())?;
    let needed = (seconds.max(1.0) * SAMPLE_RATE as f64) as usize;
    let mut st = self.state.lock().unwrap();
    *st = State::Recording {
      target,
      buf: Vec::with_capacity(needed),
      needed,
      path,
      last_pub: Instant::now(),
    };
    self.hub.set_recorder(Probe::ok(format!("录制中 0.0/{seconds:.0} s")));
    Ok(())
  }

  /// 列工作线程在目标行/相位调用；收齐后归一化并写盘。
  pub fn tap(&self, col: usize, row: usize, phase: Phase, samples: &[f32]) {
    let mut st = self.state.lock().unwrap();
    let full = {
      let State::Recording { target, buf, needed, .. } = &mut *st else {
        return;
      };
      if *target != (col, row, phase) {
        return;
      }
      buf.extend_from_slice(samples);
      buf.len() >= *needed
    };
    if !full {
      if let State::Recording { buf, needed, last_pub, .. } = &mut *st {
        if last_pub.elapsed().as_millis() >= 200 {
          *last_pub = Instant::now();
          let secs = buf.len() as f64 / SAMPLE_RATE as f64;
          let total = *needed as f64 / SAMPLE_RATE as f64;
          self.hub.set_recorder(Probe::ok(format!("录制中 {secs:.1}/{total:.0} s")));
        }
      }
      return;
    }

    let prev = std::mem::replace(&mut *st, State::Idle);
    let State::Recording { buf, needed, path, .. } = prev else {
      return;
    };
    match normalize_and_write(&buf[..needed], &path) {
      Ok(peak_db) => {
        let text = format!(
          "完成：{}（{} s，归一化后峰值 {peak_db:.1} dBFS）",
          path.display(),
          needed as f64 / SAMPLE_RATE as f64
        );
        *st = State::Done;
        self.hub.set_recorder(Probe::ok(text));
      }
      Err(e) => {
        *st = State::Failed;
        self.hub.set_recorder(Probe::unavailable(e));
      }
    }
  }
}

/// 采样目标：第一个 TSE 行的输入（降噪后）；否则最后一个降噪行的输出。
fn resolve_target(plan: &Plan) -> Option<(usize, usize, Phase)> {
  for (ci, c) in plan.columns.iter().enumerate() {
    if let Some(ri) = c.rows.iter().position(|r| r.ptype == "tse") {
      return Some((ci, ri, Phase::Pre));
    }
  }
  for (ci, c) in plan.columns.iter().enumerate() {
    if let Some(ri) = c.rows.iter().rposition(|r| r.ptype == "denoise") {
      return Some((ci, ri, Phase::Post));
    }
  }
  None
}

/// 音量归一化（RMS → -20 dBFS，峰值不超 0.99）后写 16-bit WAV，返回归一化后峰值 dBFS。
fn normalize_and_write(samples: &[f32], path: &Path) -> Result<f32, String> {
  if samples.is_empty() {
    return Err("录音为空".into());
  }
  let rms = (samples.iter().map(|x| x * x).sum::<f32>() / samples.len() as f32).sqrt();
  let peak = samples.iter().fold(0.0f32, |m, x| m.max(x.abs()));
  if rms < 1e-5 || peak < 1e-4 {
    return Err(format!("录音太小（RMS {rms:.1e}），请对着麦克风说话"));
  }
  const TARGET_RMS: f32 = 0.1; // -20 dBFS
  let mut gain = (TARGET_RMS / rms).clamp(0.1, 20.0);
  if peak * gain > 0.99 {
    gain = 0.99 / peak;
  }
  let out: Vec<f32> = samples.iter().map(|x| (x * gain).clamp(-1.0, 1.0)).collect();
  crate::wav::write_mono_16(path, &out, SAMPLE_RATE)?;
  Ok(20.0 * (peak * gain).max(1e-6).log10())
}
