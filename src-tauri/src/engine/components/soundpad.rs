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

//! 音效板组件（DESIGN.md §4 处理行）：在自身链位置把播放中的音效与信号相加。
//!
//! 移植 legacy `SoundPadPlugin` 的语义：
//! - 音效整段入内存（库内 48k 单声道 WAV，短音效起播零延迟），首次播放才读文件；
//! - 起播 / 结束 / 手动停止 / 重播一律 **hop 内线性淡变**（杜绝硬切咔哒）；
//! - 重播同一音效 = 旧声部淡出、新声部从**开头**起播（不叠加并行声部）；
//! - 音量**每个音效一个**（±10 dB），拖动实时更新正在播的声部；
//! - 行启用开关关断 = 在播声部淡出后结束（衔接无缝）。
//!
//! 播放 / 音量是运行时命令（[`MediaCmd`]，经 `MediaHub` 按 (列,行) 寻址），
//! 音效列表（pads）是计划参数（增删 = 结构性变更 → 重建会话）。

use std::collections::HashMap;
use std::sync::Arc;

use serde::{Deserialize, Serialize};

use crate::audio::HOP;
use crate::config;
use crate::engine::media::{MediaCmd, MediaSlot, PadStatus};
use crate::engine::stage::{FrameContext, Stage, StageError};

/// ±10 dB（音效文件本身已够响，legacy 同款限幅）。
const VOL_LO_DB: f64 = -10.0;
const VOL_HI_DB: f64 = 10.0;

/// 计划参数里的一个音效（JSON）。字段与 legacy `uitk/main_window.py` 的 pad 记录同构。
#[derive(Deserialize, Serialize, Clone, Debug)]
pub struct PadCfg {
  #[serde(default)]
  pub name: String,
  #[serde(default)]
  pub path: String,
  #[serde(default)]
  pub volume_db: f64,
  /// 全局热键（规范串，空 = 不绑；见 `hotkey::normalize_spec`）。
  /// **跟着计划走**：热键注册由 `plan::pad_hotkeys` 统一收口（`apply_plan` 时重注册）。
  #[serde(default)]
  pub hotkey: String,
}

/// 一次播放的声部（`data` 只读共享；`pos` 仅音频线程推进）。
struct Voice {
  data: Arc<Vec<f32>>,
  pos: usize,
  vol: f32,
  /// 置位 = 进入淡出段（结束 / 被打断 / 行关断）
  ending: bool,
  /// 淡出段起点（样本索引）；`Some(pos)` 表示本 hop 内 1→0
  fade_at: Option<usize>,
}

pub struct SoundpadStage {
  slot: Arc<MediaSlot>,
  pads: Vec<PadCfg>,
  /// 与 `pads` 对齐的在播声部（None = 未播）
  voices: Vec<Option<Voice>>,
  /// 淡出中的声部（混完即弃）
  fading: Vec<Voice>,
  /// path → 解码结果（None = 加载失败；失败原因见 `load_error`）
  cache: HashMap<String, Option<Arc<Vec<f32>>>>,
  /// 最近一次加载失败的原因（进状态，不静默）
  load_error: Option<String>,
}

fn vol_gain(db: f64) -> f32 {
  10f32.powf((db.clamp(VOL_LO_DB, VOL_HI_DB) / 20.0) as f32)
}

impl SoundpadStage {
  /// `pads_json`：计划参数（JSON 数组）；`slot`：本行的媒体槽位。
  pub fn load(pads_json: &str, slot: Arc<MediaSlot>) -> Result<Self, String> {
    let pads: Vec<PadCfg> = if pads_json.trim().is_empty() {
      Vec::new()
    } else {
      serde_json::from_str(pads_json).map_err(|e| format!("音效列表参数不对：{e}"))?
    };
    Ok(SoundpadStage {
      slot,
      pads,
      voices: Vec::new(),
      fading: Vec::new(),
      cache: HashMap::new(),
      load_error: None,
    })
  }

  // ── 控制面（命令来自界面 / 热键线程，在音频线程 drain 执行）──

  fn play(&mut self, index: usize) {
    if index >= self.pads.len() {
      return;
    }
    let path = self.pads[index].path.clone();
    if !self.cache.contains_key(&path) {
      let loaded = load_pad(&path);
      if let Err(e) = &loaded {
        self.load_error = Some(e.clone());
      }
      self.cache.insert(path.clone(), loaded.ok());
    }
    let data = self.cache.get(&path).cloned().flatten();
    // 旧声部（若有）转淡出，新声部从头起播
    if let Some(old) = self.voices.get_mut(index).and_then(|v| v.take()) {
      self.fading.push(old);
    }
    match data {
      Some(d) if !d.is_empty() => {
        if self.voices.len() < self.pads.len() {
          self.voices.resize_with(self.pads.len(), || None);
        }
        self.voices[index] = Some(Voice {
          data: d,
          pos: 0,
          vol: vol_gain(self.pads[index].volume_db),
          ending: false,
          fade_at: None,
        });
      }
      _ => {
        // 加载失败：状态里带出原因（该行其余音效不受影响）
        if self.load_error.is_none() {
          self.load_error = Some(format!("音效 {} 无法加载", self.pads[index].name));
        }
      }
    }
  }

  fn stop_one(&mut self, index: usize) {
    if let Some(v) = self.voices.get_mut(index).and_then(|v| v.take()) {
      self.fading.push(v);
    }
  }

  fn stop_all(&mut self) {
    for v in self.voices.iter_mut().filter_map(|v| v.take()) {
      self.fading.push(v);
    }
  }

  fn drain_cmds(&mut self) {
    for cmd in self.slot.drain() {
      match cmd {
        MediaCmd::SoundpadPlay(i) => self.play(i),
        MediaCmd::SoundpadStop(i) => self.stop_one(i),
        MediaCmd::SoundpadStopAll => self.stop_all(),
        MediaCmd::Volume { index: Some(i), db } => {
          if let Some(p) = self.pads.get_mut(i) {
            p.volume_db = db;
          }
          if let Some(Some(v)) = self.voices.get_mut(i) {
            v.vol = vol_gain(db);
          }
        }
        // 行关断：在播声部淡出后结束（衔接无缝，legacy FADE_THROUGH 语义）
        MediaCmd::Enabled(false) => {
          for mut v in self.voices.iter_mut().filter_map(|v| v.take()) {
            v.ending = true;
            self.fading.push(v);
          }
        }
        _ => {}
      }
    }
  }

  // ── 音频面（列工作线程）──

  fn publish_status(&self) {
    let pads: Vec<PadStatus> = self
      .pads
      .iter()
      .enumerate()
      .map(|(i, p)| PadStatus {
        name: p.name.clone(),
        playing: self.voices.get(i).map(|v| v.is_some()).unwrap_or(false),
        volume_db: p.volume_db,
      })
      .collect();
    let err = self.load_error.clone();
    self.slot.set_status(|s| {
      s.pads = pads;
      s.error = err;
      s.playing = s.pads.iter().any(|p| p.playing);
    });
  }
}

/// 读库内 WAV 整段入内存（仅 48 kHz 单声道；库内文件由 `media::import` 保证）。
fn load_pad(path: &str) -> Result<Arc<Vec<f32>>, String> {
  let p = config::expand_path(path);
  let (samples, rate) = crate::wav::read_mono(&p)?;
  if rate != crate::audio::SAMPLE_RATE {
    return Err(format!(
      "{} 是 {rate} Hz（库内文件应为 48000 Hz，请重新导入）",
      p.display()
    ));
  }
  Ok(Arc::new(samples))
}

/// 混一个 hop（就地：`frame[i] += seg[i] * vol * w[i]`）。
/// 返回 true = 本声部已播完（数据耗尽或淡出段结束）。
fn mix_voice(frame: &mut [f32], v: &mut Voice) -> bool {
  let n = frame.len().min(HOP);
  if v.pos >= v.data.len() {
    return true;
  }
  let remaining = v.data.len() - v.pos;
  let take = remaining.min(n);
  // 结束段（剩余不足一个 hop 或已置 ending）：本 hop 内 1→0
  if v.fade_at.is_none() && (v.ending || remaining <= take) {
    v.fade_at = Some(v.pos);
  }
  let fade_out = v.fade_at == Some(v.pos);
  let fade_in = v.pos == 0 && !fade_out;
  let seg = &v.data[v.pos..v.pos + take];
  let inv = 1.0 / take as f32;
  for i in 0..take {
    let w = if fade_out {
      1.0 - i as f32 * inv
    } else if fade_in {
      i as f32 * inv
    } else {
      1.0
    };
    frame[i] += seg[i] * v.vol * w;
  }
  v.pos += take;
  v.pos >= v.data.len() || v.fade_at.map(|f| v.pos > f).unwrap_or(false)
}

impl Stage for SoundpadStage {
  fn name(&self) -> &'static str {
    "soundpad"
  }

  fn process(&mut self, frame: &mut [f32], _ctx: &FrameContext) -> Result<(), StageError> {
    self.drain_cmds();

    // 在播声部
    let mut finished: Vec<usize> = Vec::new();
    for (i, slot) in self.voices.iter_mut().enumerate() {
      if let Some(v) = slot.as_mut()
        && mix_voice(frame, v)
      {
        finished.push(i);
      }
    }
    for i in finished {
      if let Some(v) = self.voices[i].take() {
        self.fading.push(v);
      }
    }
    // 淡出中的声部
    let mut still: Vec<Voice> = Vec::new();
    for mut v in std::mem::take(&mut self.fading) {
      if !mix_voice(frame, &mut v) {
        still.push(v);
      }
    }
    self.fading = still;
    let mixed = self.voices.iter().any(|v| v.is_some()) || !self.fading.is_empty();

    if mixed {
      for x in frame.iter_mut() {
        *x = x.clamp(-1.0, 1.0);
      }
    }
    self.publish_status();
    Ok(())
  }

  fn reset(&mut self) {
    self.stop_all();
  }

  fn status(&self) -> Option<String> {
    let total = self.pads.len();
    let playing = self.voices.iter().filter(|v| v.is_some()).count();
    let mut s = format!("音效 {total} 个，在播 {playing}");
    if let Some(e) = &self.load_error {
      s.push_str(&format!("（加载失败：{e}）"));
    }
    Some(s)
  }
}

#[cfg(test)]
mod tests {
  use super::*;
  use crate::engine::media::MediaHub;

  fn stage_with(pads: Vec<PadCfg>) -> SoundpadStage {
    let h = MediaHub::default();
    let g = h.begin();
    let slot = h.register(g, 0, 0, "soundpad");
    let json = serde_json::to_string(&pads).unwrap();
    SoundpadStage::load(&json, slot).unwrap()
  }

  fn pad(path: &str, db: f64) -> PadCfg {
    PadCfg {
      name: path.to_string(),
      path: path.to_string(),
      volume_db: db,
      hotkey: String::new(),
    }
  }

  #[test]
  fn load_parses_pads_json() {
    let st = stage_with(vec![pad("a.wav", 0.0), pad("b.wav", -3.0)]);
    assert_eq!(st.pads.len(), 2);
    assert_eq!(st.pads[1].volume_db, -3.0);
  }

  #[test]
  fn empty_pads_passthrough() {
    let mut st = stage_with(vec![]);
    let mut frame = [0.5f32; HOP];
    st.process(&mut frame, &FrameContext::default()).unwrap();
    assert!(frame.iter().all(|&x| x == 0.5), "无音效时必须直通");
  }

  #[test]
  fn missing_file_does_not_panic_and_surfaces_error() {
    let mut st = stage_with(vec![pad("/nonexistent/pv_test_missing.wav", 0.0)]);
    st.slot.send(MediaCmd::SoundpadPlay(0));
    let mut frame = [0.0f32; HOP];
    st.process(&mut frame, &FrameContext::default()).unwrap();
    assert!(frame.iter().all(|&x| x == 0.0));
    let s = st.status().unwrap();
    assert!(s.contains("加载失败"), "状态必须带出原因：{s}");
    assert!(st.slot.status().error.is_some());
  }

  #[test]
  fn volume_command_updates_live_voice() {
    // 造一个 1 样本音效太短会立即结束；用 480 样本（一个 hop）
    let dir = std::env::temp_dir().join("pv_soundpad_test");
    std::fs::create_dir_all(&dir).unwrap();
    let p = dir.join("tone-480.wav");
    let samples: Vec<f32> = (0..HOP).map(|i| (i as f32 / HOP as f32) - 0.5).collect();
    crate::wav::write_mono_16(&p, &samples, 48_000).unwrap();
    let mut st = stage_with(vec![pad(p.to_str().unwrap(), 0.0)]);
    st.slot.send(MediaCmd::SoundpadPlay(0));
    let mut frame = [0.0f32; HOP];
    st.process(&mut frame, &FrameContext::default()).unwrap();
    let peak_before = frame.iter().fold(0.0f32, |m, x| m.max(x.abs()));
    assert!(peak_before > 0.4, "淡入中点应到 ~0.5：{peak_before}");
    // 音量 -6 dB ≈ 半幅
    st.slot.send(MediaCmd::Volume {
      index: Some(0),
      db: -6.0,
    });
    st.slot.send(MediaCmd::SoundpadPlay(0)); // 重播 = 从头
    let mut frame2 = [0.0f32; HOP];
    st.process(&mut frame2, &FrameContext::default()).unwrap();
    let peak_after = frame2.iter().fold(0.0f32, |m, x| m.max(x.abs()));
    assert!(
      peak_after < peak_before * 0.75,
      "-6 dB 后应明显变小：{peak_after}"
    );
    let _ = std::fs::remove_file(&p);
  }

  #[test]
  fn replay_restarts_from_beginning() {
    let dir = std::env::temp_dir().join("pv_soundpad_test");
    std::fs::create_dir_all(&dir).unwrap();
    let p = dir.join("tone-replay.wav");
    // 两个 hop 的斜坡：第一 hop 全负、第二 hop 全正
    let samples: Vec<f32> = (0..HOP * 2)
      .map(|i| if i < HOP { -0.5 } else { 0.5 })
      .collect();
    crate::wav::write_mono_16(&p, &samples, 48_000).unwrap();
    let mut st = stage_with(vec![pad(p.to_str().unwrap(), 0.0)]);
    // 看 hop **中点**：起播是 hop 内线性淡入，首样本恒为 0（杜绝咔哒），
    // 断言第 0 个样本会误判成「没播」。
    let mid = HOP / 2;
    st.slot.send(MediaCmd::SoundpadPlay(0));
    let mut f1 = [0.0f32; HOP];
    st.process(&mut f1, &FrameContext::default()).unwrap();
    assert_eq!(f1[0], 0.0, "起播必须从 0 淡入（首样本无咔哒）");
    assert!(f1[mid] < 0.0, "第一 hop 中点应为负斜坡");
    // 不重播：第二 hop 走正斜坡
    let mut f2 = [0.0f32; HOP];
    st.process(&mut f2, &FrameContext::default()).unwrap();
    assert!(f2[mid] > 0.0, "第二 hop 中点应为正斜坡");
    // 重播：旧声部淡出、新声部从头起播 → 又回到负斜坡
    st.slot.send(MediaCmd::SoundpadPlay(0));
    let mut f3 = [0.0f32; HOP];
    st.process(&mut f3, &FrameContext::default()).unwrap();
    assert!(f3[mid] < 0.0, "重播必须从头（负斜坡）");
    let _ = std::fs::remove_file(&p);
  }
}
