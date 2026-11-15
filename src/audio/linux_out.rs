// PureVox — AI 麦克风降噪工具
// Copyright (C) 2024-2026 a2heng <752848283@qq.com>
//
// PureVox is licensed under the GNU General Public License v3.0 or
// later (GPL-3.0-or-later).  See LICENSE for details.
//
// SPDX-License-Identifier: GPL-3.0-or-later

//! Linux 输出后端选择：ALSA 硬件走 cpal，`pulse:<sink>`（蓝牙 / 虚拟声卡）走 pacat。

use std::sync::Arc;
use std::sync::atomic::AtomicBool;

use rtrb::Consumer;

use super::capture::CallbackStats;
use super::out::Format;

pub enum Stream {
  Cpal(super::cpal_out::Stream),
  Pulse(super::pulse_out::Stream),
}

pub fn probe(device_id: &str) -> Result<Format, String> {
  if device_id.starts_with("pulse:") {
    super::pulse_out::probe(device_id)
  } else {
    super::cpal_out::probe(device_id)
  }
}

pub fn start(
  device_id: String,
  format: Format,
  cons: Consumer<f32>,
  stats: Arc<CallbackStats>,
  primed: Arc<AtomicBool>,
) -> Result<Stream, String> {
  if device_id.starts_with("pulse:") {
    super::pulse_out::start(device_id, format, cons, stats, primed).map(Stream::Pulse)
  } else {
    super::cpal_out::start(device_id, format, cons, stats, primed).map(Stream::Cpal)
  }
}
