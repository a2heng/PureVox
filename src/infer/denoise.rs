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

//! 降噪模型流式推理（谱接口，2026-10-29 起；傅里叶在图外）：
//!
//! ```text
//! 输入  spec_in  [1,2,481]  以 t·480 为中心、±480 的周期 Hann 分析谱
//!       cache_in [1,D]      扁平流式缓存（首帧零起；D 由模型决定，如 15,668）
//!       ctrl_in  [1,2]      部署旋钮 `[floor_db, depth_gain]`（降噪深度 / 衰减倍率）
//! 输出  spec_out [1,2,481]  增强谱（图内只做 mask）
//!       cache_out[1,D]
//! ```
//!
//! 波形↔谱、OLA 与滞后 1 hop 全在 [`crate::infer::spec`]（三类模型共用一份 DSP），
//! 本文件只做会话 + 每行缓存。缓存维度从模型输入读，不写死。
//!
//! **降噪深度**（2026-11-16 起）：`ctrl_in[0]` = 掩码最深衰减（dB，`-100..-20`），
//! `ctrl_in[1]` = 衰减倍率（默认 1）。模型内部对「已判为噪声」的 bin 钳 `max(att_dB, floor_db)`，
//! 所以它只改压制深度、不改判决；应用侧每 hop 从热参数单元格读当前值（滑块实时生效，不重建会话）。

use std::path::Path;

use ort::value::TensorRef;

use crate::audio::HOP;
use crate::infer::spec::{BINS, SPEC_LEN, SpecStream};

pub struct Denoise {
  /// 从模型池借来的会话（Drop 归还；同一模型多行/多次重建不重复加载）
  model: crate::infer::LeasedSession,
  dim: usize,
  cache: Vec<f32>,
  stft: SpecStream,
  spec_out: [f32; SPEC_LEN],
}

impl Denoise {
  pub fn load(path: &Path) -> Result<Self, String> {
    let model = crate::infer::checkout(path)?;
    let dim = crate::infer::cache_dim(&model.lock(), "cache_in")?;
    crate::infer::check_spec(&model.lock(), "spec_in")?;
    crate::infer::check_ctrl(&model.lock())?;
    Ok(Denoise {
      model,
      dim,
      cache: vec![0.0; dim],
      stft: SpecStream::new(),
      spec_out: [0.0; SPEC_LEN],
    })
  }

  /// 会话/行重启：缓存与流式 DSP 状态一起归零（从零缓存起步即与模型契约一致）。
  pub fn reset(&mut self) {
    self.cache.fill(0.0);
    self.stft.reset();
  }

  /// 处理一个 480 样本 hop，返回增强后的 hop（借用内部缓冲，**滞后 1 hop**）。
  ///
  /// `depth_db` = 降噪深度（掩码最深衰减 dB，`-100..-20`）。调用方每 hop 从热参数单元格读，
  /// 滑块拖动时下一 hop 就是新值。
  ///
  /// **零分配**：谱 / cache / ctrl 都是借用张量，输出拷回预分配缓冲。
  pub fn process(&mut self, hop: &[f32], depth_db: f32) -> Result<&[f32], String> {
    debug_assert_eq!(hop.len(), HOP);
    let Self {
      model,
      dim,
      cache,
      stft,
      spec_out,
    } = self;
    let dim = *dim;
    let mut guard = model.lock();
    let session = &mut *guard;

    let run = {
      let spec = stft.analyze(hop);
      let spec_in = TensorRef::from_array_view(([1i64, 2, BINS as i64], spec))
        .map_err(|e| format!("构造 spec_in 失败：{e}"))?;
      let cache_in = TensorRef::from_array_view(([1i64, dim as i64], cache.as_slice()))
        .map_err(|e| format!("构造 cache_in 失败：{e}"))?;
      let ctrl = [depth_db, 1.0_f32];
      let ctrl_in = TensorRef::from_array_view(([1i64, 2], ctrl.as_slice()))
        .map_err(|e| format!("构造 ctrl_in 失败：{e}"))?;
      session.run(ort::inputs![
        "spec_in" => spec_in,
        "cache_in" => cache_in,
        "ctrl_in" => ctrl_in,
      ])
    };
    let outputs = match run {
      Ok(o) => o,
      Err(e) => {
        cache.fill(0.0);
        stft.reset();
        return Err(format!("推理失败：{e}"));
      }
    };
    {
      let (_, spec) = outputs["spec_out"]
        .try_extract_tensor::<f32>()
        .map_err(|e| format!("读取 spec_out 失败：{e}"))?;
      let (_, cache_out) = outputs["cache_out"]
        .try_extract_tensor::<f32>()
        .map_err(|e| format!("读取 cache_out 失败：{e}"))?;
      if spec.len() != SPEC_LEN || cache_out.len() != dim {
        return Err(format!(
          "模型输出维度异常：spec_out={}，cache_out={}（期望 {SPEC_LEN} / {dim}）",
          spec.len(),
          cache_out.len()
        ));
      }
      spec_out.copy_from_slice(&spec[..SPEC_LEN]);
      cache.copy_from_slice(cache_out);
    }
    Ok(stft.synthesize(&spec_out[..]))
  }
}

#[cfg(test)]
mod tests {
  use super::*;

  /// 降噪深度真的接到了模型 `ctrl_in`：同一段噪声，-60 dB 的输出必须明显低于 -20 dB。
  ///
  /// 手动跑：`cargo test --release -- --ignored --nocapture denoise_depth`。
  /// 这条测试是「ONNX 输入名 / 形状 / ctrl 语义」的整链自检 —— 名字写错或把深度塞错位置
  /// 都会在这里露出来（否则只能靠耳朵发现「滑块没反应」）。
  #[test]
  #[ignore = "加载 ONNX 跑真实推理，手动跑"]
  fn depth_controls_attenuation() {
    let path = crate::infer::model_path(crate::infer::MODEL_DENOISE).expect("模型文件");
    // 确定性伪随机噪声（不引依赖）：同一段信号喂两个深度，只比输出能量。
    let mut seed = 0x1234_5678u32;
    let mut noise = Vec::with_capacity(HOP * 200);
    while noise.len() < HOP * 200 {
      seed = seed.wrapping_mul(1_664_525).wrapping_add(1_013_904_223);
      noise.push((seed >> 8) as f32 / (1u32 << 24) as f32 * 0.2 - 0.1);
    }
    let energy = |depth: f32| {
      let mut d = Denoise::load(&path).expect("加载降噪模型");
      let mut sum = 0.0f64;
      for hop in noise.chunks(HOP) {
        let out = d.process(hop, depth).expect("推理");
        sum += out.iter().map(|s| (*s as f64) * (*s as f64)).sum::<f64>();
      }
      sum
    };
    let (shallow, deep) = (energy(-20.0), energy(-60.0));
    println!("depth -20 dB: {shallow:.3}；-60 dB: {deep:.3}");
    assert!(
      deep < shallow * 0.5,
      "降噪深度没生效：-60 dB 的输出（{deep:.3}）没有明显低于 -20 dB（{shallow:.3}）"
    );
  }
}
