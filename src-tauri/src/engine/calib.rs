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

//! AEC 校准：**与运行同一坐标系**求延时 + 电平配平。
//!
//! 关键：喂给校准器的是「**原始近端 hop**」与「**当前延时下的远端窗口**」（即 AEC 实际用的那对
//! 信号）。互相关峰 = 当前延时的**残余偏差**；新延时 = 旧延时 + 残余 → 自校正，且与运行坐标系
//! 严格一致（不会因为两路采集起点不同而整体偏掉）。
//!
//! 同时按原始电平做自动配平（近端/远端各自 RMS 归一到 -24 dBFS）；因为喂的是**增益前**的信号，
//! 反复校准不会叠加。结果发布到调试快照的 `calib` 字段，界面回填后重建会话。

use std::sync::{Arc, Mutex};
use std::time::Instant;

use rustfft::FftPlanner;
use rustfft::num_complex::Complex;

use crate::audio::SAMPLE_RATE;
use crate::debug::{Probe, SharedHub};

enum State {
  Idle,
  Collecting {
    target: (usize, usize),
    mic: Vec<f32>,
    far: Vec<f32>,
    needed: usize,
    max_lag: usize,
    current_delay_ms: f64,
    last_pub: Instant,
  },
  Computing,
  Done,
  Failed,
}

pub struct CalibHub {
  state: Arc<Mutex<State>>,
  hub: SharedHub,
}

impl CalibHub {
  pub fn new(hub: SharedHub) -> Self {
    CalibHub {
      state: Arc::new(Mutex::new(State::Idle)),
      hub,
    }
  }

  pub fn active(&self) -> bool {
    matches!(*self.state.lock().unwrap(), State::Collecting { .. })
  }

  /// 开始采集：`seconds` 秒、最大搜索 `max_delay_ms`；`current_delay_ms` = 当前 AEC 行延时。
  pub fn start(
    &self,
    col: usize,
    row: usize,
    seconds: f64,
    max_delay_ms: f64,
    current_delay_ms: f64,
  ) -> Result<(), String> {
    let needed = (seconds.max(1.0) * SAMPLE_RATE as f64) as usize;
    let max_lag = (max_delay_ms * SAMPLE_RATE as f64 / 1000.0) as usize;
    *self.state.lock().unwrap() = State::Collecting {
      target: (col, row),
      mic: Vec::with_capacity(needed),
      far: Vec::with_capacity(needed),
      needed,
      max_lag,
      current_delay_ms,
      last_pub: Instant::now(),
    };
    self
      .hub
      .set_calib(Probe::ok(format!("采集中 0.0/{seconds:.0} s")));
    Ok(())
  }

  /// 列工作线程喂入「原始近端 hop + 当前延时下的远端窗口」。
  pub fn feed(&self, col: usize, row: usize, mic: &[f32], far: &[f32]) {
    let mut st = self.state.lock().unwrap();
    let full = {
      let State::Collecting {
        target,
        mic: m,
        far: f,
        needed,
        ..
      } = &mut *st
      else {
        return;
      };
      if *target != (col, row) {
        return;
      }
      m.extend_from_slice(mic);
      f.extend_from_slice(far);
      m.len() >= *needed
    };
    if !full {
      if let State::Collecting {
        mic: m,
        needed,
        last_pub,
        ..
      } = &mut *st
        && last_pub.elapsed().as_millis() >= 200
      {
        *last_pub = Instant::now();
        let secs = m.len() as f64 / SAMPLE_RATE as f64;
        let total = *needed as f64 / SAMPLE_RATE as f64;
        self
          .hub
          .set_calib(Probe::ok(format!("采集中 {secs:.1}/{total:.0} s")));
      }
      return;
    }

    // 收齐 → 后台计算，别卡住列线程
    let prev = std::mem::replace(&mut *st, State::Computing);
    let State::Collecting {
      mic: m,
      far: f,
      max_lag,
      current_delay_ms,
      ..
    } = prev
    else {
      return;
    };
    drop(st);
    self.hub.set_calib(Probe::ok("计算中…".into()));

    let state = self.state.clone();
    let hub = self.hub.clone();
    std::thread::Builder::new()
      .name("aec-calib".into())
      .spawn(move || {
        dump_calib(&m, &f);
        let (residual_ms, coef, mic_rms, far_rms) = estimate_delay(&m, &f, max_lag);
        // 自校正：新延时 = 旧延时 + 残余，并取整到 10 ms（= 1 hop 的整数倍）
        let delay_ms = (((current_delay_ms + residual_ms) / 10.0).round() * 10.0).clamp(-1000.0, 1000.0);
        let mic_gain = (TARGET_RMS_DB - mic_rms).clamp(-40.0, 40.0);
        let far_gain = (TARGET_RMS_DB - far_rms).clamp(-40.0, 40.0);
        let mut st = state.lock().unwrap();
        if coef < 0.02 {
          *st = State::Failed;
          hub.set_calib(Probe::unavailable(format!(
            "相关太弱（{coef:.3}）：请让远端（扬声器）放点声音再测"
          )));
        } else {
          *st = State::Done;
          hub.set_calib(Probe::ok(format!(
            "延时 {delay_ms:.1} ms（相关 {coef:.2}）｜近端 {mic_gain:+.0} dB｜远端 {far_gain:+.0} dB"
          )));
        }
      })
      .ok();
  }
}

/// 自动配平目标：近端/远端 RMS 都归一到这个 dBFS。
const TARGET_RMS_DB: f64 = -24.0;

/// 峰值附近 ±62.5 ms 窗口的 RMS（dBFS）——只量「探针那一段」，不受静音/底噪拖累。
fn peak_window_rms_db(x: &[f32]) -> f64 {
  let n = x.len();
  if n == 0 {
    return -160.0;
  }
  let (mut pi, mut pv) = (0usize, 0.0f32);
  for (i, &v) in x.iter().enumerate() {
    if v.abs() > pv {
      pv = v.abs();
      pi = i;
    }
  }
  let w = (SAMPLE_RATE as usize / 16).min(n / 2).max(1);
  let lo = pi.saturating_sub(w);
  let hi = (pi + w).min(n);
  let seg = &x[lo..hi];
  let e: f64 = seg.iter().map(|v| (*v as f64).powi(2)).sum();
  10.0 * (e / seg.len().max(1) as f64).max(1e-20).log10()
}

/// 校准探针频带（对数扫频起止）。
const PROBE_F0: f64 = 800.0;
const PROBE_F1: f64 = 6000.0;

/// FFT 互相关求残余延时（ms）与归一化相关系数。
/// 去直流 + 频带限幅（与探针带一致，压带外杂波/底噪）+ 带内 RMS 归一化（与电平无关）。
fn estimate_delay(mic: &[f32], far: &[f32], max_lag: usize) -> (f64, f64, f64, f64) {
  let n = mic.len().min(far.len());
  if n < SAMPLE_RATE as usize / 10 {
    return (0.0, 0.0, -160.0, -160.0);
  }
  let mut a: Vec<f32> = mic[..n].to_vec();
  let mut b: Vec<f32> = far[..n].to_vec();
  for v in [&mut a, &mut b] {
    let mean = v.iter().sum::<f32>() / n as f32;
    for x in v.iter_mut() {
      *x -= mean;
    }
  }

  let size = (2 * n).next_power_of_two();
  let mut fa = vec![Complex::new(0.0f32, 0.0f32); size];
  let mut fb = vec![Complex::new(0.0f32, 0.0f32); size];
  for i in 0..n {
    fa[i] = Complex::new(a[i], 0.0);
    fb[i] = Complex::new(b[i], 0.0);
  }
  let mut planner = FftPlanner::<f32>::new();
  let fwd = planner.plan_fft_forward(size);
  let inv = planner.plan_fft_inverse(size);
  fwd.process(&mut fa);
  fwd.process(&mut fb);

  // 频带限幅（与探针带一致，两端各留一点余量）：压制带外杂波/底噪
  let (band_lo, band_hi) = ((PROBE_F0 * 0.85) as f32, (PROBE_F1 * 1.1) as f32);
  let bin_hz = SAMPLE_RATE as f32 / size as f32;
  for k in 0..size {
    let f = if k <= size / 2 {
      k as f32 * bin_hz
    } else {
      (size - k) as f32 * bin_hz
    };
    if f < band_lo || f > band_hi {
      fa[k] = Complex::new(0.0, 0.0);
      fb[k] = Complex::new(0.0, 0.0);
    }
  }
  // 带通时域（滤波后）：峰值附近 RMS——只量探针那一段，不受静音/底噪拖累
  let bp_scale = 1.0 / size as f32;
  let mut ta = fa.clone();
  let mut tb = fb.clone();
  inv.process(&mut ta);
  inv.process(&mut tb);
  let a_bp: Vec<f32> = ta.iter().map(|c| c.re * bp_scale).collect();
  let b_bp: Vec<f32> = tb.iter().map(|c| c.re * bp_scale).collect();
  let mic_rms = peak_window_rms_db(&a_bp);
  let far_rms = peak_window_rms_db(&b_bp);

  // 频带内能量归一化（各自单位 RMS），使相关系数与电平无关
  let rms = |v: &[Complex<f32>]| (v.iter().map(|c| c.norm_sqr()).sum::<f32>() / size as f32).sqrt();
  let (ra, rb) = (rms(&fa).max(1e-9), rms(&fb).max(1e-9));
  for k in 0..size {
    fa[k] /= ra;
    fb[k] /= rb;
  }
  for k in 0..size {
    fa[k] *= fb[k].conj();
  }
  inv.process(&mut fa);

  let scale = 1.0 / size as f32;
  let hi = max_lag.min(n.saturating_sub(1));
  // 对称搜索：正 lag = mic 滞后 far（残余为正）；负 = 远端超前
  let mut best: i64 = 0;
  let mut best_v = f32::MIN;
  for (k, &c) in fa.iter().enumerate().take(hi + 1) {
    let v = c.re * scale;
    if v > best_v {
      best_v = v;
      best = k as i64;
    }
  }
  for (k, &c) in fa.iter().enumerate().skip(size - hi).take(hi) {
    let v = c.re * scale;
    if v > best_v {
      best_v = v;
      best = k as i64 - size as i64;
    }
  }
  (
    best as f64 / (SAMPLE_RATE as f64 / 1000.0),
    best_v as f64,
    mic_rms,
    far_rms,
  )
}

/// 把最近一次校准的 mic / far 缓冲写盘（`~/.purevox/calib_last_{mic,far}.f32`），便于离线排查。
fn dump_calib(mic: &[f32], far: &[f32]) {
  if let Ok(dir) = crate::config::config_dir() {
    let _ = std::fs::create_dir_all(&dir);
    let bytes = |v: &[f32]| v.iter().flat_map(|x| x.to_le_bytes()).collect::<Vec<u8>>();
    let _ = std::fs::write(dir.join("calib_last_mic.f32"), bytes(mic));
    let _ = std::fs::write(dir.join("calib_last_far.f32"), bytes(far));
  }
}

/// 校准探针：单次 800→6000 Hz 对数扫频（150 ms，Hann 包络，0.9 幅度），前后留短静音。
/// 单次是刻意的（重复 chirp 会产生假峰）；起始频率抬高、时长压短，听感更利落。
pub fn make_probe() -> Vec<f32> {
  const SR: usize = SAMPLE_RATE as usize;
  let head = SR * 100 / 1000;
  let dur = SR * 150 / 1000;
  let tail = SR * 150 / 1000;
  let mut v = vec![0.0f32; head + dur + tail];
  let (f0, f1) = (PROBE_F0, PROBE_F1);
  let k = (f1 / f0).ln() / (dur as f64 / SR as f64);
  let mut phase = 0.0f64;
  for i in 0..dur {
    let t = i as f64 / SR as f64;
    let f = f0 * (k * t).exp();
    phase += 2.0 * std::f64::consts::PI * f / SR as f64;
    let w = 0.5 - 0.5 * (2.0 * std::f64::consts::PI * i as f64 / dur as f64).cos();
    v[head + i] = (0.9 * w * phase.sin()) as f32;
  }
  v
}

/// 把探针送到给定输出设备（临时播放流，放完即停）。让 far 与 mic 都能收到探针。
pub fn play_probe(hub: SharedHub, devices: Vec<String>) {
  let probe = Arc::new(make_probe());
  for (i, dev) in devices.into_iter().enumerate() {
    let fan = Arc::new(crate::audio::fanout::Fanout::new("校准探针".to_string()));
    let tag = format!("probe{i}");
    let Ok(handle) = crate::audio::playback::spawn(hub.clone(), dev.clone(), fan.clone(), tag)
    else {
      continue;
    };
    let fan2 = fan.clone();
    let probe2 = probe.clone();
    std::thread::Builder::new()
      .name("probe-push".into())
      .spawn(move || {
        // 等播放线程订阅并预热
        std::thread::sleep(std::time::Duration::from_millis(150));
        for chunk in probe2.chunks(crate::audio::HOP) {
          let mut hop = [0.0f32; crate::audio::HOP];
          hop[..chunk.len()].copy_from_slice(chunk);
          fan2.push_hop(&hop);
          std::thread::sleep(std::time::Duration::from_millis(10));
        }
        std::thread::sleep(std::time::Duration::from_millis(400));
        handle.stop();
      })
      .ok();
  }
}
