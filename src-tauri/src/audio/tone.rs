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

//! 测试音信号源：1 kHz 正弦、-20 dBFS，按系统时钟每 10 ms 产出一个 48 kHz hop。
//! 与设备时钟不同步——正好检验输出侧时钟伺服。

use std::sync::Arc;
use std::time::{Duration, Instant};

use super::fanout::Fanout;
use super::{HOP, SAMPLE_RATE};

pub const TONE_HZ: f64 = 1000.0;
pub const TONE_AMPLITUDE: f32 = 0.1; // -20 dBFS

/// 启动常驻测试音线程，返回其扇出。
pub fn spawn() -> Arc<Fanout> {
  let fanout = Arc::new(Fanout::new(format!("测试音 {TONE_HZ:.0} Hz -20 dBFS")));
  let out = fanout.clone();
  std::thread::Builder::new()
    .name("tone".into())
    .spawn(move || {
      let step = 2.0 * std::f64::consts::PI * TONE_HZ / SAMPLE_RATE as f64;
      let period = Duration::from_millis(10);
      let mut phase = 0.0f64;
      let mut hop = vec![0.0f32; HOP];
      let mut next = Instant::now();
      loop {
        for x in hop.iter_mut() {
          *x = TONE_AMPLITUDE * phase.sin() as f32;
          phase = (phase + step) % (2.0 * std::f64::consts::PI);
        }
        out.push_hop(&hop);
        next += period;
        let now = Instant::now();
        if next > now {
          std::thread::sleep(next - now);
        } else if now - next > Duration::from_millis(200) {
          next = now; // 长时间被挂起后不补发积压
        }
      }
    })
    .expect("spawn tone");
  fanout
}
