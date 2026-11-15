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

//! OBS 音频滤镜的 PureVox Stage 实现（参数/默认值/范围对齐 OBS `plugins/obs-filters`）。
//!
//! 参考文件（OBS 源码克隆在项目外一层目录：Windows `D:\obs-studio`、Linux `~/obs-studio`）：
//! - `gain-filter.c`（增益，已有独立 `gain` 行）
//! - `noise-gate-filter.c`（噪声门）
//! - `compressor-filter.c`（压缩器）
//! - `limiter-filter.c`（限制器）
//! - `expander-filter.c`（扩展器 / 向上压缩器）
//! - `invert-audio-polarity.c`（极性反转）
//!
//! 引擎是单声道 48 kHz、10 ms hop；OBS 是任意声道。这里按单声道移植，
//! 参数语义与 OBS 一致，DSP 是等价/近似实现（不是逐位复刻 OBS 的 C 代码）。
//!
//! **未包含**：Noise Suppression（用户明确不要；PureVox 用 AI 降噪）、VST 2.x
//!（需要 VST 宿主，暂不引入）、Audio Monitor / Pan / Sync（它们是源属性/路由，不是
//! OBS 滤镜；PureVox 的输出行已经承担监听）。

use crate::audio::SAMPLE_RATE;
use crate::engine::registry::{ParamValue, Params};
use crate::engine::stage::{FrameContext, Stage, StageError};

const SR: f32 = SAMPLE_RATE as f32;

fn db_to_lin(db: f32) -> f32 {
  10f32.powf(db / 20.0)
}
fn lin_to_db(x: f32) -> f32 {
  20.0 * x.max(1e-12).log10()
}
/// OBS `gain_coefficient`：`exp(-1/(sr*time))`，time 秒；0 → 0（立即到目标）。
fn coef(ms: f32) -> f32 {
  if ms <= 0.0 {
    0.0
  } else {
    (-1.0 / (SR * ms / 1000.0)).exp()
  }
}
/// OBS `detector` 参数：`"peak"` 用逐样本峰值，其余（含缺省 `"RMS"`）用 10 ms RMS。
fn peak_detector(p: &Params) -> bool {
  matches!(p.get("detector"), Some(ParamValue::Text(t)) if t.eq_ignore_ascii_case("peak"))
}

fn num(p: &Params, key: &str, default: f32) -> f32 {
  match p.get(key) {
    Some(ParamValue::Number(n)) => *n as f32,
    Some(ParamValue::Text(t)) => t.trim().parse().unwrap_or(default),
    _ => default,
  }
}

// ---------------------------------------------------------------------------
// 噪声门（noise-gate-filter.c）
// ---------------------------------------------------------------------------
pub struct NoiseGate {
  open: f32,
  close: f32,
  attack: f32,
  release: f32,
  hold: f32,
  decay: f32,
  level: f32,
  attenuation: f32,
  held: f32,
  is_open: bool,
}

impl NoiseGate {
  pub fn new(open_db: f32, close_db: f32, attack_ms: f32, hold_ms: f32, release_ms: f32) -> Self {
    let attack = if attack_ms <= 0.0 {
      1.0
    } else {
      (1.0 / (attack_ms / 1000.0 * SR)).min(1.0)
    };
    let release = if release_ms <= 0.0 {
      1.0
    } else {
      (1.0 / (release_ms / 1000.0 * SR)).min(1.0)
    };
    let open = db_to_lin(open_db);
    let close = db_to_lin(close_db);
    // OBS：阈值差在 1/75 s 内衰减
    let decay = (open - close).abs() / (SR / 75.0);
    Self {
      open,
      close,
      attack,
      release,
      hold: (hold_ms / 1000.0 * SR).max(0.0),
      decay: decay.max(1e-12),
      level: 0.0,
      attenuation: 0.0,
      held: 0.0,
      is_open: false,
    }
  }
}

impl Stage for NoiseGate {
  fn name(&self) -> &'static str {
    "noise_gate"
  }
  fn process(&mut self, frame: &mut [f32], _ctx: &FrameContext) -> Result<(), StageError> {
    for x in frame {
      let cur = x.abs();
      if cur > self.open && !self.is_open {
        self.is_open = true;
        self.held = 0.0;
      }
      if self.level < self.close && self.is_open {
        self.is_open = false;
        self.held = 0.0;
      }
      self.level = (self.level.max(cur) - self.decay).max(0.0);
      if self.is_open {
        self.held = 0.0;
        self.attenuation = (self.attenuation + self.attack).min(1.0);
      } else if self.held >= self.hold {
        self.attenuation = (self.attenuation - self.release).max(0.0);
      } else {
        self.held += 1.0;
      }
      *x *= self.attenuation;
    }
    Ok(())
  }
}

// ---------------------------------------------------------------------------
// 压缩器（compressor-filter.c）
// ---------------------------------------------------------------------------
pub struct Compressor {
  attack: f32,
  release: f32,
  slope: f32,
  threshold: f32,
  out_gain: f32,
  env: f32,
}

impl Compressor {
  pub fn new(ratio: f32, threshold_db: f32, attack_ms: f32, release_ms: f32, out_db: f32) -> Self {
    let ratio = ratio.max(1.0);
    Self {
      attack: coef(attack_ms),
      release: coef(release_ms),
      slope: 1.0 - 1.0 / ratio,
      threshold: threshold_db,
      out_gain: db_to_lin(out_db),
      env: 0.0,
    }
  }
}

impl Stage for Compressor {
  fn name(&self) -> &'static str {
    "compressor"
  }
  fn process(&mut self, frame: &mut [f32], _ctx: &FrameContext) -> Result<(), StageError> {
    for x in frame {
      let env_in = x.abs();
      self.env = if self.env < env_in {
        env_in + self.attack * (self.env - env_in)
      } else {
        env_in + self.release * (self.env - env_in)
      };
      let env_db = lin_to_db(self.env);
      let gain_db = (self.slope * (self.threshold - env_db)).min(0.0);
      *x *= db_to_lin(gain_db) * self.out_gain;
    }
    Ok(())
  }
}

// ---------------------------------------------------------------------------
// 限制器（limiter-filter.c）
// ---------------------------------------------------------------------------
pub struct Limiter {
  attack: f32,
  release: f32,
  threshold: f32,
  out_gain: f32,
  env: f32,
}

impl Limiter {
  pub fn new(threshold_db: f32, release_ms: f32) -> Self {
    Self {
      // OBS limiter 的 attack 固定 1 ms
      attack: coef(1.0),
      release: coef(release_ms),
      threshold: threshold_db,
      out_gain: 1.0,
      env: 0.0,
    }
  }
}

impl Stage for Limiter {
  fn name(&self) -> &'static str {
    "limiter"
  }
  fn process(&mut self, frame: &mut [f32], _ctx: &FrameContext) -> Result<(), StageError> {
    // 先算整块包络（与 OBS 一致：限制器是"块内峰值"包络）
    let attack = self.attack;
    let release = self.release;
    let mut env = self.env;
    let mut envs = [0.0f32; 480];
    let n = frame.len().min(envs.len());
    for i in 0..n {
      let env_in = frame[i].abs();
      env = if env < env_in {
        env_in + attack * (env - env_in)
      } else {
        env_in + release * (env - env_in)
      };
      envs[i] = env;
    }
    self.env = env;
    for i in 0..n {
      let env_db = lin_to_db(envs[i]);
      let gain_db = (self.threshold - env_db).min(0.0);
      frame[i] *= db_to_lin(gain_db) * self.out_gain;
    }
    Ok(())
  }
}

// ---------------------------------------------------------------------------
// 扩展器 / 向上压缩器（expander-filter.c）
// ---------------------------------------------------------------------------
pub struct Expander {
  attack: f32,
  release: f32,
  threshold: f32,
  slope: f32,
  out_gain: f32,
  knee: f32,
  upward: bool,
  /// 检测器：OBS 的 `detector` 参数（RMS = 10 ms 均方根，peak = 逐样本绝对值）。
  peak: bool,
  env: f32,
  runave: f32,
  gain_db: f32,
}

/// 扩展器 / 向上压缩器的构造参数（参数多，收成一个结构体：`clippy::too_many_arguments`，
/// 也让调用点一眼看出每个数字是哪个旋钮）。
pub struct ExpanderCfg {
  pub ratio: f32,
  pub threshold_db: f32,
  pub attack_ms: f32,
  pub release_ms: f32,
  pub out_db: f32,
  pub knee: f32,
  pub upward: bool,
  pub peak: bool,
}

impl Expander {
  pub fn new(c: ExpanderCfg) -> Self {
    let ratio = if c.upward {
      c.ratio.clamp(0.0, 1.0)
    } else {
      c.ratio.max(1.0)
    };
    Self {
      attack: coef(c.attack_ms),
      release: coef(c.release_ms),
      threshold: c.threshold_db,
      slope: 1.0 - ratio,
      out_gain: db_to_lin(c.out_db),
      knee: c.knee,
      upward: c.upward,
      peak: c.peak,
      env: 0.0,
      runave: 0.0,
      gain_db: 0.0,
    }
  }
}

impl Stage for Expander {
  fn name(&self) -> &'static str {
    if self.upward {
      "upward_compressor"
    } else {
      "expander"
    }
  }
  fn process(&mut self, frame: &mut [f32], _ctx: &FrameContext) -> Result<(), StageError> {
    // 检测器（OBS `detector`）：RMS = 10 ms 均方根（默认），peak = 逐样本 |x|。
    let rms_coef = (-100.0 / SR).exp2();
    let peak = self.peak;
    let attack = self.attack;
    let release = self.release;
    let inv_attack = 1.0 - attack;
    let inv_release = 1.0 - release;
    let threshold = self.threshold;
    let slope = self.slope;
    let knee = self.knee;
    let upward = self.upward;
    for x in frame {
      self.runave = rms_coef * self.runave + (1.0 - rms_coef) * *x * *x;
      self.env = if peak {
        x.abs()
      } else {
        self.runave.max(0.0).sqrt()
      };
      let env_db = lin_to_db(self.env);
      let diff = threshold - env_db;
      let gain = if upward {
        if env_db >= threshold + knee * 0.5 {
          0.0
        } else if threshold - knee * 0.5 >= env_db {
          slope * diff
        } else if knee > 0.0 {
          slope * (diff + knee * 0.5).powi(2) / (2.0 * knee)
        } else {
          0.0
        }
      } else if diff > 0.0 {
        (slope * diff).max(-60.0)
      } else {
        0.0
      };
      let prev = self.gain_db;
      self.gain_db = if gain > prev {
        attack * prev + inv_attack * gain
      } else {
        release * prev + inv_release * gain
      };
      let out_db = if upward {
        self.gain_db
      } else {
        self.gain_db.min(0.0)
      };
      *x *= db_to_lin(out_db) * self.out_gain;
    }
    Ok(())
  }
}

// ---------------------------------------------------------------------------
// 极性反转（invert-audio-polarity.c）
// ---------------------------------------------------------------------------
pub struct InvertPolarity;

impl Stage for InvertPolarity {
  fn name(&self) -> &'static str {
    "invert"
  }
  fn process(&mut self, frame: &mut [f32], _ctx: &FrameContext) -> Result<(), StageError> {
    for x in frame {
      *x = -*x;
    }
    Ok(())
  }
}

/// 从计划参数构造 OBS 滤镜 Stage（参数缺失时用 OBS 默认值）。
pub fn stage(ptype: &str, p: &Params) -> Result<Box<dyn Stage>, String> {
  Ok(match ptype {
    "noise_gate" => Box::new(NoiseGate::new(
      num(p, "open_threshold", -26.0),
      num(p, "close_threshold", -32.0),
      num(p, "attack_time", 25.0),
      num(p, "hold_time", 200.0),
      num(p, "release_time", 150.0),
    )),
    "compressor" => Box::new(Compressor::new(
      num(p, "ratio", 10.0),
      num(p, "threshold", -18.0),
      num(p, "attack_time", 6.0),
      num(p, "release_time", 60.0),
      num(p, "output_gain", 0.0),
    )),
    "limiter" => Box::new(Limiter::new(
      num(p, "threshold", -6.0),
      num(p, "release_time", 60.0),
    )),
    "expander" => Box::new(Expander::new(ExpanderCfg {
      ratio: num(p, "ratio", 2.0),
      threshold_db: num(p, "threshold", -40.0),
      attack_ms: num(p, "attack_time", 10.0),
      release_ms: num(p, "release_time", 50.0),
      out_db: num(p, "output_gain", 0.0),
      knee: 0.0,
      upward: false,
      peak: peak_detector(p),
    })),
    "upward_compressor" => Box::new(Expander::new(ExpanderCfg {
      ratio: num(p, "ratio", 0.5),
      threshold_db: num(p, "threshold", -20.0),
      attack_ms: num(p, "attack_time", 10.0),
      release_ms: num(p, "release_time", 50.0),
      out_db: num(p, "output_gain", 0.0),
      knee: num(p, "knee", 10.0),
      upward: true,
      peak: peak_detector(p),
    })),
    "invert" => Box::new(InvertPolarity),
    other => return Err(format!("未实现的 OBS 音频滤镜：{other}")),
  })
}

#[cfg(test)]
mod tests {
  use super::*;
  use crate::engine::registry::Params;

  fn run(mut s: Box<dyn Stage>, amp: f32) {
    let mut frame = vec![amp; 480];
    s.process(&mut frame, &FrameContext::default()).unwrap();
    assert!(frame.iter().all(|x| x.is_finite()));
  }

  #[test]
  fn all_obs_filters_are_finite() {
    for ptype in [
      "noise_gate",
      "compressor",
      "limiter",
      "expander",
      "upward_compressor",
      "invert",
    ] {
      run(stage(ptype, &Params::new()).unwrap(), 0.25);
      run(stage(ptype, &Params::new()).unwrap(), 0.0);
    }
  }
}
