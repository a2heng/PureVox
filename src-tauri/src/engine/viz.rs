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

//! 可视化插件（处理行 `viz`，ARCHITECTURE.md §4）：在链路的**当前位置**取电平与频谱，
//! 原样放行音频，不改变信号。
//!
//! 设计取自 legacy `uitk/viz.py`（沿用 legacy 的观感）：分段 LED 电平条 +
//! 对数分段的频谱柱。**实现是新版**：
//! - 频谱不搬 Mel 滤波器组：直接用我们的 960 点 FFT（50 Hz 一格）折成 64 段对数频率
//!   （`dsp::bands`，与调试面板的频谱同一份映射）；
//! - 电平与频谱都在**列工作线程**里算（每 hop 一次），不碰音频回调；
//! - 结果进 [`VizHub`]（全局单例，按 (列, 行) 寻址），界面 10 Hz 取一次画图；
//!   同一份也进 `/debug` 的 `viz` 段与 `GET /debug/viz`（DEBUG.md §1.2 / §1.4）。
//!
//! 界面侧只负责「怎么画」（LED 分段、峰值保持、EMA 平滑），Rust 侧只给**每 hop 的
//! 实测值**。两边各做自己该做的，判定口径只有一份。

use std::collections::HashMap;
use std::sync::atomic::{AtomicU64, Ordering::Relaxed};
use std::sync::{Mutex, OnceLock};

use crate::audio::{HOP, SAMPLE_RATE};
use crate::debug::now_ms;
use crate::dsp::bands::{BAND_HZ, BandAnalyzer, VIZ_BANDS};
use crate::engine::stage::{FrameContext, Stage, StageError};

/// 快照 schema 版本（字段只增不改名）。
pub const VIZ_SCHEMA_VERSION: u32 = 1;
const DB_FLOOR: f32 = -160.0;

fn db(x: f32) -> f32 {
  if x <= 0.0 {
    DB_FLOOR
  } else {
    (20.0 * x.log10()).max(DB_FLOOR)
  }
}

/// 插件在会话里的位置（发布键 + 代次）。代次过滤掉「先建后停」期间旧会话的残留帧。
#[derive(Clone, Copy, Debug)]
pub struct VizLoc {
  pub col: usize,
  pub row: usize,
  pub generation: u64,
}

/// 一行的最新一帧（快照 / 端点 / 界面共用同一份）。
#[derive(Clone, Debug, serde::Serialize)]
pub struct VizFrame {
  pub col: usize,
  pub row: usize,
  /// 每 hop 自增：界面用它判断「有没有新数据」
  pub seq: u64,
  pub updated_ms: u64,
  /// 当前 hop 的峰值 / RMS（dBFS）；还没收到样本时为 `null`（不伪造 0）
  pub peak_dbfs: Option<f32>,
  pub rms_dbfs: Option<f32>,
  /// 对数 64 段（dBFS，`BAND_HZ` 覆盖 50 Hz ~ 24 kHz）
  pub bands_db: Vec<f32>,
}

/// 可视化快照（`viz_snapshot` 命令 / `GET /debug/viz` / `/debug` 的 `viz` 段）。
#[derive(Clone, Debug, serde::Serialize)]
pub struct VizSnapshot {
  pub schema_version: u32,
  pub band_hz: [f32; 2],
  pub bands: usize,
  pub hop_ms: f32,
  pub frames: Vec<VizFrame>,
}

impl Default for VizSnapshot {
  fn default() -> Self {
    Self {
      schema_version: VIZ_SCHEMA_VERSION,
      band_hz: BAND_HZ,
      bands: VIZ_BANDS,
      hop_ms: 1000.0 * HOP as f32 / SAMPLE_RATE as f32,
      frames: Vec::new(),
    }
  }
}

struct FrameState {
  generation: u64,
  seq: u64,
  updated_ms: u64,
  peak: Option<f32>,
  rms: Option<f32>,
  bands: Vec<f32>,
}

/// 全局可视化中枢（与 `NetHub` / `MediaHub` 同款单例）。
pub struct VizHub {
  generation: AtomicU64,
  frames: Mutex<HashMap<(usize, usize), FrameState>>,
}

impl VizHub {
  fn new() -> Self {
    Self {
      generation: AtomicU64::new(0),
      frames: Mutex::new(HashMap::new()),
    }
  }

  /// 新会话开始构建：换代次并清掉上一代的帧（先建后停时旧线程的残留也会被代次滤掉）。
  pub fn begin(&self, generation: u64) {
    self.generation.store(generation, Relaxed);
    if let Ok(mut m) = self.frames.lock() {
      m.clear();
    }
  }

  /// 显式停引擎：清空全部帧（界面据此显示「会话未运行」，而不是冻在最后一帧）。
  pub fn clear(&self) {
    if let Ok(mut m) = self.frames.lock() {
      m.clear();
    }
  }

  /// 列工作线程每 hop 调一次。`bands` 是 64 段 dBFS。
  pub fn publish(&self, loc: VizLoc, peak_dbfs: f32, rms_dbfs: f32, bands: &[f32]) {
    let Ok(mut m) = self.frames.lock() else {
      return;
    };
    let e = m.entry((loc.col, loc.row)).or_insert_with(|| FrameState {
      generation: loc.generation,
      seq: 0,
      updated_ms: 0,
      peak: None,
      rms: None,
      bands: Vec::with_capacity(bands.len()),
    });
    e.generation = loc.generation;
    e.seq = e.seq.wrapping_add(1);
    e.updated_ms = now_ms();
    e.peak = Some(peak_dbfs);
    e.rms = Some(rms_dbfs);
    e.bands.clear();
    e.bands.extend_from_slice(bands);
  }

  /// 当前代次的全部帧（按 (列, 行) 排序，界面与 `/debug/viz` 只渲染）。
  pub fn snapshot(&self) -> VizSnapshot {
    let cur_gen = self.generation.load(Relaxed);
    let mut out = VizSnapshot {
      schema_version: VIZ_SCHEMA_VERSION,
      band_hz: BAND_HZ,
      bands: VIZ_BANDS,
      hop_ms: 1000.0 * HOP as f32 / SAMPLE_RATE as f32,
      frames: Vec::new(),
    };
    let Ok(m) = self.frames.lock() else {
      return out;
    };
    let mut keys: Vec<_> = m.keys().copied().collect();
    keys.sort_unstable();
    for k in keys {
      let Some(f) = m.get(&k) else { continue };
      if f.generation != cur_gen {
        continue;
      }
      out.frames.push(VizFrame {
        col: k.0,
        row: k.1,
        seq: f.seq,
        updated_ms: f.updated_ms,
        peak_dbfs: f.peak,
        rms_dbfs: f.rms,
        bands_db: f.bands.clone(),
      });
    }
    out
  }
}

/// 全局中枢。
pub fn hub() -> &'static VizHub {
  static HUB: OnceLock<VizHub> = OnceLock::new();
  HUB.get_or_init(VizHub::new)
}

/// 快照入口（命令与 `/debug` 共用）。
pub fn snapshot() -> VizSnapshot {
  hub().snapshot()
}

/// 插件自己的测量器：每 hop 输出峰值 / RMS / 64 段对数频谱。
///
/// 与 `dsp::meter::Meter` 同一套 FFT 归一化（满幅正弦 → 0 dBFS），区别是**不做发布周期
/// 平均**（插件要每 hop 的瞬时值），也不保存波形（插件不画波形）。
struct VizMeter {
  /// 显示用对数频谱：4800 点分析窗（100 ms，10 Hz 一格），与音频路径的 960 点无关
  bands: BandAnalyzer,
  bands_buf: Vec<f32>,
}

impl VizMeter {
  fn new() -> Self {
    Self {
      bands: BandAnalyzer::new(),
      bands_buf: Vec::with_capacity(VIZ_BANDS),
    }
  }

  /// 处理一 hop，返回 (峰值 dBFS, RMS dBFS)，内部更新 `bands_buf`。
  fn on_hop(&mut self, hop: &[f32]) -> (f32, f32) {
    let mut peak = 0.0f32;
    let mut sumsq = 0.0f64;
    for &x in hop {
      peak = peak.max(x.abs());
      sumsq += (x as f64) * (x as f64);
    }
    self.bands.on_hop(hop, &mut self.bands_buf);
    let rms = (sumsq / hop.len().max(1) as f64).sqrt() as f32;
    (db(peak), db(rms))
  }
}

/// 处理行 `viz`：只测量、不改信号。
pub struct VizStage {
  loc: VizLoc,
  meter: VizMeter,
  peak: Option<f32>,
  rms: Option<f32>,
  hops: u64,
}

impl VizStage {
  pub fn new(loc: VizLoc) -> Self {
    Self {
      loc,
      meter: VizMeter::new(),
      peak: None,
      rms: None,
      hops: 0,
    }
  }
}

impl Stage for VizStage {
  fn name(&self) -> &'static str {
    "viz"
  }

  fn process(&mut self, frame: &mut [f32], _ctx: &FrameContext) -> Result<(), StageError> {
    if frame.len() != HOP {
      return Err(StageError::Unavailable(format!(
        "可视化只接受 {} 样本的 hop（收到 {}）",
        HOP,
        frame.len()
      )));
    }
    let (peak, rms) = self.meter.on_hop(frame);
    self.peak = Some(peak);
    self.rms = Some(rms);
    self.hops = self.hops.wrapping_add(1);
    hub().publish(self.loc, peak, rms, &self.meter.bands_buf);
    Ok(())
  }

  fn status(&self) -> Option<String> {
    match (self.peak, self.rms) {
      (Some(p), Some(r)) => Some(format!(
        "电平 {p:.1} dBFS（RMS {r:.1}），已测 {} hop",
        self.hops
      )),
      _ => Some("等待音频".to_string()),
    }
  }
}

#[cfg(test)]
mod tests {
  use super::*;

  #[test]
  fn publishes_per_position_and_filters_by_generation() {
    let h = VizHub::new();
    h.begin(7);
    let loc = VizLoc {
      col: 0,
      row: 1,
      generation: 7,
    };
    h.publish(loc, -12.0, -20.0, &[(-30.0f32); VIZ_BANDS]);
    let s = h.snapshot();
    assert_eq!(s.frames.len(), 1);
    assert_eq!(s.frames[0].col, 0);
    assert_eq!(s.frames[0].row, 1);
    assert_eq!(s.frames[0].bands_db.len(), VIZ_BANDS);
    assert_eq!(s.frames[0].peak_dbfs, Some(-12.0));
    // 旧代次的发布不能盖住新会话
    h.begin(8);
    assert!(h.snapshot().frames.is_empty());
    h.publish(loc, -3.0, -6.0, &[(-20.0f32); VIZ_BANDS]);
    assert!(
      h.snapshot().frames.is_empty(),
      "旧代次的帧必须被过滤掉（先建后停期间旧线程还在跑）"
    );
  }

  #[test]
  fn meter_reads_a_full_scale_tone() {
    let mut m = VizMeter::new();
    let mut frame = [0.0f32; HOP];
    for (i, x) in frame.iter_mut().enumerate() {
      *x = (2.0 * std::f32::consts::PI * 1000.0 * i as f32 / SAMPLE_RATE as f32).sin();
    }
    let (peak, _rms) = m.on_hop(&frame);
    assert!(peak > -1.0, "满幅正弦的峰值应≈0 dBFS，实际 {peak}");
    assert_eq!(m.bands_buf.len(), VIZ_BANDS);
    // 1 kHz 应该落在中低频的那个段上
    let (i, _) = m
      .bands_buf
      .iter()
      .enumerate()
      .max_by(|a, b| a.1.partial_cmp(b.1).unwrap())
      .unwrap();
    assert!((20..=45).contains(&i), "1 kHz 的段索引应在 20~45，实际 {i}");
  }

  #[test]
  fn stage_passes_the_hop_through_unchanged() {
    let mut st = VizStage::new(VizLoc {
      col: 1,
      row: 2,
      generation: 9,
    });
    let src = [0.25f32; HOP];
    let mut out = src;
    st.process(&mut out, &FrameContext::default()).unwrap();
    assert_eq!(out, src, "可视化必须原样放行");
  }
}
