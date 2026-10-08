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

//! Linux：`dlopen` + `dlsym` 加载系统 `libopus.so`（Debian 包名 `libopus0`）。
//! 导出的编解码 API 见 [`super`]（`opus_sys.rs`）。

use std::ffi::{CStr, CString, c_void};
use std::path::{Path, PathBuf};

/// 动态库句柄（`dlopen` 返回的指针）。
pub type LibHandle = *mut c_void;

/// 候选：先 `<exe 同目录>/libopus.so*`（随包分发时），再走系统 `libopus.so`
/// （dlopen 认裸 soname 会查系统搜索路径）。
pub fn dll_search_paths() -> Vec<PathBuf> {
  let mut v = Vec::new();
  if let Ok(exe) = std::env::current_exe()
    && let Some(dir) = exe.parent()
  {
    v.push(dir.join("libopus.so.0"));
    v.push(dir.join("libopus.so"));
  }
  v.push(PathBuf::from("libopus.so.0"));
  v.push(PathBuf::from("libopus.so"));
  v.push(PathBuf::from("libopus.so.1"));
  v
}

pub unsafe fn open_lib(path: &Path) -> Result<LibHandle, String> {
  use std::os::unix::ffi::OsStrExt as _;
  let c = CString::new(path.as_os_str().as_bytes()).map_err(|_| "库名含 NUL".to_string())?;
  // SAFETY: `c` 是有效的 NUL 结尾库名/路径。
  let h = unsafe { libc::dlopen(c.as_ptr(), libc::RTLD_NOW | libc::RTLD_LOCAL) };
  if h.is_null() { Err(dl_error()) } else { Ok(h) }
}

fn dl_error() -> String {
  // SAFETY: dlerror 返回静态串或 NULL。
  unsafe {
    let p = libc::dlerror();
    if p.is_null() {
      "dlopen 失败".to_string()
    } else {
      CStr::from_ptr(p).to_string_lossy().into_owned()
    }
  }
}

pub unsafe fn sym_ptr(lib: LibHandle, name: &CStr) -> Option<*mut c_void> {
  // SAFETY: `lib` 是 dlopen 句柄；`name` 是 NUL 结尾符号名。
  let p = unsafe { libc::dlsym(lib, name.as_ptr()) };
  if p.is_null() { None } else { Some(p) }
}

pub unsafe fn close_lib(lib: LibHandle) {
  // SAFETY: `lib` 是 dlopen 成功返回的句柄，且此后不再使用。
  unsafe {
    libc::dlclose(lib);
  }
}

pub fn lib_raw(lib: LibHandle) -> usize {
  lib as usize
}

pub unsafe fn lib_from_raw(raw: usize) -> LibHandle {
  raw as *mut c_void
}
