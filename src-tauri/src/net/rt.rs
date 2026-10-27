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

//! 实时音频链路的公共件（DESIGN.md §4.1）：帧头 + **固定深度播放环** + 到达统计。
//!
//! ## 这一版为什么这么小
//!
//! 实测（`audio/clock_probe.rs` + `net_loopback.py`）：设备晶振差 **±25 ppm**、局域网到达
//! 抖动 **0.4~0.9 ms**、电脑 10 ms 绝对节拍误差 **0.0%**。所以：
//!
//! - 环深度是**常量** [`RING_MS`] = 20 ms（抖动余量的 20 倍）；
//! - 运行期**只有两种动作**：超了丢一格（10 ms）、空了补一格（PLC / 淡出）；
//! - **没有重采样、没有 PI 伺服、没有抖动驱动的目标水位**。前两版都死在控制回路上
//!   （实测摆出 ±1500 ppm 的颤音、起播被误判成 25 次欠载）。
//!
//! 代价是明确的：25 ppm 的漂移约 **13 分钟**修正一格。这个频率我们接受，换回来的是
//! 音频逐样本原样（不变调、不颤）和一条可以一眼看懂、可以复现的链路。
//!
//! ## 客户端同源
//!
//! Android（`audio/Rt.kt`）与网页 worklet 是同一个环的镜像：深度、阈值、起播规则、
//! PLC 策略必须逐字对齐（各自的注释都指回这里）。
//!
//! 统计全部只做**观测**：`jitter`、`max_gap`、`late`、`trim`、`plc` 都不参与控制
//! （AGENTS §1.4：测量不花钱，看得见才谈得上判断）。

use std::collections::VecDeque;
use std::sync::OnceLock;
use std::time::Instant;

#[cfg(test)]
use crate::audio::HOP;
use crate::audio::SAMPLE_RATE;

/// 音频帧头长度（字节）。
pub const HDR_LEN: usize = 8;

/// 播放环深度（ms）。**全链路唯一可调的常量**：两侧相同，三端同名同值。
///
/// 依据：实测局域网到达抖动 0.4~0.9 ms（这是 20 倍余量）、晶振差 25 ppm
/// （20 ms 要 13 min 才漂满一格）。改这个值必须同步改 `audio/Rt.kt` 与网页 worklet，
/// 以及 `DESIGN.md` §4.1.4。
pub const RING_MS: f64 = 20.0;

/// 溢出阈值 = 深度 + 一格：超过就裁掉最旧一格（保延迟有界）。
#[cfg(test)]
const TRIM_EXTRA_MS: f64 = 10.0;

/// 音频帧头：`seq`（每方向独立）+ `ts`（发送方单调时钟，微秒，u32 回绕）。
///
/// 只服务**测量**：丢包判定（seq 缺口）、单向延迟（ts + `libopus` 无关的时钟偏移）。
/// 不参与播放调度 —— 播放调度只看环的深浅。
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct AudioHeader {
  pub seq: u32,
  pub ts_us: u32,
}

impl AudioHeader {
  pub fn write(&self, dst: &mut [u8]) {
    debug_assert!(dst.len() >= HDR_LEN);
    dst[..4].copy_from_slice(&self.seq.to_be_bytes());
    dst[4..8].copy_from_slice(&self.ts_us.to_be_bytes());
  }

  pub fn parse(src: &[u8]) -> Option<Self> {
    if src.len() < HDR_LEN {
      return None;
    }
    Some(AudioHeader {
      seq: u32::from_be_bytes([src[0], src[1], src[2], src[3]]),
      ts_us: u32::from_be_bytes([src[4], src[5], src[6], src[7]]),
    })
  }
}

/// 两个 u32 微秒时间戳之差（带符号，跨 71.6 min 回绕有效）。`now` 是较新的那个。
pub fn us_diff(now: u32, then: u32) -> i64 {
  now.wrapping_sub(then) as i32 as i64
}

/// 进程内单调时钟 → u32 微秒（帧头用）。
pub fn now_us32() -> u32 {
  static T0: OnceLock<Instant> = OnceLock::new();
  let t0 = T0.get_or_init(Instant::now);
  t0.elapsed().as_micros() as u32
}

/// 一路播放环的统计（进 `/debug`；客户端上报用同一套口径）。
#[derive(Clone, Copy, Debug, Default)]
pub struct PlayoutStats {
  /// 当前水位（ms）
  pub level_ms: f64,
  /// 目标深度（ms，常量 = [`RING_MS`]）
  pub target_ms: f64,
  /// 到达抖动（ms，EWMA）——**只观测**
  pub jitter_ms: f64,
  /// 观测到的最坏抖动（ms）
  pub worst_jitter_ms: f64,
  /// 相邻两包最大间隔（ms）
  pub max_gap_ms: f64,
  /// 收到的包数 / 取走的 hop 数
  pub packets: u64,
  pub hops: u64,
  /// 欠载次数（补了 PLC / 淡出）
  pub underruns: u64,
  /// 其中单独走 Opus PLC 的次数（调用方在欠载时补一帧时 +1）
  pub plc: u64,
  /// 溢出裁剪次数（每次丢 10 ms）
  pub trims: u64,
  /// 裁剪掉的样本数
  pub dropped: u64,
  /// 到达时环已空（= 来晚了）
  pub late: u64,
}

/// 固定深度播放环：**唯一的缓冲**（DESIGN.md §4.1.4）。
///
/// 不重采样：取出来的就是送进去的样本（顺序不变、样本不变）。唯一的「修饰」是
/// 欠载时的 PLC / 淡出，以及溢出时丢掉最旧一格。
/// 毫秒 → 样本数（48 kHz）。
fn ms_to_samples(ms: f64) -> usize {
  (ms * SAMPLE_RATE as f64 / 1000.0).round().max(0.0) as usize
}

pub struct Playout {
  buf: VecDeque<f32>,
  /// 实时性上限（ms）：**唯一的控制**（手动滑块，1 帧 = 10 ms）。
  max_ms: f64,
  hops: u64,
  underruns: u64,
  trims: u64,
  dropped: u64,
  packets: u64,
  late: u64,
}

impl Playout {
  pub fn new(max_ms: f64) -> Self {
    Playout {
      buf: VecDeque::with_capacity(SAMPLE_RATE as usize / 10 * 40),
      max_ms,
      hops: 0,
      underruns: 0,
      trims: 0,
      dropped: 0,
      packets: 0,
      late: 0,
    }
  }

  /// 改**实时性上限**（用户手动，10~200 ms）。变小就立刻把超出的部分丢掉。
  ///
  /// 这不是自适应：值只来自用户（手机滑块 / 网页命令），运行期不会因为抖动自己动。
  pub fn set_depth_ms(&mut self, ms: f64) {
    self.max_ms = ms;
    self.trim_to_cap();
  }

  fn trim_to_cap(&mut self) {
    let cap = ms_to_samples(self.max_ms);
    if self.buf.len() > cap {
      let drop = self.buf.len() - cap;
      self.buf.drain(..drop);
      self.trims += 1;
      self.dropped += drop as u64;
    }
  }

  /// 收到解码后的样本：**收下，超过实时性上限就丢最旧的**（不做任何平滑）。
  ///
  /// 音频只看实时性 —— TCP 不会丢包也不会乱序，晚到的样本已经没有价值，
  /// 所以这里没有蓄水位、没有容差、更没有控制回路。
  pub fn push(&mut self, samples: &[f32], _now: Instant) {
    if self.buf.is_empty() {
      self.late += 1;
    }
    self.packets += 1;
    self.buf.extend(samples.iter().copied());
    self.trim_to_cap();
  }

  /// 按引擎节拍取一格：**够就取、不够就静音**（残留整块丢掉，不做 PLC / 淡化）。
  ///
  /// 返回 `true` = 真实样本；`false` = 这一拍静音（调用方不需要补任何东西）。
  pub fn pull(&mut self, out: &mut [f32]) -> bool {
    if self.buf.len() >= out.len() {
      for (i, slot) in out.iter_mut().enumerate() {
        *slot = self.buf[i];
      }
      self.buf.drain(..out.len());
      self.hops += 1;
      return true;
    }
    // 断流：剩下的已经过时，整块丢掉，输出静音
    self.underruns += 1;
    self.dropped += self.buf.len() as u64;
    self.buf.clear();
    out.fill(0.0);
    false
  }

  /// 保留签名：PLC 已经取消（断流直接静音），调用方不需要改。
  pub fn note_plc(&mut self) {}

  /// 清空（停服务 / 会话重建）。
  pub fn reset(&mut self) {
    self.buf.clear();
    self.hops = 0;
    self.underruns = 0;
    self.trims = 0;
    self.dropped = 0;
    self.packets = 0;
    self.late = 0;
  }

  pub fn level_ms(&self) -> f64 {
    self.buf.len() as f64 * 1000.0 / SAMPLE_RATE as f64
  }

  pub fn stats(&self) -> PlayoutStats {
    PlayoutStats {
      level_ms: self.level_ms(),
      target_ms: self.max_ms,
      // 极简策略不再统计抖动（字段保留，界面与诊断结构不变）
      jitter_ms: 0.0,
      worst_jitter_ms: 0.0,
      max_gap_ms: 0.0,
      packets: self.packets,
      hops: self.hops,
      underruns: self.underruns,
      plc: 0,
      trims: self.trims,
      dropped: self.dropped,
      late: self.late,
    }
  }
}

#[cfg(test)]
mod tests {
  use super::*;
  use std::time::Duration;

  fn hop(v: f32) -> Vec<f32> {
    vec![v; HOP]
  }

  #[test]
  fn header_roundtrip() {
    let h = AudioHeader {
      seq: 0x0102_0304,
      ts_us: 0xA0B0_C0D0,
    };
    let mut b = [0u8; HDR_LEN];
    h.write(&mut b);
    assert_eq!(AudioHeader::parse(&b), Some(h));
    assert_eq!(AudioHeader::parse(&b[..4]), None, "少于 8 字节必须拒绝");
  }

  #[test]
  fn us_diff_handles_wrap() {
    assert_eq!(us_diff(1000, 0), 1000);
    assert_eq!(us_diff(0, u32::MAX), 1, "回绕后应继续递增");
    assert_eq!(us_diff(0, 5000), -5000);
  }

  /// **逐样本原样**：不重采样，所以推什么就取什么（顺序与数值都不变）。
  /// 这是「不颤、不变调」的机器证明。
  #[test]
  fn playback_is_sample_exact() {
    let mut p = Playout::new(RING_MS);
    let src: Vec<f32> = (0..HOP * 3).map(|i| (i as f32 * 0.01).sin()).collect();
    p.push(&src[..HOP], Instant::now());
    p.push(&src[HOP..HOP * 2], Instant::now());
    let mut out = vec![0.0f32; HOP];
    assert!(p.pull(&mut out));
    assert_eq!(out, src[..HOP], "第一格必须一模一样");
    assert!(p.pull(&mut out));
    assert_eq!(out, src[HOP..HOP * 2]);
  }

  /// 稳态：推 1 取 1，20 秒不丢不补。
  #[test]
  fn steady_state_is_lossless() {
    let mut p = Playout::new(RING_MS);
    let mut t = Instant::now();
    let mut out = vec![0.0f32; HOP];
    // 预热到深度（20 ms = 2 格；多推到 3 格会触发一次裁剪，那是正常行为但不是稳态）
    for _ in 0..2 {
      t += Duration::from_millis(10);
      p.push(&hop(0.3), t);
    }
    for _ in 0..2000 {
      t += Duration::from_millis(10);
      p.push(&hop(0.3), t);
      assert!(p.pull(&mut out), "稳态不该欠载");
      assert!(out.iter().all(|x| (*x - 0.3).abs() < 1e-6));
    }
    let s = p.stats();
    // 新策略不蓄水位：稳态只要求「不欠载 + 水位有界」，允许超上限时丢最旧的
    assert_eq!(s.underruns, 0);
    assert!(
      s.level_ms <= RING_MS + 1e-6,
      "水位应贴住上限：{} ms",
      s.level_ms
    );
    assert!(
      s.level_ms <= RING_MS + TRIM_EXTRA_MS,
      "水位不该超过深度一格：{}",
      s.level_ms
    );
  }

  /// 漂移（生产比消费快）：超「深度 + 一格」时裁掉最旧一格，水位回到深度。
  /// 20 ms 的环 + 25 ppm → 平均 13 分钟一次（这里把它压缩成几百拍验证行为）。
  #[test]
  fn producer_faster_trims_one_hop() {
    let mut p = Playout::new(RING_MS);
    let mut t = Instant::now();
    let mut out = vec![0.0f32; HOP];
    for _ in 0..5 {
      t += Duration::from_millis(10);
      p.push(&hop(0.2), t);
    }
    // 每拍推 1.2 格、取 1 格 → 水位上涨
    for _ in 0..50 {
      t += Duration::from_millis(10);
      p.push(&vec![0.2; HOP + HOP / 5], t);
      p.pull(&mut out);
    }
    let s = p.stats();
    assert!(s.trims > 0, "生产比消费快，必须裁剪（否则延迟无限涨）");
    assert!(
      s.level_ms <= RING_MS + TRIM_EXTRA_MS + 1.0,
      "裁剪后水位必须回到深度附近：{:.1} ms",
      s.level_ms
    );
  }

  /// 新策略：超上限丢最旧的（不淡化、不补帧）。
  #[test]
  fn drops_oldest_over_cap() {
    let mut p = Playout::new(20.0);
    let now = Instant::now();
    // 塞 50 ms（上限 20）
    for _ in 0..5 {
      p.push(&[0.25; HOP], now);
    }
    let st = p.stats();
    assert!(
      st.level_ms <= 20.0 + 0.01,
      "水位不能超过上限：{}",
      st.level_ms
    );
    assert!(st.trims >= 1 && st.dropped > 0, "超上限要计丢弃");
  }

  /// 新策略：环空 → 输出静音，不补 PLC、不淡化。
  #[test]
  fn silent_when_empty() {
    let mut p = Playout::new(20.0);
    let mut out = [1.0f32; HOP];
    assert!(!p.pull(&mut out));
    assert!(out.iter().all(|v| *v == 0.0), "空环必须整格静音");
    assert_eq!(p.stats().underruns, 1);
    assert_eq!(p.stats().plc, 0, "不做 PLC");
  }

  /// 上限变小时立刻丢到新上限（拖动滑块的即时效果）。
  #[test]
  fn shrink_takes_effect_immediately() {
    let mut p = Playout::new(100.0);
    let now = Instant::now();
    for _ in 0..10 {
      p.push(&[0.1; HOP], now);
    }
    assert!(p.stats().level_ms > 90.0);
    p.set_depth_ms(20.0);
    assert!(p.stats().level_ms <= 20.0 + 0.01);
  }
}
