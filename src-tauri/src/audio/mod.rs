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

//! 音频：设备采集、重采样、10 ms hop、输出（AGENTS.md 第 3 节硬约束）。
//!
//! 输入：设备原生格式 → 回调线程下混单声道写无锁环 → 工作线程重采样到 48 kHz → 切 hop → 扇出。
//! 输出：订阅一个源（输入采集 / 测试音）的 48 kHz hop → 工作线程按时钟伺服微调比例重采样到
//! 设备采样率写无锁环 → 设备回调（唯一主时钟）取样。

pub mod capture;
pub mod fanout;
pub mod meter;
pub mod playback;
pub mod resampler;
pub mod tone;

use std::collections::HashMap;
use std::sync::atomic::{AtomicBool, Ordering::Relaxed};
use std::sync::{Arc, Mutex, OnceLock};
use std::thread::JoinHandle;

use crate::debug::SharedHub;
use fanout::Fanout;

/// 引擎内部采样率。
pub const SAMPLE_RATE: u32 = 48_000;
/// 10 ms hop，按时间派生（48 kHz 下 480 样本）。
pub const HOP: usize = (SAMPLE_RATE / 100) as usize;

/// 输出流的信号源 ID：测试音。其余源 ID 为输入设备 ID。
pub const SOURCE_TONE: &str = "tone";

/// 工作线程句柄：置停止标志并等待线程退出。
pub struct WorkerHandle {
  stop: Arc<AtomicBool>,
  join: Option<JoinHandle<()>>,
}

impl WorkerHandle {
  pub fn new(stop: Arc<AtomicBool>, join: JoinHandle<()>) -> Self {
    WorkerHandle { stop, join: Some(join) }
  }

  pub fn stop(mut self) {
    self.stop.store(true, Relaxed);
    if let Some(j) = self.join.take() {
      let _ = j.join();
    }
  }
}

/// 音频流管理：按设备 ID 启停输入采集与输出，每个设备每个方向最多一路。
pub struct AudioManager {
  hub: SharedHub,
  captures: Mutex<HashMap<String, (WorkerHandle, Arc<Fanout>)>>,
  playbacks: Mutex<HashMap<String, WorkerHandle>>,
  tone: OnceLock<Arc<Fanout>>,
}

impl AudioManager {
  pub fn new(hub: SharedHub) -> Self {
    AudioManager {
      hub,
      captures: Mutex::new(HashMap::new()),
      playbacks: Mutex::new(HashMap::new()),
      tone: OnceLock::new(),
    }
  }

  /// 打开输入设备并开始采集（阻塞到打开成功或失败，最多数秒；勿在 UI 线程调用）。
  pub fn start_capture(&self, device_id: &str) -> Result<(), String> {
    if self.captures.lock().unwrap().contains_key(device_id) {
      return Ok(());
    }
    let started = capture::spawn(self.hub.clone(), device_id.to_string())?;
    self.captures.lock().unwrap().insert(device_id.to_string(), started);
    Ok(())
  }

  pub fn stop_capture(&self, device_id: &str) {
    let entry = self.captures.lock().unwrap().remove(device_id);
    if let Some((h, _)) = entry {
      h.stop();
    }
  }

  /// 在输出设备上播放某个源（`SOURCE_TONE` 或正在采集的输入设备 ID）；已在播放则切换源。
  pub fn start_playback(&self, device_id: &str, source: &str) -> Result<(), String> {
    let fanout = if source == SOURCE_TONE {
      self.tone.get_or_init(tone::spawn).clone()
    } else {
      match self.captures.lock().unwrap().get(source) {
        Some((_, f)) => f.clone(),
        None => return Err("信号源未运行：请先启动该输入设备的采集".into()),
      }
    };
    self.stop_playback(device_id);
    let h = playback::spawn(self.hub.clone(), device_id.to_string(), fanout)?;
    self.playbacks.lock().unwrap().insert(device_id.to_string(), h);
    Ok(())
  }

  pub fn stop_playback(&self, device_id: &str) {
    let h = self.playbacks.lock().unwrap().remove(device_id);
    if let Some(h) = h {
      h.stop();
    }
  }
}
