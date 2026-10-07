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

//! 引擎唯一的重采样实现：任意原生采样率单声道 → 48 kHz，并切成 10 ms hop。
//!
//! rubato `Async` sinc（固定输入块）：输入按原生 10 ms 块喂入，输出长度随比例浮动，
//! 累积后按 HOP 切片——hop 网格建立在 48 kHz 输出侧，与原生采样率无关。
//! 原生即 48 kHz 时直通，不经过 rubato。

use rubato::audioadapter_buffers::direct::InterleavedSlice;
use rubato::{
  Async, FixedAsync, Resampler, SincInterpolationParameters, SincInterpolationType, WindowFunction,
};

use super::{HOP, SAMPLE_RATE};

pub struct ToHops {
  inner: Option<Async<f32>>,
  in_chunk: usize,
  in_buf: Vec<f32>,
  out_buf: Vec<f32>,
  pending: Vec<f32>,
  description: String,
  delay_frames: usize,
}

impl ToHops {
  pub fn new(native_rate: u32) -> Result<Self, String> {
    if native_rate == 0 {
      return Err("原生采样率为 0".into());
    }
    if native_rate == SAMPLE_RATE {
      return Ok(ToHops {
        inner: None,
        in_chunk: 0,
        in_buf: Vec::new(),
        out_buf: Vec::new(),
        pending: Vec::with_capacity(HOP * 4),
        description: format!("直通（{native_rate} Hz）"),
        delay_frames: 0,
      });
    }
    // 原生 10 ms 块；非 100 整除的采样率（如 22050）取整，输出侧 hop 网格不受影响
    let in_chunk = ((native_rate as usize) / 100).max(1);
    let ratio = SAMPLE_RATE as f64 / native_rate as f64;
    let params = SincInterpolationParameters::new(128, WindowFunction::Blackman2)
      .oversampling_factor(256)
      .interpolation(SincInterpolationType::Quadratic);
    let rs = Async::<f32>::new_sinc(ratio, 1.1, &params, in_chunk, 1, FixedAsync::Input)
      .map_err(|e| format!("创建重采样器失败：{e}"))?;
    let out_max = rs.output_frames_max();
    let delay_frames = rs.output_delay();
    Ok(ToHops {
      inner: Some(rs),
      in_chunk,
      in_buf: Vec::with_capacity(in_chunk * 8),
      out_buf: vec![0.0; out_max],
      pending: Vec::with_capacity(HOP * 4),
      description: format!("rubato sinc {native_rate} → {SAMPLE_RATE} Hz（块 {in_chunk}）"),
      delay_frames,
    })
  }

  pub fn description(&self) -> &str {
    &self.description
  }

  /// 重采样器引入的延迟（48 kHz 输出帧）。
  pub fn delay_frames(&self) -> usize {
    self.delay_frames
  }

  /// 已重采样但不足一个 hop 的剩余帧数（恒 < HOP）。
  pub fn pending_frames(&self) -> usize {
    self.pending.len()
  }

  /// 喂入原生采样率单声道样本；每凑满一个 48 kHz hop 回调一次。
  pub fn push(&mut self, input: &[f32], mut on_hop: impl FnMut(&[f32])) -> Result<(), String> {
    match &mut self.inner {
      None => self.pending.extend_from_slice(input),
      Some(rs) => {
        self.in_buf.extend_from_slice(input);
        let mut consumed = 0;
        while self.in_buf.len() - consumed >= self.in_chunk {
          let chunk = &self.in_buf[consumed..consumed + self.in_chunk];
          let inp = InterleavedSlice::new(chunk, 1, self.in_chunk).map_err(|e| e.to_string())?;
          let cap = self.out_buf.len();
          let mut out =
            InterleavedSlice::new_mut(&mut self.out_buf, 1, cap).map_err(|e| e.to_string())?;
          let (n_in, n_out) = rs
            .process_into_buffer(&inp, &mut out, None)
            .map_err(|e| format!("重采样失败：{e}"))?;
          consumed += n_in;
          self.pending.extend_from_slice(&self.out_buf[..n_out]);
        }
        self.in_buf.drain(..consumed);
      }
    }
    let mut start = 0;
    while self.pending.len() - start >= HOP {
      on_hop(&self.pending[start..start + HOP]);
      start += HOP;
    }
    self.pending.drain(..start);
    Ok(())
  }
}
