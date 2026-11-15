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

//! TSE 参考录音（ARCHITECTURE.md §7）：在列工作线程里对「进入 TSE 节点、经过节点
//! 输入增益、TSE 处理之前」的信号取样，写 48 kHz 单声道 WAV，并把实测 RMS / 峰值一起报出来。
//!
//! **保留窗口恒为 10 s，前后各丢 0.5 s**：点「录制」后**立刻开始采**（点鼠标那一下的
//! 杂音落在头部被丢掉），先录 [`REF_WARMUP_SECONDS`] 预热，再保留
//! [`REF_RECORD_SECONDS`] 秒，最后录 [`REF_TAIL_SECONDS`] 余量（丢尾，TSE 有 1 hop 滞后 +
//! 设备缓冲，保证第 10 s 的语音收全）。写盘只写中间那段，长度恒为 10 s；界面进度条
//! 也从预热结束才开始走、在尾巴开始前到 100%。
//!
//! **不做音量归一化 / 限幅**（2026-10-26 实测 + legacy 对照）：
//! - 主模型对参考电平不敏感：同一段参考整体 ±20 dB 后喂 ref_encoder + 主模型，
//!   主模型输出相对差异 < 2%（只有严重削顶的极端电平才明显变差）；
//! - legacy `pvengine/components/tse.py` + `_Recorder` 也是原样写盘；
//! - 归一化为了压峰值必然削掉/软限幅峰值，反而是「不真实」的那一步。
//!
//! 只保留最小校验（近静音拒绝），避免录出一段没法用的参考。
//!
//! 采样点由界面传入的 TSE 节点 id 决定：只录**那个节点**的输入（已经过节点输入增益），
//! 不录 TSE 输出，也不会串到别的 TSE / 后级。进度与结果发布到调试快照的 `recorder` 字段。
//!
//! 三条纪律（对应 legacy `_Recorder` 的行为）：
//! - **写盘不在音频线程**：录满后把缓冲交给后台线程写 WAV，列线程下一拍继续跑；
//! - **看门狗**：`ref_capture_seconds() + 2 s` 内没有收满（引擎没启动、行停用、麦克风没输入）
//!   就明确失败，不会永远停在「录制中 0.0/10 s」；
//! - **只录启用行**：停用的列/行不参与目标解析（否则录音永远收不到样本）。

use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::sync::Mutex;
use std::sync::atomic::{AtomicU64, Ordering::Relaxed};
use std::time::Instant;

use crate::audio::SAMPLE_RATE;
use crate::debug::{Probe, SharedHub};
use crate::plan::Plan;

/// 参考录音时长（秒）：**最终保留 10 s**（TSE 注册 1001 帧 = 10 s，不裁剪）。
pub const REF_RECORD_SECONDS: f64 = 10.0;

/// 起录**预热**（丢头，秒）：点下按钮就开路，先丢掉鼠标点击那一下的杂音；
/// 进度条也从这之后才开始走。用户要求前后各 0.5 s，总占用 11 s，保留中间 10 s。
pub const REF_WARMUP_SECONDS: f64 = 0.5;

/// 收尾**余量**（丢尾，秒）：TSE 有 1 hop 滞后 + 设备缓冲，留一段尾巴保证第 10 s 的语音收全。
pub const REF_TAIL_SECONDS: f64 = 0.5;

/// 实际占用链路的总时长（预热 + 保留 + 余量）。
pub fn ref_capture_seconds() -> f64 {
  REF_WARMUP_SECONDS + REF_RECORD_SECONDS + REF_TAIL_SECONDS
}

/// 采样相位：处理行「处理前」= 上游输出；「处理后」= 本行输出。
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Phase {
  Pre,
  Post,
}

/// 录音对外状态（界面画进度条用；调试 `recorder` 文本仍保留人话）。
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum RecorderPhase {
  Idle,
  /// 正在收中间 10 s（预热已经过去，进度条在走）
  Recording,
  /// 10 s 已收满，正在收最后 0.5 s 尾巴（进度条停在 100%，还没写盘）
  Finishing,
  /// 正在后台写 WAV
  Encoding,
  Done,
  Failed,
}

#[derive(Clone, Copy, Debug)]
pub struct RecorderStatus {
  pub phase: RecorderPhase,
  /// 本次录音代号：同一代只触发一次「完成后重建 TSE」
  pub token: u64,
  pub elapsed_s: f64,
  pub total_s: f64,
}

enum State {
  Idle,
  Recording {
    /// 本次录音的代号（看门狗只允许取消同一代）
    token: u64,
    target: (usize, usize, Phase),
    buf: Vec<f32>,
    /// 保留窗口的样本数（10 s）
    needed: usize,
    /// 开头丢掉的预热样本数
    skip: usize,
    /// 含预热 + 保留 + 余量的总样本数（收满才写盘）
    total: usize,
    path: PathBuf,
    last_pub: Instant,
  },
  /// 正在后台归一化 + 写盘（此时不再收样本，也不允许开新录音）
  Encoding {
    token: u64,
  },
  Done {
    token: u64,
  },
  Failed {
    token: u64,
  },
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

  /// 界面进度条：只读一份状态，不解析人话文本。
  pub fn status(&self) -> RecorderStatus {
    let st = self.state.lock().unwrap_or_else(|e| e.into_inner());
    match &*st {
      State::Idle => RecorderStatus {
        phase: RecorderPhase::Idle,
        token: 0,
        elapsed_s: 0.0,
        total_s: REF_RECORD_SECONDS,
      },
      State::Recording {
        token,
        buf,
        needed,
        skip,
        ..
      } => {
        let got = buf.len().saturating_sub(*skip).min(*needed);
        let phase = if buf.len() >= skip + needed {
          RecorderPhase::Finishing
        } else {
          RecorderPhase::Recording
        };
        RecorderStatus {
          phase,
          token: *token,
          elapsed_s: got as f64 / SAMPLE_RATE as f64,
          total_s: *needed as f64 / SAMPLE_RATE as f64,
        }
      }
      State::Encoding { token } => RecorderStatus {
        phase: RecorderPhase::Encoding,
        token: *token,
        elapsed_s: REF_RECORD_SECONDS,
        total_s: REF_RECORD_SECONDS,
      },
      State::Done { token } => RecorderStatus {
        phase: RecorderPhase::Done,
        token: *token,
        elapsed_s: REF_RECORD_SECONDS,
        total_s: REF_RECORD_SECONDS,
      },
      State::Failed { token } => RecorderStatus {
        phase: RecorderPhase::Failed,
        token: *token,
        elapsed_s: 0.0,
        total_s: REF_RECORD_SECONDS,
      },
    }
  }

  /// 开始录制参考：先录 [`REF_WARMUP_SECONDS`] 预热（丢头），再保留
  /// [`REF_RECORD_SECONDS`] 秒，最后录 [`REF_TAIL_SECONDS`] 余量（丢尾），写完 `path`。
  /// 返回本次录音代号（看门狗用）。
  pub fn start(&self, plan: &Plan, path: PathBuf, node: Option<u32>) -> Result<u64, String> {
    let target = resolve_target(plan, node).ok_or_else(|| {
      "选中的 TSE 节点不可用（需要已启用并接在链路上），计划里也没有其它可录的 TSE".to_string()
    })?;
    let needed = (REF_RECORD_SECONDS * SAMPLE_RATE as f64) as usize;
    let skip = (REF_WARMUP_SECONDS * SAMPLE_RATE as f64) as usize;
    let total = (ref_capture_seconds() * SAMPLE_RATE as f64) as usize;
    let token = self.next_token.fetch_add(1, Relaxed);
    let mut st = self.state.lock().unwrap_or_else(|e| e.into_inner());
    match &*st {
      State::Recording { .. } => return Err("已经在录制参考，请等这一段落完".into()),
      State::Encoding { .. } => return Err("上一段参考正在写盘，请稍候".into()),
      _ => {}
    }
    *st = State::Recording {
      token,
      target,
      buf: Vec::with_capacity(total),
      needed,
      skip,
      total,
      path,
      last_pub: Instant::now(),
    };
    drop(st);
    self
      .hub
      .set_recorder(Probe::ok(format!("录制中 0.0/{REF_RECORD_SECONDS:.0} s")));
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
      *st = State::Failed { token };
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
        target, buf, total, ..
      } = &mut *st
      else {
        return;
      };
      if *target != (col, row, phase) {
        return;
      }
      buf.extend_from_slice(samples);
      buf.len() >= *total
    };
    if !full {
      if let State::Recording {
        buf,
        needed,
        skip,
        last_pub,
        ..
      } = &mut *st
        && last_pub.elapsed().as_millis() >= 200
      {
        *last_pub = Instant::now();
        let got = buf.len().saturating_sub(*skip).min(*needed);
        let secs = got as f64 / SAMPLE_RATE as f64;
        let total = *needed as f64 / SAMPLE_RATE as f64;
        let text = if got == 0 {
          "准备中…（0.5 s 后开始计数）".to_string()
        } else if got >= *needed {
          format!("收尾中…（已收满 {total:.0} s）")
        } else {
          format!("录制中 {secs:.1}/{total:.0} s")
        };
        self.hub.set_recorder(Probe::ok(text));
      }
      return;
    }

    // 收满：立刻把状态切成 Encoding 并 release 锁；裁剪/写盘在后台线程做，
    // 绝不占住列工作线程的 10 ms 节拍。
    let token = match &*st {
      State::Recording { token, .. } => *token,
      _ => 0,
    };
    let prev = std::mem::replace(&mut *st, State::Encoding { token });
    drop(st);
    let State::Recording {
      token,
      buf,
      needed,
      skip,
      path,
      ..
    } = prev
    else {
      return;
    };
    let state = self.state.clone();
    let hub = self.hub.clone();
    let spawned = std::thread::Builder::new()
      .name("tse-ref-write".into())
      .spawn(move || {
        // 只写保留窗口：丢掉预热（头）与余量（尾），长度恒为 `needed` = 10 s。
        let result = write_reference(&buf[skip..skip + needed], &path);
        let mut st = state.lock().unwrap_or_else(|e| e.into_inner());
        match result {
          Ok(level) => {
            let text = format!(
              "完成：参考语音已保存（{secs:.0} s，RMS {rms_db:.1} dBFS，峰值 {peak_db:.1} dBFS）",
              secs = needed as f64 / SAMPLE_RATE as f64,
              rms_db = level.rms_db,
              peak_db = level.peak_db,
            );
            *st = State::Done { token };
            drop(st);
            crate::debug::events::log("info", "recorder", format!("{text} → {}", path.display()));
            hub.set_recorder(Probe::ok(text));
          }
          Err(e) => {
            *st = State::Failed { token };
            drop(st);
            crate::debug::events::log("error", "recorder", e.clone());
            hub.set_recorder(Probe::unavailable(e));
          }
        }
      });
    if let Err(e) = spawned {
      // 线程都起不来：至少别让状态停在 Encoding
      let mut st = self.state.lock().unwrap_or_else(|e| e.into_inner());
      *st = State::Failed { token };
      drop(st);
      let msg = format!("起写盘线程失败：{e}");
      crate::debug::events::log("error", "recorder", msg.clone());
      self.hub.set_recorder(Probe::unavailable(msg));
    }
  }
}

/// 采样目标：**指定 TSE 节点的输入**（已过节点输入增益、未过 TSE）；没有指定时
/// 取第一个启用 TSE 行。没有 TSE 才回退到最后一个启用降噪行的输出。
fn resolve_target(plan: &Plan, node: Option<u32>) -> Option<(usize, usize, Phase)> {
  if let Some(id) = node {
    for (ci, c) in plan.columns.iter().enumerate() {
      if !c.enabled {
        continue;
      }
      for (ri, r) in c.rows.iter().enumerate() {
        if r.enabled && r.ptype == "tse" && r.id == id {
          return Some((ci, ri, Phase::Pre));
        }
      }
    }
    return None;
  }
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

/// 原样写参考语音（16-bit WAV，**不做归一化 / 限幅 / 裁剪**），返回实测电平。
///
/// 只拒绝近静音：几乎没声音的参考不如不录，明确失败比让 TSE 悄悄直通更好排查。
fn write_reference(samples: &[f32], path: &Path) -> Result<Level, String> {
  // 落盘前垫底噪（±1e-8，≈ −160 dBFS，听不见）：参考文件里**永远没有精确 0**，
  // 下游（注册编码器 / 训练侧）不会因为全零帧踩 0/0。正常音频逐位不动。
  let mut v = samples.to_vec();
  crate::infer::tse::floor_silence(&mut v);
  let samples = &v[..];
  let r = rms(samples);
  let p = peak(samples);
  if r < 1e-5 || p < 1e-4 {
    return Err(format!("录音太小（RMS {r:.1e}），请对着麦克风说话"));
  }
  if let Some(dir) = path.parent() {
    std::fs::create_dir_all(dir).map_err(|e| format!("创建目录失败：{e}"))?;
  }
  crate::wav::write_mono_f32(path, samples, SAMPLE_RATE)?;
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
      device_name: None,
      enabled,
      params: Default::default(),
      ..Default::default()
    }
  }

  /// 进度条：预热 0.5 s 内不动；中间 10 s 走完就进入 Finishing（尾巴期间不再前进）。
  #[test]
  fn status_tracks_preroll_and_finishing() {
    let hub = crate::debug::DebugHub::new();
    let rec = RecorderHub::new(hub);
    let mut tse = row(RowKind::Process, "tse", true);
    tse.id = 7;
    let plan = Plan {
      v: 0,
      columns: vec![ColumnSpec {
        enabled: true,
        rows: vec![row(RowKind::Input, "tone", true), tse],
      }],
    };
    let p = std::env::temp_dir().join("pv-rec-progress.wav");
    let _token = rec.start(&plan, p, Some(7)).expect("开始录音");
    let st = rec.status();
    assert_eq!(st.phase, RecorderPhase::Recording);
    assert_eq!(st.elapsed_s, 0.0);
    assert_eq!(st.total_s, REF_RECORD_SECONDS);

    let skip = (REF_WARMUP_SECONDS * SAMPLE_RATE as f64) as usize;
    let needed = (REF_RECORD_SECONDS * SAMPLE_RATE as f64) as usize;
    rec.tap(0, 1, Phase::Pre, &vec![0.1; skip + needed]);
    let st = rec.status();
    assert_eq!(st.phase, RecorderPhase::Finishing);
    assert!((st.elapsed_s - REF_RECORD_SECONDS).abs() < 1e-6);
  }

  /// 停用的 TSE 行不能被选成录音目标（否则永远收不到 Pre 抽头）。
  #[test]
  fn resolve_target_skips_disabled_rows() {
    let plan = Plan {
      v: 0,
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
    assert_eq!(resolve_target(&plan, None), Some((0, 2, Phase::Post)));
  }

  /// 指定 TSE 节点时只录它自己的输入；不能退到别的 TSE / 降噪行。
  #[test]
  fn resolve_target_prefers_requested_tse_node() {
    let mut tse_a = row(RowKind::Process, "tse", true);
    tse_a.id = 7;
    let mut tse_b = row(RowKind::Process, "tse", true);
    tse_b.id = 8;
    let plan = Plan {
      v: 0,
      columns: vec![ColumnSpec {
        enabled: true,
        rows: vec![
          row(RowKind::Input, "tone", true),
          tse_a,
          tse_b,
          row(RowKind::Process, "denoise", true),
        ],
      }],
    };
    assert_eq!(resolve_target(&plan, Some(8)), Some((0, 2, Phase::Pre)));
    assert_eq!(resolve_target(&plan, Some(7)), Some((0, 1, Phase::Pre)));
    assert_eq!(resolve_target(&plan, Some(99)), None);
  }

  #[test]
  fn start_rejects_second_recording() {
    let hub = crate::debug::DebugHub::new();
    let rec = RecorderHub::new(hub);
    let plan = Plan {
      v: 0,
      columns: vec![ColumnSpec {
        enabled: true,
        rows: vec![row(RowKind::Process, "denoise", true)],
      }],
    };
    let p = std::env::temp_dir().join("pv-rec-test.wav");
    let t = rec.start(&plan, p.clone(), None).expect("第一次应当开始");
    assert!(rec.active());
    assert!(rec.start(&plan, p, None).is_err(), "录制中不许再开一段");
    rec.timeout_if_recording(t, "测试超时".into());
    assert!(!rec.active(), "看门狗必须把状态置回非录制");
  }

  /// 参考语音**原样写盘**：不做归一化 / 限幅（用户 2026-10-26 要求「更真实」）。
  /// 写的是 float32 WAV（16-bit 会把 ±1e-8 底噪量化成 0），波形与录到的一致；
  /// 回报的电平就是这段录音的实测值。
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
