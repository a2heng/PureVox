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

//! 测试音信号源：1 kHz 正弦（-20 dBFS）+ 随包测试素材（敲桌子 / 鼠标 / 键盘）。
//!
//! 每个 10 ms 产出一个 48 kHz hop；素材按「一次 + `gap_ms` 静音」重复，多选相加。
//!
//! 节拍来自 [`Tick`]（Windows 上是高精度可等待定时器）。**别改回
//! `thread::sleep(10ms)`**：Windows 默认 15.6 ms 刻度会把一拍量化成两拍一出、一拍一回，
//! 接收端只有一个 20 ms 的环，突发立刻冲爆、随后的空档立刻饿死 —— 实测 1 kHz 听感
//! 就是「不连续」，`/debug` 上表现为 `tx_packets_per_s` 偏到 102/s、接收端 `late` 32/s。
//!
//! 与设备时钟**不同步**（这是刻意的）：正好检验输出侧的时钟伺服 —— 源是系统时钟，
//! 设备是自己的晶振，两者差多少看 `/debug/audio` 的 ppm。

use std::collections::BTreeMap;
use std::sync::atomic::{AtomicBool, Ordering::Relaxed};
use std::sync::{Arc, OnceLock};
use std::time::Duration;

use super::fanout::Fanout;
use super::tick::Tick;
use super::{HOP, SAMPLE_RATE, WorkerHandle};

pub const TONE_HZ: f64 = 1000.0;
pub const TONE_AMPLITUDE: f32 = 0.1; // -20 dBFS
/// 「正弦波」在 `clips` 参数里的 id（和素材 id 一样是稳定字符串）。
pub const SINE_ID: &str = "sine";
/// 默认重复间隙（ms）：一次瞬态之后留多少静音。
pub const DEFAULT_GAP_MS: u64 = 500;

/// 随包的测试素材：**单次最小瞬态**（48 kHz 单声道 16-bit WAV，见 `assets/test-audio/`）。
///
/// 只放「一次」：重复节奏由节点的 `gap_ms` 在运行期控制，不在文件里烘焙，这样同一份
/// 素材能按任意间隙循环。素材来自真实录音（用户提供），不是合成信号。
pub struct TestClip {
  pub id: &'static str,
  pub label: &'static str,
  pub wav: &'static [u8],
}

pub const CLIPS: &[TestClip] = &[
  TestClip {
    id: "knock",
    label: "敲桌子",
    wav: include_bytes!("../../assets/test-audio/knock.wav"),
  },
  TestClip {
    id: "mouse",
    label: "鼠标",
    wav: include_bytes!("../../assets/test-audio/mouse.wav"),
  },
  TestClip {
    id: "keyboard",
    label: "键盘",
    wav: include_bytes!("../../assets/test-audio/keyboard.wav"),
  },
];

static SHARED: OnceLock<Arc<Fanout>> = OnceLock::new();

/// 全局共享的测试音源（线程只启动一次）；计划里的「测试音」输入行订阅它。
pub fn shared() -> Arc<Fanout> {
  SHARED.get_or_init(spawn).clone()
}

/// 解析节点的 `clips` 参数。**参数缺失** = 只有正弦波（旧计划的行为不变）；
/// 显式空数组 = 什么都不播（界面把勾选全部取消就是这个状态）。
pub fn selected(params: &BTreeMap<String, String>) -> Vec<String> {
  let Some(raw) = params.get("clips") else {
    return vec![SINE_ID.to_string()];
  };
  let mut ids: Vec<String> = serde_json::from_str(raw).unwrap_or_default();
  ids.retain(|id| id == SINE_ID || CLIPS.iter().any(|c| c.id == id));
  ids
}

/// 重复间隙（ms）：50~5000，缺省 [`DEFAULT_GAP_MS`]。
pub fn gap_ms(params: &BTreeMap<String, String>) -> u64 {
  params
    .get("gap_ms")
    .and_then(|v| v.trim().parse::<f64>().ok())
    .map(|v| v.clamp(50.0, 5000.0) as u64)
    .unwrap_or(DEFAULT_GAP_MS)
}

/// 节点卡片上的短描述：选了哪些素材。
pub fn label_for(ids: &[String]) -> String {
  let mut names: Vec<&str> = Vec::new();
  for id in ids {
    if id == SINE_ID {
      names.push("1 kHz 正弦");
    } else if let Some(c) = CLIPS.iter().find(|c| c.id == id) {
      names.push(c.label);
    }
  }
  if names.is_empty() {
    "静音".to_string()
  } else {
    format!("测试音（{}）", names.join(" + "))
  }
}

/// 起一个测试素材播放线程（含可选的正弦波），返回 `(句柄, 扇出)`。
///
/// 每个素材一条播放位置：播完一次后静音 `gap_ms`，再从头开始；多个素材**相加**
/// （选自同一节点时一起响）。素材必须是 48 kHz 单声道（随包素材由自检生成，固定如此）。
pub fn spawn_clips(
  ids: &[String],
  gap_ms: u64,
  tag: &str,
) -> Result<(WorkerHandle, Arc<Fanout>), String> {
  let want_sine = ids.iter().any(|id| id == SINE_ID);
  let mut players = Vec::new();
  for id in ids {
    if id == SINE_ID {
      continue;
    }
    let clip = CLIPS
      .iter()
      .find(|c| c.id == id)
      .ok_or_else(|| format!("未知测试素材：{id}"))?;
    let (samples, rate) = crate::wav::read_mono_bytes(clip.wav)
      .map_err(|e| format!("测试素材「{}」解码失败：{e}", clip.label))?;
    if rate != SAMPLE_RATE {
      return Err(format!(
        "测试素材「{}」采样率是 {rate} Hz，要求 {SAMPLE_RATE} Hz",
        clip.label
      ));
    }
    if samples.is_empty() {
      return Err(format!("测试素材「{}」没有样本", clip.label));
    }
    players.push(Player {
      samples,
      pos: 0,
      rest: 0,
    });
  }
  if players.is_empty() && !want_sine {
    return Err("没有选中的测试素材".into());
  }

  let gap = gap_ms as usize * SAMPLE_RATE as usize / 1000;
  let fanout = Arc::new(Fanout::new(format!("测试素材 {tag}")));
  let out = fanout.clone();
  let stop = Arc::new(AtomicBool::new(false));
  let stop2 = stop.clone();
  let join = std::thread::Builder::new()
    .name(format!("test-clips-{tag}"))
    .spawn(move || {
      let _rt = super::rt_guard::enter_audio_thread();
      let step = 2.0 * std::f64::consts::PI * TONE_HZ / SAMPLE_RATE as f64;
      let mut phase = 0.0f64;
      let mut hop = [0.0f32; HOP];
      let mut tick = Tick::new(Duration::from_millis(10));
      while !stop2.load(Relaxed) {
        tick.wait();
        hop.fill(0.0);
        if want_sine {
          for x in hop.iter_mut() {
            *x += TONE_AMPLITUDE * phase.sin() as f32;
            phase = (phase + step) % (2.0 * std::f64::consts::PI);
          }
        }
        for p in players.iter_mut() {
          for x in hop.iter_mut() {
            if p.rest > 0 {
              p.rest -= 1;
              continue;
            }
            if p.pos < p.samples.len() {
              *x += p.samples[p.pos];
              p.pos += 1;
            } else {
              // 一次播完：从头开始前先静音 `gap` 个样本
              p.pos = 0;
              p.rest = gap;
            }
          }
        }
        out.push_hop(&hop);
      }
    })
    .map_err(|e| format!("创建测试素材线程失败：{e}"))?;
  Ok((WorkerHandle::new(stop, join), fanout))
}

struct Player {
  samples: Vec<f32>,
  pos: usize,
  /// 还剩多少个样本的静音（播完一次后的间隙）
  rest: usize,
}

/// 启动常驻测试音线程，返回其扇出。
fn spawn() -> Arc<Fanout> {
  let fanout = Arc::new(Fanout::new(format!("测试音 {TONE_HZ:.0} Hz -20 dBFS")));
  let out = fanout.clone();
  std::thread::Builder::new()
    .name("tone".into())
    .spawn(move || {
      let _rt = super::rt_guard::enter_audio_thread();
      let step = 2.0 * std::f64::consts::PI * TONE_HZ / SAMPLE_RATE as f64;
      let mut phase = 0.0f64;
      let mut hop = vec![0.0f32; HOP];
      // 一拍 = 一个 hop = 10 ms。生产一拍、等下一拍：**不补发、不追时间**。
      let mut tick = Tick::new(Duration::from_millis(10));
      // 精度必须看得见：退化到 15.6 ms 刻度时听感就是「吞吞吐吐」，别让人只能靠猜。
      eprintln!(
        "PureVox: 测试音节拍 10 ms（{}）",
        if tick.high_res() {
          "高精度可等待定时器"
        } else {
          "普通 sleep，~15.6 ms 刻度，会突发"
        }
      );
      loop {
        tick.wait();
        for x in hop.iter_mut() {
          *x = TONE_AMPLITUDE * phase.sin() as f32;
          phase = (phase + step) % (2.0 * std::f64::consts::PI);
        }
        out.push_hop(&hop);
      }
    })
    .expect("spawn tone");
  fanout
}

#[cfg(test)]
mod tests {
  use super::*;

  fn params(clips: Option<&str>, gap: Option<&str>) -> BTreeMap<String, String> {
    let mut m = BTreeMap::new();
    if let Some(c) = clips {
      m.insert("clips".into(), c.into());
    }
    if let Some(g) = gap {
      m.insert("gap_ms".into(), g.into());
    }
    m
  }

  /// 旧计划没有 `clips` 参数：行为必须仍是正弦波（不能突然变成静音）。
  #[test]
  fn missing_clips_means_sine() {
    assert_eq!(selected(&params(None, None)), vec![SINE_ID.to_string()]);
  }

  /// 显式空数组 = 什么都不播（界面取消全部勾选）；未知 id 被丢掉。
  #[test]
  fn empty_and_unknown_clips_are_safe() {
    assert!(selected(&params(Some("[]"), None)).is_empty());
    assert_eq!(
      selected(&params(Some("[\"nope\"]"), None)),
      Vec::<String>::new()
    );
    let got = selected(&params(Some("[\"knock\",\"keyboard\"]"), None));
    assert_eq!(got, vec!["knock".to_string(), "keyboard".to_string()]);
  }

  #[test]
  fn gap_is_clamped_and_defaulted() {
    assert_eq!(gap_ms(&params(None, None)), DEFAULT_GAP_MS);
    assert_eq!(gap_ms(&params(None, Some("400"))), 400);
    assert_eq!(gap_ms(&params(None, Some("1"))), 50, "下限 50 ms");
    assert_eq!(gap_ms(&params(None, Some("999999"))), 5000, "上限 5000 ms");
    assert_eq!(gap_ms(&params(None, Some("junk"))), DEFAULT_GAP_MS);
  }

  #[test]
  fn label_lists_selected_clips() {
    let ids = vec!["knock".to_string(), "mouse".to_string()];
    assert_eq!(label_for(&ids), "测试音（敲桌子 + 鼠标）");
    assert_eq!(label_for(&[]), "静音");
  }
}
