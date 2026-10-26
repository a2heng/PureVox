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
//! 关键：喂给校准器的是「**原始近端 hop**」与「**固定参考延时下的远端窗口**」（far 网格按实时
//! 推进，两者坐标系一致）。互相关峰 = 相对该参考的偏差；新延时 = 参考 + 偏差 = **绝对延时**，
//! 与当前存储值无关（若用存储值取窗口，旧值很错时窗口会落在探针之外 → 相关为 0、无法自纠）。
//!
//! 同时按原始电平做自动配平（近端/远端各自 RMS 归一到 -24 dBFS）；因为喂的是**增益前**的信号，
//! 反复校准不会叠加。结果发布到调试快照的 `calib` 字段，界面回填后重建会话。
//!
//! 只采集**精确窗口**（far 网格按实时推进，见 `aec.rs` 的 `spawn_far_pump`）：回退的「最近段」
//! 没有按延时偏移，量到的是绝对偏移，参与测量会让延时越加越大。精确窗口一直取不到（回环设备
//! 没数据）时按超时失败并给出原因，不让界面停在采集中。

use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

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
    current_delay_ms: f64,
    last_pub: Instant,
    /// 采集超时（精确窗口一直取不到时的兜底）
    deadline: Instant,
  },
  Computing,
  Done,
  Failed,
}

pub struct CalibHub {
  state: Arc<Mutex<State>>,
  hub: SharedHub,
  /// 重置代号：校准开始时自增，far 网格泵与列线程据此把各自序号归零（far 与 mic 同一原点）
  epoch: Arc<AtomicU64>,
}

impl CalibHub {
  pub fn new(hub: SharedHub) -> Self {
    CalibHub {
      state: Arc::new(Mutex::new(State::Idle)),
      hub,
      epoch: Arc::new(AtomicU64::new(0)),
    }
  }

  /// 重置代号句柄（far 泵与列线程共享；会话起步与每次校准都会自增）。
  pub fn epoch(&self) -> Arc<AtomicU64> {
    self.epoch.clone()
  }

  pub fn active(&self) -> bool {
    matches!(*self.state.lock().unwrap(), State::Collecting { .. })
  }

  /// 校准是否**还在进行中**（采集中 **或** 正在算）。
  ///
  /// 临时启动会话做校准时用它决定何时收尾停会话——只看 `active()` 会在「已收齐、正在
  /// 后台算」的那一小段里误判成结束，把列线程拆掉、结果也就算不出来了。
  pub fn in_flight(&self) -> bool {
    matches!(
      *self.state.lock().unwrap(),
      State::Collecting { .. } | State::Computing
    )
  }

  /// 开始采集：`seconds` 秒、最大搜索 `max_delay_ms`；`current_delay_ms` = 当前 AEC 行延时。
  pub fn start(
    &self,
    col: usize,
    row: usize,
    seconds: f64,
    _max_delay_ms: f64,
    current_delay_ms: f64,
  ) -> Result<(), String> {
    let needed = (seconds.max(1.0) * SAMPLE_RATE as f64) as usize;
    *self.state.lock().unwrap() = State::Collecting {
      target: (col, row),
      mic: Vec::with_capacity(needed),
      far: Vec::with_capacity(needed),
      needed,
      current_delay_ms,
      last_pub: Instant::now(),
      deadline: Instant::now() + Duration::from_secs_f64(seconds.max(1.0) * 3.0),
    };
    // 通知 far 泵与列线程重置序号（与本次采集从同一原点开始）
    self.epoch.fetch_add(1, Ordering::Relaxed);
    self
      .hub
      .set_calib(Probe::ok(format!("采集中 0.0/{seconds:.0} s")));
    Ok(())
  }

  /// 列工作线程喂入「原始近端 hop + 当前延时下的远端窗口」。`exact` = 该窗口是否为精确段：
  /// 回退段（最近段）没有按延时偏移，参与测量会把绝对偏移当残余累加，故不采集、只用于超时判定。
  pub fn feed(&self, col: usize, row: usize, mic: &[f32], far: &[f32], exact: bool) {
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
      if exact {
        m.extend_from_slice(mic);
        f.extend_from_slice(far);
      }
      m.len() >= *needed
    };
    if !full {
      // 超时兜底：精确窗口一直取不到（回环设备没数据）时明确失败，别让界面永远停在采集
      let expired = matches!(
        &*st,
        State::Collecting { deadline, .. } if Instant::now() >= *deadline
      );
      if expired {
        *st = State::Failed;
        self.hub.set_calib(Probe::unavailable(
          "远端窗口一直不可用：回环设备没有数据（确认扬声器在播放、回环设备正确）",
        ));
        return;
      }
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
        // **只做一件事**：带通（1.5–4 kHz）→ 各自找「窗内能量峰值时刻」→ 差即时延。
        // 不做互相关、不算相关系数：扫频是单调的，频率就是时间标签；底噪是随机噪声，
        // 高通掉低频能量汇聚的部分之后，峰值位置非常稳（2026-10-26 实测：同一段数据
        // 互相关 0.19 需要额外判据，峰值法窗内比底噪高 49 dB，一眼可信）。
        let band = PEAK_BAND;
        let Some((delay_raw, mic_snr, mic_rms, far_rms)) = estimate_delay_by_peaks(&m, &f, band)
        else {
          if let Ok(mut g) = state.lock() {
            *g = State::Failed;
          }
          hub.set_calib(Probe::unavailable("采集到的数据太短，无法测延时".to_string()));
          return;
        };
        // 残余 = 两路峰值时刻之差 − 采集时用的固定参考延时（`CALIB_REF_MS`）
        let residual_ms = delay_raw - CALIB_REF_MS;
        // 自校正：新延时 = 旧延时 + 残余，取整到 10 ms（= 1 hop 的整数倍）
        let delay_ms = (((current_delay_ms + residual_ms) / 10.0).round() * 10.0).clamp(-1000.0, 1000.0);
        let mic_gain = (TARGET_RMS_DB - mic_rms).clamp(-40.0, 40.0);
        let far_gain = (TARGET_RMS_DB - far_rms).clamp(-40.0, 40.0);
        let mut st = state.lock().unwrap();
        // 判据：**麦克风带内峰值要比它自己的底噪高 ≥15 dB**（= 确实听到了探针）。
        //
        // 为什么不是「跟 far 差 ≤10 dB」：far 是回环，拿的是**数字满刻度**（探针 0.9 幅度
        // ≈ −1 dBFS），mic 拿的是**经过房间的声学信号**，两者天然差几十 dB（本机实测 57 dB）。
        // 拿它们比大小只会把「音量正常但空间大」判成失败（2026-10-26 实测：mic 带内峰值
        // −67 dBFS、比自身底噪高 10 dB，far −9 dBFS）。
        // 差值照旧写进结果里，供判断音量够不够。
        let gap = mic_rms - far_rms;
        if mic_snr < 15.0 {
          *st = State::Failed;
          hub.set_calib(Probe::unavailable(format!(
            "麦克风没听到探针（{:.0}–{:.0} Hz 带内只比自身底噪高 {mic_snr:.0} dB，需要 ≥15 dB；比远端低 {:.0} dB）：检查麦克风是否静音 / 选错设备 / 扬声器音量",
            band.0, band.1, -gap
          )));
        } else {
          *st = State::Done;
          hub.set_calib(Probe::ok(format!(
            "延时 {delay_ms:.1} ms（峰值匹配 {:.0}–{:.0} Hz，麦克风窗内 +{mic_snr:.0} dB）｜近端 {mic_gain:+.0} dB｜远端 {far_gain:+.0} dB",
            band.0, band.1
          )));
        }
      })
      .ok();
  }
}

/// 自动配平目标：近端/远端 RMS 都归一到这个 dBFS。
const TARGET_RMS_DB: f64 = -24.0;

/// 校准取 far 窗口用的**固定参考延时**（ms）：与当前存储的延时值无关。
///
/// 探针是一次性短信号：若用当前（可能很错的）延时取窗口，窗口会落在探针之外 → 相关为 0，
/// 校准无法自我纠正。改用固定的小参考延时后，测得的就是**绝对延时**（新延时 = 参考 + 残余），
/// 从任何旧值出发都一次到位。
pub const CALIB_REF_MS: f64 = 20.0;

/// 固定参考延时的采样数（列工作线程取 far 窗口用）。
pub const CALIB_REF_SAMPLES: i64 = (CALIB_REF_MS * SAMPLE_RATE as f64 / 1000.0) as i64;

/// 校准探针频带（对数扫频起止）。
const PROBE_F0: f64 = 1500.0;
const PROBE_F1: f64 = 2500.0;

/// FFT 互相关求残余延时（ms）与归一化相关系数。
/// 去直流 + 频带限幅（与探针带一致，压带外杂波/底噪）+ 带内 RMS 归一化（与电平无关）。
/// 把最近一次校准的原始缓冲落到 `~/.purevox/calib_last_{mic,far}.f32`：
/// 离线排查（`tools/automation/aec_probe.py` / 频谱图）与「到底谁没出声」全靠它。
fn dump_calib(mic: &[f32], far: &[f32]) {
  if let Ok(dir) = crate::config::config_dir() {
    let _ = std::fs::create_dir_all(&dir);
    let bytes = |v: &[f32]| v.iter().flat_map(|x| x.to_le_bytes()).collect::<Vec<u8>>();
    let _ = std::fs::write(dir.join("calib_last_mic.f32"), bytes(mic));
    let _ = std::fs::write(dir.join("calib_last_far.f32"), bytes(far));
  }
}

/// 峰值匹配（**唯一算法**）：带通 → 各自找窗内能量峰值时刻 → 差即时延。
///
/// 不做互相关、不算相关系数。判据只有一条：麦克风窗内的峰值要明显高于它自己的
/// 底噪中位数（≥6 dB 由调用方判），否则说明它根本没听到探针。
///
/// 返回 `(延时 ms, mic 窗内 SNR dB, mic 窗内 RMS dBFS, far 窗内 RMS dBFS)`。
fn estimate_delay_by_peaks(
  mic: &[f32],
  far: &[f32],
  band: (f32, f32),
) -> Option<(f64, f64, f64, f64)> {
  let n = mic.len().min(far.len());
  if n < SAMPLE_RATE as usize / 5 {
    return None;
  }
  let (im, snr, mic_rms) = band_peak(&mic[..n], band.0, band.1)?;
  let (ifar, _, far_rms) = band_peak(&far[..n], band.0, band.1)?;
  let delay_ms = (im as f64 - ifar as f64) * 1000.0 / SAMPLE_RATE as f64;
  Some((delay_ms, snr, mic_rms, far_rms))
}

/// 带内「能量峰值时刻」：砖墙带通 → 逐帧（21 ms / hop 2.7 ms）算 RMS → 取最大帧。
/// 同时返回窗内峰值比整体中位数高多少 dB（判断「这段到底有没有信号」）。
fn band_peak(x: &[f32], lo: f32, hi: f32) -> Option<(usize, f64, f64)> {
  let n = x.len();
  if n < 4096 {
    return None;
  }
  let size = n.next_power_of_two();
  let mut buf = vec![Complex::new(0.0f32, 0.0f32); size];
  for (i, &v) in x.iter().enumerate() {
    buf[i] = Complex::new(v, 0.0);
  }
  let mut planner = FftPlanner::<f32>::new();
  let fwd = planner.plan_fft_forward(size);
  let inv = planner.plan_fft_inverse(size);
  fwd.process(&mut buf);
  let bin_hz = SAMPLE_RATE as f32 / size as f32;
  for (k, c) in buf.iter_mut().enumerate() {
    let f = if k <= size / 2 {
      k as f32 * bin_hz
    } else {
      (size - k) as f32 * bin_hz
    };
    if f < lo || f > hi {
      *c = Complex::new(0.0, 0.0);
    }
  }
  inv.process(&mut buf);
  let scale = 1.0 / size as f32;
  let sig: Vec<f32> = buf[..n].iter().map(|c| c.re * scale).collect();

  let win = SAMPLE_RATE as usize / 48; // 21 ms
  let hop = SAMPLE_RATE as usize / 375; // 2.7 ms
  let mut energies = Vec::new();
  let mut i = 0usize;
  while i + win <= n {
    let e: f64 = sig[i..i + win].iter().map(|v| (*v as f64).powi(2)).sum();
    energies.push(e);
    i += hop;
  }
  // 帧能量做 5 帧滑动平均再取峰：底噪是随机的，单帧最大值会时不时被一个毛刺顶上去；
  // 平滑之后峰值位置才稳（与 python 侧画频谱用的倍频程平滑是同一个道理）。
  let k = 2usize;
  let smooth: Vec<f64> = (0..energies.len())
    .map(|idx| {
      let lo = idx.saturating_sub(k);
      let hi = (idx + k + 1).min(energies.len());
      energies[lo..hi].iter().sum::<f64>() / (hi - lo) as f64
    })
    .collect();
  let (bi, bv) =
    smooth.iter().enumerate().fold(
      (0usize, 0.0f64),
      |acc, (idx, &v)| if v > acc.1 { (idx, v) } else { acc },
    );
  let best = (bi * hop + win / 2, bv);
  if energies.is_empty() {
    return None;
  }
  energies.sort_by(|a, b| a.partial_cmp(b).unwrap_or(std::cmp::Ordering::Equal));
  let med = energies[energies.len() / 2].max(1e-20);
  let snr = 10.0 * (best.1.max(1e-20) / med).log10();
  // 峰值窗的 RMS（给「自动配平」用；与 win 长度无关，换算成 dBFS）
  let rms_db = 10.0 * (best.1.max(1e-20) / win as f64).log10();
  Some((best.0, snr, rms_db))
}

/// 峰值匹配用的**唯一频窗**：高通 1.5 kHz 避开低频能量汇聚（扫频峰 + 房间低频 + 工频），
/// 低通 2.5 kHz 正好卡住探针带宽（再往上是麦克风 3.3 kHz 滚降前的过渡区，信噪比在掉）。
/// 随机噪声的峰值位置不稳，但**固定的探针**在这一窗内的峰值时刻非常稳 —— 这就是量延时
/// 需要的全部信息（扫频单调 → 频率就是时间标签）。
const PEAK_BAND: (f32, f32) = (1500.0, 2500.0);

/// 校准探针：**窄带** 1500→2500 Hz 线性扫频（120 ms，Hann 包络，0.9 幅度），前后留静音。
///
/// 为什么不再是 800→6000 的对数扫频：那样能量铺满整个频带，频谱是个大鼓包、
/// 峰值被包络和低频倾斜拖着走（实测峰落在 2 kHz 之前、而且不尖）。窄带之后：
/// - 频谱是一个 **1.5–2.5 kHz 的尖峰**，峰值位置稳定在 2 kHz 附近；
/// - 峰值匹配窗只要开 1500–2500，避开低频能量汇聚处与麦克风 3.3 kHz 之后的滚降；
/// - 仍是扫频（不是单音），房间反射在不同频点出现，不会和直达声在同一个频点叠成双峰。
pub fn make_probe() -> Vec<f32> {
  const SR: usize = SAMPLE_RATE as usize;
  let head = SR * 100 / 1000;
  let dur = SR * 120 / 1000;
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

/// 把探针送到「远端监听的那只输出」与给定输出设备（临时播放流，放完即停）。
///
/// - `far_sink`：远端回环的目标 sink（Linux 用 `pacat` 直送；`None`/空 = 默认 sink）。
///   探针必须从这只 sink 发出，far 才收得到（cpal 在 Linux 打不开 Pulse sink 名）。
/// - `devices`：列计划的输出设备（cpal 路径）；Windows 的回环端点也在此。
pub fn play_probe(hub: SharedHub, devices: Vec<String>, far_sink: Option<String>) {
  let probe = Arc::new(make_probe());
  if let Some(sink) = far_sink {
    let p = probe.clone();
    let hub2 = hub.clone();
    std::thread::Builder::new()
      .name("probe-far".into())
      .spawn(move || {
        let target = if sink.is_empty() {
          None
        } else {
          Some(sink.as_str())
        };
        if let Err(e) = crate::audio::probe::play(&p, target) {
          hub2.push_ui("error", format!("校准探针（远端 sink）播放失败：{e}"));
        }
      })
      .ok();
  }
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

#[cfg(test)]
mod tests {
  use super::*;

  /// 合成信号钉死符号约定：mic(i) = far(i − K)（mic 滞后 far K 个样本）→ 估计器应返回 +K ms。
  #[test]
  fn estimate_delay_positive_when_mic_lags_far() {
    let far = make_probe();
    let k = (0.1 * SAMPLE_RATE as f64) as usize; // 100 ms
    let mut mic = vec![0.0f32; far.len()];
    mic[k..].copy_from_slice(&far[..far.len() - k]);
    let (ms, snr, ..) = estimate_delay_by_peaks(&mic, &far, PEAK_BAND).expect("应当能量到");
    assert!(snr > 20.0, "窗内 SNR 太低：{snr}");
    assert!((ms - 100.0).abs() < 5.0, "期望 +100 ms，得到 {ms} ms");
  }

  /// 绝对测量公式：mic(i) = far(i − D)，窗口取 far(i − ref) → 残余 = D − ref，新延时 = D。
  #[test]
  fn absolute_delay_from_fixed_reference() {
    let far = make_probe();
    let d = (0.06 * SAMPLE_RATE as f64) as usize; // 真实延时 60 ms
    let r = (CALIB_REF_MS / 1000.0 * SAMPLE_RATE as f64) as usize; // 参考 20 ms
    // 模拟列线程：喂 mic(i) 与 far(i − r)
    let n = far.len() - d;
    let mic = &far[..n];
    let win = &far[d - r..d - r + n];
    let (residual_ms, snr, ..) = estimate_delay_by_peaks(mic, win, PEAK_BAND).expect("应当能量到");
    assert!(snr > 20.0, "窗内 SNR 太低：{snr}");
    let new_ms = CALIB_REF_MS + residual_ms;
    assert!((new_ms - 60.0).abs() < 5.0, "期望 60 ms，得到 {new_ms} ms");
  }
}
