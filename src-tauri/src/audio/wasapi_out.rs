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

//! Windows 输出：直接走 WASAPI（不再经 cpal）。
//!
//! 为什么自写：cpal 0.18 的 WASAPI 后端只会 `IAudioClient::Initialize`，
//! 回调周期被引擎钉在 `GetDevicePeriod()`（实测 480 帧 / 10 ms），
//! 而且还会把两次回调并成一个 22 ms 的大请求。`IAudioClient3::InitializeSharedAudioStream`
//! 允许把周期降到端点的 `min`（VB-Cable 实测 128 帧 / 2.67 ms），本模块优先走它，
//! 端点不支持时退回普通共享模式（行为与 cpal 一致，只是少了 cpal 那层）。
//!
//! 数据面不变：工作线程往无锁环里推设备采样率的单声道样本，回调按帧取一个、
//! 复制到所有声道。回调里不做策略，取不到就补静音并记欠载。

use std::sync::atomic::{AtomicBool, Ordering::Relaxed};
use std::sync::{Arc, mpsc};
use std::thread::JoinHandle;

use rtrb::Consumer;
use windows::core::Interface;
use windows::Win32::Foundation::{CloseHandle, WAIT_OBJECT_0, WAIT_TIMEOUT};
use windows::Win32::Media::Audio::{
  AUDCLNT_SHAREMODE_SHARED, AUDCLNT_STREAMFLAGS_EVENTCALLBACK, IAudioClient, IAudioClient3,
  IAudioRenderClient, WAVEFORMATEX,
};
use windows::Win32::System::Com::{
  CLSCTX_ALL, COINIT_MULTITHREADED, CoInitializeEx, CoTaskMemFree,
};
use windows::Win32::System::Threading::{CreateEventW, WaitForSingleObject};

use super::capture::{CallbackStats, OPEN_TIMEOUT};
use super::out::{Format, Sample};

/// 已启动的输出流。析构即停止（回调线程退出并释放 COM 对象）。
pub struct Stream {
  stop: Arc<AtomicBool>,
  _join: Option<JoinHandle<()>>,
}

impl Drop for Stream {
  fn drop(&mut self) {
    self.stop.store(true, Relaxed);
    if let Some(j) = self._join.take() {
      let _ = j.join();
    }
  }
}

/// 问端点：混音格式是什么、能用多小的回调周期。
///
/// 失败说明这个端点打不开（已拔出 / 被独占），调用方据此报错，不要静默换设备。
pub fn probe(device_id: &str) -> Result<Format, String> {
  unsafe {
    let _ = CoInitializeEx(None, COINIT_MULTITHREADED);
  }
  let device = super::loopback::resolve_device(Some(device_id))?;
  let client: IAudioClient = unsafe { device.Activate(CLSCTX_ALL, None) }
    .map_err(|e| format!("激活输出端点失败：{e}"))?;
  let mix = unsafe { client.GetMixFormat() }.map_err(|e| format!("读取混音格式失败：{e}"))?;
  let (rate, channels, bits, sample) = unsafe { classify(mix) }?;

  let mut period_frames = 0u32;
  let mut low_latency = false;
  if let Ok(c3) = client.cast::<IAudioClient3>() {
    let (mut d, mut f, mut mn, mut mx) = (0u32, 0u32, 0u32, 0u32);
    if unsafe { c3.GetSharedModeEnginePeriod(mix, &mut d, &mut f, &mut mn, &mut mx) }.is_ok()
      && mn > 0
    {
      period_frames = mn;
      low_latency = true;
    }
  }
  if period_frames == 0 {
    let (mut def, mut min) = (0i64, 0i64);
    unsafe { client.GetDevicePeriod(Some(&mut def), Some(&mut min)) }
      .map_err(|e| format!("读取设备周期失败：{e}"))?;
    // 共享模式实际按 default 回调；hns → 帧。
    period_frames = ((def.max(1) as f64 * rate as f64 / 1e7).round() as u32).max(1);
  }
  unsafe { CoTaskMemFree(Some(mix as *const _)) };

  Ok(Format {
    rate,
    channels,
    bits,
    sample,
    period_frames,
    low_latency,
  })
}

/// 打开输出端点并开始回放；阻塞到打开成功/失败（最多 [`OPEN_TIMEOUT`]）。
pub fn start(
  device_id: String,
  format: Format,
  cons: Consumer<f32>,
  stats: Arc<CallbackStats>,
  primed: Arc<AtomicBool>,
) -> Result<Stream, String> {
  let stop = Arc::new(AtomicBool::new(false));
  let (tx, rx) = mpsc::channel::<Result<(), String>>();
  let stop2 = stop.clone();
  let fmt2 = format.clone();
  let join = std::thread::Builder::new()
    .name("wasapi-out".into())
    .spawn(move || run(device_id, fmt2, cons, stats, primed, stop2, tx))
    .map_err(|e| format!("创建输出线程失败：{e}"))?;
  match rx.recv_timeout(OPEN_TIMEOUT) {
    Ok(Ok(())) => Ok(Stream {
      stop,
      _join: Some(join),
    }),
    Ok(Err(e)) => {
      let _ = join.join();
      Err(e)
    }
    Err(_) => {
      stop.store(true, Relaxed);
      Err("打开输出设备超时".into())
    }
  }
}

fn run(
  device_id: String,
  format: Format,
  mut cons: Consumer<f32>,
  stats: Arc<CallbackStats>,
  primed: Arc<AtomicBool>,
  stop: Arc<AtomicBool>,
  tx: mpsc::Sender<Result<(), String>>,
) {
  // 回调线程必须是实时优先级：128 帧周期只有 2.67 ms，被调度掉一次就欠载。
  let _rt = super::rt_guard::enter_audio_thread();
  unsafe {
    let _ = CoInitializeEx(None, COINIT_MULTITHREADED);
  }

  let opened = open(&device_id, &format);
  let (client, render, event, buffer_frames) = match opened {
    Ok(v) => v,
    Err(e) => {
      let _ = tx.send(Err(e));
      return;
    }
  };
  let _ = tx.send(Ok(()));

  let ch = format.channels.max(1) as usize;
  while !stop.load(Relaxed) {
    let r = unsafe { WaitForSingleObject(event, 200) };
    if r == WAIT_TIMEOUT {
      continue;
    }
    if r != WAIT_OBJECT_0 {
      break;
    }
    let padding = unsafe { client.GetCurrentPadding() }.unwrap_or(0);
    let frames = buffer_frames.saturating_sub(padding);
    if frames == 0 {
      continue;
    }
    stats.on_block(frames);
    let Ok(ptr) = (unsafe { render.GetBuffer(frames) }) else {
      continue;
    };
    let missing = fill(ptr, frames, ch, format.sample, &mut cons);
    if missing > 0 && primed.load(Relaxed) {
      stats.xruns.fetch_add(missing, Relaxed);
    }
    let _ = unsafe { render.ReleaseBuffer(frames, 0) };
  }

  let _ = unsafe { client.Stop() };
  let _ = unsafe { CloseHandle(event) };
}

type Opened = (IAudioClient, IAudioRenderClient, windows::Win32::Foundation::HANDLE, u32);

fn open(device_id: &str, format: &Format) -> Result<Opened, String> {
  super::with_device_retry(|| open_once(device_id, format))
}

fn open_once(device_id: &str, format: &Format) -> Result<Opened, String> {
  let device = super::loopback::resolve_device(Some(device_id))?;
  let client: IAudioClient = unsafe { device.Activate(CLSCTX_ALL, None) }
    .map_err(|e| format!("激活输出端点失败：{e}"))?;
  let mix = unsafe { client.GetMixFormat() }.map_err(|e| format!("读取混音格式失败：{e}"))?;

  let event = match unsafe { CreateEventW(None, false, false, None) } {
    Ok(e) => e,
    Err(e) => {
      unsafe { CoTaskMemFree(Some(mix as *const _)) };
      return Err(format!("创建输出事件失败：{e}"));
    }
  };

  // 优先低延迟共享；端点不支持 `IAudioClient3` 就退回普通共享（引擎默认周期）。
  let mut init = Err(String::new());
  let mut used_low_latency = false;
  if format.low_latency {
    match client.cast::<IAudioClient3>() {
      Ok(c3) => {
        match unsafe {
          c3.InitializeSharedAudioStream(
            AUDCLNT_STREAMFLAGS_EVENTCALLBACK,
            format.period_frames,
            mix,
            None,
          )
        } {
          Ok(()) => {
            init = Ok(());
            used_low_latency = true;
          }
          Err(e) => init = Err(format!("低延迟共享模式打开失败：{e}")),
        }
      }
      Err(_) => init = Err("端点没有低延迟共享接口".into()),
    }
  }
  if init.is_err() {
    let (mut def, mut min) = (0i64, 0i64);
    let _ = unsafe { client.GetDevicePeriod(Some(&mut def), Some(&mut min)) };
    init = unsafe {
      client.Initialize(
        AUDCLNT_SHAREMODE_SHARED,
        AUDCLNT_STREAMFLAGS_EVENTCALLBACK,
        def,
        0,
        mix,
        None,
      )
    }
    .map_err(|e| format!("打开共享输出失败：{e}"));
  }
  unsafe { CoTaskMemFree(Some(mix as *const _)) };
  init?;

  // 事件句柄必须在 Initialize 之后、Start 之前设置。
  unsafe { client.SetEventHandle(event) }.map_err(|e| {
    let _ = unsafe { CloseHandle(event) };
    format!("绑定输出事件失败：{e}")
  })?;
  let buffer_frames = unsafe { client.GetBufferSize() }.map_err(|e| {
    let _ = unsafe { CloseHandle(event) };
    format!("读取输出缓冲大小失败：{e}")
  })?;
  let render: IAudioRenderClient = unsafe { client.GetService() }.map_err(|e| {
    let _ = unsafe { CloseHandle(event) };
    format!("获取 IAudioRenderClient 失败：{e}")
  })?;
  unsafe { client.Start() }.map_err(|e| {
    let _ = unsafe { CloseHandle(event) };
    format!("启动输出失败（端点被独占？）：{e}")
  })?;

  let _ = used_low_latency;
  Ok((client, render, event, buffer_frames))
}

/// 按帧填设备缓冲（每帧取一个单声道样本，复制到所有声道）。返回补了几帧静音。
fn fill(
  ptr: *mut u8,
  frames: u32,
  ch: usize,
  sample: Sample,
  cons: &mut Consumer<f32>,
) -> u64 {
  let mut missing = 0u64;
  match sample {
    Sample::F32 => {
      let p = ptr as *mut f32;
      for f in 0..frames as usize {
        let v = pop_or_silence(cons, &mut missing);
        for c in 0..ch {
          unsafe { *p.add(f * ch + c) = v };
        }
      }
    }
    Sample::I16 => {
      let p = ptr as *mut i16;
      for f in 0..frames as usize {
        let v = (pop_or_silence(cons, &mut missing).clamp(-1.0, 1.0) * 32767.0) as i16;
        for c in 0..ch {
          unsafe { *p.add(f * ch + c) = v };
        }
      }
    }
    Sample::I32 => {
      let p = ptr as *mut i32;
      for f in 0..frames as usize {
        let v =
          (pop_or_silence(cons, &mut missing).clamp(-1.0, 1.0) * 2_147_483_647.0) as i32;
        for c in 0..ch {
          unsafe { *p.add(f * ch + c) = v };
        }
      }
    }
    Sample::U16 => {
      let p = ptr as *mut u16;
      for f in 0..frames as usize {
        let v = ((pop_or_silence(cons, &mut missing).clamp(-1.0, 1.0) * 32767.0) as i32
          + 32768) as u16;
        for c in 0..ch {
          unsafe { *p.add(f * ch + c) = v };
        }
      }
    }
  }
  missing
}

#[inline]
fn pop_or_silence(cons: &mut Consumer<f32>, missing: &mut u64) -> f32 {
  match cons.pop() {
    Ok(v) => v,
    Err(_) => {
      *missing += 1;
      0.0
    }
  }
}

/// 读混音格式：采样率 / 声道 / 位深 / 样本类型。共享混音格式基本只有 f32 和 i16。
unsafe fn classify(mix: *const WAVEFORMATEX) -> Result<(u32, u16, u16, Sample), String> {
  const WAVE_FORMAT_PCM: u16 = 1;
  const WAVE_FORMAT_IEEE_FLOAT: u16 = 3;
  const WAVE_FORMAT_EXTENSIBLE: u16 = 0xFFFE;

  let f = unsafe { &*mix };
  let bits = f.wBitsPerSample;
  let tag = if f.wFormatTag == WAVE_FORMAT_EXTENSIBLE {
    // WAVEFORMATEXTENSIBLE.SubFormat 的第一个 u16 就是格式 tag。
    unsafe { *((mix as *const u8).add(std::mem::size_of::<WAVEFORMATEX>() + 2) as *const u16) }
  } else {
    f.wFormatTag
  };
  let sample = match (tag, bits) {
    (WAVE_FORMAT_IEEE_FLOAT, 32) => Sample::F32,
    (WAVE_FORMAT_PCM, 16) => Sample::I16,
    (WAVE_FORMAT_PCM, 32) => Sample::I32,
    _ => {
      return Err(format!(
        "输出混音格式不支持：tag={tag} bits={bits}（只支持 f32 / i16 / i32）"
      ));
    }
  };
  Ok((f.nSamplesPerSec, f.nChannels, bits, sample))
}
