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

//! 测试音信号源：1 kHz 正弦、-20 dBFS，每 10 ms 产出一个 48 kHz hop。
//!
//! 节拍来自 [`Tick`]（Windows 上是高精度可等待定时器）。**别改回
//! `thread::sleep(10ms)`**：Windows 默认 15.6 ms 刻度会把一拍量化成两拍一出、一拍一回，
//! 接收端只有一个 20 ms 的环，突发立刻冲爆、随后的空档立刻饿死 —— 实测 1 kHz 听感
//! 就是「不连续」，`/debug` 上表现为 `tx_packets_per_s` 偏到 102/s、接收端 `late` 32/s。
//!
//! 与设备时钟**不同步**（这是刻意的）：正好检验输出侧的时钟伺服 —— 源是系统时钟，
//! 设备是自己的晶振，两者差多少看 `/debug/audio` 的 ppm。

use std::sync::{Arc, OnceLock};
use std::time::Duration;

use super::fanout::Fanout;
use super::tick::Tick;
use super::{HOP, SAMPLE_RATE};

pub const TONE_HZ: f64 = 1000.0;
pub const TONE_AMPLITUDE: f32 = 0.1; // -20 dBFS

static SHARED: OnceLock<Arc<Fanout>> = OnceLock::new();

/// 全局共享的测试音源（线程只启动一次）；计划里的「测试音」输入行订阅它。
pub fn shared() -> Arc<Fanout> {
  SHARED.get_or_init(spawn).clone()
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
