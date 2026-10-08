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

//! Windows：`LoadLibraryExW` + `GetProcAddress` 加载随包预编译 x64 `opus.dll`。
//! 导出的编解码 API 见 [`super`]（`opus_sys.rs`）。

use std::ffi::{CStr, c_void};
use std::os::windows::ffi::OsStrExt as _;
use std::path::{Path, PathBuf};

use windows::Win32::Foundation::{FreeLibrary, HMODULE};
use windows::Win32::System::LibraryLoader::{
  GetProcAddress, LOAD_LIBRARY_SEARCH_DEFAULT_DIRS, LoadLibraryExW,
};
use windows::core::{PCSTR, PCWSTR};

/// 动态库句柄。
pub type LibHandle = HMODULE;

/// 候选：先 `<exe 同目录>/opus.dll`（安装包与开发版产物旁），再退到仓库 `src-tauri/opus.dll`
/// （预编译 x64 libopus，`tauri.conf.json` 的 resources 也从这里取）。
pub fn dll_search_paths() -> Vec<PathBuf> {
  let mut v = Vec::new();
  if let Ok(exe) = std::env::current_exe()
    && let Some(dir) = exe.parent()
  {
    v.push(dir.join("opus.dll"));
  }
  v.push(Path::new(env!("CARGO_MANIFEST_DIR")).join("opus.dll"));
  v
}

pub unsafe fn open_lib(path: &Path) -> Result<LibHandle, String> {
  let wide: Vec<u16> = path
    .as_os_str()
    .encode_wide()
    .chain(std::iter::once(0))
    .collect();
  // SAFETY: 路径来自固定候选列表；DEFAULT_DIRS 限定依赖按标准顺序（含 dll 同目录）查找。
  unsafe {
    LoadLibraryExW(
      PCWSTR(wide.as_ptr()),
      None,
      LOAD_LIBRARY_SEARCH_DEFAULT_DIRS,
    )
  }
  .map_err(|e| e.to_string())
}

pub unsafe fn sym_ptr(lib: LibHandle, name: &CStr) -> Option<*mut c_void> {
  // SAFETY: `name` 是 NUL 结尾符号名；`lib` 是已打开的句柄。
  unsafe { GetProcAddress(lib, PCSTR(name.as_ptr() as *const u8)) }.map(|f| f as *mut c_void)
}

pub unsafe fn close_lib(lib: LibHandle) {
  // SAFETY: `lib` 是 LoadLibraryExW 成功返回的句柄，且此后不再使用。
  unsafe {
    let _ = FreeLibrary(lib);
  }
}

pub fn lib_raw(lib: LibHandle) -> usize {
  lib.0 as usize
}

pub unsafe fn lib_from_raw(raw: usize) -> LibHandle {
  HMODULE(raw as *mut c_void)
}
