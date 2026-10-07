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

//! AEC 合成回声自检：mic = far 延迟 D 样本 × 增益，看模型能把回声压多少 dB。
//! 用法：cargo run --release --example aec_bench [模型路径] [延迟ms] [增益]
//! 默认：../models/purevox_aec_202609_cpx_ep0375.onnx  5ms  0.5

use ort::session::Session;
use ort::value::{Tensor, ValueType};

const HOP: usize = 480;

fn main() -> Result<(), Box<dyn std::error::Error>> {
  let args: Vec<String> = std::env::args().collect();
  let path = args.get(1).cloned().unwrap_or_else(|| {
    "../models/purevox_aec_202609_cpx_ep0375.onnx".to_string()
  });
  let delay_ms: f64 = args.get(2).and_then(|s| s.parse().ok()).unwrap_or(5.0);
  let gain: f32 = args.get(3).and_then(|s| s.parse().ok()).unwrap_or(0.5);
  let d = (delay_ms * 48.0) as usize;

  let mut session = Session::builder()?
    .with_intra_threads(1)?
    .with_inter_threads(1)?
    .with_intra_op_spinning(false)?
    .commit_from_file(&path)?;
  let dim = session
    .inputs()
    .iter()
    .find(|i| i.name() == "cache_in")
    .and_then(|i| match i.dtype() {
      ValueType::Tensor { shape, .. } => Some(shape.num_elements()),
      _ => None,
    })
    .expect("no cache_in");
  println!("model {path}  cache_in={dim}  delay={delay_ms}ms  gain={gain}");

  let mut cache = vec![0.0f32; dim];
  let mut far_all: Vec<f32> = Vec::new();
  let mut seed = 12345u32;
  let mut rnd = || {
    seed = seed.wrapping_mul(1664525).wrapping_add(1013904223);
    ((seed >> 8) as f32 / (1 << 24) as f32) * 2.0 - 1.0
  };

  let hops = 300;
  let mut sum_mic = 0.0f64;
  let mut sum_out = 0.0f64;
  for k in 0..hops {
    let far_hop: Vec<f32> = (0..HOP).map(|_| rnd() * 0.3).collect();
    // mic = 当前 far 延迟 d 样本后的回声（含增益）
    let base = far_all.len();
    let mic_hop: Vec<f32> = (0..HOP)
      .map(|i| {
        let src = base as isize - d as isize + i as isize;
        if src >= 0 && (src as usize) < far_all.len() {
          far_all[src as usize] * gain
        } else {
          0.0
        }
      })
      .collect();
    far_all.extend_from_slice(&far_hop);

    let mic = Tensor::from_array(([1i64, HOP as i64], mic_hop.clone()))?;
    let far = Tensor::from_array(([1i64, HOP as i64], far_hop.clone()))?;
    let cache_in = std::mem::take(&mut cache);
    let cache_t = Tensor::from_array(([1i64, dim as i64], cache_in))?;
    let outs = session.run(ort::inputs!["mic_hop" => mic, "far_hop" => far, "cache_in" => cache_t])?;
    let (_, enh) = outs["enh_hop"].try_extract_tensor::<f32>()?;
    let (_, cout) = outs["cache_out"].try_extract_tensor::<f32>()?;
    cache.clear();
    cache.extend_from_slice(cout);

    if k > 60 {
      sum_mic += mic_hop.iter().map(|x| (*x as f64).powi(2)).sum::<f64>();
      sum_out += enh[..HOP].iter().map(|x| (*x as f64).powi(2)).sum::<f64>();
    }
  }
  let db = |e: f64, n: usize| 10.0 * (e / n as f64).max(1e-20).log10();
  let (dm, do_) = (db(sum_mic, hops - 60), db(sum_out, hops - 60));
  println!("mic {dm:.1} dB   out {do_:.1} dB   抑制 {:.1} dB", dm - do_);
  Ok(())
}
