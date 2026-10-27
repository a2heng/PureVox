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
//! 推进，两者坐标系一致）。窄带探针在带内能量包络上的峰值时刻之差 = 相对该参考的偏差；
//! 新延时 = 参考 + 偏差 = **绝对延时**，与当前存储值无关（若用存储值取窗口，旧值很错时窗口会
//! 落在探针之外 → 测不到、无法自纠）。
//!
//! **结果是一份结构化 `CalibReport`**（AGENTS.md §1.4）：进调试快照 `calib_report`、专用端点
//! `GET /debug/calib`、并落盘 `~/.purevox/calib_last.json`；界面只渲染这份数据。
//! 原来那句中文串（`calib` 字段）保留向后兼容，但不再是唯一结果。
//!
//! 只采集**精确窗口**（far 网格按实时推进，见 `aec.rs` 的 `spawn_far_pump`）：回退的「最近段」
//! 没有按延时偏移，量到的是绝对偏移，参与测量会让延时越加越大。精确窗口一直取不到（回环设备
//! 没数据）时按超时失败并给出原因，不让界面停在采集中。

use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use rustfft::FftPlanner;
use rustfft::num_complex::Complex;
use serde::Serialize;

use crate::audio::SAMPLE_RATE;
use crate::debug::{Probe, SharedHub, now_ms};

enum State {
  Idle,
  Collecting {
    target: (usize, usize),
    mic: Vec<f32>,
    far: Vec<f32>,
    needed: usize,
    current_delay_ms: f64,
    /// 诊断用的设备标识（AEC 行当前的 mic / far 取值，原样记录，AI 读报告时需要）
    mic_device: Option<String>,
    far_device: Option<String>,
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

/// 一次校准的启动参数（设备标识只进报告，不参与算法）。
pub struct CalibStart {
  pub col: usize,
  pub row: usize,
  pub seconds: f64,
  /// 当前 AEC 行的 `far_delay_ms`（新延时 = 它 + 残差）
  pub current_delay_ms: f64,
  pub mic_device: Option<String>,
  pub far_device: Option<String>,
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

  /// 开始采集：`cfg.seconds` 秒；`current_delay_ms` = 当前 AEC 行延时。
  pub fn start(&self, cfg: CalibStart) -> Result<(), String> {
    let needed = (cfg.seconds.max(1.0) * SAMPLE_RATE as f64) as usize;
    *self.state.lock().unwrap() = State::Collecting {
      target: (cfg.col, cfg.row),
      mic: Vec::with_capacity(needed),
      far: Vec::with_capacity(needed),
      needed,
      current_delay_ms: cfg.current_delay_ms,
      mic_device: cfg.mic_device,
      far_device: cfg.far_device,
      last_pub: Instant::now(),
      deadline: Instant::now() + Duration::from_secs_f64(cfg.seconds.max(1.0) * 3.0),
    };
    // 通知 far 泵与列线程重置序号（与本次采集从同一原点开始）
    self.epoch.fetch_add(1, Ordering::Relaxed);
    // 上一轮的报告立刻失效：界面不该在新一轮进行中继续展示旧结论
    self.hub.set_calib_report(Probe::Pending);
    self
      .hub
      .set_calib(Probe::ok(format!("采集中 0.0/{:.0} s", cfg.seconds)));
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
        let (c, r, md, fd) = match &*st {
          State::Collecting {
            target,
            mic_device,
            far_device,
            ..
          } => (target.0, target.1, mic_device.clone(), far_device.clone()),
          _ => (col, row, None, None),
        };
        *st = State::Failed;
        drop(st);
        let text = "远端窗口一直不可用：回环设备没有数据（确认扬声器在播放、回环设备正确）";
        let report = CalibReport {
          verdict_code: "far_unavailable".into(),
          verdict_text: text.into(),
          next_step: Some(
            "确认 AEC 行的「扬声器」选的是正在出声的那只输出；系统没在播放时回环也不会出数据。"
              .into(),
          ),
          ..CalibReport::new(c, r, md, fd)
        };
        publish_report(&self.hub, report, Probe::unavailable(text.to_string()));
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
      target,
      mic: m,
      far: f,
      current_delay_ms,
      mic_device,
      far_device,
      ..
    } = prev
    else {
      return;
    };
    drop(st);
    self.hub.set_calib(Probe::ok("计算中…".into()));

    let state = self.state.clone();
    let hub = self.hub.clone();
    let base = CalibReport::new(target.0, target.1, mic_device, far_device);
    std::thread::Builder::new()
      .name("aec-calib".into())
      .spawn(move || {
        // **只做一件事**：带通（1.5–2.5 kHz）→ 各自找「窗内能量峰值时刻」→ 差即时延。
        // 不做互相关、不算相关系数：扫频是单调的，频率就是时间标签；底噪是随机噪声，
        // 高通掉低频能量汇聚的部分之后，峰值位置非常稳（2026-10-26 实测：同一段数据
        // 互相关 0.19 需要额外判据，峰值法窗内比底噪高 49 dB，一眼可信）。
        let band = PEAK_BAND;
        let (mic_dump, far_dump) = dump_calib(&m, &f);
        let dumps = CalibDumps {
          mic: mic_dump,
          far: far_dump,
          json: None, // publish_report 落盘时填
        };
        let mut report = CalibReport {
          dumps,
          ..base
        };
        let Some(an) = analyze(&m, &f, band) else {
          if let Ok(mut g) = state.lock() {
            *g = State::Failed;
          }
          report.ok = false;
          report.verdict_code = "too_short".into();
          report.verdict_text = "采集到的数据太短，无法测延时".into();
          report.next_step = Some("重试一次；若仍失败，确认会话在运行、AEC 行设备正常。".into());
          publish_report(&hub, report, Probe::unavailable("采集到的数据太短，无法测延时"));
          return;
        };

        report.mic_peak_ms = Some(an.mic.peak_ms);
        report.far_peak_ms = Some(an.far.peak_ms);
        report.delay_raw_ms = Some(an.delay_raw_ms);
        report.echo_path_db = Some(an.far.peak_db - an.mic.peak_db);
        report.mic_band_rms_db = Some(an.mic.peak_db);
        report.far_band_rms_db = Some(an.far.peak_db);
        report.mic_snr_db = Some(an.mic.snr_db);
        report.series = Some(an.series());

        // 残余 = 两路峰值时刻之差 − 采集时用的固定参考延时（`CALIB_REF_MS`）
        let residual_ms = an.delay_raw_ms - CALIB_REF_MS;
        // 自校正：新延时 = 旧延时 + 残余，取整到 10 ms（= 1 hop 的整数倍）
        let delay_ms =
          (((current_delay_ms + residual_ms) / 10.0).round() * 10.0).clamp(-1000.0, 1000.0);
        report.delay_ms = Some(delay_ms);

        let gap = an.mic.peak_db - an.far.peak_db;
        // 两端各自归一到同一个**带内**目标（-20 dBFS），AEC 模型才有一致的动态范围。
        // 报告同时给「应用后的带内电平」和「折算后的全频段人声电平」（-12 dBFS）。
        let target_band_db = TARGET_BAND_RMS_DB;
        let mic_gain = (TARGET_BAND_RMS_DB - an.mic.peak_db).clamp(-40.0, 40.0);
        let far_gain = (TARGET_BAND_RMS_DB - an.far.peak_db).clamp(-40.0, 40.0);
        // 判据：麦克风带内峰值要比自己的底噪高 ≥15 dB（= 确实听到了探针）。
        //
        // 不是「跟 far 差 ≤10 dB」：far 是回环数字满刻度，mic 是经过房间的声学信号，
        // 天然差几十 dB；拿两者比大小只会把「音量正常但空间大」判成失败。
        let heard = an.mic.snr_db >= MIC_SNR_MIN_DB;
        {
          let mut st = state.lock().unwrap();
          *st = if heard { State::Done } else { State::Failed };
        }
        if !heard {
          report.ok = false;
          report.verdict_code = "mic_not_heard".into();
          report.verdict_text = format!(
            "麦克风没听到探针（{:.0}–{:.0} Hz 带内只比自身底噪高 {:.0} dB，需要 ≥{:.0} dB；比远端低 {:.0} dB）",
            band.0, band.1, an.mic.snr_db, MIC_SNR_MIN_DB, -gap
          );
          report.next_step = Some(
            "把扬声器音量调到平时使用的音量；确认麦克风没有静音、AEC 行的「麦克风」没选错；再测一次。"
              .into(),
          );
          publish_report(
            &hub,
            report,
            Probe::unavailable(format!(
              "麦克风没听到探针（{:.0}–{:.0} Hz 带内只比自身底噪高 {:.0} dB，需要 ≥{:.0} dB；比远端低 {:.0} dB）：检查麦克风是否静音 / 选错设备 / 扬声器音量",
              band.0, band.1, an.mic.snr_db, MIC_SNR_MIN_DB, -gap
            )),
          );
        } else {
          report.ok = true;
          report.verdict_code = "ok".into();
          report.mic_gain_db = Some(mic_gain);
          report.far_gain_db = Some(far_gain);
          report.target_band_rms_db = Some(target_band_db);
          report.verdict_text = format!(
            "延时 {delay_ms:.1} ms（峰值匹配 {:.0}–{:.0} Hz，麦克风窗内 +{:.0} dB）｜近端 {mic_gain:+.0} dB｜远端 {far_gain:+.0} dB",
            band.0, band.1, an.mic.snr_db
          );
          publish_report(
            &hub,
            report,
            Probe::ok(format!(
              "延时 {delay_ms:.1} ms（峰值匹配 {:.0}–{:.0} Hz，麦克风窗内 +{:.0} dB）｜近端 {mic_gain:+.0} dB｜远端 {far_gain:+.0} dB",
              band.0, band.1, an.mic.snr_db
            )),
          );
        }
      })
      .ok();
  }
}

/// 探针带内的校准目标（dBFS）：**对所有设备统一的校准标准**。
///
/// 第一次实测按 -20 dBFS 归一化得到近端 +28 dB（用户判断高了 8 dB）→ 下压到 **-28 dBFS**：
/// 两端归一化到同一条线（峰值一致），增益 ≈ +20 / -24。目标不针对某支麦克风，
/// 所有人同一套标准；探针幅度保持原来的 0.9（2026-10-26 用户确认它本来就够响）。
const TARGET_BAND_RMS_DB: f64 = -28.0;

/// 判据（**只有一个**）：麦克风带内峰值要比它自己的底噪高这么多 dB，才算「听到了探针」。
const MIC_SNR_MIN_DB: f64 = 15.0;

/// 报告结构版本（AGENTS.md §1.4：只增不改名）。
pub const CALIB_SCHEMA_VERSION: u32 = 1;

/// 校准取 far 窗口用的**固定参考延时**（ms）：与当前存储的延时值无关。
///
/// 探针是一次性短信号：若用当前（可能很错的）延时取窗口，窗口会落在探针之外 → 测不到，
/// 校准无法自我纠正。改用固定的小参考延时后，测得的就是**绝对延时**（新延时 = 参考 + 残余），
/// 从任何旧值出发都一次到位。
pub const CALIB_REF_MS: f64 = 20.0;

/// 固定参考延时的采样数（列工作线程取 far 窗口用）。
pub const CALIB_REF_SAMPLES: i64 = (CALIB_REF_MS * SAMPLE_RATE as f64 / 1000.0) as i64;

/// 校准探针频带（对数扫频起止）。
const PROBE_F0: f64 = 1500.0;
const PROBE_F1: f64 = 2500.0;

/// 探针扫频时长（ms）：进报告，离线工具按它对齐图上的时间轴。
const PROBE_DUR_MS: f64 = 120.0;

/// 峰值匹配用的**唯一频窗**：高通 1.5 kHz 避开低频能量汇聚（扫频峰 + 房间低频 + 工频），
/// 低通 2.5 kHz 正好卡住探针带宽（再往上是麦克风 3.3 kHz 滚降前的过渡区，信噪比在掉）。
const PEAK_BAND: (f32, f32) = (1500.0, 2500.0);

/// 包络帧长 / 步进（21 ms / 2.7 ms）：与界面、离线工具画的是同一条曲线。
///
/// **逐帧 STFT（Hann 窗）而不是对整段做砖墙带通**：整段 FFT 滤波是循环卷积，探针的能量会
/// 绕回缓冲开头 → 图上出现「探针还没开始就先慢慢涨起来」的假前奏（2026-10-26 用户实测
/// 指出「前面起落不对劲」）。逐帧只看这一帧里真实存在的带内能量，没有这种环绕伪影。
const ENV_WIN: usize = 1024;
const ENV_HOP: usize = 128;

/// 包络平滑：13 帧 ≈ 35 ms。用户要求「超级平滑」——单帧最大值会被底噪毛刺顶上去，
/// 峰值位置就不稳；平滑之后两条曲线的峰值时刻才可复现。
const ENV_SMOOTH_FRAMES: usize = 13;

/// 报告里曲线最多保留多少点（约 4 s @2.7 ms；1.6 s 采集 ≈ 570 点 → 采样到 ≤480）。
const SERIES_MAX: usize = 480;

/// 校准结果（**唯一数据源**）：快照 `calib_report`、端点 `/debug/calib`、
/// `~/.purevox/calib_last.json` 都是它。字段只增不改名；没测到的值是 `null`，不是 0。
#[derive(Clone, Debug, Serialize)]
pub struct CalibReport {
  pub schema_version: u32,
  pub ok: bool,
  pub created_ms: u64,
  pub col: usize,
  pub row: usize,
  /// mic 设备标识（AEC 行的 `device`，原样记录）
  pub mic_device: Option<String>,
  /// far 取值（`loopback` / `loopback:<id>` / 输入设备 ID）
  pub far_device: Option<String>,
  pub band_hz: [f32; 2],
  pub probe_ms: f64,
  /// mic 带内能量峰值在采集缓冲里的时刻
  pub mic_peak_ms: Option<f64>,
  /// far 带内能量峰值在采集缓冲里的时刻
  pub far_peak_ms: Option<f64>,
  /// mic 峰值 − far 峰值（相对本次采集参考的原始延时）
  pub delay_raw_ms: Option<f64>,
  /// 最终要回填的 `far_delay_ms`（绝对延时，取整到 10 ms）
  pub delay_ms: Option<f64>,
  /// far→mic 衰减：far 峰值电平 − mic 峰值电平（正 = 麦克风收到的更弱）
  pub echo_path_db: Option<f64>,
  /// 峰值窗内 RMS（dBFS，1.5–2.5 kHz 探针带）
  pub mic_band_rms_db: Option<f64>,
  pub far_band_rms_db: Option<f64>,
  /// mic 窗内峰值比自身底噪高多少 dB
  pub mic_snr_db: Option<f64>,
  pub mic_snr_min_db: f64,
  /// 配平目标（探针带内 RMS，dBFS）：**所有设备共用的一条线**
  pub target_band_rms_db: Option<f64>,
  /// 实际回填的增益（未成功时 null：不能把「没测到」当成 0 dB 应用）
  pub mic_gain_db: Option<f64>,
  pub far_gain_db: Option<f64>,
  /// 稳定判定码：ok / mic_not_heard / far_unavailable / too_short
  pub verdict_code: String,
  /// 人话结论（诊断文本，不翻译；界面按 verdict_code 出本地化结论）
  pub verdict_text: String,
  /// 下一步可执行建议（失败时给）
  pub next_step: Option<String>,
  /// 平滑后的带内能量包络（界面画图 / 离线复现用；不是原始波形）
  pub series: Option<CalibSeries>,
  /// 原始缓冲与 JSON 的落盘位置
  pub dumps: CalibDumps,
}

#[derive(Clone, Debug, Serialize)]
pub struct CalibSeries {
  /// 包络点间隔（ms）
  pub hop_ms: f64,
  pub t_ms: Vec<f64>,
  pub mic_env_db: Vec<f64>,
  pub far_env_db: Vec<f64>,
}

#[derive(Clone, Debug, Serialize, Default)]
pub struct CalibDumps {
  pub mic: Option<String>,
  pub far: Option<String>,
  pub json: Option<String>,
}

impl CalibReport {
  fn new(col: usize, row: usize, mic_device: Option<String>, far_device: Option<String>) -> Self {
    CalibReport {
      schema_version: CALIB_SCHEMA_VERSION,
      ok: false,
      created_ms: now_ms(),
      col,
      row,
      mic_device,
      far_device,
      band_hz: [PEAK_BAND.0, PEAK_BAND.1],
      probe_ms: PROBE_DUR_MS,
      mic_peak_ms: None,
      far_peak_ms: None,
      delay_raw_ms: None,
      delay_ms: None,
      echo_path_db: None,
      mic_band_rms_db: None,
      far_band_rms_db: None,
      mic_snr_db: None,
      mic_snr_min_db: MIC_SNR_MIN_DB,
      target_band_rms_db: None,
      mic_gain_db: None,
      far_gain_db: None,
      // 还在跑时不该被发布；调用方一定会设一个确定的码
      verdict_code: "pending".into(),
      verdict_text: String::new(),
      next_step: None,
      series: None,
      dumps: CalibDumps::default(),
    }
  }
}

/// 发布报告：落盘 `~/.purevox/calib_last.json` → 进调试快照（端点序列化的是同一份）。
///
/// 写入顺序无所谓（快照拿到的是内存里的结构），但落盘在发布前，保证「看到结果时磁盘上也有」。
fn publish_report(hub: &SharedHub, mut report: CalibReport, status: Probe<String>) {
  if let Ok(dir) = crate::config::config_dir() {
    let path = dir.join("calib_last.json");
    report.dumps.json = Some(path.display().to_string());
    match serde_json::to_string_pretty(&report) {
      Ok(s) => {
        if let Err(e) = std::fs::write(&path, s) {
          crate::debug::events::log("warn", "calib", format!("写 {} 失败：{e}", path.display()));
        }
      }
      Err(e) => crate::debug::events::log("warn", "calib", format!("序列化校准报告失败：{e}")),
    }
  }
  hub.set_calib_report(Probe::ok(report));
  hub.set_calib(status);
}

/// 把最近一次校准的原始缓冲落到 `~/.purevox/calib_last_{mic,far}.f32`：
/// 离线排查（`tools/automation/aec_probe.py --from-dump`）与「到底谁没出声」全靠它。
fn dump_calib(mic: &[f32], far: &[f32]) -> (Option<String>, Option<String>) {
  let Ok(dir) = crate::config::config_dir() else {
    return (None, None);
  };
  let _ = std::fs::create_dir_all(&dir);
  let bytes = |v: &[f32]| v.iter().flat_map(|x| x.to_le_bytes()).collect::<Vec<u8>>();
  let mp = dir.join("calib_last_mic.f32");
  let fp = dir.join("calib_last_far.f32");
  let _ = std::fs::write(&mp, bytes(mic));
  let _ = std::fs::write(&fp, bytes(far));
  (
    Some(mp.display().to_string()),
    Some(fp.display().to_string()),
  )
}

/// 一条带内能量包络：峰值时刻 / 峰值电平 / 窗内 SNR + 画图用的曲线。
struct BandCurve {
  peak_ms: f64,
  /// 峰值窗内 RMS（dBFS）
  peak_db: f64,
  /// 峰值比整体帧能量中位数高多少 dB
  snr_db: f64,
  t_ms: Vec<f64>,
  env_db: Vec<f64>,
}

/// 带内「能量峰值时刻」：**逐帧 STFT**（1024 点 Hann，hop 128）→ 带内功率 →
/// 13 帧（≈35 ms）滑动平均 → 取最大帧。
///
/// 不再对整段做砖墙 FFT 带通：整段滤波是循环卷积，探针能量会绕到缓冲开头，
/// 图上出现「探针还没到就先涨起来」的假前奏（2026-10-26 用户实测指出「前面起落不对劲」）。
/// 逐帧只看这一帧里真实存在的带内能量，没有环绕伪影；帧内加窗后按窗功率归一，dBFS 仍可比。
fn band_curve(x: &[f32], lo: f32, hi: f32) -> Option<BandCurve> {
  let n = x.len();
  if n < ENV_WIN {
    return None;
  }
  let win: Vec<f32> = (0..ENV_WIN)
    .map(|i| 0.5 - 0.5 * (2.0 * std::f32::consts::PI * i as f32 / (ENV_WIN - 1) as f32).cos())
    .collect();
  let mean_w2 = win.iter().map(|v| v * v).sum::<f32>() / ENV_WIN as f32;
  let bin_hz = SAMPLE_RATE as f32 / ENV_WIN as f32;
  let k_lo = (lo / bin_hz).ceil() as usize;
  let k_hi = ((hi / bin_hz).floor() as usize).min(ENV_WIN / 2);
  if k_hi <= k_lo {
    return None;
  }
  let mut planner = FftPlanner::<f32>::new();
  let fft = planner.plan_fft_forward(ENV_WIN);
  let mut buf = vec![Complex::new(0.0f32, 0.0f32); ENV_WIN];
  let frames = (n - ENV_WIN) / ENV_HOP + 1;
  let mut energies = Vec::with_capacity(frames);
  for f in 0..frames {
    let off = f * ENV_HOP;
    for i in 0..ENV_WIN {
      buf[i] = Complex::new(x[off + i] * win[i], 0.0);
    }
    fft.process(&mut buf);
    let power: f32 = buf[k_lo..=k_hi].iter().map(|c| c.norm_sqr()).sum();
    // 单边谱 ×2；除以 N² 与窗功率，得到「未加窗信号的带内均方」→ 10log10 即 dBFS。
    // 下限 -120 dBFS：数字静音（回环在探针之前真的全是 0）不该把图拉到 -200。
    let e = (2.0 * power / (ENV_WIN as f32 * ENV_WIN as f32 * mean_w2)).max(1e-12);
    energies.push(e as f64);
  }
  if energies.is_empty() {
    return None;
  }
  let half = ENV_SMOOTH_FRAMES / 2;
  let smooth: Vec<f64> = (0..energies.len())
    .map(|idx| {
      let a = idx.saturating_sub(half);
      let b = (idx + half + 1).min(energies.len());
      energies[a..b].iter().sum::<f64>() / (b - a) as f64
    })
    .collect();
  let (bi, bv) =
    smooth.iter().enumerate().fold(
      (0usize, 0.0f64),
      |acc, (idx, &v)| if v > acc.1 { (idx, v) } else { acc },
    );
  let mut sorted = energies;
  sorted.sort_by(|a, b| a.partial_cmp(b).unwrap_or(std::cmp::Ordering::Equal));
  let med = sorted[sorted.len() / 2].max(1e-20);
  let snr_db = 10.0 * (bv.max(1e-20) / med).log10();
  let peak_db = 10.0 * bv.max(1e-20).log10();
  let t_ms: Vec<f64> = (0..smooth.len())
    .map(|idx| (idx * ENV_HOP + ENV_WIN / 2) as f64 * 1000.0 / SAMPLE_RATE as f64)
    .collect();
  let env_db: Vec<f64> = smooth.iter().map(|e| 10.0 * e.max(1e-20).log10()).collect();
  Some(BandCurve {
    peak_ms: t_ms[bi],
    peak_db,
    snr_db,
    t_ms,
    env_db,
  })
}

/// 一次校准的测量结果（算法唯一入口）。
struct Analysis {
  mic: BandCurve,
  far: BandCurve,
  delay_raw_ms: f64,
}

impl Analysis {
  /// 报告里的曲线：降采样到 ≤ `SERIES_MAX` 点（峰值时刻另有精确字段，不靠曲线取）。
  fn series(&self) -> CalibSeries {
    let n = self
      .mic
      .t_ms
      .len()
      .min(self.mic.env_db.len())
      .min(self.far.env_db.len());
    let stride = n.div_ceil(SERIES_MAX).max(1);
    let mut t_ms = Vec::with_capacity(n / stride + 1);
    let mut mic_env_db = Vec::with_capacity(n / stride + 1);
    let mut far_env_db = Vec::with_capacity(n / stride + 1);
    for i in (0..n).step_by(stride) {
      t_ms.push(self.mic.t_ms[i]);
      mic_env_db.push(self.mic.env_db[i]);
      far_env_db.push(self.far.env_db[i]);
    }
    CalibSeries {
      hop_ms: 1000.0 * ENV_HOP as f64 * stride as f64 / SAMPLE_RATE as f64,
      t_ms,
      mic_env_db,
      far_env_db,
    }
  }
}

/// 峰值匹配（**唯一算法**）：带通 → 各自找窗内能量峰值时刻 → 差即时延。
///
/// 不做互相关、不算相关系数。判据只有一条：麦克风窗内的峰值要明显高于它自己的底噪。
fn analyze(mic: &[f32], far: &[f32], band: (f32, f32)) -> Option<Analysis> {
  let n = mic.len().min(far.len());
  if n < SAMPLE_RATE as usize / 5 {
    return None;
  }
  let m = band_curve(&mic[..n], band.0, band.1)?;
  let f = band_curve(&far[..n], band.0, band.1)?;
  Some(Analysis {
    delay_raw_ms: m.peak_ms - f.peak_ms,
    mic: m,
    far: f,
  })
}

/// 校准探针：**窄带** 1500→2500 Hz 对数扫频（120 ms，Hann 包络，0.9 幅度），前后留静音。
///
/// 为什么不是 800→6000 的宽扫频：那样能量铺满整个频带，频谱是个大鼓包、
/// 峰值被包络和低频倾斜拖着走（实测峰落在 2 kHz 之前、而且不尖）。窄带之后：
/// - 频谱是一个 **1.5–2.5 kHz 的尖峰**，峰值位置稳定在 2 kHz 附近；
/// - 峰值匹配窗只要开 1500–2500，避开低频能量汇聚处与麦克风 3.3 kHz 之后的滚降；
/// - 仍是扫频（不是单音），房间反射在不同频点出现，不会和直达声在同一个频点叠成双峰。
pub fn make_probe() -> Vec<f32> {
  const SR: usize = SAMPLE_RATE as usize;
  let head = SR * 100 / 1000;
  let dur = SR * PROBE_DUR_MS as usize / 1000;
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
/// - `net_probe`：远端是**网络出站**（`remote_mic` 的 AEC、`far_device = network`）时，
///   探针注入网络出站（手机放、手机麦克风收），`devices` / `far_sink` 应为空。
pub fn play_probe(hub: SharedHub, devices: Vec<String>, far_sink: Option<String>, net_probe: bool) {
  let probe = Arc::new(make_probe());
  // 网络 AEC 的 far = 网络出站：探针只能从网络发出去（手机放、手机麦克风收）。
  // 按真实出站的 10 ms 节奏一格一格推 —— 一次倒进去会撞上出站交接格（4 格）被丢。
  if net_probe {
    let p = probe.clone();
    let net = crate::net::hub();
    let h = hub.clone();
    std::thread::Builder::new()
      .name("probe-net".into())
      .spawn(move || {
        // 等客户端环预热（与播放线程的 150 ms 对齐，起播前还有预缓冲）
        std::thread::sleep(std::time::Duration::from_millis(150));
        for chunk in p.chunks(crate::audio::HOP) {
          let mut hop = [0.0f32; crate::audio::HOP];
          hop[..chunk.len()].copy_from_slice(chunk);
          net.push_probe_hop(&hop);
          std::thread::sleep(std::time::Duration::from_millis(10));
        }
        if net.sub_count() == 0 {
          h.push_ui(
            "warn",
            "校准探针已注入网络出站，但没有客户端在订阅音频：手机端不会放，麦克风也收不到".into(),
          );
        }
      })
      .ok();
  }
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
    let an = analyze(&mic, &far, PEAK_BAND).expect("应当能量到");
    assert!(an.mic.snr_db > 20.0, "窗内 SNR 太低：{}", an.mic.snr_db);
    assert!(
      (an.delay_raw_ms - 100.0).abs() < 5.0,
      "期望 +100 ms，得到 {} ms",
      an.delay_raw_ms
    );
  }

  /// 绝对测量公式：mic(i) = far(i − D)，窗口取 far(i − r) → 残余 = D − r，新延时 = D。
  #[test]
  fn absolute_delay_from_fixed_reference() {
    let far = make_probe();
    let d = (0.06 * SAMPLE_RATE as f64) as usize; // 真实延时 60 ms
    let r = (CALIB_REF_MS / 1000.0 * SAMPLE_RATE as f64) as usize; // 参考 20 ms
    // 模拟列线程：喂 mic(i) 与 far(i − r)
    let n = far.len() - d;
    let mic = &far[..n];
    let win = &far[d - r..d - r + n];
    let an = analyze(mic, win, PEAK_BAND).expect("应当能量到");
    assert!(an.mic.snr_db > 20.0, "窗内 SNR 太低：{}", an.mic.snr_db);
    let new_ms = CALIB_REF_MS + an.delay_raw_ms;
    assert!((new_ms - 60.0).abs() < 5.0, "期望 60 ms，得到 {new_ms} ms");
  }

  /// 报告曲线与峰值同源：点数受限、时间轴单调、峰值附近就是最大值。
  #[test]
  fn series_matches_peak_and_is_downsampled() {
    let far = make_probe();
    let k = (0.1 * SAMPLE_RATE as f64) as usize;
    let mut mic = vec![0.0f32; far.len()];
    mic[k..].copy_from_slice(&far[..far.len() - k]);
    let an = analyze(&mic, &far, PEAK_BAND).expect("应当能量到");
    let s = an.series();
    assert!(!s.t_ms.is_empty() && s.t_ms.len() <= SERIES_MAX);
    assert_eq!(s.t_ms.len(), s.mic_env_db.len());
    assert_eq!(s.t_ms.len(), s.far_env_db.len());
    assert!(s.t_ms.windows(2).all(|w| w[1] > w[0]), "时间轴必须单调");
    let (i, v) = s
      .mic_env_db
      .iter()
      .enumerate()
      .fold(
        (0, f64::NEG_INFINITY),
        |a, (i, &v)| if v > a.1 { (i, v) } else { a },
      );
    assert!(
      (s.t_ms[i] - an.mic.peak_ms).abs() <= 100.0,
      "曲线峰值离精确峰值太远：{v}"
    );
  }

  /// 报告结构（AGENTS §1.4）：带 schema 版本；没测到的值是 null 而不是 0；
  /// 带内 / 人声目标与换算偏移都要随报告给出（脚本与 AI 直接读）。
  #[test]
  fn report_json_uses_schema_and_nulls() {
    let mut r = CalibReport::new(0, 1, Some("wasapi:mic".into()), Some("loopback".into()));
    r.verdict_code = "far_unavailable".into();
    r.verdict_text = "远端窗口一直不可用".into();
    let v = serde_json::to_value(&r).expect("报告必须可序列化");
    assert_eq!(v["schema_version"], CALIB_SCHEMA_VERSION);
    assert_eq!(v["verdict_code"], "far_unavailable");
    assert!(v["mic_peak_ms"].is_null(), "没测到必须 null，不能是 0");
    assert!(v["mic_gain_db"].is_null(), "没成功时不能给出增益");
    assert_eq!(v["band_hz"][0], 1500.0);
    assert_eq!(v["band_hz"][1], 2500.0);
    assert_eq!(v["target_band_rms_db"], serde_json::Value::Null);
  }
}
