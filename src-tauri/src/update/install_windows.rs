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

//! Windows 静默安装：NSIS 安装器的 `/S /UPDATE /NS /R`（见 Tauri 的
//! `installer.nsi`：`/S` 静默、`/UPDATE` 跳过重装页与快捷方式、`/NS` 不建快捷方式、
//! `/R` 装完重启应用）。
//!
//! **Windows 只发 NSIS 一个安装包**（`-setup.exe`），故这里没有第二个分支：
//! `src-tauri/tauri.windows.conf.json` 把 `bundle.targets` 收成 `["nsis"]`，
//! 不再产出 MSI（一个平台一条安装路径，见 AGENTS §2.1）。少一种产物 = 少一种
//! 「更新时挑错包」的可能，也不用再维护 msiexec 分支。
//!
//! **先退进程再装**：NSIS 安装器开头有 `CheckIfAppIsRunning`，正在运行的
//! `purevox.exe` 会被它拦下弹框。这里用一个**脱离的 cmd**先歇几秒等本进程退出，
//! 再拉安装器；随后调用方 `app.exit(0)`，安装器接着覆盖文件并按 `/R` 重启。

use super::{Phase, Status};

/// 本平台在 release 产物里认的后缀：**只有 NSIS**（`tauri.windows.conf.json` 只出它）。
pub const ASSET_SUFFIXES: &[&str] = &["-setup.exe"];

/// 本平台认的产物后缀（与 [`ASSET_SUFFIXES`] 同一份；名字与 Linux 侧对齐，`mod.rs` 只认函数）。
pub fn asset_suffixes() -> &'static [&'static str] {
  ASSET_SUFFIXES
}

/// 安装方式短名（进 `/debug` 的 `update.install_kind`）。Windows 只发 NSIS，
/// 正常就是 `nsis`；开发构建（没打标）如实报 `unknown`。
pub fn kind_name() -> &'static str {
  use tauri::utils::config::BundleType;
  use tauri::utils::platform::bundle_type;
  // 测试钩子：`PUREVOX_UPDATE_KIND=nsis` 让**没打标的开发构建**也走发布版的分支
  // （否则 `target/debug/purevox.exe` 永远是 unknown，更新流程在开发机上验不了）。
  // 只影响名字/日志，不影响安装本身（Windows 只有 NSIS 一条路）。
  if let Some(v) = std::env::var_os("PUREVOX_UPDATE_KIND") {
    let v = v.to_string_lossy().to_ascii_lowercase();
    if v == "nsis" || v == "msi" {
      return if v == "nsis" { "nsis" } else { "msi" };
    }
  }
  match bundle_type() {
    Some(BundleType::Nsis) => "nsis",
    Some(BundleType::Msi) => "msi",
    _ => "unknown",
  }
}

/// 拉起安装器并返回一句给界面看的话；调用方随后应退出进程。
///
/// 状态已由 `mod.rs` 推到 `Installing`（「安装器接管、进程即将退出」），这里不再动它。
pub fn install(st: &Status) -> Result<String, String> {
  let exe = std::path::Path::new(&st.saved_path);
  if !exe.is_file() {
    return Err(format!("安装包不见了：{}", st.saved_path));
  }
  use std::os::windows::process::CommandExt;
  // 只给一个**没有窗口**的控制台：cmd 与它拉起的 `ping` / 安装器都在这个控制台里，
  // 全程不出窗口。**不能加 `DETACHED_PROCESS`**：那是「不给控制台」，cmd 的子进程
  // `ping` 是控制台程序，系统会为它**新建一个可见控制台**（Windows 11 上就是一个
  // 标题为 `ping -n 5 127.0.0.1` 的终端窗口 —— 2026-10-28 用户报的「出来的是 ping」）。
  const CREATE_NO_WINDOW: u32 = 0x0800_0000;
  std::process::Command::new("cmd")
    // **必须 `raw_arg` 原样传**：`.args(["/C", cmdline])` 会让 Rust 给含空格的参数
    // 自己加一层引号、把内部的 `"` 转义成 `\"`，cmd 收到的是坏路径 → 安装器根本不
    // 启动（实测 rc=1，界面只看到 ping 跑完就没了）。
    .raw_arg(install_cmdline(exe))
    .creation_flags(CREATE_NO_WINDOW)
    .spawn()
    .map_err(|e| format!("拉起安装器失败：{e}"))?;
  // 显式写一次：Windows 的最终阶段就是 `Installing`（安装器接管、本进程退出，
  // 由安装器的 `/R` 重启），调用方据此决定是否退进程。Linux 那边最终是 `Installed`。
  super::with(|s| s.phase = Phase::Installing);
  Ok("安装器已启动，正在退出并更新…".into())
}

/// 交给 `cmd` 的完整命令行（含 `/S /C`）：`ping` 先歇约 4 s 等本进程退出，再静默装。
///
/// `/S` 让 cmd **只剥首尾一对引号**，里面给安装包路径的那一对引号原样保留 ——
/// 路径带空格/中文（`C:\Users\John Smith\…`）也不会散架。没有 `/S` 时 cmd 的引号
/// 剥离规则在「整行既有引号又有 `&`」时按位置猜，不值得赌。
fn install_cmdline(exe: &std::path::Path) -> String {
  format!(
    r#"/S /C "ping -n 5 127.0.0.1 >nul & "{}" /S /UPDATE /NS /R""#,
    exe.display()
  )
}

#[cfg(test)]
mod tests {
  use super::*;

  /// 引号/开关**逐字**钉死：这条字符串是 cmd 的输入，改错一位就是「点了没反应」。
  /// 曾经的两个坑（都实际发生过）：`.args` 把内部引号转义成 `\"`（安装器不启动）、
  /// `DETACHED_PROCESS` 让 `ping` 弹出可见控制台。这里同时钉住「路径被引号包住」。
  /// 假路径按真实规则写：下载目录 `~/.purevox/update` + 产物名 `PureVox-<版本>-windows-x64-setup.exe`。
  #[test]
  fn install_cmdline_quotes_path_and_keeps_nsis_switches() {
    let exe = std::path::Path::new(
      r"C:\Users\John Smith\.purevox\update\PureVox-26.10.28-windows-x64-setup.exe",
    );
    assert_eq!(
      install_cmdline(exe),
      r#"/S /C "ping -n 5 127.0.0.1 >nul & "C:\Users\John Smith\.purevox\update\PureVox-26.10.28-windows-x64-setup.exe" /S /UPDATE /NS /R""#
    );
  }
}
