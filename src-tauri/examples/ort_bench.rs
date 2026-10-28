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

//! 独立基准：加载降噪模型并连续推理，打印每 hop 耗时（不经过音频链路）。
//! 运行：`cargo run --release --example ort_bench [模型路径]`
//!
//! 输出两行：**只算 ONNX** 与 **ONNX + 图外 FFT/OLA**（`infer/spec.rs` 原样复用，
//! 不抄第二份）。两者之差就是「傅里叶出图」换来的应用侧开销。

#[path = "../src/infer/spec.rs"]
mod spec;

use std::path::PathBuf;
use std::time::Instant;

use ort::session::Session;
use ort::value::{Tensor, ValueType};

fn main() -> Result<(), Box<dyn std::error::Error>> {
  let path = std::env::args()
    .nth(1)
    .map(PathBuf::from)
    .unwrap_or_else(|| {
      PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../models/purevox_denoise.onnx")
    });
  println!("model: {}", path.display());
  let t = Instant::now();
  // 与 `infer::build_session` 完全一致：单线程推理 + 关闭自旋。
  // 默认多线程池会忙等，实测把 1.8 ms 的推理拖到 10 ms 以上（bench 自己踩过这个坑）。
  let env = ort::environment::init()
    .with_name("PureVox-bench")
    .build()?;
  let mut session = Session::builder(&env)?
    .with_intra_threads(1)?
    .with_inter_threads(1)?
    .with_intra_op_spinning(false)?
    .commit_from_file(&path)?;
  println!(
    "session ready in {:.0} ms",
    t.elapsed().as_secs_f64() * 1000.0
  );
  let dim = session
    .inputs()
    .iter()
    .find(|i| i.name() == "cache_in")
    .and_then(|i| match i.dtype() {
      ValueType::Tensor { shape, .. } => Some(shape.num_elements()),
      _ => None,
    })
    .ok_or("no cache_in")?;
  println!("cache dim = {dim}");

  // 输入名按谱接口写死（旧波形契约的模型会在下面的 session.run 里直接报错）
  let (spec_in, spec_out) = if session.inputs().iter().any(|i| i.name() == "spec_in") {
    ("spec_in", "spec_out")
  } else {
    return Err("模型不是谱接口（没有 spec_in）——本 bench 只支持 2026-10 起的模型".into());
  };

  let mut cache = vec![0.0f32; dim];
  let mut stft = spec::SpecStream::new();
  let hop = [0.0f32; spec::HOP];
  let (mut model_ms, mut full_ms) = (Vec::new(), Vec::new());
  for i in 0..300 {
    // ① 只算模型
    let t = Instant::now();
    {
      let sp = Tensor::from_array(([1i64, 2, spec::BINS as i64], vec![0.0f32; spec::SPEC_LEN]))?;
      let cin = Tensor::from_array(([1i64, dim as i64], std::mem::take(&mut cache)))?;
      let outs = session.run(ort::inputs![spec_in => sp, "cache_in" => cin])?;
      let (_, cout) = outs["cache_out"].try_extract_tensor::<f32>()?;
      cache.clear();
      cache.extend_from_slice(cout);
    }
    let ms = t.elapsed().as_secs_f64() * 1000.0;
    if i >= 20 {
      model_ms.push(ms);
    }

    // ② 模型 + 图外分析/合成（应用侧真实开销）
    let t = Instant::now();
    let spec = stft.analyze(&hop).to_vec();
    let out_spec = {
      let sp = Tensor::from_array(([1i64, 2, spec::BINS as i64], spec))?;
      let cin = Tensor::from_array(([1i64, dim as i64], std::mem::take(&mut cache)))?;
      let outs = session.run(ort::inputs![spec_in => sp, "cache_in" => cin])?;
      let (_, out_spec) = outs[spec_out].try_extract_tensor::<f32>()?;
      let out_spec = out_spec.to_vec();
      let (_, cout) = outs["cache_out"].try_extract_tensor::<f32>()?;
      cache.clear();
      cache.extend_from_slice(cout);
      out_spec
    };
    let _ = stft.synthesize(&out_spec);
    let ms = t.elapsed().as_secs_f64() * 1000.0;
    if i >= 20 {
      full_ms.push(ms);
    }
  }
  let stat = |v: &mut Vec<f64>| {
    v.sort_by(|a, b| a.partial_cmp(b).unwrap());
    (v[v.len() / 2], v[(v.len() as f64 * 0.95) as usize - 1])
  };
  let (m_med, m_p95) = stat(&mut model_ms);
  let (f_med, f_p95) = stat(&mut full_ms);
  println!(
    "  只算 ONNX      : {m_med:.3} ms/hop (p95 {m_p95:.3})  RTF {:.3}",
    m_med / 10.0
  );
  println!(
    "  ONNX + 图外 FFT: {f_med:.3} ms/hop (p95 {f_p95:.3})  RTF {:.3}   (FFT {:.3})",
    f_med / 10.0,
    f_med - m_med
  );
  Ok(())
}
