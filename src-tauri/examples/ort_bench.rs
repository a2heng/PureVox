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

use std::path::PathBuf;
use std::time::Instant;

use ort::session::Session;
use ort::value::{Tensor, ValueType};

fn main() -> Result<(), Box<dyn std::error::Error>> {
  let path = std::env::args().nth(1).map(PathBuf::from).unwrap_or_else(|| {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../models/purevox_denoise_202609c_ep0012.onnx")
  });
  println!("model: {}", path.display());
  let t = Instant::now();
  let mut session = Session::builder()?.commit_from_file(&path)?;
  println!("session ready in {:.0} ms", t.elapsed().as_secs_f64() * 1000.0);
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

  // 与采集工作线程一致：在子线程里创建会话并推理
  let handle = std::thread::spawn(move || {
    println!("[thread] start");
    let mut cache = vec![0.0f32; dim];
    for i in 0..50 {
      let mix = Tensor::from_array(([1i64, 480], vec![0.0f32; 480])).unwrap();
      let t = Instant::now();
      let cin = Tensor::from_array(([1i64, dim as i64], std::mem::take(&mut cache))).unwrap();
      let outputs = session.run(ort::inputs!["mix_hop" => mix, "cache_in" => cin]).unwrap();
      let (_, cache_out) = outputs["cache_out"].try_extract_tensor::<f32>().unwrap();
      cache = cache_out.to_vec();
      let ms = t.elapsed().as_secs_f64() * 1000.0;
      if i < 5 || i % 10 == 0 {
        println!("[thread] run {i:>3}: {ms:6.2} ms/hop");
      }
    }
    println!("[thread] done");
  });
  handle.join().unwrap();
  Ok(())
}
