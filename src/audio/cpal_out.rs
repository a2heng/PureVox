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

//! 非 Windows 输出后端：cpal 默认输出流（Windows 走 `wasapi_out`，见 `audio/out.rs`）。
//!
//! 回调周期由平台决定，`probe` 只能给一个估计值（一个 10 ms hop），
//! 设备环目标水位因此按 `DEVICE_RING_MS` 兜底。

use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering::Relaxed};

use cpal::traits::{DeviceTrait, StreamTrait};
use cpal::{FromSample, SampleFormat, SizedSample, StreamConfig};
use rtrb::Consumer;

use super::capture::CallbackStats;
use super::out::{Format, Sample};

/// 已启动的输出流；析构即停。
pub struct Stream {
  _stream: cpal::Stream,
}

pub fn probe(device_id: &str) -> Result<Format, String> {
  let (dev, _) = crate::devices::find(device_id, false)?;
  let supported = dev
    .default_output_config()
    .map_err(|e| format!("读取默认输出格式失败：{e}"))?;
  let cfg = supported.config();
  let sample = match supported.sample_format() {
    SampleFormat::F32 => Sample::F32,
    SampleFormat::I16 => Sample::I16,
    SampleFormat::I32 => Sample::I32,
    SampleFormat::U16 => Sample::U16,
    other => return Err(format!("不支持的采样格式 {other}")),
  };
  let bits = match sample {
    Sample::F32 | Sample::I32 => 32,
    Sample::I16 | Sample::U16 => 16,
  };
  Ok(Format {
    rate: cfg.sample_rate,
    channels: cfg.channels,
    bits,
    sample,
    period_frames: (cfg.sample_rate / 100).max(1),
    low_latency: false,
  })
}

pub fn start(
  device_id: String,
  format: Format,
  cons: Consumer<f32>,
  stats: Arc<CallbackStats>,
  primed: Arc<AtomicBool>,
) -> Result<Stream, String> {
  let (dev, _) = crate::devices::find(&device_id, false)?;
  let cfg = StreamConfig {
    channels: format.channels,
    sample_rate: format.rate,
    buffer_size: cpal::BufferSize::Default,
  };
  let stream = match format.sample {
    Sample::F32 => build_stream::<f32>(&dev, &cfg, cons, stats, primed),
    Sample::I16 => build_stream::<i16>(&dev, &cfg, cons, stats, primed),
    Sample::I32 => build_stream::<i32>(&dev, &cfg, cons, stats, primed),
    Sample::U16 => build_stream::<u16>(&dev, &cfg, cons, stats, primed),
  }?;
  stream.play().map_err(|e| format!("启动输出流失败：{e}"))?;
  Ok(Stream { _stream: stream })
}

fn build_stream<T>(
  dev: &cpal::Device,
  cfg: &StreamConfig,
  mut cons: Consumer<f32>,
  stats: Arc<CallbackStats>,
  primed: Arc<AtomicBool>,
) -> Result<cpal::Stream, String>
where
  T: SizedSample + FromSample<f32>,
{
  let ch = cfg.channels.max(1) as usize;
  let err_stats = stats.clone();
  dev
    .build_output_stream::<T, _, _>(
      *cfg,
      move |data: &mut [T], _info| {
        stats.on_block((data.len() / ch) as u32);
        let mut silent = 0u64;
        // 环里是**交织帧**：每帧 ch 个样本，逐通道落位（多通道输出真正分开）。
        for f in data.chunks_exact_mut(ch) {
          let ok = cons.slots() >= ch;
          if !ok {
            silent += 1;
          }
          for x in f.iter_mut() {
            let s = if ok { cons.pop().unwrap_or(0.0) } else { 0.0 };
            *x = <T as FromSample<f32>>::from_sample_(s);
          }
        }
        if silent > 0 && primed.load(Relaxed) {
          stats.xruns.fetch_add(silent, Relaxed);
        }
      },
      move |e| err_stats.on_error(e.to_string()),
      None,
    )
    .map_err(|e| format!("打开输出流失败：{e}"))
}
