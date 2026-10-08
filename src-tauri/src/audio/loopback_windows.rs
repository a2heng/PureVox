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

//! 输出设备 WASAPI 回环采集（AEC 远端参考：设备间回声，如音响 ↔ 麦克风）。
//!
//! 直接用 Core Audio：`IAudioClient` 以 `AUDCLNT_STREAMFLAGS_LOOPBACK` 共享模式打开
//! 渲染端点（不传设备 ID 时用系统默认渲染端点）。一次把队列里的包全部取空，避免积压
//! 造成不连续；`AUDCLNT_BUFFERFLAGS_SILENT` 按静音处理；共享混音格式为 float32。
//!
//! 参考旧实现（`legacy-v2026.09.30.1944/pvplatform/audio/speaker_capture_win.py`）的行为，
//! 但用 `windows` crate 的 COM 接口重写，采集数据面复用 [`capture::worker`]。
//! COM 接口对象 `!Send`：全部在本线程（轮询线程）内创建与释放，格式经通道回传。

use std::sync::atomic::{AtomicBool, Ordering::Relaxed};
use std::sync::{Arc, mpsc};
use std::time::Duration;

/// far 网格按**实时**推进（不足补零）：WASAPI 回环在渲染端点空闲时几乎不回调，
/// 不按实时推进的话 far 序号会越落越后（见 `engine/aec.rs` 的 `spawn_far_pump`）。
pub const FAR_GRID_REALTIME: bool = true;

use rtrb::RingBuffer;
use windows::Win32::Media::Audio::{
  AUDCLNT_BUFFERFLAGS_SILENT, AUDCLNT_SHAREMODE_SHARED, AUDCLNT_STREAMFLAGS_LOOPBACK,
  IAudioCaptureClient, IAudioClient, IMMDevice, IMMDeviceEnumerator, MMDeviceEnumerator,
  WAVEFORMATEX, eConsole, eRender,
};
use windows::Win32::System::Com::{
  CLSCTX_ALL, COINIT_MULTITHREADED, CoCreateInstance, CoInitializeEx, CoTaskMemFree,
};

use super::capture::{self, CallbackStats, Capture, OPEN_TIMEOUT, SourceInfo};
use super::fanout::Fanout;
use crate::debug::SharedHub;

/// 回环缓冲区时长（100 ns 单位）：10 ms，与旧实现一致。
const BUFFER_DURATION: i64 = 100_000;

/// 打开输出设备的回环并启动采集线程。
/// `output_id`：渲染端点 ID（cpal 形如 `wasapi:{...}`）；`None` 用系统默认渲染端点。
pub fn spawn(hub: SharedHub, output_id: Option<String>, tag: String) -> Result<Capture, String> {
  let stop = Arc::new(AtomicBool::new(false));
  let (tx, rx) = mpsc::channel::<Result<Arc<Fanout>, String>>();
  let stop2 = stop.clone();
  let name = tag.clone();
  let join = std::thread::Builder::new()
    .name(format!("loopback-{name}"))
    .spawn(move || run(hub, output_id, tag, stop2, tx))
    .map_err(|e| format!("创建回环采集线程失败：{e}"))?;
  match rx.recv_timeout(OPEN_TIMEOUT) {
    Ok(Ok(fanout)) => Ok(Capture {
      handle: super::WorkerHandle::new(stop, join),
      fanout,
    }),
    Ok(Err(e)) => {
      let _ = join.join();
      Err(e)
    }
    Err(_) => {
      stop.store(true, Relaxed);
      Err("打开回环超时（5 s）".into())
    }
  }
}

fn run(
  hub: SharedHub,
  output_id: Option<String>,
  tag: String,
  stop: Arc<AtomicBool>,
  tx: mpsc::Sender<Result<Arc<Fanout>, String>>,
) {
  let stats = Arc::new(CallbackStats::new());
  let (prod, cons) = RingBuffer::<f32>::new(48_000); // 1 s 单声道
  let (fmt_tx, fmt_rx) = mpsc::channel::<Result<(u32, u16, u16, String), String>>();

  let stop2 = stop.clone();
  let stats2 = stats.clone();
  let poll = std::thread::Builder::new()
    .name("loopback-poll".into())
    .spawn(move || open_and_poll(output_id, prod, fmt_tx, stop2, stats2))
    .expect("spawn loopback poll");

  let (rate, channels, bits, name) = match fmt_rx.recv_timeout(OPEN_TIMEOUT) {
    Ok(Ok(v)) => v,
    Ok(Err(e)) => {
      let _ = tx.send(Err(e));
      return;
    }
    Err(_) => {
      stop.store(true, Relaxed);
      let _ = tx.send(Err("打开回环超时（5 s）".into()));
      return;
    }
  };
  let info = SourceInfo {
    name,
    device_id: "loopback".into(),
    native_rate: rate,
    channels,
    sample_format: if bits == 32 {
      "f32".into()
    } else {
      "i16".into()
    },
  };
  capture::worker(
    hub,
    tag,
    info,
    capture::WorkerSrc {
      cons,
      stats,
      keep: poll,
      stop,
    },
    tx,
  );
}

/// 打开回环并把数据写进无锁环（本线程持有全部 COM 对象）。格式经 `fmt_tx` 回传。
fn open_and_poll(
  output_id: Option<String>,
  mut prod: rtrb::Producer<f32>,
  fmt_tx: mpsc::Sender<Result<(u32, u16, u16, String), String>>,
  stop: Arc<AtomicBool>,
  stats: Arc<CallbackStats>,
) {
  unsafe {
    let _ = CoInitializeEx(None, COINIT_MULTITHREADED);
  }
  let opened = (|| -> Result<_, String> {
    let device = resolve_device(output_id.as_deref())?;
    let client: IAudioClient = unsafe { device.Activate(CLSCTX_ALL, None) }
      .map_err(|e| format!("激活 IAudioClient 失败：{e}"))?;
    let mix = unsafe { client.GetMixFormat() }.map_err(|e| format!("GetMixFormat 失败：{e}"))?;
    let (rate, ch, bits) = unsafe { read_format(mix) };
    if bits != 32 && bits != 16 {
      unsafe { CoTaskMemFree(Some(mix as *const _)) };
      return Err(format!("回环混音格式位深不支持：{bits} bit（仅 16/32）"));
    }
    let init = unsafe {
      client.Initialize(
        AUDCLNT_SHAREMODE_SHARED,
        AUDCLNT_STREAMFLAGS_LOOPBACK,
        BUFFER_DURATION,
        0,
        mix,
        None,
      )
    };
    unsafe { CoTaskMemFree(Some(mix as *const _)) };
    init.map_err(|e| format!("初始化回环失败：{e}"))?;
    let capture: IAudioCaptureClient =
      unsafe { client.GetService() }.map_err(|e| format!("获取 IAudioCaptureClient 失败：{e}"))?;
    unsafe { client.Start() }.map_err(|e| format!("启动回环失败：{e}"))?;
    let name = match output_id.as_deref() {
      Some(id) => format!("回环：{}", short_id(id)),
      None => "回环：系统默认输出".to_string(),
    };
    Ok((client, capture, rate, ch, bits, name))
  })();

  let (client, capture, rate, ch, bits, name) = match opened {
    Ok(v) => v,
    Err(e) => {
      let _ = fmt_tx.send(Err(e));
      return;
    }
  };
  let _ = fmt_tx.send(Ok((rate, ch, bits, name)));

  let chn = ch.max(1) as usize;
  while !stop.load(Relaxed) {
    let mut drained = false;
    loop {
      if stop.load(Relaxed) {
        break;
      }
      let frames = match unsafe { capture.GetNextPacketSize() } {
        Ok(f) => f,
        Err(_) => break,
      };
      if frames == 0 {
        break;
      }
      let mut pdata: *mut u8 = std::ptr::null_mut();
      let mut n = 0u32;
      let mut flags = 0u32;
      if unsafe { capture.GetBuffer(&mut pdata, &mut n, &mut flags, None, None) }.is_err() {
        break;
      }
      if n > 0 {
        stats.on_block(n);
        let silent = flags & (AUDCLNT_BUFFERFLAGS_SILENT.0 as u32) != 0;
        let total = n as usize * chn;
        if silent || pdata.is_null() {
          for _ in 0..n {
            let _ = prod.push(0.0);
          }
        } else if bits == 32 {
          let s = unsafe { std::slice::from_raw_parts(pdata as *const f32, total) };
          for f in 0..n as usize {
            let mut acc = 0.0f32;
            for c in 0..chn {
              acc += s[f * chn + c];
            }
            let _ = prod.push(acc / chn as f32);
          }
        } else {
          let s = unsafe { std::slice::from_raw_parts(pdata as *const i16, total) };
          for f in 0..n as usize {
            let mut acc = 0.0f32;
            for c in 0..chn {
              acc += s[f * chn + c] as f32 / 32768.0;
            }
            let _ = prod.push(acc / chn as f32);
          }
        }
        let _ = unsafe { capture.ReleaseBuffer(n) };
      }
      drained = true;
    }
    if !drained {
      std::thread::sleep(Duration::from_millis(1));
    }
  }
  unsafe {
    let _ = client.Stop();
  }
}

fn resolve_device(id: Option<&str>) -> Result<IMMDevice, String> {
  let enumerator: IMMDeviceEnumerator =
    unsafe { CoCreateInstance(&MMDeviceEnumerator, None, CLSCTX_ALL) }
      .map_err(|e| format!("创建 MMDeviceEnumerator 失败：{e}"))?;
  match id.filter(|s| !s.is_empty()) {
    Some(s) => {
      let sid = s.strip_prefix("wasapi:").unwrap_or(s);
      unsafe { enumerator.GetDevice(&windows::core::HSTRING::from(sid)) }
        .map_err(|e| format!("找不到渲染端点 {sid}：{e}"))
    }
    None => unsafe { enumerator.GetDefaultAudioEndpoint(eRender, eConsole) }
      .map_err(|e| format!("取默认渲染端点失败：{e}")),
  }
}

unsafe fn read_format(wfx: *const WAVEFORMATEX) -> (u32, u16, u16) {
  let f = unsafe { &*wfx };
  (f.nSamplesPerSec, f.nChannels, f.wBitsPerSample)
}

fn short_id(id: &str) -> String {
  id.chars()
    .rev()
    .take(8)
    .collect::<String>()
    .chars()
    .rev()
    .collect()
}
