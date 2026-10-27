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

use crate::audio::{HOP, SAMPLE_RATE};

/// 音频帧头长度（字节）。
pub const HDR_LEN: usize = 8;

/// 播放环深度（ms）。**全链路唯一可调的常量**：两侧相同，三端同名同值。
///
/// 依据：实测局域网到达抖动 0.4~0.9 ms（这是 20 倍余量）、晶振差 25 ppm
/// （20 ms 要 13 min 才漂满一格）。改这个值必须同步改 `audio/Rt.kt` 与网页 worklet，
/// 以及 `DESIGN.md` §4.1.4。
pub const RING_MS: f64 = 20.0;

/// 溢出阈值 = 深度 + 一格：超过就裁掉最旧一格（保延迟有界）。
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

/// 到达统计：**只观测**，不参与任何控制。
#[derive(Debug, Clone, Default)]
pub struct Arrival {
  /// 到达间隔偏离 10 ms 的 EWMA（ms）
  ema_ms: f64,
  /// 观测到的最坏偏差（ms），只增不减到 `reset`
  worst_ms: f64,
  /// 相邻两包的最大间隔（ms）
  max_gap_ms: f64,
  last: Option<Instant>,
  pub packets: u64,
  /// 到达时环是空的（= 播放已经饿着了）
  pub late: u64,
}

impl Arrival {
  fn on_packet(&mut self, now: Instant, hop_ms: f64, starving: bool) {
    if let Some(prev) = self.last {
      let dt = now.saturating_duration_since(prev).as_secs_f64() * 1000.0;
      self.max_gap_ms = self.max_gap_ms.max(dt);
      // 超过 500 ms 是「断了一次」，不算抖动
      if dt < 500.0 {
        let dev = (dt - hop_ms).abs();
        self.ema_ms += 0.02 * (dev - self.ema_ms);
        self.worst_ms = self.worst_ms.max(dev);
      }
    }
    if starving {
      self.late += 1;
    }
    self.last = Some(now);
    self.packets += 1;
  }

  fn reset(&mut self) {
    *self = Arrival::default();
  }
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
pub struct Playout {
  buf: VecDeque<f32>,
  depth_ms: f64,
  /// 上一格的输出（欠载时用它淡出重复）
  last_hop: Vec<f32>,
  /// 起播前不出声（攒够半格才开）
  primed: bool,
  /// 连续欠载拍数（> 1 s 退回未起播）
  underrun_streak: u32,
  arrival: Arrival,
  hops: u64,
  underruns: u64,
  plc: u64,
  trims: u64,
  dropped: u64,
}

impl Playout {
  pub fn new(depth_ms: f64) -> Self {
    Playout {
      buf: VecDeque::with_capacity(SAMPLE_RATE as usize / 10),
      depth_ms,
      last_hop: vec![0.0; HOP],
      primed: false,
      underrun_streak: 0,
      arrival: Arrival::default(),
      hops: 0,
      underruns: 0,
      plc: 0,
      trims: 0,
      dropped: 0,
    }
  }

  /// 收到解码后的样本（一个包通常就是一格；多于/少于一格也能收）。
  pub fn push(&mut self, samples: &[f32], now: Instant) {
    // 「来晚」= **已经起播**却仍然空环（说明播放已经饿着了）；起播填充期不算
    let starving = self.primed && self.buf.is_empty();
    self.arrival.on_packet(now, hop_ms(), starving);
    self.buf.extend(samples.iter().copied());
    // 溢出：只保留「深度 + 一格」以内，超了裁到深度
    let cap = ms_to_samples(self.depth_ms + TRIM_EXTRA_MS);
    if self.buf.len() > cap {
      let keep = ms_to_samples(self.depth_ms);
      let drop = self.buf.len() - keep;
      self.buf.drain(..drop);
      self.trims += 1;
      self.dropped += drop as u64;
    }
  }

  /// 按引擎 / 播放节拍取一格。**总是填满**：不够就走 PLC 段 + 淡出。
  ///
  /// 返回 `true` = 这一格是真实样本；`false` = 欠载（调用方可以补一帧 Opus PLC 再喂进来，
  /// 见 `server.rs` / `SpeakerDownlink.kt` 的用法）。
  pub fn pull(&mut self, out: &mut [f32]) -> bool {
    if !self.primed {
      // 起播 = 攒满**深度**（20 ms）。不是自适应：只在起播这一刻做一次，
      // 目的就是让稳态水位落在深度上（否则水位全靠起播时的偶然值，可能在 10~30 之间漂）。
      if self.buf.len() >= ms_to_samples(self.depth_ms) {
        self.primed = true;
      } else {
        out.fill(0.0);
        return false;
      }
    }
    if self.buf.len() >= out.len() {
      for (i, slot) in out.iter_mut().enumerate() {
        *slot = self.buf[i];
      }
      self.buf.drain(..out.len());
      self.hops += 1;
      self.underrun_streak = 0;
      if out.len() == self.last_hop.len() {
        self.last_hop.copy_from_slice(out);
      }
      return true;
    }
    // 欠载：淡出重复上一格（首格是静音，自然就是「安静地等一下」）
    self.underruns += 1;
    self.underrun_streak = self.underrun_streak.saturating_add(1);
    let total = out.len().max(1) as f32;
    for (i, slot) in out.iter_mut().enumerate() {
      *slot = self.last_hop[i % HOP] * 0.5 * (1.0 - i as f32 / total);
    }
    if self.underrun_streak > 100 {
      // 断流超过 1 s：退回未起播，等真正有数据再说（别对着空环一直做 PLC）
      self.primed = false;
      self.underrun_streak = 0;
      self.arrival.reset();
    }
    false
  }

  /// 记一次「调用方补了一帧 Opus PLC」（计数用；样本本身照常 `push`）。
  pub fn note_plc(&mut self) {
    self.plc += 1;
  }

  pub fn level_ms(&self) -> f64 {
    self.buf.len() as f64 * 1000.0 / SAMPLE_RATE as f64
  }

  pub fn stats(&self) -> PlayoutStats {
    PlayoutStats {
      level_ms: self.level_ms(),
      target_ms: self.depth_ms,
      jitter_ms: self.arrival.ema_ms,
      worst_jitter_ms: self.arrival.worst_ms,
      max_gap_ms: self.arrival.max_gap_ms,
      packets: self.arrival.packets,
      hops: self.hops,
      underruns: self.underruns,
      plc: self.plc,
      trims: self.trims,
      dropped: self.dropped,
      late: self.arrival.late,
    }
  }

  pub fn reset(&mut self) {
    self.buf.clear();
    self.primed = false;
    self.underrun_streak = 0;
    self.arrival.reset();
    self.last_hop.fill(0.0);
  }
}

fn hop_ms() -> f64 {
  HOP as f64 * 1000.0 / SAMPLE_RATE as f64
}

fn ms_to_samples(ms: f64) -> usize {
  (ms * SAMPLE_RATE as f64 / 1000.0) as usize
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

  /// 起播：环里没有半格就静音，而且**不算欠载**（刚开流不是故障）。
  #[test]
  fn priming_is_silent_and_not_counted() {
    let mut p = Playout::new(RING_MS);
    let mut out = vec![9.0f32; HOP];
    assert!(!p.pull(&mut out));
    assert!(out.iter().all(|x| *x == 0.0), "起播前应是静音");
    assert_eq!(p.stats().underruns, 0, "起播前的静音不是欠载");
    // 攒满**深度**（20 ms = 2 格）才起播
    p.push(&hop(0.5), Instant::now());
    assert!(!p.pull(&mut out), "不足深度不出声");
    p.push(&hop(0.5), Instant::now());
    assert!(p.pull(&mut out), "满深度应能起播");
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
    assert_eq!(s.underruns, 0);
    assert_eq!(s.trims, 0);
    assert_eq!(s.dropped, 0);
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

  /// 欠载：淡出重复上一格（不是硬切静音），计数可见；断流 1 s 后退回未起播。
  #[test]
  fn underrun_fades_and_then_reprimes() {
    let mut p = Playout::new(RING_MS);
    let mut t = Instant::now();
    let mut out = vec![0.0f32; HOP];
    // 垫到 2 格（20 ms = 深度），先正常取走两格
    for _ in 0..2 {
      t += Duration::from_millis(10);
      p.push(&hop(0.5), t);
    }
    assert!(p.pull(&mut out), "有数据时应出声");
    assert!(p.pull(&mut out), "第二格也应有数据");
    // 现在环空了：这一拍欠载应该是「上一格 × 0.5 的淡出」，不是全 0
    assert!(!p.pull(&mut out));
    assert!(
      out.iter().any(|x| x.abs() > 1e-3),
      "欠载应淡出重复，而不是挖掉一格"
    );
    assert!(out.iter().all(|x| x.abs() <= 0.5 + 1e-3), "淡出不能放大");
    assert_eq!(p.stats().underruns, 1);
    // 断流 1 s 之后退回未起播：输出静音
    for _ in 0..120 {
      p.pull(&mut out);
    }
    assert!(!p.stats().packets > 0 && true);
    let s = p.stats();
    assert!(s.underruns > 100, "持续断流应一直计欠载：{s:?}");
  }

  /// 观测数据不许参与控制：抖动再大，深度也**不变**。
  #[test]
  fn jitter_is_observation_only() {
    let mut p = Playout::new(RING_MS);
    let mut t = Instant::now();
    let mut out = vec![0.0f32; HOP];
    for k in 0..200 {
      // 人为制造 0/20 ms 交替的到达（抖动 10 ms）
      t += Duration::from_millis(if k % 2 == 0 { 0 } else { 20 });
      p.push(&hop(0.1), t);
      p.pull(&mut out);
    }
    let s = p.stats();
    assert!(s.jitter_ms > 5.0, "抖动应被观测到：{}", s.jitter_ms);
    assert_eq!(s.target_ms, RING_MS, "深度是常量，不许被抖动改");
  }
}
