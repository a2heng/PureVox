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
//! 允许把周期降到端点的 `min`（VB-Cable 实测 128 帧 / 2.67 ms），本模块优先走它。
//!
//! 兼容性策略（与 cpal 的 WASAPI 后端同一套思路，见 `cpal-0.18.2/src/host/wasapi`）：
//! 1. **原生混音格式**优先：`GetMixFormat()` 返回什么就填什么（PCM 8/16/24/32、
//!    IEEE float 32/64 全部支持），不为「只有 f32 能用」而拒绝设备；
//! 2. 原生格式下先试 `IAudioClient3` 低延迟周期，失败退回普通共享的默认周期；
//! 3. 混音格式无法识别（或原生初始化失败）时，用 **f32 + `AUTOCONVERTPCM |
//!    SRC_DEFAULT_QUALITY`** 请 Windows 音频引擎做位深/声道转换——PortAudio 的
//!    「随便什么设备都能开」就是这条路径；
//! 4. 全程 shared 模式，从不抢独占；周期不变，不靠加大缓冲换兼容。
//!
//! 数据面不变：工作线程往无锁环里推设备采样率的单声道样本，回调按帧取一个、
//! 复制到所有声道。回调里不做策略，取不到就补静音并记欠载。

use std::sync::atomic::{AtomicBool, Ordering::Relaxed};
use std::sync::{Arc, mpsc};
use std::thread::JoinHandle;

use rtrb::Consumer;
use windows::Win32::Foundation::{CloseHandle, WAIT_OBJECT_0, WAIT_TIMEOUT};
use windows::Win32::Media::Audio::{
  AUDCLNT_SHAREMODE_SHARED, AUDCLNT_STREAMFLAGS_AUTOCONVERTPCM, AUDCLNT_STREAMFLAGS_EVENTCALLBACK,
  AUDCLNT_STREAMFLAGS_SRC_DEFAULT_QUALITY, IAudioClient, IAudioClient3, IAudioRenderClient,
  WAVEFORMATEX, WAVEFORMATEXTENSIBLE, WAVEFORMATEXTENSIBLE_0,
};
use windows::Win32::Media::KernelStreaming::WAVE_FORMAT_EXTENSIBLE;
use windows::Win32::System::Com::{
  CLSCTX_ALL, COINIT_MULTITHREADED, CoInitializeEx, CoTaskMemFree,
};
use windows::Win32::System::Threading::{CreateEventW, WaitForSingleObject};
use windows::core::{GUID, Interface};

use super::capture::{CallbackStats, OPEN_TIMEOUT};
use super::out::{Format, Sample};

/// WaveFormatExtensible.SubFormat 的两种合法值（PCM / IEEE float）。
/// 只比较 GUID；不要用 `wFormatTag == 0xFFFE` 时的 `dwChannelMask` 当 tag。
const SUBTYPE_PCM: GUID = GUID::from_u128(0x0000_0001_0000_0010_8000_00aa_0038_9b71);
const SUBTYPE_FLOAT: GUID = GUID::from_u128(0x0000_0003_0000_0010_8000_00aa_0038_9b71);

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

/// 混音格式的解析结果。
struct MixInfo {
  bits: u16,
  sample: Sample,
}

/// 问端点：混音格式是什么、能用多小的回调周期。
///
/// 失败说明这个端点打不开（已拔出 / 被独占），调用方据此报错，不要静默换设备。
/// 混音格式不认识**不**在这里失败：返回 f32 + `engine_convert`，由 `start` 走
/// AUTOCONVERT 路径（与 cpal / PortAudio 一致，任何合法共享混音格式都能开）。
pub fn probe(device_id: &str) -> Result<Format, String> {
  unsafe {
    let _ = CoInitializeEx(None, COINIT_MULTITHREADED);
  }
  let device = super::loopback::resolve_device(Some(device_id))?;
  let client: IAudioClient =
    unsafe { device.Activate(CLSCTX_ALL, None) }.map_err(|e| format!("激活输出端点失败：{e}"))?;
  let mix = unsafe { client.GetMixFormat() }.map_err(|e| format!("读取混音格式失败：{e}"))?;
  let (rate, channels) = unsafe { ((*mix).nSamplesPerSec, (*mix).nChannels) };

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
  let info = unsafe { classify(mix) };
  unsafe { CoTaskMemFree(Some(mix as *const _)) };

  match info {
    Ok(m) => Ok(Format {
      rate,
      channels,
      bits: m.bits,
      sample: m.sample,
      period_frames,
      low_latency,
      engine_convert: false,
    }),
    Err(_) => Ok(Format {
      rate,
      channels,
      bits: 32,
      sample: Sample::F32,
      period_frames,
      low_latency,
      engine_convert: true,
    }),
  }
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
  let (client, render, event, buffer_frames, fill_sample) = match opened {
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
    let missing = fill(ptr, frames, ch, fill_sample, &mut cons);
    if missing > 0 && primed.load(Relaxed) {
      stats.xruns.fetch_add(missing, Relaxed);
    }
    let _ = unsafe { render.ReleaseBuffer(frames, 0) };
  }

  let _ = unsafe { client.Stop() };
  let _ = unsafe { CloseHandle(event) };
}

type Opened = (
  IAudioClient,
  IAudioRenderClient,
  windows::Win32::Foundation::HANDLE,
  u32,
  Sample,
);

fn open(device_id: &str, format: &Format) -> Result<Opened, String> {
  super::with_device_retry(|| open_once(device_id, format))
}

/// 按兼容性从高到低依次尝试：
/// 1. 原生混音格式 + `IAudioClient3` 最小周期（最低延迟，不转换）
/// 2. 原生混音格式 + 普通共享默认周期（同上，周期稍大）
/// 3. f32 + `AUTOCONVERTPCM | SRC_DEFAULT_QUALITY`（引擎转位深/声道，最兼容）
/// 4. f32 + 低延迟周期 + AUTOCONVERT（端点支持时同时拿到低延迟和转换）
fn open_once(device_id: &str, format: &Format) -> Result<Opened, String> {
  let device = super::loopback::resolve_device(Some(device_id))?;
  let mut errors: Vec<String> = Vec::new();

  if !format.engine_convert {
    if format.low_latency {
      match try_open(&device, format, true, false) {
        Ok(v) => return Ok(v),
        Err(e) => errors.push(e),
      }
    }
    match try_open(&device, format, false, false) {
      Ok(v) => return Ok(v),
      Err(e) => errors.push(e),
    }
  }

  // 兜底：请求 f32，让 Windows 音频引擎做格式转换（PortAudio 同款）。
  if format.low_latency {
    match try_open(&device, format, true, true) {
      Ok(v) => return Ok(v),
      Err(e) => errors.push(e),
    }
  }
  match try_open(&device, format, false, true) {
    Ok(v) => Ok(v),
    Err(e) => {
      errors.push(e);
      Err(format!("打开共享输出失败：{}", errors.join("；")))
    }
  }
}

/// 一次 `Activate → Initialize → Start` 尝试。`convert=true` 请求 f32 + 引擎转换。
fn try_open(
  device: &windows::Win32::Media::Audio::IMMDevice,
  format: &Format,
  low_latency: bool,
  convert: bool,
) -> Result<Opened, String> {
  let client: IAudioClient =
    unsafe { device.Activate(CLSCTX_ALL, None) }.map_err(|e| format!("激活输出端点失败：{e}"))?;
  let mix = unsafe { client.GetMixFormat() }.map_err(|e| format!("读取混音格式失败：{e}"))?;

  let info = unsafe { classify(mix) };
  let (fill_sample, requested) = if convert {
    // 请求 48k/设备率的 f32；AUTOCONVERT 让引擎把位深/声道转成端点的混音格式。
    let f32_format = f32_like(mix);
    (Sample::F32, f32_format)
  } else {
    let sample = match info {
      Ok(ref m) => m.sample,
      Err(e) => {
        unsafe { CoTaskMemFree(Some(mix as *const _)) };
        return Err(e);
      }
    };
    // 原生路径直接复用 GetMixFormat；requested 只用于取指针，见下方 `fmt_ptr`。
    (sample, unsafe {
      std::mem::zeroed::<WAVEFORMATEXTENSIBLE>()
    })
  };

  let fmt_ptr: *const WAVEFORMATEX = if convert { &requested.Format } else { mix };

  let (mut def, _min) = (0i64, 0i64);
  let _ = unsafe { client.GetDevicePeriod(Some(&mut def), None) };

  let flags = if convert {
    AUDCLNT_STREAMFLAGS_EVENTCALLBACK
      | AUDCLNT_STREAMFLAGS_AUTOCONVERTPCM
      | AUDCLNT_STREAMFLAGS_SRC_DEFAULT_QUALITY
  } else {
    AUDCLNT_STREAMFLAGS_EVENTCALLBACK
  };

  let init = if low_latency {
    match client.cast::<IAudioClient3>() {
      Ok(c3) => {
        unsafe { c3.InitializeSharedAudioStream(flags, format.period_frames, fmt_ptr, None) }
          .map_err(|e| format!("低延迟共享模式打开失败：{e}"))
      }
      Err(_) => Err("端点没有低延迟共享接口".into()),
    }
  } else {
    unsafe {
      client.Initialize(
        AUDCLNT_SHAREMODE_SHARED,
        flags,
        def.max(1),
        0,
        fmt_ptr,
        None,
      )
    }
    .map_err(|e| format!("打开共享输出失败：{e}"))
  };

  unsafe { CoTaskMemFree(Some(mix as *const _)) };
  init?;

  // 事件句柄必须在 Initialize 之后、Start 之前设置。
  let event = match unsafe { CreateEventW(None, false, false, None) } {
    Ok(e) => e,
    Err(e) => return Err(format!("创建输出事件失败：{e}")),
  };
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

  Ok((client, render, event, buffer_frames, fill_sample))
}

/// 构造一个与混音格式同采样率/声道/声道掩码的 f32 `WAVEFORMATEXTENSIBLE`。
fn f32_like(mix: *const WAVEFORMATEX) -> WAVEFORMATEXTENSIBLE {
  let f = unsafe { &*mix };
  let mask = if f.wFormatTag as u32 == WAVE_FORMAT_EXTENSIBLE {
    unsafe {
      std::ptr::addr_of!((*(mix as *const WAVEFORMATEXTENSIBLE)).dwChannelMask).read_unaligned()
    }
  } else {
    0
  };
  let ch = f.nChannels.max(1);
  WAVEFORMATEXTENSIBLE {
    Format: WAVEFORMATEX {
      wFormatTag: WAVE_FORMAT_EXTENSIBLE as u16,
      nChannels: ch,
      nSamplesPerSec: f.nSamplesPerSec,
      nAvgBytesPerSec: f.nSamplesPerSec * ch as u32 * 4,
      nBlockAlign: ch * 4,
      wBitsPerSample: 32,
      cbSize: 22,
    },
    Samples: WAVEFORMATEXTENSIBLE_0 {
      wValidBitsPerSample: 32,
    },
    dwChannelMask: mask,
    SubFormat: SUBTYPE_FLOAT,
  }
}

/// 按帧填设备缓冲（环里是**交织帧**：每帧 `ch` 个样本，逐通道落位）。
/// 返回补了几帧静音。
///
/// 热路径：F32 直接按连续块 `copy_nonoverlapping`，其它位深逐样本走 cpal 的
/// `FromSample`（与 cpal/PortAudio 同一套缩放，不改语义）；24-bit 按容器格式
/// 分别处理（3 字节紧凑 / 32 位容器高 24 位），不会把低位当有效数据。
fn fill(ptr: *mut u8, frames: u32, ch: usize, sample: Sample, cons: &mut Consumer<f32>) -> u64 {
  let want = frames as usize * ch.max(1);
  let take = cons.slots().min(want);
  if take > 0
    && let Ok(chunk) = cons.read_chunk(take)
  {
    let (a, b) = chunk.as_slices();
    unsafe {
      write_samples(ptr, sample, 0, a);
      write_samples(ptr, sample, a.len(), b);
    }
    chunk.commit_all();
  }
  let got = take / ch.max(1);
  let missing = frames.saturating_sub(got as u32) as u64;
  if missing > 0 {
    let bytes = sample.container_bits() as usize / 8;
    unsafe {
      std::ptr::write_bytes(ptr.add(take * bytes), 0, (want - take) * bytes);
    }
  }
  missing
}

/// 把一段 f32 单声道样本写到设备缓冲的 `off`（样本下标）位置。
///
/// # Safety
/// `dst` 至少有 `(off + src.len()) * 每样本字节数` 字节，对齐满足 `sample`。
unsafe fn write_samples(dst: *mut u8, sample: Sample, off: usize, src: &[f32]) {
  if src.is_empty() {
    return;
  }
  match sample {
    Sample::F32 => unsafe {
      std::ptr::copy_nonoverlapping(src.as_ptr(), (dst as *mut f32).add(off), src.len());
    },
    Sample::I16 => unsafe {
      let p = (dst as *mut i16).add(off);
      for (i, &v) in src.iter().enumerate() {
        *p.add(i) = conv::<i16>(v);
      }
    },
    Sample::I24Packed => unsafe {
      let p = dst.add(off * 3);
      for (i, &v) in src.iter().enumerate() {
        let x = i24_bits(v);
        let q = p.add(i * 3);
        *q = x as u8;
        *q.add(1) = (x >> 8) as u8;
        *q.add(2) = (x >> 16) as u8;
      }
    },
    Sample::I24In32 => unsafe {
      let p = (dst as *mut i32).add(off);
      for (i, &v) in src.iter().enumerate() {
        *p.add(i) = i24_bits(v) << 8;
      }
    },
    Sample::I32 => unsafe {
      let p = (dst as *mut i32).add(off);
      for (i, &v) in src.iter().enumerate() {
        *p.add(i) = conv::<i32>(v);
      }
    },
    Sample::I64 => unsafe {
      let p = (dst as *mut i64).add(off);
      for (i, &v) in src.iter().enumerate() {
        *p.add(i) = conv::<i64>(v);
      }
    },
    Sample::U8 => unsafe {
      let p = dst.add(off);
      for (i, &v) in src.iter().enumerate() {
        *p.add(i) = conv::<u8>(v);
      }
    },
    Sample::F64 => unsafe {
      let p = (dst as *mut f64).add(off);
      for (i, &v) in src.iter().enumerate() {
        *p.add(i) = v as f64;
      }
    },
    // 以下只可能来自非 Windows 的 cpal 后端；WASAPI 不会产生。
    Sample::I8 | Sample::U16 | Sample::U24 | Sample::U32 | Sample::U64 => {
      unreachable!("WASAPI 混音格式不会出现 {sample:?}")
    }
  }
}

#[inline]
fn conv<T: cpal::FromSample<f32>>(v: f32) -> T {
  T::from_sample_(v)
}

#[inline]
fn i24_bits(v: f32) -> i32 {
  // 1.0 必须落在 0x7FFFFF（不能用 unchecked 的 0x800000 溢出到符号位）。
  ((v * 8_388_608.0) as i32).clamp(-8_388_608, 8_388_607)
}

/// 读混音格式：采样率 / 声道 / 位深 / 样本类型 / 声道掩码。
///
/// 关键：`WAVE_FORMAT_EXTENSIBLE` 的格式类型在 `SubFormat` GUID 里（偏移 24），
/// **不是** `dwChannelMask`（偏移 20）；曾经读错偏移，于是立体声 mask=3 被当成
/// IEEE_FLOAT、单声道 mask=4 被当成未知标签，只有 32-bit float 立体声“碰巧能用”。
unsafe fn classify(mix: *const WAVEFORMATEX) -> Result<MixInfo, String> {
  const WAVE_FORMAT_PCM: u16 = 1;
  const WAVE_FORMAT_IEEE_FLOAT: u16 = 3;
  const WAVE_FORMAT_EXTENSIBLE: u16 = 0xFFFE;

  let f = unsafe { &*mix };
  let bits = f.wBitsPerSample;
  let bytes_per_sample = if f.nChannels > 0 {
    f.nBlockAlign as usize / f.nChannels as usize
  } else {
    0
  };
  let (tag, valid_bits) = if f.wFormatTag == WAVE_FORMAT_EXTENSIBLE {
    // `WAVEFORMATEXTENSIBLE` 在 windows crate 里是 `packed(1)`：字段必须按未对齐读，
    // 不能拿字段引用（E0793）。SubFormat 在偏移 24，和 dwChannelMask（偏移 20）分开。
    let e = mix as *const WAVEFORMATEXTENSIBLE;
    let sub = unsafe { std::ptr::addr_of!((*e).SubFormat).read_unaligned() };
    let valid = unsafe { (std::ptr::addr_of!((*e).Samples) as *const u16).read_unaligned() };
    let tag = if sub == SUBTYPE_PCM {
      WAVE_FORMAT_PCM
    } else if sub == SUBTYPE_FLOAT {
      WAVE_FORMAT_IEEE_FLOAT
    } else {
      return Err(format!(
        "输出混音格式不支持：SubFormat={sub:?}（只支持 PCM / IEEE float）"
      ));
    };
    (tag, valid)
  } else {
    (f.wFormatTag, bits)
  };

  let sample = match (tag, bits) {
    (WAVE_FORMAT_IEEE_FLOAT, 32) => Sample::F32,
    (WAVE_FORMAT_IEEE_FLOAT, 64) => Sample::F64,
    (WAVE_FORMAT_PCM, 8) => Sample::U8,
    (WAVE_FORMAT_PCM, 16) => Sample::I16,
    (WAVE_FORMAT_PCM, 24) => {
      if bytes_per_sample == 3 {
        Sample::I24Packed
      } else {
        Sample::I24In32
      }
    }
    (WAVE_FORMAT_PCM, 32) => {
      if valid_bits == 24 {
        Sample::I24In32
      } else {
        Sample::I32
      }
    }
    (WAVE_FORMAT_PCM, 64) => Sample::I64,
    _ => {
      return Err(format!(
        "输出混音格式不支持：tag={tag} bits={bits}（PCM 8/16/24/32/64、float 32/64）"
      ));
    }
  };

  Ok(MixInfo { bits, sample })
}

#[cfg(test)]
mod tests {
  use super::*;
  use std::mem::size_of;

  fn ext(pcm: bool, bits: u16, valid: u16, ch: u16, mask: u32) -> Vec<u8> {
    let fmt = WAVEFORMATEXTENSIBLE {
      Format: WAVEFORMATEX {
        wFormatTag: WAVE_FORMAT_EXTENSIBLE as u16,
        nChannels: ch,
        nSamplesPerSec: 48000,
        nAvgBytesPerSec: 48000 * ch as u32 * (bits as u32 / 8),
        nBlockAlign: ch * (bits / 8),
        wBitsPerSample: bits,
        cbSize: 22,
      },
      Samples: WAVEFORMATEXTENSIBLE_0 {
        wValidBitsPerSample: valid,
      },
      dwChannelMask: mask,
      SubFormat: if pcm { SUBTYPE_PCM } else { SUBTYPE_FLOAT },
    };
    let mut v = vec![0u8; size_of::<WAVEFORMATEXTENSIBLE>()];
    unsafe {
      std::ptr::copy_nonoverlapping(&fmt as *const _ as *const u8, v.as_mut_ptr(), v.len())
    };
    v
  }

  #[test]
  fn classify_extensible_uses_subformat_not_channel_mask() {
    for (bits, mask, want) in [
      (16u16, 4u32, Sample::I16), // 1ch 16-bit：以前 mask=4 会失败
      (16, 3, Sample::I16),       // 2ch 16-bit：以前 mask=3 被当 float
      (24, 3, Sample::I24Packed), // 2ch 紧凑 24-bit
      (32, 3, Sample::F32),       // 2ch float
    ] {
      let b = ext(bits != 32, bits, bits, if mask == 4 { 1 } else { 2 }, mask);
      let m = unsafe { classify(b.as_ptr() as *const WAVEFORMATEX) }.unwrap();
      assert_eq!(m.sample, want, "bits={bits} mask={mask}");
    }
  }

  #[test]
  fn classify_24_in_32_is_i24() {
    let b = ext(true, 32, 24, 2, 3);
    let m = unsafe { classify(b.as_ptr() as *const WAVEFORMATEX) }.unwrap();
    assert_eq!(m.sample, Sample::I24In32);
  }

  #[test]
  fn fill_packs_24_bit_little_endian() {
    let (mut prod, mut cons) = rtrb::RingBuffer::<f32>::new(4);
    for v in [1.0, -1.0, 0.0, 0.5] {
      prod.push(v).ok();
    }
    let mut buf = [0u8; 4 * 3];
    let missing = fill(buf.as_mut_ptr(), 4, 1, Sample::I24Packed, &mut cons);
    assert_eq!(missing, 0);
    assert_eq!(&buf[0..3], &[0xFF, 0xFF, 0x7F]); // +max
    assert_eq!(&buf[3..6], &[0x00, 0x00, 0x80]); // -max
  }

  #[test]
  fn fill_writes_i16_and_zero_pads() {
    let (mut prod, mut cons) = rtrb::RingBuffer::<f32>::new(4);
    prod.push(1.0).ok();
    prod.push(-1.0).ok();
    let mut buf = [0i16; 4];
    let missing = fill(buf.as_mut_ptr() as *mut u8, 4, 1, Sample::I16, &mut cons);
    assert_eq!(missing, 2);
    assert_eq!(buf[0], i16::MAX);
    assert!(buf[1] < -30000);
    assert_eq!(&buf[2..], &[0, 0]);
  }
}
