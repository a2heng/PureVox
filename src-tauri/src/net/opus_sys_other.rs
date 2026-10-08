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

//! 其它平台（非 Windows / 非 Linux）：Opus 动态加载未实现，明确不可用（不静默）。

#![allow(dead_code)]

use std::ffi::{CStr, c_void};
use std::path::{Path, PathBuf};

pub type LibHandle = *mut c_void;

pub fn dll_search_paths() -> Vec<PathBuf> {
  Vec::new()
}

pub unsafe fn open_lib(_path: &Path) -> Result<LibHandle, String> {
  Err("Opus 动态加载仅 Windows / Linux 支持".into())
}

pub unsafe fn sym_ptr(_lib: LibHandle, _name: &CStr) -> Option<*mut c_void> {
  None
}

pub unsafe fn close_lib(_lib: LibHandle) {}

pub fn lib_raw(_lib: LibHandle) -> usize {
  0
}

pub unsafe fn lib_from_raw(_raw: usize) -> LibHandle {
  std::ptr::null_mut()
}
