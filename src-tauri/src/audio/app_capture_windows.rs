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

//! Windows **单应用音频**采集（WASAPI process loopback，Win10 2004+）。
//!
//! 与设备回环（[`super::loopback`]）的区别只在**怎么拿到 `IAudioClient`**：
//! - 设备回环：`IMMDeviceEnumerator` → 渲染端点 → `Activate(IAudioClient)`；
//! - 单应用：`ActivateAudioInterfaceAsync(VIRTUAL_AUDIO_DEVICE_PROCESS_LOOPBACK, ...)`
//!   + `AUDIOCLIENT_ACTIVATION_TYPE_PROCESS_LOOPBACK`（目标 PID）。
//!
//! 拿到 client 之后完全复用同一套「共享模式 + LOOPBACK + 交织环 + [`capture::worker`]」，
//! 所以不需要为它再写一条数据面。
//!
//! **只采集目标进程树**（`PROCESS_LOOPBACK_MODE_INCLUDE_TARGET_PROCESS_TREE`）：浏览器
//! 的多个渲染进程、游戏的子进程都能覆盖到。

use std::sync::atomic::{AtomicBool, Ordering::Relaxed};
use std::sync::{Arc, Condvar, Mutex, mpsc};
use std::time::Duration;

use rtrb::RingBuffer;
use windows::Win32::Foundation::CloseHandle;
use windows::Win32::Media::Audio::{
  AUDCLNT_BUFFERFLAGS_SILENT, AUDCLNT_SHAREMODE_SHARED, AUDCLNT_STREAMFLAGS_LOOPBACK,
  AUDIOCLIENT_ACTIVATION_PARAMS, AUDIOCLIENT_ACTIVATION_PARAMS_0,
  AUDIOCLIENT_ACTIVATION_TYPE_PROCESS_LOOPBACK, AUDIOCLIENT_PROCESS_LOOPBACK_PARAMS,
  ActivateAudioInterfaceAsync, AudioSessionStateActive, IActivateAudioInterfaceAsyncOperation,
  IActivateAudioInterfaceCompletionHandler, IActivateAudioInterfaceCompletionHandler_Impl,
  IAudioCaptureClient, IAudioClient, IAudioSessionControl2, IAudioSessionManager2,
  IMMDeviceEnumerator, MMDeviceEnumerator, PROCESS_LOOPBACK_MODE_INCLUDE_TARGET_PROCESS_TREE,
  VIRTUAL_AUDIO_DEVICE_PROCESS_LOOPBACK, WAVEFORMATEX, eConsole, eRender,
};
use windows::Win32::System::Com::StructuredStorage::{
  PROPVARIANT, PROPVARIANT_0, PROPVARIANT_0_0, PROPVARIANT_0_0_0,
};
use windows::Win32::System::Com::{BLOB, CLSCTX_ALL, COINIT_MULTITHREADED, CoInitializeEx};
use windows::Win32::System::Threading::{
  OpenProcess, PROCESS_NAME_WIN32, PROCESS_QUERY_LIMITED_INFORMATION, QueryFullProcessImageNameW,
};
use windows::Win32::System::Variant::VT_BLOB;
use windows::core::{HRESULT, IUnknown, Interface, PWSTR, Ref, implement};

use super::capture::{self, CallbackStats, Capture, OPEN_TIMEOUT, SourceInfo};
use super::fanout::MultiFanout;
use crate::debug::SharedHub;

/// 回环缓冲区时长（100 ns 单位）：10 ms，与设备回环一致。
const BUFFER_DURATION: i64 = 100_000;

type OpenOut = (u32, u16, u16, String, rtrb::Consumer<f32>);

/// 打开某个进程（及其子进程树）的音频回环并启动采集线程。
pub fn spawn(hub: SharedHub, pid: u32, tag: String) -> Result<Capture, String> {
  let stop = Arc::new(AtomicBool::new(false));
  let (tx, rx) = mpsc::channel::<Result<Arc<MultiFanout>, String>>();
  let stop2 = stop.clone();
  let tag2 = tag.clone();
  let join = std::thread::Builder::new()
    .name(format!("appcap-{pid}"))
    .spawn(move || run(hub, pid, tag2, stop2, tx))
    .map_err(|e| format!("创建单应用采集线程失败：{e}"))?;
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
      Err("打开单应用音频超时（5 s）".into())
    }
  }
}

/// 列出**当前正在出声**的应用进程（PID + 进程名），供界面选择。
///
/// 只列有活动音频会话的进程：用户想要的是「这个正在放声音的应用」，不是整张进程表。
pub fn list_apps() -> Result<Vec<(u32, String)>, String> {
  unsafe {
    let _ = CoInitializeEx(None, COINIT_MULTITHREADED);
  }
  unsafe {
    let enumerator: IMMDeviceEnumerator =
      windows::Win32::System::Com::CoCreateInstance(&MMDeviceEnumerator, None, CLSCTX_ALL)
        .map_err(|e| format!("创建设备枚举器失败：{e}"))?;
    let device = enumerator
      .GetDefaultAudioEndpoint(eRender, eConsole)
      .map_err(|e| format!("取默认输出端点失败：{e}"))?;
    let mgr: IAudioSessionManager2 = device
      .Activate(CLSCTX_ALL, None)
      .map_err(|e| format!("取音频会话管理器失败：{e}"))?;
    let sessions = mgr
      .GetSessionEnumerator()
      .map_err(|e| format!("枚举音频会话失败：{e}"))?;
    let count = sessions
      .GetCount()
      .map_err(|e| format!("音频会话计数失败：{e}"))?;
    let mut out: Vec<(u32, String)> = Vec::new();
    for i in 0..count {
      let Ok(ctrl) = sessions.GetSession(i) else {
        continue;
      };
      let Ok(c2) = ctrl.cast::<IAudioSessionControl2>() else {
        continue;
      };
      let Ok(pid) = c2.GetProcessId() else {
        continue;
      };
      if pid == 0 || pid == std::process::id() {
        continue;
      }
      if ctrl.GetState().ok() != Some(AudioSessionStateActive) {
        continue;
      }
      if out.iter().any(|(p, _)| *p == pid) {
        continue;
      }
      out.push((pid, process_name(pid)));
    }
    out.sort_by_key(|a| a.1.to_lowercase());
    Ok(out)
  }
}

fn process_name(pid: u32) -> String {
  unsafe {
    let Ok(h) = OpenProcess(PROCESS_QUERY_LIMITED_INFORMATION, false, pid) else {
      return format!("PID {pid}");
    };
    let mut buf = [0u16; 512];
    let mut len = buf.len() as u32;
    let ok =
      QueryFullProcessImageNameW(h, PROCESS_NAME_WIN32, PWSTR(buf.as_mut_ptr()), &mut len).is_ok();
    let _ = CloseHandle(h);
    if !ok {
      return format!("PID {pid}");
    }
    let full = String::from_utf16_lossy(&buf[..len as usize]);
    std::path::Path::new(&full)
      .file_name()
      .and_then(|s| s.to_str())
      .unwrap_or(&full)
      .to_string()
  }
}

fn run(
  hub: SharedHub,
  pid: u32,
  tag: String,
  stop: Arc<AtomicBool>,
  tx: mpsc::Sender<Result<Arc<MultiFanout>, String>>,
) {
  let stats = Arc::new(CallbackStats::new());
  let (fmt_tx, fmt_rx) = mpsc::channel::<Result<OpenOut, String>>();
  let stop2 = stop.clone();
  let stats2 = stats.clone();
  let poll = std::thread::Builder::new()
    .name("appcap-poll".into())
    .spawn(move || open_and_poll(pid, fmt_tx, stop2, stats2))
    .expect("spawn app capture poll");

  let (rate, channels, bits, name, cons) = match fmt_rx.recv_timeout(OPEN_TIMEOUT) {
    Ok(Ok(v)) => v,
    Ok(Err(e)) => {
      let _ = tx.send(Err(e));
      return;
    }
    Err(_) => {
      stop.store(true, Relaxed);
      let _ = tx.send(Err("打开单应用音频超时（5 s）".into()));
      return;
    }
  };
  let info = SourceInfo {
    name,
    device_id: format!("app:{pid}"),
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

fn open_and_poll(
  pid: u32,
  fmt_tx: mpsc::Sender<Result<OpenOut, String>>,
  stop: Arc<AtomicBool>,
  stats: Arc<CallbackStats>,
) {
  unsafe {
    let _ = CoInitializeEx(None, COINIT_MULTITHREADED);
  }
  let opened = (|| -> Result<_, String> {
    let client = activate(pid)?;
    let (rate, ch) = (48_000u32, 2u16);
    // 进程回环对格式比设备回环挑：先试 32-bit float，不行退 16-bit PCM。
    let mut chosen: Option<u16> = None;
    for bits in [32u16, 16u16] {
      let format = WAVEFORMATEX {
        wFormatTag: if bits == 32 { 3 } else { 1 }, // WAVE_FORMAT_IEEE_FLOAT / PCM
        nChannels: ch,
        nSamplesPerSec: rate,
        nAvgBytesPerSec: rate * ch as u32 * (bits as u32 / 8),
        nBlockAlign: ch * (bits / 8),
        wBitsPerSample: bits,
        cbSize: 0,
      };
      if unsafe {
        client.Initialize(
          AUDCLNT_SHAREMODE_SHARED,
          AUDCLNT_STREAMFLAGS_LOOPBACK,
          BUFFER_DURATION,
          0,
          &format,
          None,
        )
      }
      .is_ok()
      {
        chosen = Some(bits);
        break;
      }
    }
    let bits = chosen.ok_or_else(|| "初始化单应用回环失败（32/16 bit 都不被接受）".to_string())?;
    let capture: IAudioCaptureClient =
      unsafe { client.GetService() }.map_err(|e| format!("获取采集接口失败：{e}"))?;
    unsafe { client.Start() }.map_err(|e| format!("启动单应用回环失败：{e}"))?;
    let name = format!("应用：{}（PID {}）", process_name(pid), pid);
    Ok((client, capture, rate, ch, bits, name))
  })();

  let (client, capture, rate, ch, bits, name) = match opened {
    Ok(v) => v,
    Err(e) => {
      let _ = fmt_tx.send(Err(e));
      return;
    }
  };
  let chn = ch.max(1) as usize;
  let (mut prod, cons) = RingBuffer::<f32>::new((rate as usize * chn / 2).max(48_000));
  let _ = fmt_tx.send(Ok((rate, ch, bits, name, cons)));

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
        if let Ok(chunk) = prod.write_chunk_uninit(total) {
          if silent || pdata.is_null() {
            chunk.fill_from_iter(std::iter::repeat_n(0.0f32, total));
          } else if bits == 32 {
            let s = unsafe { std::slice::from_raw_parts(pdata as *const f32, total) };
            chunk.fill_from_iter(s.iter().copied());
          } else {
            let s = unsafe { std::slice::from_raw_parts(pdata as *const i16, total) };
            chunk.fill_from_iter(s.iter().map(|&v| v as f32 / 32768.0));
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

/// COM 完成回调：`ActivateAudioInterfaceAsync` 是异步的，这里只负责唤醒等待方。
#[implement(IActivateAudioInterfaceCompletionHandler)]
struct Handler {
  done: Arc<(Mutex<bool>, Condvar)>,
}

impl IActivateAudioInterfaceCompletionHandler_Impl for Handler_Impl {
  fn ActivateCompleted(
    &self,
    _operation: Ref<IActivateAudioInterfaceAsyncOperation>,
  ) -> windows::core::Result<()> {
    let (m, cv) = &*self.done;
    *m.lock().unwrap_or_else(|e| e.into_inner()) = true;
    cv.notify_all();
    Ok(())
  }
}

fn activate(pid: u32) -> Result<IAudioClient, String> {
  // **必须放堆上并泄漏**：异步激活完成后系统仍可能在别的线程读这块参数；
  // 放栈上会在 `activate` 返回后变成悬垂指针（实测 0xC0000409 栈缓冲区越界）。
  let params: &'static AUDIOCLIENT_ACTIVATION_PARAMS =
    Box::leak(Box::new(AUDIOCLIENT_ACTIVATION_PARAMS {
      ActivationType: AUDIOCLIENT_ACTIVATION_TYPE_PROCESS_LOOPBACK,
      Anonymous: AUDIOCLIENT_ACTIVATION_PARAMS_0 {
        ProcessLoopbackParams: AUDIOCLIENT_PROCESS_LOOPBACK_PARAMS {
          TargetProcessId: pid,
          ProcessLoopbackMode: PROCESS_LOOPBACK_MODE_INCLUDE_TARGET_PROCESS_TREE,
        },
      },
    }));
  let pv = PROPVARIANT {
    Anonymous: PROPVARIANT_0 {
      Anonymous: std::mem::ManuallyDrop::new(PROPVARIANT_0_0 {
        vt: VT_BLOB,
        wReserved1: 0,
        wReserved2: 0,
        wReserved3: 0,
        Anonymous: PROPVARIANT_0_0_0 {
          blob: BLOB {
            cbSize: std::mem::size_of::<AUDIOCLIENT_ACTIVATION_PARAMS>() as u32,
            pBlobData: params as *const _ as *mut u8,
          },
        },
      }),
    },
  };
  let done = Arc::new((Mutex::new(false), Condvar::new()));
  let handler: IActivateAudioInterfaceCompletionHandler = Handler { done: done.clone() }.into();
  let op = unsafe {
    ActivateAudioInterfaceAsync(
      VIRTUAL_AUDIO_DEVICE_PROCESS_LOOPBACK,
      &IAudioClient::IID,
      Some(&pv),
      &handler,
    )
  }
  .map_err(|e| format!("启动单应用激活失败：{e}"))?;

  let (m, cv) = &*done;
  let guard = m.lock().unwrap_or_else(|e| e.into_inner());
  let (guard, timeout) = cv
    .wait_timeout_while(guard, Duration::from_secs(5), |done| !*done)
    .unwrap_or_else(|e| e.into_inner());
  if timeout.timed_out() || !*guard {
    return Err("激活单应用音频超时（进程可能已退出）".into());
  }
  drop(guard);

  let mut hr = HRESULT(0);
  let mut iface: Option<IUnknown> = None;
  unsafe { op.GetActivateResult(&mut hr, &mut iface) }
    .map_err(|e| format!("取激活结果失败：{e}"))?;
  if hr.is_err() {
    return Err(format!(
      "单应用激活失败：{hr:?}（需要 Windows 10 2004+，且目标进程还在运行）"
    ));
  }
  let unk = iface.ok_or_else(|| "激活结果为空".to_string())?;
  let c = unk.cast::<IAudioClient>();
  c.map_err(|e| format!("激活结果不是 IAudioClient：{e}"))
}

#[cfg(test)]
mod tests {
  /// 手动跑：`cargo test --release -- --ignored --nocapture list_apps`
  /// 只验证 COM 枚举不崩（有没有正在出声的应用取决于本机）。
  #[test]
  #[ignore = "需要真实 Windows 音频会话"]
  fn list_apps_runs() {
    match super::list_apps() {
      Ok(v) => {
        println!("正在出声的应用：{}", v.len());
        for (pid, name) in v {
          println!("  {pid}  {name}");
        }
      }
      Err(e) => panic!("枚举失败：{e}"),
    }
  }

  /// 端到端（手动）：起一个 PowerShell 循环播放测试音 → 枚举到它 → 真的采到非静音。
  /// `cargo test --release -- --ignored --nocapture app_capture_end_to_end`
  #[test]
  #[ignore = "打开真实应用音频回环（Windows 10 2004+）"]
  fn app_capture_end_to_end() {
    use rtrb::Consumer;
    let dir = std::env::temp_dir().join("pv-appcap-test");
    std::fs::create_dir_all(&dir).unwrap();
    let wav = dir.join("tone.wav");
    let bytes = crate::cues::wav_bytes(crate::cues::PRESETS[0].0, "start");
    std::fs::write(&wav, &bytes).unwrap();
    let mut child = std::process::Command::new("powershell")
      .args([
        "-NoProfile",
        "-Command",
        &format!(
          "$p=New-Object Media.SoundPlayer '{}'; $p.PlayLooping(); Start-Sleep 8",
          wav.display()
        ),
      ])
      .spawn()
      .expect("spawn powershell");
    std::thread::sleep(std::time::Duration::from_millis(1800));
    let apps = super::list_apps().expect("枚举失败");
    println!("apps: {apps:?}");
    let Some((pid, _)) = apps
      .iter()
      .find(|(_, n)| n.to_lowercase().contains("powershell"))
      .or_else(|| apps.first())
    else {
      println!("没有正在出声的应用，跳过（本机没播起来）");
      let _ = child.kill();
      let _ = child.wait();
      return;
    };
    let hub = crate::debug::DebugHub::new();
    let cap = super::spawn(hub, *pid, "appcap-test".into()).expect("打开采集失败");
    let mut cons: Consumer<f32> = cap
      .fanout
      .subscribe_n("appcap-test", 1)
      .into_iter()
      .next()
      .unwrap();
    let mut peak = 0.0f32;
    let deadline = std::time::Instant::now() + std::time::Duration::from_millis(2500);
    while std::time::Instant::now() < deadline {
      while let Ok(v) = cons.pop() {
        peak = peak.max(v.abs());
      }
      std::thread::sleep(std::time::Duration::from_millis(5));
    }
    println!("peak = {peak}");
    let _ = child.kill();
    let _ = child.wait();
    drop(cap);
    assert!(peak > 0.001, "应从目标应用采到声音，实测 peak={peak}");
  }
}
