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

//! 开机自启（**唯一实现**）：Windows 写 HKCU 的 `Run` + `StartupApproved\Run`。
//!
//! - 只写 HKCU：HKLM 需要管理员（会弹 UAC），自启不值得为它提权。
//! - `StartupApproved\Run` 的「已启用」标记不能少：只写 `Run` 的话，任务管理器
//!   「启动」页会把条目当成**被禁用**（2026-10-26 实测）。标记格式是 12 字节，
//!   `0x02` 开头 = 启用；禁用是 `0x03` 开头。
//! - 值名与 `Run` 项同一个（`PureVox`），路径取当前 exe；移动安装目录后重新开关一次即可。

#[cfg(windows)]
mod plat {
  use windows::Win32::Foundation::ERROR_SUCCESS;
  use windows::Win32::System::Registry::{
    HKEY, HKEY_CURRENT_USER, KEY_QUERY_VALUE, KEY_SET_VALUE, REG_OPTION_NON_VOLATILE, REG_SZ,
    RegCloseKey, RegCreateKeyExW, RegDeleteValueW, RegOpenKeyExW, RegQueryValueExW, RegSetValueExW,
  };
  use windows::core::PCWSTR;

  const APP: &str = "PureVox";
  const RUN_KEY: &str = r"Software\Microsoft\Windows\CurrentVersion\Run";
  const APPROVED_KEY: &str =
    r"Software\Microsoft\Windows\CurrentVersion\Explorer\StartupApproved\Run";

  fn wide(s: &str) -> Vec<u16> {
    s.encode_utf16().chain(std::iter::once(0)).collect()
  }

  fn open(key: &str, access: windows::Win32::System::Registry::REG_SAM_FLAGS) -> Option<HKEY> {
    let path = wide(key);
    let mut h = HKEY::default();
    let rc = unsafe {
      RegOpenKeyExW(
        HKEY_CURRENT_USER,
        PCWSTR(path.as_ptr()),
        Some(0),
        access,
        &mut h,
      )
    };
    (rc == ERROR_SUCCESS).then_some(h)
  }

  fn create(key: &str) -> Result<HKEY, String> {
    let path = wide(key);
    let mut h = HKEY::default();
    let disp = 0u32;
    let rc = unsafe {
      RegCreateKeyExW(
        HKEY_CURRENT_USER,
        PCWSTR(path.as_ptr()),
        Some(0),
        PCWSTR::null(),
        REG_OPTION_NON_VOLATILE,
        KEY_SET_VALUE | KEY_QUERY_VALUE,
        None,
        &mut h,
        Some(&mut windows::Win32::System::Registry::REG_CREATE_KEY_DISPOSITION(disp)),
      )
    };
    if rc == ERROR_SUCCESS {
      Ok(h)
    } else {
      Err(format!("打开注册表项 {key} 失败：{}", rc.0))
    }
  }

  fn set_string(key: &str, name: &str, value: &str) -> Result<(), String> {
    let h = create(key)?;
    let name_w = wide(name);
    let value_w = wide(value);
    let bytes =
      unsafe { std::slice::from_raw_parts(value_w.as_ptr() as *const u8, value_w.len() * 2) };
    let rc = unsafe { RegSetValueExW(h, PCWSTR(name_w.as_ptr()), Some(0), REG_SZ, Some(bytes)) };
    unsafe {
      let _ = RegCloseKey(h);
    }
    if rc == ERROR_SUCCESS {
      Ok(())
    } else {
      Err(format!("写入注册表值 {name} 失败：{}", rc.0))
    }
  }

  fn set_enabled_marker(name: &str, on: bool) -> Result<(), String> {
    let h = create(APPROVED_KEY)?;
    let name_w = wide(name);
    if on {
      // 启用标记：0x02 + 11 个 0
      let marker = [0x02u8, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0];
      let rc = unsafe {
        RegSetValueExW(
          h,
          PCWSTR(name_w.as_ptr()),
          Some(0),
          windows::Win32::System::Registry::REG_BINARY,
          Some(&marker),
        )
      };
      unsafe {
        let _ = RegCloseKey(h);
      }
      if rc != ERROR_SUCCESS {
        return Err(format!("写启动项启用标记失败：{}", rc.0));
      }
    } else {
      unsafe {
        let _ = RegDeleteValueW(h, PCWSTR(name_w.as_ptr()));
      }
      unsafe {
        let _ = RegCloseKey(h);
      }
    }
    Ok(())
  }

  fn delete_value(key: &str, name: &str) {
    let Some(h) = open(key, KEY_SET_VALUE) else {
      return;
    };
    let name_w = wide(name);
    unsafe {
      let _ = RegDeleteValueW(h, PCWSTR(name_w.as_ptr()));
    }
    unsafe {
      let _ = RegCloseKey(h);
    }
  }

  fn read_value(key: &str, name: &str) -> Option<String> {
    let h = open(key, KEY_QUERY_VALUE)?;
    let name_w = wide(name);
    let mut ty = windows::Win32::System::Registry::REG_VALUE_TYPE(0);
    let mut len = 0u32;
    let rc = unsafe {
      RegQueryValueExW(
        h,
        PCWSTR(name_w.as_ptr()),
        None,
        Some(&mut ty),
        None,
        Some(&mut len),
      )
    };
    if rc != ERROR_SUCCESS || len == 0 {
      unsafe {
        let _ = RegCloseKey(h);
      }
      return None;
    }
    let mut buf = vec![0u8; len as usize];
    let rc = unsafe {
      RegQueryValueExW(
        h,
        PCWSTR(name_w.as_ptr()),
        None,
        Some(&mut ty),
        Some(buf.as_mut_ptr()),
        Some(&mut len),
      )
    };
    unsafe {
      let _ = RegCloseKey(h);
    }
    if rc != ERROR_SUCCESS {
      return None;
    }
    let units: Vec<u16> = buf[..len as usize]
      .as_chunks::<2>()
      .0
      .iter()
      .map(|c| u16::from_le_bytes(*c))
      .take_while(|c| *c != 0)
      .collect();
    Some(String::from_utf16_lossy(&units))
  }

  pub fn set(on: bool) -> Result<(), String> {
    if on {
      let exe = std::env::current_exe().map_err(|e| format!("找不到当前程序路径：{e}"))?;
      // 带引号：路径里有空格（Program Files）时 Run 项才不会被截断
      let cmd = format!("\"{}\"", exe.display());
      if cmd.contains('\0') {
        return Err("程序路径含 NUL，无法写入启动项".into());
      }
      set_string(RUN_KEY, APP, &cmd)?;
      set_enabled_marker(APP, true)?;
      Ok(())
    } else {
      delete_value(RUN_KEY, APP);
      set_enabled_marker(APP, false)?;
      Ok(())
    }
  }

  pub fn is_enabled() -> bool {
    read_value(RUN_KEY, APP).is_some_and(|v| !v.is_empty())
  }
}

#[cfg(windows)]
#[allow(unused_imports)]
pub use plat::{is_enabled, set};

#[cfg(target_os = "linux")]
#[path = "autostart_linux.rs"]
mod plat;
#[cfg(target_os = "linux")]
pub use plat::{is_enabled, set};

#[cfg(not(any(windows, target_os = "linux")))]
#[path = "autostart_other.rs"]
mod plat;
#[cfg(not(any(windows, target_os = "linux")))]
pub use plat::{is_enabled, set};
