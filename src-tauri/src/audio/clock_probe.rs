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

//! **实测**时钟：设备采样时钟 vs 系统时钟，以及系统定时器的实际精度。
//!
//! 为什么要这块：整条链上「生产」和「消费」是不同晶振驱动的两套时钟，差多少**必须量**，
//! 不能凭历史或猜。量出来才有资格决定「要不要伺服」「缓冲要多深」：
//!
//! - 差 < 30 ppm：20 ms 的缓冲 11 分钟才漂满一格，一次 10 ms 的修正可以忽略不计；
//! - 差 200 ppm（廉价晶振 / HAL 重采样）：100 s 就漂满，必须闭环。
//!
//! 这些测试**要真设备**，默认不编译（`#[cfg(feature = "hw-tests")]`），手动跑：
//!
//! ```text
//! cargo test --features hw-tests clock_probe -- --nocapture --test-threads=1
//! ```
//!
//! 只写静音、不动系统音量、不碰用户设置（DEBUG.md §1.3「不许动系统的音量 / 静音」）。

#[cfg(test)]
#[cfg_attr(not(feature = "hw-tests"), allow(unused_imports, dead_code))]
mod inner {
  use cpal::traits::{DeviceTrait, HostTrait, StreamTrait};
  // cpal 0.18 的 `name()` 在 `DeviceTrait` 上（下游还有别的 `name`，导入要显式）
  use std::sync::Arc;
  use std::sync::atomic::{AtomicU64, Ordering};
  use std::time::{Duration, Instant};

  /// 测量时长：越长越准（20 s 已经能把 10 ppm 量出来：20 s × 48 kHz × 10 ppm ≈ 10 样本）。
  const MEASURE: Duration = Duration::from_secs(20);

  /// 采集 / 播放实测速率（frames/s），换算成 ppm。
  fn ppm(frames: u64, rate: u32, elapsed: Duration) -> f64 {
    let want = rate as f64 * elapsed.as_secs_f64();
    (frames as f64 - want) / want * 1e6
  }

  /// **输出设备**的采样时钟 vs 系统时钟。
  ///
  /// 这就是 `audio/playback.rs` 里伺服要吸收的那个差，也是「网络扬声器」这条链上
  /// **电脑这一侧**的时钟差（另一侧是手机，只能在手机上量，见 `st` 的 `clock_ppm`）。
  #[test]
  #[cfg(feature = "hw-tests")]
  fn output_device_clock_ppm() {
    let host = cpal::default_host();
    let Some(dev) = host.default_output_device() else {
      println!("[clock] 没有默认输出设备，跳过");
      return;
    };
    // 名字取不到也无所谓：这里只报数字（cpal 0.18 的设备名 API 与旧版不同）
    let name =
      crate::devices::display_name("", false).unwrap_or_else(|_: String| "默认设备".into());
    let supported = dev.default_output_config().expect("默认输出配置");
    let cfg = supported.config();
    let rate = cfg.sample_rate;
    let ch = cfg.channels as usize;
    let frames = Arc::new(AtomicU64::new(0));

    let f = frames.clone();
    let stream = dev
      .build_output_stream(
        cfg,
        move |data: &mut [f32], _| {
          // 只写静音：不碰系统音量、不制造声音
          data.fill(0.0);
          f.fetch_add((data.len() / ch) as u64, Ordering::Relaxed);
        },
        |e| eprintln!("[clock] 输出流错误：{e}"),
        None,
      )
      .expect("打开默认输出设备");
    stream.play().expect("启动输出流");

    // 先让设备把预填的缓冲跑过去，再开始计时
    std::thread::sleep(Duration::from_millis(500));
    let start_frames = frames.load(Ordering::Relaxed);
    let t0 = Instant::now();
    std::thread::sleep(MEASURE);
    let elapsed = t0.elapsed();
    let n = frames.load(Ordering::Relaxed) - start_frames;
    drop(stream);

    let measured_rate = n as f64 / elapsed.as_secs_f64();
    println!(
      "[clock] 输出「{name}」：{} Hz / {} ch｜{n} 帧 / {:.3} s = {measured_rate:.2} Hz｜**{:+.1} ppm**",
      rate,
      ch,
      elapsed.as_secs_f64(),
      ppm(n, rate, elapsed)
    );
    // 不设硬断言：这块只报事实。真要判定，看打印出来的 ppm 量级。
  }

  /// **输入设备**的采样时钟 vs 系统时钟（同一颗晶振的两个方向常常不一样）。
  #[test]
  #[cfg(feature = "hw-tests")]
  fn input_device_clock_ppm() {
    let host = cpal::default_host();
    let Some(dev) = host.default_input_device() else {
      println!("[clock] 没有默认输入设备，跳过");
      return;
    };
    // 名字取不到也无所谓：这里只报数字（cpal 0.18 的设备名 API 与旧版不同）
    let name =
      crate::devices::display_name("", false).unwrap_or_else(|_: String| "默认设备".into());
    let supported = dev.default_input_config().expect("默认输入配置");
    let cfg = supported.config();
    let rate = cfg.sample_rate;
    let frames = Arc::new(AtomicU64::new(0));

    let f = frames.clone();
    let stream = dev
      .build_input_stream(
        cfg,
        move |data: &[f32], _| {
          f.fetch_add(
            (data.len() / cfg.channels as usize) as u64,
            Ordering::Relaxed,
          );
        },
        |e| eprintln!("[clock] 输入流错误：{e}"),
        None,
      )
      .expect("打开默认输入设备");
    stream.play().expect("启动输入流");

    std::thread::sleep(Duration::from_millis(500));
    let start_frames = frames.load(Ordering::Relaxed);
    let t0 = Instant::now();
    std::thread::sleep(MEASURE);
    let elapsed = t0.elapsed();
    let n = frames.load(Ordering::Relaxed) - start_frames;
    drop(stream);

    let measured_rate = n as f64 / elapsed.as_secs_f64();
    println!(
      "[clock] 输入「{name}」：{} Hz｜{n} 帧 / {:.3} s = {measured_rate:.2} Hz｜**{:+.1} ppm**",
      rate,
      elapsed.as_secs_f64(),
      ppm(n, rate, elapsed)
    );
  }

  /// 系统定时器的实际精度（Windows 默认 15.6 ms，`timeBeginPeriod(1)` 后 1 ms）。
  ///
  /// **现状**：网络出站已经**不再用定时器**（引擎每出一 hop 就发一包），所以这条只用来
  /// 说明「为什么当初 sleep 循环只能产 64 包/s」这类历史问题，不再影响现役链路。
  #[test]

  fn sleep_accuracy() {
    for (label, period) in [("1 ms", 1u64), ("10 ms", 10)] {
      let n = 200u32;
      let mut next = Instant::now() + Duration::from_millis(period);
      let t0 = Instant::now();
      for _ in 0..n {
        while Instant::now() < next {
          std::thread::sleep(Duration::from_micros(200));
        }
        next += Duration::from_millis(period);
      }
      let elapsed = t0.elapsed();
      println!(
        "[clock] 定时器 {label}：{n} 拍用了 {:.1} ms（应 {} ms）→ 误差 {:+.1}%",
        elapsed.as_secs_f64() * 1000.0,
        n as u64 * period,
        (elapsed.as_secs_f64() * 1000.0 / (n as f64 * period as f64) - 1.0) * 100.0
      );
    }
  }
}
