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

//! 配置持久化：会话计划存 `~/.purevox/session.json`（DESIGN.md §5、§6.2 的目录约定）。

use std::io::Write;
use std::path::{Path, PathBuf};
use std::sync::{OnceLock, mpsc};
use std::time::Duration;

use crate::plan::Plan;

fn home_dir() -> Result<PathBuf, String> {
  std::env::var_os("USERPROFILE")
    .or_else(|| std::env::var_os("HOME"))
    .map(PathBuf::from)
    .ok_or_else(|| "找不到用户目录（USERPROFILE/HOME）".to_string())
}

pub fn config_dir() -> Result<PathBuf, String> {
  // 单测**绝不许**碰用户真实的 `~/.purevox/`：`audio/mod.rs` 的测试会 `persist_plan`，
  // 一次 `cargo test` 就能把开发者的会话计划覆盖成默认计划（2026-10-18 实际踩到）。
  // 目录**按测试线程**各给一份（测试是并行跑的，共用一份又会互相覆盖）。
  #[cfg(test)]
  return Ok(test_dir::get());
  #[cfg(not(test))]
  Ok(home_dir()?.join(".purevox"))
}

/// 单测用的配置目录（`cfg(test)` 专属）：每个测试线程一份、懒建、互不干扰。
#[cfg(test)]
mod test_dir {
  use std::cell::RefCell;
  use std::path::PathBuf;
  use std::sync::atomic::{AtomicU64, Ordering};

  thread_local! {
    static DIR: RefCell<Option<PathBuf>> = const { RefCell::new(None) };
  }
  static SEQ: AtomicU64 = AtomicU64::new(0);

  pub fn get() -> PathBuf {
    DIR.with(|d| {
      let mut d = d.borrow_mut();
      d.get_or_insert_with(|| {
        let n = SEQ.fetch_add(1, Ordering::Relaxed);
        let p = std::env::temp_dir().join(format!("purevox-test-cfg-{}-{n}", std::process::id()));
        let _ = std::fs::create_dir_all(&p);
        p
      })
      .clone()
    })
  }
}

pub fn session_path() -> Result<PathBuf, String> {
  Ok(config_dir()?.join("session.json"))
}

/// 展开路径里的 `~`（用户目录）。
pub fn expand_path(p: &str) -> PathBuf {
  let p = p.trim();
  if let Some(rest) = p.strip_prefix("~/").or_else(|| p.strip_prefix("~\\"))
    && let Ok(home) = home_dir()
  {
    return home.join(rest);
  }
  PathBuf::from(p)
}

/// TSE 参考录音默认路径（`~/.purevox/tse_reference.wav`）。
/// **唯一路径**：界面不给路径设置，录音 / 加载 / 「打开文件夹」都从这里取。
pub fn default_tse_reference() -> PathBuf {
  config_dir()
    .map(|d| d.join("tse_reference.wav"))
    .unwrap_or_else(|_| PathBuf::from("tse_reference.wav"))
}

/// 界面语言（`zh` / `en`），存 `~/.purevox/settings.json`（供托盘菜单等 Rust 侧使用）。
pub fn load_lang() -> String {
  load_settings().lang
}

pub fn save_lang(lang: &str) -> Result<(), String> {
  let mut s = load_settings();
  s.lang = if lang == "en" {
    "en".to_string()
  } else {
    "zh".to_string()
  };
  save_settings(&s)
}

/// 应用设置（语言 / 热键 / 提示音）。
#[derive(serde::Serialize, serde::Deserialize, Clone, Debug)]
pub struct AppSettings {
  #[serde(default = "d_lang")]
  pub lang: String,
  #[serde(default = "d_true")]
  pub hotkey_on: bool,
  /// 规范键位串（空 = 不监听）；见 `hotkey::parse_spec`
  #[serde(default = "d_hotkey")]
  pub hotkey: String,
  #[serde(default = "d_true")]
  pub cue_on: bool,
  #[serde(default = "d_cue")]
  pub cue_start: String,
  #[serde(default = "d_cue")]
  pub cue_stop: String,
  /// 启动时自动运行引擎并隐藏窗口（收到托盘）
  #[serde(default)]
  pub start_hidden: bool,
  /// 网络服务监听端口（**固定但可编辑**，DESIGN.md §4.1.1）。默认 `proto::PORT`。
  /// 改了要重启服务才生效（监听端口变了必须重绑）。
  #[serde(default = "d_net_port")]
  pub net_port: u16,
  /// 主窗口上次的位置与大小（物理像素；关到托盘 / 重启后回到原处）。
  /// `None` = 还没记过（首次运行用 `tauri.conf.json` 的默认尺寸）。
  #[serde(default)]
  pub window: Option<WindowGeom>,
  /// 网络服务对外广告用的网卡地址（空 = 自动用默认路由那张）。
  ///
  /// **为什么不直接记网卡名**：DHCP 换地址后名字还在、地址没了，记地址反而会指到一个
  /// 已经不属于本机的 IP 上。所以记的是**地址本身**，取不到就回落自动（见 `addrs.rs`）。
  #[serde(default)]
  pub net_ip: String,
}

/// 主窗口几何（**物理像素**）。
///
/// 存物理量而不是逻辑量：同一个窗口在不同缩放比的显示器上，逻辑尺寸会变，
/// 物理尺寸才是「用户上次看到的那块地方」。
///
/// 尺寸语义按平台（读写都在 [`crate::winstate`]，两边自洽、不会因边框宽度反复缩水）：
/// - Windows：`WINDOWPLACEMENT.rcNormalPosition` 的**外框**（含标题栏/边框），最大化时也是还原外框；
/// - Linux / 其它：`outer_position`（外框左上角）+ `inner_size`（客户区）。
#[derive(serde::Serialize, serde::Deserialize, Clone, Copy, Debug, PartialEq, Eq)]
pub struct WindowGeom {
  pub x: i32,
  pub y: i32,
  pub w: u32,
  pub h: u32,
  /// 记录时是否最大化；最大化时 `x/y/w/h` 仍是**还原尺寸**（见 `winstate`）
  #[serde(default)]
  pub maximized: bool,
}

/// 设置文件的读-改-写互斥：窗口几何的定时保存与界面里的设置保存不能互相覆盖。
static SETTINGS_LOCK: std::sync::Mutex<()> = std::sync::Mutex::new(());

/// 只更新窗口几何（不动其它设置）。
pub fn save_window_geom(g: WindowGeom) -> Result<(), String> {
  let _guard = SETTINGS_LOCK.lock().unwrap_or_else(|e| e.into_inner());
  let mut s = load_settings();
  if s.window == Some(g) {
    return Ok(());
  }
  s.window = Some(g);
  write_settings(&s)
}

fn d_net_port() -> u16 {
  crate::net::proto::PORT
}

fn d_lang() -> String {
  "zh".to_string()
}
fn d_true() -> bool {
  true
}
fn d_hotkey() -> String {
  "Alt+.".to_string()
}
fn d_cue() -> String {
  "soft".to_string()
}

impl Default for AppSettings {
  fn default() -> Self {
    AppSettings {
      lang: d_lang(),
      hotkey_on: true,
      hotkey: d_hotkey(),
      cue_on: true,
      cue_start: d_cue(),
      cue_stop: d_cue(),
      start_hidden: false,
      net_port: d_net_port(),
      net_ip: String::new(),
      window: None,
    }
  }
}

fn settings_path() -> Result<PathBuf, String> {
  Ok(config_dir()?.join("settings.json"))
}

/// 原子写：先写 `<名>.tmp` + `fsync`，再把旧文件备份成 `<名>.bak`，最后 `rename` 覆盖。
///
/// **不许直接 `std::fs::write`**：写到一半断电/被杀就是一个半截 JSON，下一次启动
/// `load_*` 解析失败 → 用户的计划/设置被默认值顶掉。rename 在同一目录内是原子的。
fn write_atomic(path: &Path, text: &str) -> Result<(), String> {
  let name = path
    .file_name()
    .map(|s| s.to_string_lossy().to_string())
    .ok_or_else(|| format!("非法配置路径：{}", path.display()))?;
  let dir = path.parent().unwrap_or_else(|| Path::new("."));
  std::fs::create_dir_all(dir).map_err(|e| format!("创建配置目录失败：{e}"))?;
  let tmp = dir.join(format!("{name}.tmp"));
  {
    let mut f = std::fs::File::create(&tmp).map_err(|e| format!("写临时文件失败：{e}"))?;
    f.write_all(text.as_bytes())
      .map_err(|e| format!("写临时文件失败：{e}"))?;
    f.sync_all().map_err(|e| format!("刷盘失败：{e}"))?;
  }
  if path.is_file() {
    let bak = dir.join(format!("{name}.bak"));
    let _ = std::fs::copy(path, &bak);
  }
  std::fs::rename(&tmp, path).map_err(|e| format!("替换配置文件失败：{e}"))
}

/// 解析失败时把坏文件改名留档（不覆盖、不静默丢弃），并写事件日志。
fn quarantine(path: &Path, reason: &str) {
  let stamp = crate::debug::now_ms();
  let name = path
    .file_name()
    .map(|s| s.to_string_lossy().to_string())
    .unwrap_or_else(|| "config".into());
  let bad = path.with_file_name(format!("{name}.corrupt-{stamp}"));
  let moved = std::fs::rename(path, &bad);
  crate::debug::events::log(
    "error",
    "config",
    match &moved {
      Ok(()) => format!(
        "配置 {} 解析失败（{reason}），原文件已保留为 {}，本次使用默认值",
        path.display(),
        bad.display()
      ),
      Err(e) => format!(
        "配置 {} 解析失败（{reason}），且无法改名留档（{e}）；本次使用默认值",
        path.display()
      ),
    },
  );
}

pub fn load_settings() -> AppSettings {
  let Ok(path) = settings_path() else {
    return AppSettings::default();
  };
  let text = match std::fs::read_to_string(&path) {
    Ok(t) => t,
    Err(e) if e.kind() == std::io::ErrorKind::NotFound => return AppSettings::default(),
    Err(e) => {
      crate::debug::events::log(
        "error",
        "config",
        format!("读取设置失败（{}）：{e}；本次使用默认值", path.display()),
      );
      return AppSettings::default();
    }
  };
  match serde_json::from_str(&text) {
    Ok(s) => s,
    Err(e) => {
      quarantine(&path, &e.to_string());
      AppSettings::default()
    }
  }
}

pub fn save_settings(s: &AppSettings) -> Result<(), String> {
  let _guard = SETTINGS_LOCK.lock().unwrap_or_else(|e| e.into_inner());
  write_settings(s)
}

/// 真正落盘（调用方已持 [`SETTINGS_LOCK`]）。
fn write_settings(s: &AppSettings) -> Result<(), String> {
  let text = serde_json::to_string_pretty(s).map_err(|e| format!("序列化失败：{e}"))?;
  write_atomic(&settings_path()?, &text)
}

/// 读取已保存的计划；无文件返回默认计划。
///
/// 解析失败：**保留原文件**（改名为 `.corrupt-<ts>`）并写事件日志，再返回默认计划。
/// 这样下一次保存默认计划不会把用户的配置永久覆盖掉。
pub fn load_plan() -> Plan {
  let Ok(path) = session_path() else {
    return Plan::default();
  };
  let text = match std::fs::read_to_string(&path) {
    Ok(t) => t,
    Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Plan::default(),
    Err(e) => {
      crate::debug::events::log(
        "error",
        "config",
        format!(
          "读取会话计划失败（{}）：{e}；本次使用默认计划",
          path.display()
        ),
      );
      return Plan::default();
    }
  };
  match serde_json::from_str(&text) {
    Ok(p) => p,
    Err(e) => {
      quarantine(&path, &e.to_string());
      Plan::default()
    }
  }
}

pub fn save_plan(plan: &Plan) -> Result<(), String> {
  let text = serde_json::to_string_pretty(plan).map_err(|e| format!("序列化失败：{e}"))?;
  write_atomic(&session_path()?, &text)
}

// ---------------------------------------------------------------------------
// 热参数落盘：合并连续写入（滑杆一次拖动几十个事件，不该几十次 fsync）
// ---------------------------------------------------------------------------

static SAVER: OnceLock<mpsc::Sender<Plan>> = OnceLock::new();

fn saver() -> &'static mpsc::Sender<Plan> {
  SAVER.get_or_init(|| {
    let (tx, rx) = mpsc::channel::<Plan>();
    std::thread::Builder::new()
      .name("config-saver".into())
      .spawn(move || {
        let mut pending: Option<Plan> = None;
        loop {
          match rx.recv_timeout(Duration::from_millis(250)) {
            Ok(p) => pending = Some(p),
            Err(mpsc::RecvTimeoutError::Timeout) => {
              if let Some(p) = pending.take()
                && let Err(e) = save_plan(&p)
              {
                crate::debug::events::log("error", "config", format!("保存会话计划失败：{e}"));
              }
            }
            Err(mpsc::RecvTimeoutError::Disconnected) => {
              if let Some(p) = pending.take() {
                let _ = save_plan(&p);
              }
              break;
            }
          }
        }
      })
      .ok();
    tx
  })
}

/// 请求落盘（合并 250 ms 内的连续改动）。结构性变更仍走 [`save_plan`] 立即写。
pub fn save_plan_debounced(plan: Plan) {
  if let Err(e) = saver().send(plan) {
    crate::debug::events::log("error", "config", format!("投递保存请求失败：{e}"));
  }
}

#[cfg(test)]
mod tests {
  use super::*;

  #[test]
  fn save_plan_is_atomic_and_keeps_backup() {
    save_plan(&Plan::default()).expect("第一次保存");
    save_plan(&Plan::default()).expect("第二次保存");
    let path = session_path().expect("测试配置路径");
    assert!(path.is_file(), "正式文件必须存在");
    let bak = path.with_file_name("session.json.bak");
    assert!(bak.is_file(), "覆盖前应留下 .bak");
    let tmp = path.with_file_name("session.json.tmp");
    assert!(!tmp.exists(), "临时文件必须被 rename 掉，不能残留");
  }

  /// 解析失败必须**保留原文件**（改名留档），而不是静默返回默认值然后被下一次保存覆盖。
  #[test]
  fn corrupt_plan_is_quarantined_not_lost() {
    let path = session_path().expect("测试配置路径");
    std::fs::create_dir_all(path.parent().unwrap()).expect("建测试目录");
    std::fs::write(&path, "{ 这不是 JSON").expect("写坏配置");
    let p = load_plan();
    assert!(!p.columns.is_empty(), "回退到默认计划");
    assert!(!path.exists(), "坏文件应被改名留档，而不是留在原处");
    let dir = path.parent().unwrap();
    let quarantined = std::fs::read_dir(dir)
      .expect("读测试目录")
      .filter_map(|e| e.ok())
      .any(|e| {
        e.file_name()
          .to_string_lossy()
          .contains("session.json.corrupt-")
      });
    assert!(quarantined, "必须能在目录里找到 .corrupt-* 留档");
  }
}
