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

//! TSE 参考录音（ARCHITECTURE.md §7）：在列工作线程里对「降噪后、TSE 前」的信号取样，
//! 录满 N 秒后**原样**写 48 kHz 单声道 WAV，并把实测 RMS / 峰值一起报出来。
//!
//! **不做音量归一化 / 限幅**（2026-10-26 实测 + legacy 对照）：
//! - 主模型对参考电平不敏感：同一段参考整体 ±20 dB 后喂 ref_encoder + 主模型，
//!   主模型输出相对差异 < 2%（只有严重削顶的极端电平才明显变差）；
//! - legacy `pvengine/components/tse.py` + `_Recorder` 也是原样写盘；
//! - 归一化为了压峰值必然削掉/软限幅峰值，反而是「不真实」的那一步。
//!
//! 只保留最小校验（近静音拒绝），避免录出一段没法用的参考。
//!
//! 采样点由计划决定：优先第一个**启用**的 TSE 行的输入（降噪后）；没有 TSE 行则取最后一个
//! 启用降噪行的输出。进度与结果发布到调试快照的 `recorder` 字段。
//!
//! 三条纪律（对应 legacy `_Recorder` 的行为）：
//! - **写盘不在音频线程**：录满后把缓冲交给后台线程归一化 + 写 WAV，列线程下一拍继续跑；
//! - **看门狗**：`seconds + 2 s` 内没有收满（引擎没启动、行停用、麦克风没输入）就明确失败，
//!   不会永远停在「录制中 0.0/10 s」；
//! - **只录启用行**：停用的列/行不参与目标解析（否则录音永远收不到样本）。

use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::sync::Mutex;
use std::sync::atomic::{AtomicU64, Ordering::Relaxed};
use std::time::Instant;

use crate::audio::SAMPLE_RATE;
use crate::debug::{Probe, SharedHub};
use crate::plan::Plan;

/// 采样相位：处理行「处理前」= 上游输出；「处理后」= 本行输出。
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Phase {
  Pre,
  Post,
}

enum State {
  Idle,
  Recording {
    /// 本次录音的代号（看门狗只允许取消同一代）
    token: u64,
    target: (usize, usize, Phase),
    buf: Vec<f32>,
    needed: usize,
    path: PathBuf,
    last_pub: Instant,
  },
  /// 正在后台归一化 + 写盘（此时不再收样本，也不允许开新录音）
  Encoding,
  Done,
  Failed,
}

pub struct RecorderHub {
  /// 与后台写盘线程共享（写盘线程完成后要更新状态）
  state: Arc<Mutex<State>>,
  hub: SharedHub,
  next_token: AtomicU64,
}

impl RecorderHub {
  pub fn new(hub: SharedHub) -> Self {
    RecorderHub {
      state: Arc::new(Mutex::new(State::Idle)),
      hub,
      next_token: AtomicU64::new(1),
    }
  }

  /// 是否正在录制（列工作线程每轮查一次，避免每行都锁）。
  pub fn active(&self) -> bool {
    matches!(
      *self.state.lock().unwrap_or_else(|e| e.into_inner()),
      State::Recording { .. }
    )
  }

  /// 开始录制：`seconds` 秒，写 `path`。返回本次录音代号（看门狗用）。
  pub fn start(&self, plan: &Plan, seconds: f64, path: PathBuf) -> Result<u64, String> {
    let target = resolve_target(plan)
      .ok_or_else(|| "计划里没有启用的 TSE 行，也没有降噪行；请先加一个再录参考".to_string())?;
    let needed = (seconds.max(1.0) * SAMPLE_RATE as f64) as usize;
    let token = self.next_token.fetch_add(1, Relaxed);
    let mut st = self.state.lock().unwrap_or_else(|e| e.into_inner());
    match &*st {
      State::Recording { .. } => return Err("已经在录制参考，请等这一段落完".into()),
      State::Encoding => return Err("上一段参考正在写盘，请稍候".into()),
      _ => {}
    }
    *st = State::Recording {
      token,
      target,
      buf: Vec::with_capacity(needed),
      needed,
      path,
      last_pub: Instant::now(),
    };
    drop(st);
    self
      .hub
      .set_recorder(Probe::ok(format!("录制中 0.0/{seconds:.0} s")));
    Ok(token)
  }

  /// 看门狗：`token` 仍是当前录音且仍未收满 → 明确失败（不是永远「录制中」）。
  pub fn timeout_if_recording(&self, token: u64, reason: String) {
    let mut st = self.state.lock().unwrap_or_else(|e| e.into_inner());
    let hit = matches!(
      &*st,
      State::Recording { token: t, .. } if *t == token
    );
    if hit {
      *st = State::Failed;
      drop(st);
      crate::debug::events::log("warn", "recorder", reason.clone());
      self.hub.set_recorder(Probe::unavailable(reason));
    }
  }

  /// 列工作线程在目标行/相位调用；收齐后交给后台线程归一化并写盘。
  pub fn tap(&self, col: usize, row: usize, phase: Phase, samples: &[f32]) {
    let mut st = self.state.lock().unwrap_or_else(|e| e.into_inner());
    let full = {
      let State::Recording {
        target,
        buf,
        needed,
        ..
      } = &mut *st
      else {
        return;
      };
      if *target != (col, row, phase) {
        return;
      }
      buf.extend_from_slice(samples);
      buf.len() >= *needed
    };
    if !full {
      if let State::Recording {
        buf,
        needed,
        last_pub,
        ..
      } = &mut *st
        && last_pub.elapsed().as_millis() >= 200
      {
        *last_pub = Instant::now();
        let secs = buf.len() as f64 / SAMPLE_RATE as f64;
        let total = *needed as f64 / SAMPLE_RATE as f64;
        self
          .hub
          .set_recorder(Probe::ok(format!("录制中 {secs:.1}/{total:.0} s")));
      }
      return;
    }

    // 收满：立刻把状态切成 Encoding 并 release 锁；归一化/写盘在后台线程做，
    // 绝不占住列工作线程的 10 ms 节拍。
    let prev = std::mem::replace(&mut *st, State::Encoding);
    drop(st);
    let State::Recording {
      buf, needed, path, ..
    } = prev
    else {
      return;
    };
    let state = self.state.clone();
    let hub = self.hub.clone();
    let spawned = std::thread::Builder::new()
      .name("tse-ref-write".into())
      .spawn(move || {
        let result = write_reference(&buf[..needed], &path);
        let mut st = state.lock().unwrap_or_else(|e| e.into_inner());
        match result {
          Ok(level) => {
            let text = format!(
              "完成：参考语音已保存（{secs:.0} s，RMS {rms_db:.1} dBFS，峰值 {peak_db:.1} dBFS）",
              secs = needed as f64 / SAMPLE_RATE as f64,
              rms_db = level.rms_db,
              peak_db = level.peak_db,
            );
            *st = State::Done;
            drop(st);
            crate::debug::events::log("info", "recorder", format!("{text} → {}", path.display()));
            hub.set_recorder(Probe::ok(text));
          }
          Err(e) => {
            *st = State::Failed;
            drop(st);
            crate::debug::events::log("error", "recorder", e.clone());
            hub.set_recorder(Probe::unavailable(e));
          }
        }
      });
    if let Err(e) = spawned {
      // 线程都起不来：至少别让状态停在 Encoding
      let mut st = self.state.lock().unwrap_or_else(|e| e.into_inner());
      *st = State::Failed;
      drop(st);
      let msg = format!("起写盘线程失败：{e}");
      crate::debug::events::log("error", "recorder", msg.clone());
      self.hub.set_recorder(Probe::unavailable(msg));
    }
  }
}

/// 采样目标：第一个**启用**的 TSE 行的输入（降噪后）；否则最后一个启用降噪行的输出。
fn resolve_target(plan: &Plan) -> Option<(usize, usize, Phase)> {
  for (ci, c) in plan.columns.iter().enumerate() {
    if !c.enabled {
      continue;
    }
    for (ri, r) in c.rows.iter().enumerate() {
      if r.enabled && r.ptype == "tse" {
        return Some((ci, ri, Phase::Pre));
      }
    }
  }
  for (ci, c) in plan.columns.iter().enumerate() {
    if !c.enabled {
      continue;
    }
    for (ri, r) in c.rows.iter().enumerate().rev() {
      if r.enabled && r.ptype == "denoise" {
        return Some((ci, ri, Phase::Post));
      }
    }
  }
  None
}

/// 写盘后的实测电平（单位 dBFS；RMS 与峰值分开报，不拿峰值冒充「归一化结果」）。
struct Level {
  rms_db: f32,
  peak_db: f32,
}

/// 原样写参考语音（16-bit WAV，**不做归一化 / 限幅**），返回实测电平。
///
/// 只拒绝近静音：几乎没声音的参考不如不录，明确失败比让 TSE 悄悄直通更好排查。
fn write_reference(samples: &[f32], path: &Path) -> Result<Level, String> {
  let r = rms(samples);
  let p = peak(samples);
  if r < 1e-5 || p < 1e-4 {
    return Err(format!("录音太小（RMS {r:.1e}），请对着麦克风说话"));
  }
  if let Some(dir) = path.parent() {
    std::fs::create_dir_all(dir).map_err(|e| format!("创建目录失败：{e}"))?;
  }
  crate::wav::write_mono_16(path, samples, SAMPLE_RATE)?;
  Ok(Level {
    rms_db: 20.0 * r.max(1e-6).log10(),
    peak_db: 20.0 * p.max(1e-6).log10(),
  })
}

fn rms(samples: &[f32]) -> f32 {
  if samples.is_empty() {
    return 0.0;
  }
  (samples.iter().map(|x| x * x).sum::<f32>() / samples.len() as f32).sqrt()
}

fn peak(samples: &[f32]) -> f32 {
  samples.iter().fold(0.0f32, |m, x| m.max(x.abs()))
}

#[cfg(test)]
mod tests {
  use super::*;
  use crate::plan::{ColumnSpec, RowKind, RowSpec};

  fn row(kind: RowKind, ptype: &str, enabled: bool) -> RowSpec {
    RowSpec {
      kind,
      ptype: ptype.into(),
      device: None,
      enabled,
      params: Default::default(),
    }
  }

  /// 停用的 TSE 行不能被选成录音目标（否则永远收不到 Pre 抽头）。
  #[test]
  fn resolve_target_skips_disabled_rows() {
    let plan = Plan {
      columns: vec![ColumnSpec {
        enabled: true,
        rows: vec![
          row(RowKind::Input, "tone", true),
          row(RowKind::Process, "tse", false),
          row(RowKind::Process, "denoise", true),
          row(RowKind::Output, "audio_output", true),
        ],
      }],
    };
    assert_eq!(resolve_target(&plan), Some((0, 2, Phase::Post)));
  }

  #[test]
  fn start_rejects_second_recording() {
    let hub = crate::debug::DebugHub::new();
    let rec = RecorderHub::new(hub);
    let plan = Plan {
      columns: vec![ColumnSpec {
        enabled: true,
        rows: vec![row(RowKind::Process, "denoise", true)],
      }],
    };
    let p = std::env::temp_dir().join("pv-rec-test.wav");
    let t = rec.start(&plan, 1.0, p.clone()).expect("第一次应当开始");
    assert!(rec.active());
    assert!(rec.start(&plan, 1.0, p).is_err(), "录制中不许再开一段");
    rec.timeout_if_recording(t, "测试超时".into());
    assert!(!rec.active(), "看门狗必须把状态置回非录制");
  }

  /// 参考语音**原样写盘**：不做归一化 / 限幅（用户 2026-10-26 要求「更真实」）。
  /// 写进去的波形与录到的一致，只差 16-bit 量化；回报的电平就是这段录音的实测值。
  #[test]
  fn write_reference_keeps_waveform_unchanged() {
    let v: Vec<f32> = (0..4_800)
      .map(|i| 0.2 * ((i as f32) * 0.02).sin() * (1.0 - (i as f32) / 9_600.0))
      .collect();
    let path = std::env::temp_dir().join(format!("pv-rec-raw-{}.wav", std::process::id()));
    let level = write_reference(&v, &path).expect("应能写盘");
    let (back, rate) = crate::wav::read_mono(&path).expect("应能读回");
    let _ = std::fs::remove_file(&path);
    assert_eq!(rate, SAMPLE_RATE);
    assert_eq!(back.len(), v.len());
    assert!(
      back.iter().zip(&v).all(|(b, x)| (b - x).abs() <= 1e-4),
      "写盘只能有 16-bit 量化误差，不允许改增益 / 限幅"
    );
    let expect_rms = 20.0 * rms(&v).log10();
    let expect_peak = 20.0 * peak(&v).log10();
    assert!((level.rms_db - expect_rms).abs() < 0.05);
    assert!((level.peak_db - expect_peak).abs() < 0.05);
  }

  #[test]
  fn write_reference_rejects_silence() {
    let path = std::env::temp_dir().join(format!("pv-rec-sil-{}.wav", std::process::id()));
    assert!(write_reference(&vec![0.0; 480], &path).is_err());
    assert!(!path.exists(), "近静音不该落盘");
  }
}
