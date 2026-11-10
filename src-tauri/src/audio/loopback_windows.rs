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
//! 行为沿用旧 Python 实现（快照已删除；要点见下），
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
    // 0x88890003 = AUDCLNT_E_WRONG_ENDPOINT_TYPE：拿到的是录音端点。配置里把麦克风
    // 的端点 ID 写进了回环行（或 AEC 远端）时就会走到这里；裸 HRESULT 用户看不懂，
    // 这里翻成「哪一步错、该改成什么」。
    init.map_err(|e| {
      if e.code() == windows::core::HRESULT(0x8889_0003u32 as i32) {
        "初始化回环失败：该设备是录音端点，回环行 / AEC 远端只能选播放（输出）端点".to_string()
      } else {
        format!("初始化回环失败：{e}")
      }
    })?;
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

/// 端点缓冲能力探测：**只在本机手动跑**，用来决定要不要自写 WASAPI 后端。
///
/// ```text
/// cargo test --bin purevox wasapi_caps -- --ignored --nocapture
/// ```
///
/// 打印每个渲染/录音端点的：混音格式、`GetDevicePeriod`（共享模式的回调周期，
/// 客户端**改不了**）、`IAudioClient3::GetSharedModeEnginePeriod`（能不能降到
/// 更小的低延迟周期）、以及独占模式能不能开、开了以后缓冲多少帧。
#[cfg(test)]
mod wasapi_caps {
  use super::*;
  use cpal::traits::{DeviceTrait, HostTrait};
  use windows::Win32::Media::Audio::{
    AUDCLNT_SHAREMODE_EXCLUSIVE, AUDCLNT_STREAMFLAGS_EVENTCALLBACK, IAudioClient3,
  };
  use windows::core::Interface;

  fn ms(hns: i64) -> f64 {
    hns as f64 / 10_000.0
  }

  fn probe(kind: &str, name: &str, id: &str) {
    let mut out = format!("\n[{kind}] {name}\n  id={id}");
    let r = (|| -> Result<(), String> {
      let device = resolve_device(Some(id))?;
      let client: IAudioClient =
        unsafe { device.Activate(CLSCTX_ALL, None) }.map_err(|e| format!("Activate 失败：{e}"))?;
      let mix = unsafe { client.GetMixFormat() }.map_err(|e| format!("GetMixFormat 失败：{e}"))?;
      let (rate, ch, bits) = unsafe { read_format(mix) };
      out += &format!("\n  mix={rate} Hz / {ch} ch / {bits} bit");

      let (mut def, mut min) = (0i64, 0i64);
      match unsafe { client.GetDevicePeriod(Some(&mut def), Some(&mut min)) } {
        Ok(()) => {
          out += &format!(
            "\n  GetDevicePeriod: default={:.2} ms ({} f)  min={:.2} ms ({} f)",
            ms(def),
            def as f64 * rate as f64 / 1e7,
            ms(min),
            min as f64 * rate as f64 / 1e7
          );
        }
        Err(e) => out += &format!("\n  GetDevicePeriod 失败：{e}"),
      }

      match client.cast::<IAudioClient3>() {
        Ok(c3) => {
          let (mut pd, mut pf, mut pm, mut px) = (0u32, 0u32, 0u32, 0u32);
          match unsafe { c3.GetSharedModeEnginePeriod(mix, &mut pd, &mut pf, &mut pm, &mut px) } {
            Ok(()) => {
              out += &format!(
                "\n  shared3: default={pd} f  fundamental={pf} f  min={pm} f ({:.2} ms)  max={px} f",
                pm as f64 * 1000.0 / rate as f64
              )
            }
            Err(e) => out += &format!("\n  shared3: GetSharedModeEnginePeriod 失败：{e}"),
          }
        }
        Err(_) => out += "\n  shared3: 端点没有 IAudioClient3（只能走 IAudioClient 的固定周期）",
      }

      let hr = unsafe { client.IsFormatSupported(AUDCLNT_SHAREMODE_EXCLUSIVE, mix, None) };
      if hr.is_err() {
        out += &format!("\n  exclusive: 混音格式不支持（{hr:?}）");
      } else {
        out += "\n  exclusive: 混音格式支持";
        let init = unsafe {
          client.Initialize(
            AUDCLNT_SHAREMODE_EXCLUSIVE,
            AUDCLNT_STREAMFLAGS_EVENTCALLBACK,
            min,
            min,
            mix,
            None,
          )
        };
        match init {
          Ok(()) => {
            let buf = unsafe { client.GetBufferSize() }.unwrap_or(0);
            out += &format!(
              "\n  exclusive init: ok, buffer={buf} f ({:.2} ms)",
              buf as f64 * 1000.0 / rate as f64
            );
          }
          Err(e) => out += &format!("\n  exclusive init 失败：{e}"),
        }
      }

      unsafe { CoTaskMemFree(Some(mix as *const _)) };
      Ok(())
    })();
    if let Err(e) = r {
      out += &format!("\n  探测失败：{e}");
    }
    println!("{out}");
  }

  #[test]
  #[ignore = "需要真实音频设备，只在本机手动跑"]
  fn wasapi_caps() {
    unsafe {
      let _ = CoInitializeEx(None, COINIT_MULTITHREADED);
    }
    let host = cpal::default_host();
    for (kind, list) in [
      (
        "输出",
        host
          .output_devices()
          .map(|v| v.collect::<Vec<_>>())
          .unwrap_or_default(),
      ),
      (
        "输入",
        host
          .input_devices()
          .map(|v| v.collect::<Vec<_>>())
          .unwrap_or_default(),
      ),
    ] {
      for d in list {
        let name = d
          .description()
          .map(|x| x.name().to_string())
          .unwrap_or_else(|_| "?".into());
        let Ok(id) = d.id() else { continue };
        probe(kind, &name, &id.to_string());
      }
    }
  }
}

/// 低延迟共享模式**实测**：用 `IAudioClient3::InitializeSharedAudioStream` 把回调周期
/// 压到 `min` 帧，看引擎是否真的按这个周期回调（而不是又合并成 1056 帧的大请求）。
///
/// ```text
/// cargo test --bin purevox wasapi_lowlat -- --ignored --nocapture
/// ```
#[cfg(test)]
mod wasapi_lowlat {
  use super::*;
  use windows::Win32::Foundation::{CloseHandle, WAIT_OBJECT_0};
  use windows::Win32::Media::Audio::{
    AUDCLNT_STREAMFLAGS_EVENTCALLBACK, IAudioClient3, IAudioRenderClient,
  };
  use windows::Win32::System::Threading::{CreateEventW, WaitForSingleObject};
  use windows::core::Interface;

  const ENDPOINT: &str = "wasapi:{0.0.0.00000000}.{dc3c156b-88e0-44b4-81d6-0167ae457462}";
  const PERIOD_FRAMES: u32 = 128;
  const RUN_MS: u64 = 3000;

  #[test]
  #[ignore = "需要 VB-CABLE 设备，只在本机手动跑"]
  fn wasapi_lowlat() {
    unsafe {
      let _ = CoInitializeEx(None, COINIT_MULTITHREADED);
    }
    let device = match resolve_device(Some(ENDPOINT)) {
      Ok(d) => d,
      Err(e) => {
        println!("找不到端点：{e}");
        return;
      }
    };
    let client: IAudioClient = unsafe { device.Activate(CLSCTX_ALL, None) }.expect("Activate 失败");
    let mix = unsafe { client.GetMixFormat() }.expect("GetMixFormat 失败");
    let (rate, ch, bits) = unsafe { read_format(mix) };
    println!("mix={rate} Hz/{ch} ch/{bits} bit");

    let c3: IAudioClient3 = client.cast().expect("没有 IAudioClient3");
    let (mut pd, mut pf, mut pm, mut px) = (0u32, 0u32, 0u32, 0u32);
    unsafe { c3.GetSharedModeEnginePeriod(mix, &mut pd, &mut pf, &mut pm, &mut px) }
      .expect("GetSharedModeEnginePeriod 失败");
    println!("period: default={pd} fundamental={pf} min={pm} max={px}");

    let ev = unsafe { CreateEventW(None, false, false, None) }.expect("CreateEvent 失败");
    unsafe {
      c3.InitializeSharedAudioStream(AUDCLNT_STREAMFLAGS_EVENTCALLBACK, PERIOD_FRAMES, mix, None)
    }
    .expect("InitializeSharedAudioStream 失败");
    // 事件句柄必须在 Initialize 之后、Start 之前设（先设会报 AUDCLNT_E_NOT_INITIALIZED）。
    unsafe { client.SetEventHandle(ev) }.expect("SetEventHandle 失败");
    unsafe { CoTaskMemFree(Some(mix as *const _)) };
    let buf_frames = unsafe { client.GetBufferSize() }.unwrap_or(0);
    println!(
      "buffer={buf_frames} f ({:.2} ms)",
      buf_frames as f64 * 1000.0 / rate as f64
    );

    let render: IAudioRenderClient = unsafe { client.GetService() }.expect("GetService 失败");
    unsafe { client.Start() }.expect("Start 失败");

    // 和正式代码一样进 MMCSS：不加的话这个测试线程自己就会被调度掉，
    // 量出来的「15 ms 长间隔」是测试线程的锅，不是引擎的。
    let _rt = crate::audio::rt_guard::enter_audio_thread();
    let mut sizes: Vec<u32> = Vec::new();
    let mut gaps_ms: Vec<f64> = Vec::new();
    let start = std::time::Instant::now();
    let mut last = start;
    while start.elapsed().as_millis() < RUN_MS as u128 {
      if unsafe { WaitForSingleObject(ev, 500) } != WAIT_OBJECT_0 {
        println!("等事件超时");
        break;
      }
      // 事件只表示「可以填了」；能填多少是 bufferSize - 已排队(padding)。
      let pending = unsafe { client.GetCurrentPadding() }.unwrap_or(0);
      let avail = buf_frames.saturating_sub(pending);
      if avail == 0 {
        continue;
      }
      let now = std::time::Instant::now();
      gaps_ms.push(now.duration_since(last).as_secs_f64() * 1000.0);
      last = now;
      if let Ok(p) = unsafe { render.GetBuffer(avail) } {
        unsafe { std::ptr::write_bytes(p, 0, (avail * ch as u32 * bits as u32 / 8) as usize) };
        let _ = unsafe { render.ReleaseBuffer(avail, 0) };
        sizes.push(avail);
      }
    }
    let _ = unsafe { client.Stop() };
    let _ = unsafe { CloseHandle(ev) };

    sizes.sort_unstable();
    gaps_ms.sort_by(|a, b| a.partial_cmp(b).unwrap_or(std::cmp::Ordering::Equal));
    let q = |v: &[f64], p: f64| -> f64 { v[((v.len().saturating_sub(1)) as f64 * p) as usize] };
    println!(
      "回调 {} 次，块大小 min={:?} p50={:?} p99={:?} max={:?}",
      sizes.len(),
      sizes.first(),
      sizes.get(sizes.len() / 2),
      sizes.get(sizes.len() * 99 / 100),
      sizes.last()
    );
    if !gaps_ms.is_empty() {
      let over6 = gaps_ms.iter().filter(|g| **g > 6.0).count();
      let over10 = gaps_ms.iter().filter(|g| **g > 10.0).count();
      println!(
        "间隔 ms: p50={:.2} p99={:.2} max={:.2} | >6ms {} 次, >10ms {} 次",
        q(&gaps_ms, 0.5),
        q(&gaps_ms, 0.99),
        gaps_ms.last().unwrap(),
        over6,
        over10
      );
    }
  }
}
