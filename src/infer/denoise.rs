// PureVox — AI 麦克风降噪工具
// Copyright (C) 2024-2026 a2heng <752848283@qq.com>
//
// PureVox is licensed under the GNU General Public License v3.0 or
// later (GPL-3.0-or-later).  See LICENSE for details.
//
// The bundled speech-denoising models are distributed under the BSD-3-Clause
// license together with the `purevox_denoise_rust` inference library
// (https://github.com/a2heng/purevox_denoise_rust).  Other bundled models
// (AEC, TSE) are NOT covered by the GPL.  See MODEL-LICENSE.md for details.
//
// SPDX-License-Identifier: GPL-3.0-or-later

//! 降噪流式推理 —— **进程内纯 Rust**（`purevox_denoise_rust`，BSD-3-Clause）。
//!
//! ```text
//! 输入  spec_in  [2,481]   以 t·480 为中心、±480 的周期 Hann 分析谱（实部在前）
//!       cache_in [D]      扁平流式缓存（首帧零起；D 由模型决定，如 10,208）
//! 输出  spec_out [2,481]  增强谱（实数幅度掩码，保留含噪相位）
//!       cache_out[D]
//! ```
//!
//! 波形↔谱、OLA 与滞后 1 hop 仍由 [`crate::infer::spec`] 负责（与 AEC/TSE 共用同一份 DSP）；
//! 本文件只做会话 + 每行缓存。缓存维度由模型自己报（`Engine::cache_len`），不写死。
//!
//! 没有降噪深度旋钮：四个模型就是四档，换模型 = 换 `models/` 下的目录。

use std::path::Path;

use crate::audio::HOP;
use crate::infer::spec::{SPEC_LEN, SpecStream};

use purevox_denoise_rust::Engine;

pub struct Denoise {
  engine: Engine,
  stft: SpecStream,
  spec_out: [f32; SPEC_LEN],
}

impl Denoise {
  /// `path` 是模型**目录**（`meta.json + weights.safetensors`）。
  pub fn load(path: &Path) -> Result<Self, String> {
    let engine = Engine::load(path).map_err(|e| format!("加载降噪模型失败：{e}"))?;
    Ok(Denoise {
      engine,
      stft: SpecStream::new(),
      spec_out: [0.0; SPEC_LEN],
    })
  }

  /// 模型标识（`meta.json` 里的 `model`），用于调试页与日志。
  pub fn model_name(&self) -> &str {
    self.engine.name()
  }

  /// 流式缓存宽度（f32 个数），调试页可直接展示。
  pub fn cache_len(&self) -> usize {
    self.engine.cache_len()
  }

  /// 会话/行重启：缓存与流式 DSP 状态一起归零（从零缓存起步即与模型契约一致）。
  pub fn reset(&mut self) {
    self.engine.set_cache_flat(&vec![0.0; self.engine.cache_len()]);
    self.stft.reset();
  }

  /// 处理一个 480 样本 hop，返回增强后的 hop（借用内部缓冲，**滞后 1 hop**）。
  ///
  /// 零 ONNX 会话、零推理分配（`process_hop` 内部只有层内临时缓冲）。
  pub fn process(&mut self, hop: &[f32]) -> Result<&[f32], String> {
    debug_assert_eq!(hop.len(), HOP);
    let spec = self.stft.analyze(hop);
    let out = self.engine.process_hop(spec);
    if out.len() != SPEC_LEN {
      return Err(format!("降噪输出维度异常：{}（期望 {SPEC_LEN}）", out.len()));
    }
    self.spec_out.copy_from_slice(&out);
    Ok(self.stft.synthesize(&self.spec_out[..]))
  }
}

#[cfg(test)]
mod tests {
  use super::*;

  /// 进程内纯 Rust 降噪的接线检查：加载四款里的第一款，喂 20 个 hop，看输出有限且非零。
  #[test]
  fn rust_engine_processes_hops() {
    let dir = crate::infer::model_path(crate::infer::MODEL_DENOISE);
    let dir = match dir {
      Ok(d) => d,
      Err(_) => return, // 模型目录不在（例如裁剪过的构建）→ 跳过
    };
    let mut d = Denoise::load(&dir).expect("load denoise");
    assert!(d.cache_len() > 0);
    assert!(d.model_name().contains("purevox_denoise"));
    let mut phase = 0.0f32;
    let mut energy = 0.0f64;
    for _ in 0..20 {
      let hop: Vec<f32> = (0..HOP)
        .map(|i| {
          let x = (phase + i as f32 * 0.05).sin() * 0.1;
          x
        })
        .collect();
      phase += HOP as f32 * 0.05;
      let out = d.process(&hop).expect("process hop");
      assert_eq!(out.len(), HOP);
      for v in out {
        assert!(v.is_finite(), "non-finite output");
        energy += (*v as f64) * (*v as f64);
      }
    }
    assert!(energy > 0.0, "output is silent");
  }
}
