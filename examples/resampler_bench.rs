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

//! 采样热路径基准（真实实现原样引入，不碰设备与推理）。
//!
//! 用法：cargo run --release --example resampler_bench [每档秒数]
//!
//! 两张表：
//! 1. `Converter`（单通道重采样，10 ms 原生块，比例按伺服实际微调 ±ppm）；
//! 2. 采集一拍（5 ms）：交织拆分 [`split_active`] + 每个在用通道 `ToHops`，
//!    即 `audio/capture.rs` 工作线程里最烫的那两段。

// `resampler.rs` 只依赖 `crate::audio` 的两个常量；example 里给同款定义即可原样复用。
mod audio {
  pub const SAMPLE_RATE: u32 = 48_000;
  pub const HOP: usize = 480;
}

#[path = "../src/dsp/resampler.rs"]
#[allow(dead_code)]
mod resampler;
#[path = "../src/dsp/split.rs"]
#[allow(dead_code)]
mod split;

use std::hint::black_box;
use std::time::Instant;

use resampler::{Converter, ToHops};
use split::split_active;

/// 单通道重采样：原生 10 ms 块 → 48 kHz。
fn bench_convert(in_rate: u32, rel: f64, secs: f64) -> (f64, String) {
  let chunk = (in_rate as usize / 100).max(1);
  let mut conv = Converter::new(in_rate, 48_000, chunk, true).expect("converter");
  conv.set_relative_ratio(rel).expect("ratio");
  let desc = conv.description().to_string();
  let input: Vec<f32> = (0..chunk)
    .map(|i| ((i as f64 * 0.05).sin() * 0.4) as f32)
    .collect();

  let warm = Instant::now();
  while warm.elapsed().as_secs_f64() < 0.15 {
    black_box(conv.process(black_box(&input)).unwrap());
  }
  let mut blocks = 0u64;
  let t0 = Instant::now();
  while t0.elapsed().as_secs_f64() < secs {
    black_box(conv.process(black_box(&input)).unwrap());
    blocks += 1;
  }
  (t0.elapsed().as_secs_f64() * 1e9 / blocks as f64, desc)
}

/// 采集一拍（5 ms）：`channels` 通道里只有 `active` 路有订阅者。
fn bench_capture(channels: usize, active_n: usize, native_rate: u32, secs: f64) -> f64 {
  let frames = (native_rate as usize / 200).max(1); // 5 ms 轮询
  let inter: Vec<f32> = (0..frames * channels)
    .map(|i| ((i as f64 * 0.01).sin() * 0.3) as f32)
    .collect();
  let active: Vec<bool> = (0..channels).map(|i| i < active_n).collect();
  let mut scratch: Vec<Vec<f32>> = (0..channels)
    .map(|_| Vec::with_capacity(frames * 2))
    .collect();
  let mut hops: Vec<ToHops> = (0..active_n)
    .map(|_| ToHops::new(native_rate).expect("to_hops"))
    .collect();

  let mut run = |blocks: &mut u64| {
    for s in scratch.iter_mut() {
      s.clear();
    }
    split_active(
      black_box([&inter[..], &[]]),
      channels,
      &active,
      &mut scratch,
    );
    for (i, t) in hops.iter_mut().enumerate() {
      t.push(&scratch[i], |hop| {
        black_box(hop);
      })
      .unwrap();
    }
    *blocks += 1;
  };
  let mut warm_blocks = 0u64;
  let warm = Instant::now();
  while warm.elapsed().as_secs_f64() < 0.15 {
    run(&mut warm_blocks);
  }
  let mut blocks = 0u64;
  let t0 = Instant::now();
  while t0.elapsed().as_secs_f64() < secs {
    run(&mut blocks);
  }
  t0.elapsed().as_secs_f64() * 1e9 / blocks as f64
}

fn main() {
  let secs: f64 = std::env::args()
    .nth(1)
    .and_then(|s| s.parse().ok())
    .unwrap_or(1.2);
  // 伺服实际把比例压到标称附近 ±几百 ppm；取 +400 ppm 当代表值。
  println!("单通道重采样（10 ms 原生块 → 48 kHz）");
  println!("{:<20} {:>12} {:>10}  实现", "场景", "ns/块", "单核占用");
  for (rate, rel, name) in [
    (48_000u32, 1.0004, "48k → 48k"),
    (96_000, 1.0004, "96k → 48k"),
    (192_000, 1.0004, "192k → 48k"),
    (44_100, 1.0004, "44.1k → 48k"),
  ] {
    let (ns, desc) = bench_convert(rate, rel, secs);
    println!("{name:<20} {ns:>12.0} {:>9.2}%  {desc}", ns / 1e7 * 100.0);
  }

  println!("\n采集一拍（5 ms：交织拆分 + 在用通道 ToHops + 切 hop）");
  println!("{:<20} {:>12} {:>10}", "场景", "ns/拍", "单核占用");
  for (ch, act) in [(1usize, 1usize), (2, 2), (10, 2)] {
    for rate in [48_000u32, 192_000] {
      let ns = bench_capture(ch, act, rate, secs);
      println!(
        "{:<20} {ns:>12.0} {:>9.3}%",
        format!("{ch}ch/{act}路 {rate}/48k"),
        ns / 5e6 * 100.0
      );
    }
  }
}
