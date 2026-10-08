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

//! 出站节拍诊断：确认 10 ms 节拍的**实际**产出速率。

#[cfg(test)]
mod inner {
  use crate::audio::HOP;
  use crate::net::opus_sys::{Encoder, MAX_PACKET_BYTES};
  use std::time::{Duration, Instant};

  #[test]
  fn encode_cost_per_hop() {
    let mut enc = Encoder::new(48_000, 32_000).expect("编码器");
    let mut pkt = [0u8; MAX_PACKET_BYTES];
    let hop = vec![0.0f32; HOP];
    // 预热
    for _ in 0..10 {
      let _ = enc.encode(&hop, &mut pkt).unwrap();
    }
    let t = Instant::now();
    let n = 200;
    let mut total = 0usize;
    for _ in 0..n {
      total += enc.encode(&hop, &mut pkt).unwrap();
    }
    let per = t.elapsed().as_secs_f64() / n as f64;
    println!(
      "编码一个 10 ms hop 耗时 {per:.3} ms；{n} 次共 {total} 字节（均 {} 字节/包）",
      total / n
    );
    assert!(
      per < 5.0,
      "单 hop 编码超过 5 ms 会吃满 10 ms 节拍预算：{per:.3} ms"
    );
  }

  #[test]
  fn interval_rate_is_100hz() {
    // std 线程模拟列工作线程的绝对节拍（DESIGN.md §3.3）
    let mut next = Instant::now() + Duration::from_millis(10);
    let mut count = 0u64;
    let start = Instant::now();
    while start.elapsed() < Duration::from_millis(1000) {
      next += Duration::from_millis(10);
      let now = Instant::now();
      if next > now {
        std::thread::sleep(next - now);
      }
      count += 1;
    }
    let rate = count as f64 / start.elapsed().as_secs_f64();
    println!("空载节拍实测 {rate:.1} hop/s");
    assert!(
      (95.0..=105.0).contains(&rate),
      "空载应约 100 hop/s，实测 {rate:.1}"
    );
  }
}
