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

//! 引擎唯一的重采样实现（rubato `Async` sinc，固定输入块，单声道）。
//!
//! - [`Converter`]：任意采样率 → 任意采样率，可在运行时微调比例（输出侧 ASRC 时钟伺服用）。
//! - [`ToHops`]：输入侧，原生采样率 → 48 kHz，并在 48k 输出侧切 10 ms hop。

use rubato::audioadapter_buffers::direct::InterleavedSlice;
use rubato::{
  Adjustable, Async, FixedAsync, Resampler, SincInterpolationParameters, SincInterpolationType,
  WindowFunction,
};

use crate::audio::{HOP, SAMPLE_RATE};

/// 比例可调范围：允许相对标称比例 ±10%（伺服本身另行限幅到 ±3%）。
const MAX_RELATIVE: f64 = 1.1;

pub struct Converter {
  inner: Option<Async<f32>>,
  chunk: usize,
  out_buf: Vec<f32>,
  description: String,
  delay_frames: usize,
}

impl Converter {
  /// `chunk`：每次 [`process`](Self::process) 的输入帧数。
  /// `adjustable`：需要运行时微调比例时为 true（此时即使采样率相同也走 rubato）。
  pub fn new(in_rate: u32, out_rate: u32, chunk: usize, adjustable: bool) -> Result<Self, String> {
    if in_rate == 0 || out_rate == 0 {
      return Err(format!("非法采样率 {in_rate} → {out_rate}"));
    }
    if in_rate == out_rate && !adjustable {
      return Ok(Converter {
        inner: None,
        chunk,
        out_buf: Vec::new(),
        description: format!("直通（{in_rate} Hz）"),
        delay_frames: 0,
      });
    }
    let ratio = out_rate as f64 / in_rate as f64;
    let params = SincInterpolationParameters::new(128, WindowFunction::Blackman2)
      .oversampling_factor(256)
      .interpolation(SincInterpolationType::Quadratic);
    let rs = Async::<f32>::new_sinc(ratio, MAX_RELATIVE, &params, chunk, 1, FixedAsync::Input)
      .map_err(|e| format!("创建重采样器失败：{e}"))?;
    let out_buf = vec![0.0; rs.output_frames_max()];
    let delay_frames = rs.output_delay();
    Ok(Converter {
      inner: Some(rs),
      chunk,
      out_buf,
      description: format!("rubato sinc {in_rate} → {out_rate} Hz（块 {chunk}）"),
      delay_frames,
    })
  }

  pub fn description(&self) -> &str {
    &self.description
  }

  pub fn is_passthrough(&self) -> bool {
    self.inner.is_none()
  }

  /// 重采样器引入的延迟（输出帧）。
  pub fn delay_frames(&self) -> usize {
    self.delay_frames
  }

  /// 单次 process 最多产出的帧数。
  pub fn output_frames_max(&self) -> usize {
    match &self.inner {
      None => self.chunk,
      Some(rs) => rs.output_frames_max(),
    }
  }

  /// 微调比例（相对标称比例）：>1 输出变多，<1 输出变少。
  pub fn set_relative_ratio(&mut self, rel: f64) -> Result<(), String> {
    match &mut self.inner {
      None => Err("直通转换器不能调比例".into()),
      Some(rs) => rs.set_resample_ratio_relative(rel, true).map_err(|e| format!("调比例失败：{e}")),
    }
  }

  /// 处理恰好 `chunk` 帧输入，返回本次输出。
  pub fn process<'a>(&'a mut self, input: &'a [f32]) -> Result<&'a [f32], String> {
    debug_assert_eq!(input.len(), self.chunk);
    match &mut self.inner {
      None => Ok(input),
      Some(rs) => {
        let inp = InterleavedSlice::new(input, 1, self.chunk).map_err(|e| e.to_string())?;
        let cap = self.out_buf.len();
        let mut out =
          InterleavedSlice::new_mut(&mut self.out_buf, 1, cap).map_err(|e| e.to_string())?;
        let (_, n_out) = rs
          .process_into_buffer(&inp, &mut out, None)
          .map_err(|e| format!("重采样失败：{e}"))?;
        Ok(&self.out_buf[..n_out])
      }
    }
  }
}

/// 输入侧：原生采样率 → 48 kHz，并切 10 ms hop。原生即 48 kHz 时直通。
pub struct ToHops {
  conv: Converter,
  in_buf: Vec<f32>,
  pending: Vec<f32>,
}

impl ToHops {
  pub fn new(native_rate: u32) -> Result<Self, String> {
    // 原生 10 ms 块；非 100 整除的采样率（如 22050）取整，输出侧 hop 网格不受影响
    let chunk = ((native_rate as usize) / 100).max(1);
    let conv = Converter::new(native_rate, SAMPLE_RATE, chunk, false)?;
    Ok(ToHops { conv, in_buf: Vec::with_capacity(chunk * 8), pending: Vec::with_capacity(HOP * 4) })
  }

  pub fn description(&self) -> &str {
    self.conv.description()
  }

  pub fn delay_frames(&self) -> usize {
    self.conv.delay_frames()
  }

  /// 已重采样但不足一个 hop 的剩余帧数（恒 < HOP）。
  pub fn pending_frames(&self) -> usize {
    self.pending.len()
  }

  /// 喂入原生采样率单声道样本；每凑满一个 48 kHz hop 回调一次。
  pub fn push(&mut self, input: &[f32], mut on_hop: impl FnMut(&[f32])) -> Result<(), String> {
    if self.conv.is_passthrough() {
      self.pending.extend_from_slice(input);
    } else {
      self.in_buf.extend_from_slice(input);
      let chunk = self.conv.chunk;
      let mut consumed = 0;
      while self.in_buf.len() - consumed >= chunk {
        let out = self.conv.process(&self.in_buf[consumed..consumed + chunk])?;
        self.pending.extend_from_slice(out);
        consumed += chunk;
      }
      self.in_buf.drain(..consumed);
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
