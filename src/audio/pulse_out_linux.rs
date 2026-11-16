// PureVox — AI 麦克风降噪工具
// Copyright (C) 2024-2026 a2heng <752848283@qq.com>
//
// PureVox is licensed under the GNU General Public License v3.0 or
// later (GPL-3.0-or-later).  See LICENSE for details.
//
// SPDX-License-Identifier: GPL-3.0-or-later

//! Linux 输出后端：`pulse:<sink>` → `pacat --playback`。
//!
//! 为什么不用 cpal/ALSA：蓝牙耳机、PureVox 虚拟声卡（`purevox_out`）都不是 ALSA
//! 硬件节点，`plughw` 根本打不开；它们只存在于 PipeWire/Pulse 里。`pacat` 是
//! `parec`（输入侧）的对称实现，写入阻塞本身提供设备节拍，延时由 `--latency-msec`
//! 控制。
//!
//! 输出固定协商 `float32le 48 kHz 2ch`：引擎内部已经是 F32 48k，pacat 负责和
//! 目标 sink 的原生格式转换；2ch 是耳机/虚拟声卡的通用口径。

use std::io::Write;
use std::process::{Child, Command, Stdio};
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::thread::JoinHandle;

use rtrb::Consumer;

use super::capture::CallbackStats;
use super::out::{Format, Sample};

const RATE: u32 = 48_000;
const CHANNELS: u16 = 2;
const PERIOD_FRAMES: u32 = RATE / 100;

/// 已启动的 pacat 输出；析构即停。
pub struct Stream {
  child: Child,
  stop: Arc<AtomicBool>,
  thread: Option<JoinHandle<()>>,
}

pub fn probe(device_id: &str) -> Result<Format, String> {
  let sink = device_id
    .strip_prefix("pulse:")
    .filter(|s| !s.is_empty())
    .ok_or_else(|| format!("不是 Pulse 输出：{device_id}"))?;
  // 只验证名字非空；pacat 会在 start 时真正解析目标。
  let _ = sink;
  Ok(Format {
    rate: RATE,
    channels: CHANNELS,
    bits: 32,
    sample: Sample::F32,
    period_frames: PERIOD_FRAMES,
    low_latency: true,
    engine_convert: false,
  })
}

pub fn start(
  device_id: String,
  format: Format,
  mut cons: Consumer<f32>,
  stats: Arc<CallbackStats>,
  primed: Arc<AtomicBool>,
) -> Result<Stream, String> {
  let sink = device_id
    .strip_prefix("pulse:")
    .filter(|s| !s.is_empty())
    .ok_or_else(|| format!("不是 Pulse 输出：{device_id}"))?
    .to_string();
  let mut child = Command::new("pacat")
    .args([
      "--playback",
      "--device",
      &sink,
      "--format=float32le",
      &format!("--rate={}", format.rate.max(1)),
      &format!("--channels={}", format.channels.max(1)),
      "--latency-msec=20",
    ])
    .stdin(Stdio::piped())
    .stdout(Stdio::null())
    .stderr(Stdio::null())
    .spawn()
    .map_err(|e| format!("启动 pacat 失败：{e}（需要 pulseaudio-utils）"))?;
  let mut stdin = child
    .stdin
    .take()
    .ok_or_else(|| "pacat 没有 stdin".to_string())?;
  let stop = Arc::new(AtomicBool::new(false));
  let stop_thread = stop.clone();
  let ch = format.channels.max(1) as usize;
  let period = format.period_frames.max(1) as usize;
  let thread = std::thread::Builder::new()
    .name("pulse-out".into())
    .spawn(move || {
      let mut buf = vec![0.0f32; period * ch];
      let mut bytes = Vec::with_capacity(buf.len() * 4);
      let mut started = false;
      while !stop_thread.load(Ordering::Relaxed) {
        let mut missing = 0u64;
        for slot in &mut buf {
          *slot = cons.pop().unwrap_or_else(|_| {
            missing += 1;
            0.0
          });
        }
        // 预热前环本来就是空的，不算 xrun；预热后的空样本才是真的欠载。
        if missing > 0 && primed.load(Ordering::Relaxed) {
          stats.xruns.fetch_add(missing, Ordering::Relaxed);
        }
        bytes.clear();
        for s in &buf {
          bytes.extend_from_slice(&s.to_le_bytes());
        }
        if stdin.write_all(&bytes).is_err() {
          stats.on_error("pacat 写入失败（sink 可能已断开）".into());
          break;
        }
        stats.on_block(period as u32);
        if !started {
          started = true;
          primed.store(true, Ordering::Relaxed);
        }
      }
    })
    .map_err(|e| format!("启动输出线程失败：{e}"))?;
  Ok(Stream {
    child,
    stop,
    thread: Some(thread),
  })
}

impl Drop for Stream {
  fn drop(&mut self) {
    self.stop.store(true, Ordering::Relaxed);
    // 杀掉 pacat 会关闭 stdin，写线程随之退出。
    let _ = self.child.kill();
    let _ = self.child.wait();
    if let Some(t) = self.thread.take() {
      let _ = t.join();
    }
  }
}
