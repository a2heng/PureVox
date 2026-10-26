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

//! Linux 安装：**按安装方式分流**（AppImage 自替换 / deb、rpm 交给系统包管理器）。
//!
//! **本机是哪种安装由 Tauri 3 自己回答**：打包时 bundler 把
//! `__TAURI_BUNDLE_TYPE_VAR_{APP,DEB,RPM}` 打进二进制，`tauri::utils::platform::bundle_type()`
//! 读它即可（实测本仓库产物：deb 里是 `VAR_DEB`、AppImage 里是 `VAR_APP`）。
//! 所以不用去问 dpkg/rpm、不用猜 `/usr/bin` 还是 `~/.local`，也不受包数据库被改坏影响。
//!
//! - **AppImage**：自身就是一个可替换的文件 → 写同目录临时名再 `rename`（正在跑的进程
//!   用的是挂载副本，覆盖源文件不影响它，重启即新版）。
//! - **deb / rpm**：文件属于包管理器（`/usr/bin/purevox` 是 root 所有，程序无权替换自己
//!   ——实测 `cp` 覆盖运行中的二进制会 `ETXTBSY`），故用 `pkexec` 走系统包管理器
//!   （polkit 图形授权框；与上游 updater 2.10+ 同一条路）。装完**不退出**，重启生效。
//!
//! **三种安装只认自己那种包**（见 [`suffixes_for`]）：deb 用户不会下到 140 MB 的
//! AppImage —— 那玩意儿他装不上，白下还白显示「已下载」。

use std::path::Path;
use std::sync::OnceLock;

use super::{Phase, Status};

/// 安装方式（由二进制里的 bundle type 标记决定，见 [`kind`]）。
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Kind {
  AppImage,
  Deb,
  Rpm,
  /// 认不出来（没打标的二进制，例如直接跑 `target/debug/purevox`）：**不下载、不安装**
  Unknown,
}

impl Kind {
  /// 短名（进 `/debug` 的 `update.install_kind`，也出现在报错里；与 `BundleType` 写法一致）。
  pub fn as_str(self) -> &'static str {
    match self {
      Kind::AppImage => "appimage",
      Kind::Deb => "deb",
      Kind::Rpm => "rpm",
      Kind::Unknown => "unknown",
    }
  }
}

/// **每种安装方式只认自己的产物后缀**（唯一来源，顺序即优先级）。
///
/// 一条方式一条后缀：跨格式兜底在这里是有害的 —— 把 AppImage 塞给 deb 用户，
/// 他装不上，而界面已经显示「已下载」。宁可报「这个 release 没有你的包」。
pub const ASSET_SUFFIXES: &[(Kind, &[&str])] = &[
  (Kind::Deb, &[".deb"]),
  (Kind::Rpm, &[".rpm"]),
  (Kind::AppImage, &[".appimage"]),
];

/// 本机的安装方式。`bundle_type()` 读的是编译期打进二进制的常量，同一次运行内不变，缓存。
pub fn kind() -> Kind {
  static K: OnceLock<Kind> = OnceLock::new();
  *K.get_or_init(detect)
}

fn detect() -> Kind {
  use tauri::utils::config::BundleType;
  use tauri::utils::platform::bundle_type;
  // 测试钩子：`PUREVOX_UPDATE_KIND=deb|rpm|appimage` 让**没打标的开发构建**
  // （直接跑 `target/debug/purevox`）也能走发布版的下载/安装分支 —— 否则 Linux 上
  // 只能等打完包才发现「更新点了没动静」。
  if let Some(v) = std::env::var_os("PUREVOX_UPDATE_KIND") {
    match v.to_string_lossy().to_ascii_lowercase().as_str() {
      "deb" => return Kind::Deb,
      "rpm" => return Kind::Rpm,
      "appimage" | "app" => return Kind::AppImage,
      _ => {}
    }
  }
  match bundle_type() {
    Some(BundleType::Deb) => Kind::Deb,
    Some(BundleType::Rpm) => Kind::Rpm,
    Some(BundleType::AppImage) => Kind::AppImage,
    // 没打标的二进制仍可能是 AppImage（`APPIMAGE` 由 AppImage 运行时设置，是运行时事实）。
    // 正常发布的包走不到这里（上面已经按标记认出来了）。
    _ if std::env::var_os("APPIMAGE").is_some_and(|v| !v.is_empty()) => Kind::AppImage,
    _ => Kind::Unknown,
  }
}

/// 安装方式短名（进 `/debug` 的 `update.install_kind`）。
pub fn kind_name() -> &'static str {
  kind().as_str()
}

/// 本机安装方式认的产物后缀（空 = 不下载，`check` 会给出原因）。
pub fn asset_suffixes() -> &'static [&'static str] {
  suffixes_for(kind())
}

/// 某种安装方式认的产物后缀（纯函数，单测直接查这张表）。
pub fn suffixes_for(k: Kind) -> &'static [&'static str] {
  ASSET_SUFFIXES
    .iter()
    .find(|(x, _)| *x == k)
    .map_or(&[], |(_, s)| *s)
}

pub fn install(st: &Status) -> Result<String, String> {
  let pkg = Path::new(&st.saved_path);
  if !pkg.is_file() {
    return Err(format!("安装包不见了：{}", st.saved_path));
  }
  let k = kind();
  let want = suffixes_for(k);
  if want.is_empty() {
    let msg = format!(
      "认不出本机安装方式（{} 安装），无法自动更新；请到 release 页面手动下载安装",
      k.as_str()
    );
    super::with(|s| {
      s.phase = Phase::Unsupported;
      s.reason = msg.clone();
    });
    return Err(msg);
  }
  // 防线：状态里可能残留着「上一次按别的安装方式下载的包」（例如换了安装方式没重开程序）
  let name = pkg
    .file_name()
    .map(|n| n.to_string_lossy().to_ascii_lowercase())
    .unwrap_or_default();
  if !want.iter().any(|s| name.ends_with(s)) {
    let msg = format!(
      "下载的 {name} 不是本机安装方式（{}）该用的包（应为 {}），请重新检查更新再下载",
      k.as_str(),
      want.join(" / ")
    );
    super::with(|s| {
      s.phase = Phase::Failed;
      s.reason = msg.clone();
    });
    return Err(msg);
  }
  match k {
    Kind::AppImage => replace_appimage(pkg),
    Kind::Deb => install_package(pkg, "dpkg", "-i"),
    Kind::Rpm => install_package(pkg, "rpm", "-U"),
    Kind::Unknown => Err("认不出本机安装方式，无法安装".into()),
  }
}

/// AppImage 自替换：写同目录临时名 → 设可执行位 → `rename`（同一文件系统内原子）。
fn replace_appimage(pkg: &Path) -> Result<String, String> {
  let Some(cur) = std::env::var_os("APPIMAGE").map(std::path::PathBuf::from) else {
    let msg = "认不出当前 AppImage 的路径（`APPIMAGE` 环境变量为空），无法替换".to_string();
    super::with(|s| {
      s.phase = Phase::Failed;
      s.reason = msg.clone();
    });
    return Err(msg);
  };
  let tmp = cur.with_extension("new");
  std::fs::copy(pkg, &tmp).map_err(|e| format!("写入新版本失败：{e}"))?;
  std::fs::set_permissions(&tmp, std::os::unix::fs::PermissionsExt::from_mode(0o755))
    .map_err(|e| format!("设置可执行权限失败：{e}"))?;
  std::fs::rename(&tmp, &cur).map_err(|e| {
    let _ = std::fs::remove_file(&tmp);
    format!("替换当前 AppImage 失败：{e}")
  })?;
  super::with(|s| s.phase = Phase::Installed);
  Ok("已替换 AppImage，重启 PureVox 生效".into())
}

/// deb / rpm：交给系统包管理器，提权走 `pkexec`（polkit 图形授权框）。
///
/// **不代改用户权限、不静默降级**：`pkexec` 缺失 / 用户取消授权 / 包管理器报错，
/// 都把真实原因（含包路径）带回去，界面据此显示。
fn install_package(pkg: &Path, cmd: &str, arg: &str) -> Result<String, String> {
  let (ok, why) = match std::process::Command::new("pkexec")
    .arg(cmd)
    .arg(arg)
    .arg(pkg)
    .output()
  {
    Ok(o) => (o.status.success(), tail(&o.stderr)),
    Err(e) => (
      false,
      format!("找不到或无法执行 pkexec（polkit 图形授权）：{e}"),
    ),
  };
  if ok {
    super::with(|s| s.phase = Phase::Installed);
    Ok(format!(
      "已通过系统包管理器安装（{cmd} {arg}），重启 PureVox 生效"
    ))
  } else {
    let msg = format!("安装失败（{cmd} {arg} {}）：{why}", pkg.display());
    super::with(|s| {
      s.phase = Phase::Failed;
      s.reason = msg.clone();
    });
    Err(msg)
  }
}

/// 包管理器输出可能很长（dpkg 会絮叨），只取最后几行非空内容当原因。
fn tail(stderr: &[u8]) -> String {
  let text = String::from_utf8_lossy(stderr);
  let lines: Vec<&str> = text
    .lines()
    .map(str::trim)
    .filter(|l| !l.is_empty())
    .collect();
  if lines.is_empty() {
    return "包管理器没有给出原因".into();
  }
  lines[lines.len().saturating_sub(3)..].join("；")
}
