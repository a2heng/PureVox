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

//! 启动 / 停止提示音：**自己合成**（短促、成对，start 上行 / stop 下行），
//! 不依赖系统声音、不用音频文件。参照旧实现 `pvengine/cues.py` 的 6 套预设。
//! 播放走系统默认输出（`PlaySoundW` + `SND_MEMORY`，短命线程，失败静默）。

const SR: f64 = 48_000.0;
const PEAK: f32 = 0.6;
const DEFAULT_PRESET: &str = "soft";

/// 预设 (id, 界面名)；id 持久化在设置里，勿改名。
pub const PRESETS: &[(&str, &str)] = &[
  ("soft", "柔和"),
  ("crisp", "清脆"),
  ("pop", "气泡"),
  ("blip", "电子"),
  ("wood", "木鱼"),
  ("chime", "风铃"),
];

fn n_of(ms: f64) -> usize {
  ((SR * ms / 1000.0) as usize).max(1)
}

/// 快起慢落包络（线性起音 + 指数衰减，无咔哒）。
fn env(n: usize, attack_ms: f64, tau_ms: f64) -> Vec<f32> {
  let tau = (tau_ms / 1000.0).max(1e-6);
  let a = ((attack_ms / 1000.0 * SR) as usize).min(n);
  (0..n)
    .map(|i| {
      let t = i as f64 / SR;
      let mut e = (-t / tau).exp();
      if a > 0 && i < a {
        e *= i as f64 / a as f64;
      }
      e as f32
    })
    .collect()
}

fn tone(freq: f64, ms: f64, tau_ms: f64, attack_ms: f64, partials: &[f32]) -> Vec<f32> {
  let n = n_of(ms);
  let e = env(n, attack_ms, tau_ms);
  let ratios = [1.0f64, 2.76, 5.40];
  (0..n)
    .map(|i| {
      let t = i as f64 / SR;
      let mut s = 0.0f32;
      for (k, &amp) in partials.iter().enumerate() {
        let r = if k < 3 { ratios[k] } else { (k + 1) as f64 };
        s += amp * (2.0 * std::f64::consts::PI * freq * r * t).sin() as f32;
      }
      s * e[i]
    })
    .collect()
}

fn square(freq: f64, ms: f64) -> Vec<f32> {
  let n = n_of(ms);
  let e = env(n, 2.0, 24.0);
  (0..n)
    .map(|i| {
      let t = i as f64 / SR;
      let w = if (2.0 * std::f64::consts::PI * freq * t).sin() >= 0.0 { 1.0 } else { -1.0 };
      (w * 0.5) as f32 * e[i]
    })
    .collect()
}

fn voice(preset: &str, start: bool) -> Vec<f32> {
  match preset {
    "crisp" => {
      let f = if start { 1318.5 } else { 880.0 };
      tone(f, 95.0, 26.0, 2.0, &[1.0, 0.45, 0.18])
    }
    "pop" => {
      let (f0, f1) = if start { (1200.0f64, 520.0f64) } else { (700.0f64, 300.0f64) };
      let n = n_of(70.0);
      let e = env(n, 2.0, 22.0);
      let mut phase = 0.0f64;
      (0..n)
        .map(|i| {
          let frac = i as f64 / (n - 1).max(1) as f64;
          let f = f0 * (f1 / f0).powf(frac);
          phase += 2.0 * std::f64::consts::PI * f / SR;
          (phase.sin() as f32) * e[i]
        })
        .collect()
    }
    "blip" => {
      let mut v = square(1568.0, 45.0);
      if !start {
        v.extend(square(1046.5, 55.0));
      }
      v
    }
    "wood" => {
      let f = if start { 760.0f64 } else { 520.0f64 };
      let n = n_of(65.0);
      let e = env(n, 0.5, 5.0);
      let body = tone(f, 65.0, 20.0, 1.0, &[1.0, 0.35, 0.12]);
      let mut seed = 0u32;
      (0..n)
        .map(|i| {
          seed = seed.wrapping_mul(1664525).wrapping_add(1013904223);
          let r = ((seed >> 8) as f32 / (1u32 << 24) as f32) * 2.0 - 1.0;
          0.35 * (r * e[i]) + body.get(i).copied().unwrap_or(0.0)
        })
        .collect()
    }
    "chime" => {
      let f = if start { 784.0 } else { 587.33 };
      tone(f, 180.0, 80.0, 6.0, &[1.0, 0.30, 0.14])
    }
    _ => {
      // soft（默认）
      let (a, b) = if start { (587.33, 880.0) } else { (880.0, 587.33) };
      let mut v = tone(a, 70.0, 40.0, 4.0, &[1.0]);
      v.extend(tone(b, 80.0, 45.0, 4.0, &[1.0]));
      v
    }
  }
}

/// 合成一段提示音（mono f32 @48kHz，峰值 0.6）。
fn render(preset: &str, kind: &str) -> Vec<f32> {
  let start = kind != "stop";
  let p = PRESETS.iter().any(|(id, _)| *id == preset);
  let mut sig = voice(if p { preset } else { DEFAULT_PRESET }, start);
  let peak = sig.iter().fold(0.0f32, |m, x| m.max(x.abs()));
  if peak > 0.0 {
    let g = PEAK / peak;
    for x in sig.iter_mut() {
      *x *= g;
    }
  }
  sig
}

/// 提示音的 16bit PCM WAV 字节（mono 48kHz）。
pub fn wav_bytes(preset: &str, kind: &str) -> Vec<u8> {
  let sig = render(preset, kind);
  let n = sig.len() as u32;
  let data_len = n * 2;
  let mut out = Vec::with_capacity(44 + data_len as usize);
  out.extend_from_slice(b"RIFF");
  out.extend_from_slice(&(36 + data_len).to_le_bytes());
  out.extend_from_slice(b"WAVEfmt ");
  out.extend_from_slice(&16u32.to_le_bytes());
  out.extend_from_slice(&1u16.to_le_bytes());
  out.extend_from_slice(&1u16.to_le_bytes());
  out.extend_from_slice(&(SR as u32).to_le_bytes());
  out.extend_from_slice(&((SR as u32) * 2).to_le_bytes());
  out.extend_from_slice(&2u16.to_le_bytes());
  out.extend_from_slice(&16u16.to_le_bytes());
  out.extend_from_slice(b"data");
  out.extend_from_slice(&data_len.to_le_bytes());
  for &s in &sig {
    let v = (s.clamp(-1.0, 1.0) * 32767.0) as i16;
    out.extend_from_slice(&v.to_le_bytes());
  }
  out
}

/// 播放提示音（短命线程，失败静默；`preset` 为空 = 关闭）。
pub fn play(preset: &str, kind: &str) {
  if preset.is_empty() {
    return;
  }
  let wav = wav_bytes(preset, kind);
  std::thread::Builder::new()
    .name("cue".into())
    .spawn(move || {
      #[cfg(windows)]
      unsafe {
        use windows::Win32::Media::Audio::{PlaySoundW, SND_MEMORY};
        let _ = PlaySoundW(
          windows::core::PCWSTR(wav.as_ptr() as *const u16),
          None,
          SND_MEMORY,
        );
      }
      #[cfg(not(windows))]
      {
        let _ = &wav;
      }
    })
    .ok();
}
