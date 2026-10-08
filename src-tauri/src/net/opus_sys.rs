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

//! Opus 编解码（DESIGN.md §4.1）。
//!
//! **不引入 C 编译链**：`libopus` 以预编译 x64 `opus.dll` 随包分发，运行时用
//! `LoadLibraryExW` + `GetProcAddress` 加载（与旧实现 `opuslib` 同一路子）。
//! 好处：构建不依赖 cmake 与 libopus 源码；debug / release / CI 走同一条路径。
//!
//! 帧长：PC 侧固定 10 ms（`rate/100` 样本 @48 kHz = 480，正好一个 hop）；接收端允许
//! 任意帧长（2.5~60 ms），按**样本数累积**后由 [`Decoder::take_hop`] 切回 hop，
//! 因此引擎侧的 10 ms 网格（DESIGN.md §2）不受 Android 侧 20 ms 帧影响。
//!
//! ABI 事实（实测 libopus 1.3.1 x64）：`opus_encoder_get_size` / `opus_decoder_get_size`
//! 是**兼容桩，恒返回 0**（头文件里真函数是 `..._get_size2` 的宏）。故本模块**不查大小**，
//! 一律用 `opus_encoder_create` / `opus_decoder_create`（状态由 libopus 内部分配）。

use std::collections::VecDeque;
use std::ffi::{CStr, CString, c_char, c_float, c_int, c_void};
use std::os::windows::ffi::OsStrExt as _;
use std::path::{Path, PathBuf};
use std::sync::OnceLock;

use windows::Win32::Foundation::{FreeLibrary, HMODULE};
use windows::Win32::System::LibraryLoader::{
  GetProcAddress, LOAD_LIBRARY_SEARCH_DEFAULT_DIRS, LoadLibraryExW,
};
use windows::core::{PCSTR, PCWSTR};

/// 加载失败或符号缺失时的原因（直接进调试接口，不伪造可用）。
type Api = Result<OpusApi, String>;

/// DLL 内要用的函数指针（x64 只有一种调用约定，`system` 与 C 的 cdecl 一致）。
struct OpusApi {
  /// dll 句柄（存整数：`HMODULE` 内含裸指针，不能放进 `static`）
  lib: usize,
  encoder_create: unsafe extern "system" fn(c_int, c_int, c_int, *mut c_int) -> *mut c_void,
  encoder_ctl: unsafe extern "system" fn(*mut c_void, c_int, ...) -> c_int,
  encode_float:
    unsafe extern "system" fn(*mut c_void, *const c_float, c_int, *mut u8, c_int) -> c_int,
  encoder_destroy: unsafe extern "system" fn(*mut c_void),
  decoder_create: unsafe extern "system" fn(c_int, c_int, *mut c_int) -> *mut c_void,
  decoder_ctl: unsafe extern "system" fn(*mut c_void, c_int, ...) -> c_int,
  decode_float:
    unsafe extern "system" fn(*mut c_void, *const u8, c_int, *mut c_float, c_int, c_int) -> c_int,
  decoder_destroy: unsafe extern "system" fn(*mut c_void),
  strerror: unsafe extern "system" fn(c_int) -> *const c_char,
  version: unsafe extern "system" fn() -> *const c_char,
}

/// libopus 常量（`opus_defines.h`，ABI 稳定，不随版本变）。
const APPLICATION_VOIP: c_int = 2048;
const SIGNAL_VOIP: c_int = 3001;
const CTL_SET_BITRATE: c_int = 4002;
const CTL_SET_COMPLEXITY: c_int = 4010;
const CTL_SET_SIGNAL: c_int = 4024;
const CTL_SET_GAIN: c_int = 4034;

/// 任意合法 Opus 帧的最坏包长（60 ms @48 kHz）。
pub const MAX_PACKET_BYTES: usize = 1275;

static API: OnceLock<Api> = OnceLock::new();

/// 库状态探针（AGENTS §1.2：新模块必须上报自身状态）：`Ok(version)` / `Err(原因)`。
pub fn probe() -> Result<String, String> {
  api().map(|a| a.version_string())
}

/// 已加载 dll 的完整路径（探测项：便于排查「装了包却加载不到」）。
pub fn loaded_path() -> Option<String> {
  DLL_LOADED.get().cloned()
}

static DLL_LOADED: OnceLock<String> = OnceLock::new();

fn api() -> Result<&'static OpusApi, String> {
  match API.get_or_init(load) {
    Ok(a) => Ok(a),
    Err(e) => Err(e.clone()),
  }
}

/// 查找 `opus.dll`：先 `<exe 同目录>/opus.dll`（安装包与开发版产物旁），再退到仓库里的
/// `server/opus.dll`（旧实现遗留的预编译 x64 产物，按原路径保留以备复用）。
pub fn dll_search_paths() -> Vec<PathBuf> {
  let mut v = Vec::new();
  if let Ok(exe) = std::env::current_exe()
    && let Some(dir) = exe.parent()
  {
    v.push(dir.join("opus.dll"));
  }
  let root = Path::new(env!("CARGO_MANIFEST_DIR")).join("..");
  v.push(root.join("server/opus.dll"));
  v.push(root.join("src-tauri/opus.dll"));
  v
}

fn load() -> Api {
  let mut tried = Vec::new();
  for path in dll_search_paths() {
    tried.push(path.display().to_string());
    if !path.is_file() {
      continue;
    }
    let wide: Vec<u16> = path
      .as_os_str()
      .encode_wide()
      .chain(std::iter::once(0))
      .collect();
    // SAFETY: 路径来自上面的固定候选列表；DEFAULT_DIRS 限定依赖按标准顺序（含 dll 同目录）查找。
    let lib = match unsafe {
      LoadLibraryExW(
        PCWSTR(wide.as_ptr()),
        None,
        LOAD_LIBRARY_SEARCH_DEFAULT_DIRS,
      )
    } {
      Ok(h) => h,
      Err(e) => return Err(format!("加载 {} 失败：{e}", path.display())),
    };
    match bind(lib) {
      Ok(api) => {
        let _ = DLL_LOADED.set(path.display().to_string());
        return Ok(api);
      }
      Err(e) => {
        // SAFETY: `lib` 是上面 LoadLibraryExW 成功返回的句柄。
        unsafe {
          let _ = FreeLibrary(lib);
        }
        return Err(format!("{}：{e}", path.display()));
      }
    }
  }
  Err(format!("找不到 opus.dll（已试：{}）", tried.join("、")))
}

macro_rules! sym {
  ($lib:expr, $name:literal, $t:ty) => {{
    let cname = CString::new($name).expect("符号名无 NUL");
    // SAFETY: `$name` 是字面量且以 NUL 结尾；`$t` 与 libopus 导出签名逐字对应（见文件头）。
    let p = unsafe { GetProcAddress($lib, PCSTR(cname.as_ptr() as *const u8)) };
    match p {
      Some(f) => unsafe { std::mem::transmute::<unsafe extern "system" fn() -> isize, $t>(f) },
      None => return Err(format!("缺少符号 {}", $name)),
    }
  }};
}

fn bind(lib: HMODULE) -> Result<OpusApi, String> {
  Ok(OpusApi {
    lib: lib.0 as usize,
    encoder_create: sym!(
      lib,
      "opus_encoder_create",
      unsafe extern "system" fn(c_int, c_int, c_int, *mut c_int) -> *mut c_void
    ),
    encoder_ctl: sym!(
      lib,
      "opus_encoder_ctl",
      unsafe extern "system" fn(*mut c_void, c_int, ...) -> c_int
    ),
    encode_float: sym!(
      lib,
      "opus_encode_float",
      unsafe extern "system" fn(*mut c_void, *const c_float, c_int, *mut u8, c_int) -> c_int
    ),
    encoder_destroy: sym!(
      lib,
      "opus_encoder_destroy",
      unsafe extern "system" fn(*mut c_void)
    ),
    decoder_create: sym!(
      lib,
      "opus_decoder_create",
      unsafe extern "system" fn(c_int, c_int, *mut c_int) -> *mut c_void
    ),
    decoder_ctl: sym!(
      lib,
      "opus_decoder_ctl",
      unsafe extern "system" fn(*mut c_void, c_int, ...) -> c_int
    ),
    decode_float: sym!(
      lib,
      "opus_decode_float",
      unsafe extern "system" fn(*mut c_void, *const u8, c_int, *mut c_float, c_int, c_int) -> c_int
    ),
    decoder_destroy: sym!(
      lib,
      "opus_decoder_destroy",
      unsafe extern "system" fn(*mut c_void)
    ),
    strerror: sym!(
      lib,
      "opus_strerror",
      unsafe extern "system" fn(c_int) -> *const c_char
    ),
    version: sym!(
      lib,
      "opus_get_version_string",
      unsafe extern "system" fn() -> *const c_char
    ),
  })
}

fn cstr(p: *const c_char) -> String {
  if p.is_null() {
    return "未知".to_string();
  }
  // SAFETY: libopus 的字符串返回都是静态 NUL 结尾串。
  unsafe { CStr::from_ptr(p) }.to_string_lossy().into_owned()
}

fn errstr(a: &OpusApi, code: c_int) -> String {
  // SAFETY: `code` 来自 opus 自身的返回码。
  let p = unsafe { (a.strerror)(code) };
  if p.is_null() {
    format!("Opus 错误 {code}")
  } else {
    format!("Opus 错误 {code}：{}", cstr(p))
  }
}

impl Drop for OpusApi {
  fn drop(&mut self) {
    // SAFETY: 句柄来自 LoadLibraryExW（存成整数只为能进 static）；此时所有编解码器均已销毁。
    unsafe {
      let _ = FreeLibrary(HMODULE(self.lib as *mut c_void));
    }
  }
}

impl OpusApi {
  fn version_string(&self) -> String {
    // SAFETY: 无参、无副作用。
    cstr(unsafe { (self.version)() })
  }
}

/// 单声道编码器（VOIP 模式）。`bitrate` 单位 bit/s。
pub struct Encoder {
  st: *mut c_void,
}

// SAFETY: libopus 的编码器状态由本类型独占拥有（不可克隆、每次操作都在同一线程内串行完成），
// 因此可以在线程之间**移动**（tokio 任务与测试线程），只需不被并发访问。
unsafe impl Send for Encoder {}

impl Encoder {
  pub fn new(rate: c_int, bitrate: i32) -> Result<Self, String> {
    let a = api()?;
    let mut err: c_int = 0;
    // SAFETY: 参数合法（rate ∈ {8k..48k}、单声道、VOIP）；`err` 是出参。
    let st = unsafe { (a.encoder_create)(rate, 1, APPLICATION_VOIP, &mut err) };
    if st.is_null() || err != 0 {
      return Err(errstr(a, err));
    }
    let enc = Encoder { st };
    // SAFETY: st 是本类型独占的状态指针。
    unsafe {
      if (a.encoder_ctl)(st, CTL_SET_SIGNAL, SIGNAL_VOIP) != 0 {
        return Err("Opus：设置语音模式失败".into());
      }
      if (a.encoder_ctl)(st, CTL_SET_BITRATE, bitrate as c_int) != 0 {
        return Err("Opus：设置码率失败".into());
      }
      if (a.encoder_ctl)(st, CTL_SET_COMPLEXITY, 5) != 0 {
        return Err("Opus：设置复杂度失败".into());
      }
    }
    Ok(enc)
  }

  /// 编码一帧。`frame.len()` 须是 Opus 合法帧长（`rate` 的 2.5/5/10/20/40/60 ms 之一）。
  pub fn encode(
    &mut self,
    frame: &[f32],
    out: &mut [u8; MAX_PACKET_BYTES],
  ) -> Result<usize, String> {
    let a = api()?;
    // SAFETY: st 有效；frame / out 长度已由调用方保证（out 固定 MAX_PACKET_BYTES）。
    let n = unsafe {
      (a.encode_float)(
        self.st,
        frame.as_ptr(),
        frame.len() as c_int,
        out.as_mut_ptr(),
        MAX_PACKET_BYTES as c_int,
      )
    };
    if n < 0 {
      Err(errstr(a, n))
    } else {
      Ok(n as usize)
    }
  }
}

impl Drop for Encoder {
  fn drop(&mut self) {
    if let Ok(a) = api() {
      // SAFETY: st 是本类型独占的状态指针，且不再被使用。
      unsafe {
        (a.encoder_destroy)(self.st);
      }
    }
  }
}

/// 单声道解码器：解出的样本累积在 `pending`，由 [`Decoder::take_hop`] 切成一个 hop。
pub struct Decoder {
  st: *mut c_void,
  pending: VecDeque<f32>,
  /// 解码暂存区（复用，避免每包一次分配）
  scratch: Vec<f32>,
}

// SAFETY: 同 `Encoder`：状态独占拥有，可在任务之间移动，不可并发访问。
unsafe impl Send for Decoder {}

impl Decoder {
  pub fn new(rate: c_int) -> Result<Self, String> {
    let a = api()?;
    let mut err: c_int = 0;
    // SAFETY: 同 `Encoder::new`。
    let st = unsafe { (a.decoder_create)(rate, 1, &mut err) };
    if st.is_null() || err != 0 {
      return Err(errstr(a, err));
    }
    let cap = (rate as usize / 1000 * 60).max(2048);
    Ok(Decoder {
      st,
      pending: VecDeque::with_capacity(cap),
      scratch: vec![0.0; cap],
    })
  }

  /// 解一个 Opus 包（任意帧长），样本**累积**到内部缓冲，由 [`Decoder::take_hop`] 切 hop。
  /// `gain_q8` 是 opus 的 Q8 增益（0 = 不变）。返回解出的样本数。
  #[allow(dead_code)]
  pub fn decode(&mut self, packet: &[u8], gain_q8: i32) -> Result<usize, String> {
    let n = self.decode_raw(packet, gain_q8)?;
    self.pending.extend(self.scratch[..n].iter().copied());
    Ok(n)
  }

  /// 解一个 Opus 包并把样本**直接**交给调用方（入站路径：解完立刻进队列，不再二次累积）。
  /// `out` 会被清空后重填；返回样本数。
  pub fn decode_packet(
    &mut self,
    packet: &[u8],
    gain_q8: i32,
    out: &mut Vec<f32>,
  ) -> Result<usize, String> {
    let n = self.decode_raw(packet, gain_q8)?;
    out.clear();
    out.extend_from_slice(&self.scratch[..n]);
    Ok(n)
  }

  /// 解码到内部暂存区（两条解码路径共用的唯一实现）。
  fn decode_raw(&mut self, packet: &[u8], gain_q8: i32) -> Result<usize, String> {
    let a = api()?;
    if gain_q8 != 0 {
      // SAFETY: st 有效；ctl 是 opus 规定的变参调用。
      unsafe {
        (a.decoder_ctl)(self.st, CTL_SET_GAIN, gain_q8);
      }
    }
    // SAFETY: st 有效；scratch 容量按 60 ms 上限分配，任何合法包都解得下。
    let n = unsafe {
      (a.decode_float)(
        self.st,
        packet.as_ptr(),
        packet.len() as c_int,
        self.scratch.as_mut_ptr(),
        self.scratch.len() as c_int,
        0,
      )
    };
    if n < 0 {
      return Err(errstr(a, n));
    }
    Ok(n as usize)
  }

  /// 攒够一个 hop 就取出（`out.len()` 样本）；不够返回 false（调用方补静音）。
  ///
  /// 入站路径用 [`Decoder::decode_packet`] 直接进队列，不经这里；本方法服务于
  /// 「解码后自行按 hop 取用」的调用方与单测。
  #[allow(dead_code)]
  pub fn take_hop(&mut self, out: &mut [f32]) -> bool {
    if self.pending.len() < out.len() {
      return false;
    }
    for (i, slot) in out.iter_mut().enumerate() {
      *slot = self.pending[i];
    }
    self.pending.drain(..out.len());
    true
  }

  /// 剩余样本数（进调试接口）。
  #[allow(dead_code)]
  pub fn pending(&self) -> usize {
    self.pending.len()
  }
}

impl Drop for Decoder {
  fn drop(&mut self) {
    if let Ok(a) = api() {
      // SAFETY: st 是本类型独占的状态指针，且不再被使用。
      unsafe {
        (a.decoder_destroy)(self.st);
      }
    }
  }
}

#[cfg(test)]
mod tests {
  use super::*;
  use crate::audio::{HOP, SAMPLE_RATE};

  /// 端到端：1 kHz 正弦 → Opus(10 ms) → 解码 → 切出 hop。
  /// 覆盖 dll 加载、符号绑定、VOIP 模式、10 ms 帧长（480 样本）与 hop 边界。
  #[test]
  fn roundtrip_ten_ms_frames() {
    let rate = SAMPLE_RATE as i32;
    let mut enc = Encoder::new(rate, 32_000).expect("创建编码器");
    let mut dec = Decoder::new(rate).expect("创建解码器");
    let mut out = [0u8; MAX_PACKET_BYTES];
    let mut frame = vec![0.0f32; HOP];
    let mut hop = [0.0f32; HOP];
    let mut hops = 0usize;
    let mut peak = 0.0f32;
    for k in 0..30 {
      for (i, x) in frame.iter_mut().enumerate() {
        let n = (k * HOP + i) as f32;
        *x = 0.2 * (2.0 * std::f32::consts::PI * 1000.0 * n / rate as f32).sin();
      }
      let len = enc.encode(&frame, &mut out).expect("编码一帧");
      assert!(len > 0 && len <= MAX_PACKET_BYTES, "包长异常 {len}");
      dec.decode(&out[..len], 0).expect("解码一个包");
      while dec.take_hop(&mut hop) {
        for x in hop.iter() {
          peak = peak.max(x.abs());
        }
        hops += 1;
      }
    }
    // 30 帧进、hop 出：解码样本略滞后于编码（解码器有 1~2 帧预热），但应拿到 ≥20 个 hop
    assert!(hops >= 20, "应切出约 30 个 hop，实得 {hops}");
    assert!(peak > 0.05, "解码波形幅度异常，峰值 {peak}");
    // 残余必须落在 hop 边界内（协议按样本累积重切，边界才对齐）
    assert_eq!(
      dec.pending() % HOP,
      0,
      "残余样本 {} 不是 hop 整数倍",
      dec.pending()
    );
  }

  /// 静音段应解出接近 0 的 hop（验证丢包时补静音而不是补旧数据）。
  #[test]
  fn silence_stays_silent() {
    let rate = SAMPLE_RATE as i32;
    let mut enc = Encoder::new(rate, 32_000).expect("创建编码器");
    let mut dec = Decoder::new(rate).expect("创建解码器");
    let mut out = [0u8; MAX_PACKET_BYTES];
    let frame = vec![0.0f32; HOP];
    let mut hop = [0.0f32; HOP];
    for _ in 0..8 {
      let len = enc.encode(&frame, &mut out).expect("编码静音");
      dec.decode(&out[..len], 0).expect("解码静音");
    }
    let mut peak = 0.0f32;
    while dec.take_hop(&mut hop) {
      for x in hop.iter() {
        peak = peak.max(x.abs());
      }
    }
    assert!(peak < 1e-3, "静音解出噪声，峰值 {peak}");
  }

  #[test]
  fn dll_found_and_versioned() {
    let v = probe().expect("应能加载 opus.dll");
    assert!(v.to_lowercase().contains("opus"), "版本串异常：{v}");
    assert!(loaded_path().is_some(), "应记录已加载 dll 路径");
  }
}
