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

//! 开机自启：写/删 `HKCU\Software\Microsoft\Windows\CurrentVersion\Run` 下的 `PureVox`
//! 值（当前可执行文件路径），只影响当前用户，不需要管理员。

#[cfg(windows)]
const RUN_KEY: windows::core::PCWSTR = windows::core::w!("Software\\Microsoft\\Windows\\CurrentVersion\\Run");
#[cfg(windows)]
const VALUE_NAME: windows::core::PCWSTR = windows::core::w!("PureVox");

#[cfg(windows)]
pub fn set(on: bool) -> Result<(), String> {
  use windows::Win32::System::Registry::{
    HKEY, HKEY_CURRENT_USER, KEY_SET_VALUE, REG_SZ, RegCloseKey, RegDeleteValueW, RegOpenKeyExW,
    RegSetValueExW,
  };
  unsafe {
    let mut hkey = HKEY::default();
    if RegOpenKeyExW(HKEY_CURRENT_USER, RUN_KEY, None, KEY_SET_VALUE, &mut hkey).0 != 0 {
      return Err("打开注册表 Run 键失败".into());
    }
    let code = if on {
      let exe = std::env::current_exe().map_err(|e| format!("取可执行文件路径失败：{e}"))?;
      let s = format!("\"{}\"", exe.display());
      let mut val: Vec<u16> = s.encode_utf16().collect();
      val.push(0);
      let bytes = std::slice::from_raw_parts(val.as_ptr() as *const u8, val.len() * 2);
      RegSetValueExW(hkey, VALUE_NAME, None, REG_SZ, Some(bytes)).0
    } else {
      RegDeleteValueW(hkey, VALUE_NAME).0
    };
    let _ = RegCloseKey(hkey);
    if code == 0 {
      Ok(())
    } else {
      Err(format!("注册表写入失败（{code}）"))
    }
  }
}

#[cfg(windows)]
pub fn get() -> bool {
  use windows::Win32::System::Registry::{
    HKEY, HKEY_CURRENT_USER, KEY_QUERY_VALUE, RegCloseKey, RegOpenKeyExW, RegQueryValueExW,
  };
  unsafe {
    let mut hkey = HKEY::default();
    if RegOpenKeyExW(HKEY_CURRENT_USER, RUN_KEY, None, KEY_QUERY_VALUE, &mut hkey).0 != 0 {
      return false;
    }
    let mut size = 0u32;
    let code = RegQueryValueExW(hkey, VALUE_NAME, None, None, None, Some(&mut size)).0;
    let _ = RegCloseKey(hkey);
    code == 0
  }
}

#[cfg(not(windows))]
pub fn set(_on: bool) -> Result<(), String> {
  Err("开机自启仅支持 Windows".into())
}

#[cfg(not(windows))]
pub fn get() -> bool {
  false
}
