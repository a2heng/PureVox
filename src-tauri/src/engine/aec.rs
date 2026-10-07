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

use std::sync::atomic::{AtomicBool, Ordering::Relaxed};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use rtrb::Consumer;

use crate::audio::fanout::Fanout;
use crate::audio::{HOP, WorkerHandle};
use crate::infer::aec::Aec;

/// far 历史网格容量（2 s @48 kHz，DESIGN.md §8）。
pub const FAR_HIST_SAMPLES: usize = 48_000 * 2;

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
}

#[derive(Clone, Copy, PartialEq, Eq)]
pub enum FarWin {
  Exact,
  Latest,
  None,
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
  if !far.is_empty() && crate::devices::find(far, false).is_ok() {
    return Some(far.to_string());
  }
  None
}

/// 启动 far 采集的消费线程：把 far 的 hop 灌进历史网格。返回句柄（随会话停止）。
pub fn spawn_far_pump(fan: &Arc<Fanout>, hist: Arc<Mutex<FarHistory>>, tag: &str) -> WorkerHandle {
  let mut cons = fan.subscribe(&format!("{tag}-far"));
  let stop = Arc::new(AtomicBool::new(false));
  let stop2 = stop.clone();
  let join = std::thread::Builder::new()
    .name(format!("aec-far-{tag}"))
    .spawn(move || {
      let mut hop = [0.0f32; HOP];
      while !stop2.load(Relaxed) {
        // far 只作参考：积压就丢最旧，避免越拖越迟
        if cons.slots() > 8 * HOP
          && let Ok(c) = cons.read_chunk(cons.slots() - 2 * HOP)
        {
          c.commit_all();
        }
        if cons.slots() >= HOP {
          if let Ok(c) = cons.read_chunk(HOP) {
            let (a, b) = c.as_slices();
            hop[..a.len()].copy_from_slice(a);
            hop[a.len()..].copy_from_slice(b);
            c.commit_all();
            if let Ok(mut h) = hist.lock() {
              h.push(&hop);
            }
          }
        } else {
          std::thread::sleep(Duration::from_millis(2));
        }
      }
    })
    .expect("spawn aec far pump");
  WorkerHandle::new(stop, join)
}
