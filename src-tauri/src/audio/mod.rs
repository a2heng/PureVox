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

//! 音频：设备采集、重采样到 48 kHz、10 ms hop 切片。
//!
//! 数据流（AGENTS.md 第 3 节硬约束）：
//! 设备原生格式 → 回调线程下混单声道写无锁环 → 工作线程 rubato 重采样到 48 kHz
//! → 按 HOP（480 样本 = 10 ms）切片 → 下游（目前只有调试测量）。

pub mod capture;
pub mod meter;
pub mod resampler;

use std::collections::HashMap;
use std::sync::Mutex;

use crate::debug::SharedHub;

/// 引擎内部采样率。
pub const SAMPLE_RATE: u32 = 48_000;
/// 10 ms hop，按时间派生（48 kHz 下 480 样本）。
pub const HOP: usize = (SAMPLE_RATE / 100) as usize;

/// 采集流管理：按设备 ID 启停，同一设备只开一路。
pub struct CaptureManager {
  hub: SharedHub,
  streams: Mutex<HashMap<String, capture::CaptureHandle>>,
}

impl CaptureManager {
  pub fn new(hub: SharedHub) -> Self {
    CaptureManager { hub, streams: Mutex::new(HashMap::new()) }
  }

  /// 打开设备并开始采集（阻塞到设备打开成功或失败，最多数秒；勿在 UI 线程调用）。
  pub fn start(&self, device_id: &str) -> Result<(), String> {
    if self.streams.lock().unwrap().contains_key(device_id) {
      return Ok(());
    }
    let handle = capture::spawn(self.hub.clone(), device_id.to_string())?;
    self.streams.lock().unwrap().insert(device_id.to_string(), handle);
    Ok(())
  }

  pub fn stop(&self, device_id: &str) {
    let handle = self.streams.lock().unwrap().remove(device_id);
    if let Some(h) = handle {
      h.stop();
    }
  }
}
