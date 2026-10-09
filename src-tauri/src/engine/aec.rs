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

//! AEC 行（DESIGN.md §4）：一个输入行 =「一路 mic + 一路 far」。
//!
//! far 参考只喂模型、不进信号路径：far 采集的 hop 由 [`spawn_far_pump`] 灌进 2 s 采样网格，
//! 列工作线程按「mic 采样序号 − far_delay」从网格取一段作为 far_hop。理想窗口未就绪时，
//! 有历史就先用最近一段（模型多抽头自对齐），完全没有则直通 mic（不丢人声、不动缓存）。

use std::sync::atomic::{AtomicBool, AtomicU64, Ordering::Relaxed};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use rtrb::Consumer;

use crate::audio::fanout::Fanout;
use crate::audio::{HOP, SAMPLE_RATE, WorkerHandle};
use crate::infer::aec::Aec;

/// far 历史网格容量（2 s @48 kHz，DESIGN.md §8）。
pub const FAR_HIST_SAMPLES: usize = 48_000 * 2;

/// Linux far 网格的**前置余量**（样本）：网格序号 = mic 序号 + 余量，保证列线程取「精确窗口」
/// 永远取得到（否则窗口会落在网格末尾之外 → 全变「回退」）。常量偏移由 far_delay 吸收。
const FAR_GRID_LEAD: usize = 48_000 * 3 / 10; // 300 ms

/// 48 kHz 采样网格（环形）：按绝对采样序号读写，支持取任意历史窗口。
pub struct FarHistory {
  buf: Vec<f32>,
  cap: usize,
  total: u64,
}

impl FarHistory {
  pub fn new(cap: usize) -> Self {
    FarHistory {
      buf: vec![0.0; cap],
      cap,
      total: 0,
    }
  }

  pub fn push(&mut self, hop: &[f32]) {
    let mut off = (self.total % self.cap as u64) as usize;
    for &s in hop {
      self.buf[off] = s;
      off += 1;
      if off == self.cap {
        off = 0;
      }
      self.total += 1;
    }
  }

  /// 取 [start, start+out.len())；越界（太旧 / 还没到）返回 false。
  pub fn window(&self, start: i64, out: &mut [f32]) -> bool {
    if start < 0 {
      return false;
    }
    let start = start as u64;
    let end = start + out.len() as u64;
    if end > self.total || start + self.cap as u64 <= self.total {
      return false;
    }
    for (i, o) in out.iter_mut().enumerate() {
      *o = self.buf[((start + i as u64) % self.cap as u64) as usize];
    }
    true
  }

  /// 优先精确窗口，否则退到最近一段；都没有则 None。
  pub fn window_or_latest(&self, start: i64, out: &mut [f32]) -> FarWin {
    if self.window(start, out) {
      return FarWin::Exact;
    }
    let n = out.len() as u64;
    if self.total >= n && self.window((self.total - n) as i64, out) {
      return FarWin::Latest;
    }
    FarWin::None
  }

  /// 当前网格序号（= 已写入样本数），诊断用。
  pub fn total(&self) -> u64 {
    self.total
  }
}

#[derive(Clone, Copy, PartialEq, Eq)]
pub enum FarWin {
  Exact,
  Latest,
  None,
}

/// 回声诊断（滚动 1 s 窗）：
/// - `rho`：远端窗口对近端的**解释度**（归一化互相关）——判断「参考/对齐对不对」；
///   对齐好且回声主导时 ρ 应接近 1；ρ 很低说明麦克风里主要是本底噪声（回声太弱）。
/// - `supp_db`：**回声抑制** = 10log10(Σ近端² / Σ模型输出²)——判断「模型有没有在消」。
///   两者一起看才能分清「参考没对齐」还是「模型没出力」。
#[derive(Default)]
pub struct EchoMetrics {
  mic_pow: f64,
  far_pow: f64,
  cross: f64,
  out_pow: f64,
  hops: u32,
  pub rho: f64,
  pub supp_db: f64,
}

impl EchoMetrics {
  /// 喂一个 hop 的（近端、远端窗口、模型输出）——都用**进模型前**的增益后信号。
  pub fn push(&mut self, mic: &[f32], far: &[f32], out: &[f32]) {
    for i in 0..mic.len().min(far.len()) {
      let (m, f) = (mic[i] as f64, far[i] as f64);
      self.mic_pow += m * m;
      self.far_pow += f * f;
      self.cross += m * f;
    }
    for &o in out {
      self.out_pow += (o as f64) * (o as f64);
    }
    self.hops += 1;
    if self.hops >= 100 {
      // 100 hop = 1 s：算好一窗再清零，概要里显示的是「最近 1 秒」
      self.rho = self.cross / (self.mic_pow.sqrt() * self.far_pow.sqrt()).max(1e-12);
      self.supp_db = 10.0 * (self.mic_pow / self.out_pow.max(1e-12)).log10();
      self.mic_pow = 0.0;
      self.far_pow = 0.0;
      self.cross = 0.0;
      self.out_pow = 0.0;
      self.hops = 0;
    }
  }
}

/// 一行的 AEC 运行状态（每行一份模型缓存 + far 网格 + 对齐计数）。
pub struct AecRow {
  pub engine: Aec,
  pub hist: Arc<Mutex<FarHistory>>,
  /// far_delay（采样）；far 窗口起点 = mic 采样序号 − delay
  pub delay_samples: i64,
  pub mic_hops: u64,
  /// 本行 mic hop 读端（mic 采集扇出的订阅）
  pub mic: Consumer<f32>,
  /// 进模型前的近端 / 远端增益（线性）
  pub mic_gain: f32,
  pub far_gain: f32,
  /// 直通：跳过 AEC，直接过 mic（A/B 对比用）
  pub bypass: bool,
  /// mic 序号时钟（采样）：列线程每 hop 更新，far 泵用它当网格时钟（Linux）
  pub mic_clock: Arc<AtomicU64>,
  /// 回声诊断（滚动 1 s 窗）：远端对近端的解释度与模型抑制量
  pub echo: EchoMetrics,
  pub exact: u64,
  pub latest: u64,
  pub pass: u64,
}

/// 远端参考是否为「输出回环」：返回要回环的输出端点 ID（空串 = 系统默认输出）。
/// - `loopback` → 默认输出；`loopback:<id>` → 指定输出；
/// - 直接是输出设备 ID（扬声器）→ 回环该输出；
/// - 其它（输入设备）→ None（按普通输入采集）。
pub fn far_loopback_id(far: &str) -> Option<String> {
  if far == "loopback" {
    return Some(String::new());
  }
  if let Some(id) = far.strip_prefix("loopback:") {
    return Some(id.to_string());
  }
  if crate::devices::loopback_accepts_device_id()
    && !far.is_empty()
    && crate::devices::find(far, false).is_ok()
  {
    return Some(far.to_string());
  }
  None
}

/// 启动 far 采集的消费线程：把 far 灌进历史网格。返回句柄（随会话停止）。
///
/// 网格按**实时**推进：每轮按已流逝时间写入应到的样本数，设备没出数据（WASAPI 回环在渲染
/// 端点空闲时几乎不回调）就补零。这样 far 采样序号与 mic 采样序号始终同步，两者之间的常量
/// 偏移由 `far_delay` 吸收；否则 far 序号会随会话时长越落越后，取窗口永远失败 → 一直回退
/// 最近段 → 延时校准量到绝对偏移、越测越大。
pub fn spawn_far_pump(
  fan: &Arc<Fanout>,
  hist: Arc<Mutex<FarHistory>>,
  tag: &str,
  _epoch: Arc<AtomicU64>,
  mic_clock: Arc<AtomicU64>,
) -> WorkerHandle {
  // 平台推进策略（`loopback_<平台>.rs`）：
  // - Windows：用**系统时钟**推进（WASAPI 回环空闲不回调，必须按实时补零）；
  // - Linux：用 **mic 的序号**当网格时钟——两边是独立时钟，用系统时钟会让 mic 序号
  //   越跑越前（实测 20 s 后窗口全取不到 → 校准超时）；以 mic 为钟则永远跟得上。
  let realtime = crate::audio::loopback::FAR_GRID_REALTIME;
  let mut cons = fan.subscribe(&format!("{tag}-far"));
  let stop = Arc::new(AtomicBool::new(false));
  let stop2 = stop.clone();
  let join = std::thread::Builder::new()
    .name(format!("aec-far-{tag}"))
    .spawn(move || {
      let t0 = Instant::now();
      let mut written: u64 = 0;
      let mut buf = [0.0f32; HOP];
      while !stop2.load(Relaxed) {
        // 网格时钟：Windows 用系统时钟；Linux 用 mic 序号 + 前置余量
        // （余量保证「精确窗口」永远取得到；常量偏移由 far_delay 吸收）。
        // 两边都**单调推进、不重置**：序号一旦重置，far 泵与列线程的重置顺序会打架
        // （泵按旧的大序号写一大段 → 窗口全取不到 → 校准超时）。
        let want = if realtime {
          (t0.elapsed().as_secs_f64() * SAMPLE_RATE as f64) as u64
        } else {
          mic_clock.load(Relaxed) + FAR_GRID_LEAD as u64
        };
        let mut need = want.saturating_sub(written);
        if need == 0 {
          std::thread::sleep(Duration::from_millis(1));
          continue;
        }
        // far 只作参考：积压就丢最旧，避免越拖越迟
        if cons.slots() > 8 * HOP
          && let Ok(c) = cons.read_chunk(cons.slots() - 2 * HOP)
        {
          c.commit_all();
        }
        while need > 0 {
          let take = need.min(HOP as u64) as usize;
          let avail = cons.slots();
          let got = if avail == 0 {
            0
          } else {
            let g = take.min(avail);
            match cons.read_chunk(g) {
              Ok(c) => {
                let (a, b) = c.as_slices();
                let n = a.len() + b.len();
                buf[..a.len()].copy_from_slice(a);
                buf[a.len()..n].copy_from_slice(b);
                c.commit_all();
                n
              }
              Err(_) => 0,
            }
          };
          // 设备没出数据 → 补零，保持网格序号与实时一致
          for x in buf[got..take].iter_mut() {
            *x = 0.0;
          }
          if let Ok(mut h) = hist.lock() {
            h.push(&buf[..take]);
          }
          written += take as u64;
          need -= take as u64;
        }
      }
    })
    .expect("spawn aec far pump");
  WorkerHandle::new(stop, join)
}
