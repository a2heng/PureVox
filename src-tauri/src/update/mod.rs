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

//! 更新检查（桌面）：查本仓库 GitHub Releases 的 `latest`，有新版本就下载安装包。
//!
//! **唯一数据源 = GitHub Releases API**（`repos/{owner}/{repo}/releases/latest`），
//! 产物由 `release.yml` 在打 tag 时挂上去，两端靠**文件名后缀**匹配（不靠版本号，
//! 因为版本号只有年月日、同日多次发布会重复，见 `version.rs`）。
//!
//! **比较依据 = tag 的完整时间戳**（`v2026.10.12.1430`），不是显示版本号
//! `26.10.12`（`yy.M.d`）：同日发两次时显示版本相同，靠时分才能分出新旧。
//! 开发构建没有 CI 注入的时间戳（`version::is_dev()`），此时**不下结论**，
//! 报「开发构建」而不是拿 Cargo 的 0.1.0 去比。
//!
//! 状态机（[`Phase`]）单一实例挂在全局（与 `NetHub` / `MediaHub` 同款），
//! 同时进调试接口 `/debug` 的 `update` 段（AGENTS.md §1.2）。
//! 下载在后台线程跑（带进度），**不进音频线程**；安装是平台实现（分文件）。

use std::sync::{Mutex, OnceLock};

use serde::Serialize;

use crate::version;

/// GitHub 仓库（发版唯一去处，桌面 / 安卓两端共用）。
pub const REPO: &str = "a2heng/PureVox";

/// GitHub API 端点（拼自 [`REPO`]，避免两处写死仓库名）。
fn api_latest() -> String {
  format!("https://api.github.com/repos/{REPO}/releases/latest")
}

/// 状态机阶段。字符串直接进 JSON（`/debug` 的 `update.phase` 与界面同源）。
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum Phase {
  /// 还没查过
  Idle,
  /// 正在查 GitHub
  Checking,
  /// 已是最新
  UpToDate,
  /// 有新版本（未下载）
  Available,
  /// 正在下载安装包
  Downloading,
  /// 已下载到本地，可安装
  Ready,
  /// 正在安装（安装器/包管理器已接管；界面显示「正在安装…」）
  Installing,
  /// **装好了，等用户重启**（Linux：AppImage 已替换完或 deb/rpm 已由包管理器装完；
  /// 本进程继续跑，下次启动就是新版）
  /// （Windows / 其它平台不会构造它，故显式忽略 dead_code）
  #[allow(dead_code)]
  Installed,
  /// 本平台不支持自动更新（给出原因 + release 页面）
  /// （只在部分平台的 `install_*.rs` 里构造——本平台支持时该分支不会被走到）
  #[allow(dead_code)]
  Unsupported,
  /// 失败（`reason` 是原因）
  Failed,
}

/// 对外状态（界面轮询 / `/debug` 共用这一份）。
#[derive(Clone, Debug, Serialize)]
pub struct Status {
  pub phase: Phase,
  /// 本机显示版本（`version::VERSION`，如 `26.10.12`）
  pub current_version: String,
  /// 本机构建时间戳（`version::STAMP`，开发态为空）
  pub current_stamp: String,
  /// 本机的**安装方式**（`appimage` / `deb` / `rpm` / 其它平台的 `nsis` …）。
  /// Linux 侧由二进制里的 bundle type 标记决定（`tauri::utils::platform::bundle_type()`），
  /// **产物按它来挑**：deb 用户只下 `.deb`，不会去下 140 MB 的 AppImage。
  pub install_kind: String,
  /// 最新 release 的 tag（`v2026.10.12.1430`）
  pub latest_tag: String,
  /// 最新 release 的显示版本（由 tag 推导的 `yy.M.d`）
  pub latest_version: String,
  /// 更新说明（release body，前若干行由界面截断显示）
  pub notes: String,
  /// release 页面地址（打不开 / 不支持时给用户手动下载）
  pub url: String,
  /// 发布日期
  pub published_at: String,
  /// 匹配到的产物名（空 = 没匹配到）
  pub asset_name: String,
  /// 匹配到的产物下载地址（与 `asset_name` 同源，`pick_asset` 一起给出）
  pub asset_url: String,
  /// 产物字节数（0 = 未知）
  pub asset_size: u64,
  /// 已下载字节（`Downloading` 时变化）
  pub received: u64,
  /// 下载完成的本地路径（`Ready` 时非空）
  pub saved_path: String,
  /// 失败 / 不支持的原因（Rust 诊断文本，按 AGENTS.md §4 边界不上 i18n）
  pub reason: String,
}

impl Default for Status {
  fn default() -> Self {
    Status {
      phase: Phase::Idle,
      current_version: version::VERSION.to_string(),
      current_stamp: version::STAMP.to_string(),
      install_kind: imp::kind_name().to_string(),
      latest_tag: String::new(),
      latest_version: String::new(),
      notes: String::new(),
      url: String::new(),
      published_at: String::new(),
      asset_name: String::new(),
      asset_url: String::new(),
      asset_size: 0,
      received: 0,
      saved_path: String::new(),
      reason: String::new(),
    }
  }
}

/// 全局状态单例（与 `NetHub` / `MediaHub` 同款）。
struct Hub {
  status: Mutex<Status>,
}

static HUB: OnceLock<Hub> = OnceLock::new();

fn hub() -> &'static Hub {
  HUB.get_or_init(|| Hub {
    status: Mutex::new(Status::default()),
  })
}

/// 当前状态快照（界面 / 调试接口都读它）。
pub fn status() -> Status {
  hub().status.lock().unwrap().clone()
}

/// 就地改状态（各阶段共用）。
fn with<R>(f: impl FnOnce(&mut Status) -> R) -> R {
  let mut g = hub().status.lock().unwrap();
  f(&mut g)
}

/// 下载目录（`~/.purevox/update`）。
fn download_dir() -> Result<std::path::PathBuf, String> {
  let d = crate::config::config_dir()?.join("update");
  std::fs::create_dir_all(&d).map_err(|e| format!("创建下载目录失败：{e}"))?;
  Ok(d)
}

/// 设置失败状态（原因一定带上，别静默）。
pub fn fail(reason: impl Into<String>) {
  with(|s| {
    s.phase = Phase::Failed;
    s.reason = reason.into();
  });
}

// ── GitHub release JSON（只取用得到的字段）──

#[derive(serde::Deserialize)]
struct Release {
  tag_name: String,
  #[serde(default)]
  body: String,
  html_url: String,
  #[serde(default)]
  published_at: String,
  #[serde(default)]
  assets: Vec<Asset>,
}

#[derive(serde::Deserialize)]
struct Asset {
  name: String,
  browser_download_url: String,
  #[serde(default)]
  size: u64,
}

/// `v2026.10.12.1430` → `2026.10.12.1430`；不合法返回 None。
fn tag_to_stamp(tag: &str) -> Option<String> {
  let t = tag.strip_prefix('v').unwrap_or(tag);
  version::parse_stamp(t).map(|_| t.to_string())
}

/// `2026.10.08.1430` → `26.10.8`（界面显示版本，与 `version.py` 的映射同款）。
/// 段按整数取值再拼，故 `08` 打成 `8`——必须与 CI 产出的包版本逐字相同。
fn stamp_to_version(stamp: &str) -> String {
  let p: Vec<&str> = stamp.split('.').collect();
  if p.len() < 3 {
    return stamp.to_string();
  }
  let y: i64 = p[0].parse().unwrap_or(0);
  let mo: i64 = p[1].parse().unwrap_or(0);
  let d: i64 = p[2].parse().unwrap_or(0);
  format!("{}.{}.{}", y % 100, mo, d)
}

/// 按给定后缀优先级挑产物（纯函数，单测直接喂后缀表）。
///
/// 后缀表来自平台实现（`install_*.rs` 的 `asset_suffixes()`）：Linux 是**按安装方式**
/// 给出一条（deb 只认 `.deb`），不再有「AppImage 优先」的跨格式兜底 —— 别的格式的包
/// 用户装不上，下了也是白下。
fn pick_asset_with(assets: &[Asset], suffixes: &[&str]) -> Option<(String, String, u64)> {
  // CI 产出的文件名见 `release.yml` / `release_stage.sh`（统一重命名成
  // `PureVox-<版本>-<平台>.<ext>`，故后缀稳定）。
  for suf in suffixes {
    if let Some(a) = assets
      .iter()
      .find(|a| a.name.to_ascii_lowercase().ends_with(suf))
    {
      return Some((a.name.clone(), a.browser_download_url.clone(), a.size));
    }
  }
  None
}

/// 查 GitHub Releases（`latest`）。成功即更新全局状态；失败写 `Failed` + 原因。
pub async fn check() -> Result<Status, String> {
  with(|s| {
    s.phase = Phase::Checking;
    s.reason.clear();
  });
  let client = match http_client() {
    Ok(c) => c,
    Err(e) => {
      fail(e.clone());
      return Err(e);
    }
  };
  let rel = match client
    .get(api_latest())
    .header("Accept", "application/vnd.github+json")
    .header("User-Agent", "PureVox-Desktop")
    .send()
    .await
  {
    Ok(r) => match r.error_for_status() {
      Ok(r) => match r.json::<Release>().await {
        Ok(v) => v,
        Err(e) => {
          let msg = format!("解析 GitHub 返回失败：{e}");
          fail(msg.clone());
          return Err(msg);
        }
      },
      Err(e) => {
        let msg = format!("GitHub 返回错误：{e}");
        fail(msg.clone());
        return Err(msg);
      }
    },
    Err(e) => {
      let msg = format!("连不上 GitHub（检查网络 / 代理）：{e}");
      fail(msg.clone());
      return Err(msg);
    }
  };

  let latest_stamp = match tag_to_stamp(&rel.tag_name) {
    Some(s) => s,
    None => {
      let msg = format!("无法识别的版本号：{}", rel.tag_name);
      fail(msg.clone());
      return Err(msg);
    }
  };
  let asset = pick_asset_with(&rel.assets, imp::asset_suffixes());

  with(|s| {
    s.latest_tag = rel.tag_name.clone();
    s.latest_version = stamp_to_version(&latest_stamp);
    s.notes = rel.body.clone();
    s.url = rel.html_url.clone();
    s.published_at = rel.published_at.clone();
    match &asset {
      Some((name, url, size)) => {
        s.asset_name = name.clone();
        s.asset_url = url.clone();
        s.asset_size = *size;
      }
      None => {
        s.asset_name.clear();
        s.asset_url.clear();
        s.asset_size = 0;
      }
    }
    // 开发构建没有可比的时间戳 → **不能**下「已是最新」的结论。
    //
    // 但也**不能**就此什么都不做：那样「更新到底能不能用」只能等发版以后赌一次
    // （2026-10-26：Windows / Linux 两侧都报「不能更新」，而开发构建连查都查不到，
    // 根本没法复现）。所以开发构建一律按「有新版」处理 —— 界面照常给出最新版本、
    // 更新说明、产物名，检查 → 下载 → 安装三步都能点，reason 里写清这是测试版逻辑。
    let dev = version::is_dev();
    s.phase = if dev {
      Phase::Available
    } else {
      decide(version::STAMP, &latest_stamp)
    };
    if dev {
      s.reason = "开发构建（无 CI 版本戳）：按「有新版」处理，供测试更新流程".into();
    }
    // 只有「真有新版」时才值得报「本机这种安装方式没有对应的包」——已是最新时
    // 报这个只会让人以为更新坏了。
    if s.phase == Phase::Available && asset.is_none() {
      let suffixes = imp::asset_suffixes();
      if suffixes.is_empty() {
        // 认不出安装方式（没打标的二进制）：结构性不可用，别让用户反复点
        s.phase = Phase::Unsupported;
        s.reason = format!(
          "认不出本机安装方式（{}），无法自动更新；请到 release 页面手动下载安装{}",
          imp::kind_name(),
          if dev {
            "（测试版可用环境变量 PUREVOX_UPDATE_KIND 指定安装方式）"
          } else {
            ""
          }
        );
      } else {
        s.phase = Phase::Failed;
        s.reason = format!(
          "这个 release 里没有 {} 安装用的包（找过 {}）",
          imp::kind_name(),
          suffixes.join(", ")
        );
      }
    }
  });
  Ok(status())
}

/// 更新判定（**纯函数，不碰网络**）：本机构建戳 vs 最新 release 的 tag 戳 → 阶段。
///
/// 抽出来的理由：这段判定原先埋在 [`check`] 的状态闭包里，**只有联网跑完整条链路才覆盖得到**
/// ——而 CI 不该、也做不到去 GitHub 真查一次。抽成纯函数后，`cargo test` 就能把
/// 「同日多次发版靠时分区分」「本机更新时不应提示」这两条最容易错的地方钉死。
///
/// `mine` 为空串（开发构建）时 `parse_stamp` 返回 None → 落到 `UpToDate`；调用方必须
/// **先判 `version::is_dev()` 再调本函数**（`Idle` + 明确原因，别拿 Cargo 回退的 0.1.0 去比）。
fn decide(mine_stamp: &str, latest_stamp: &str) -> Phase {
  match (
    version::parse_stamp(mine_stamp),
    version::parse_stamp(latest_stamp),
  ) {
    (Some(mine), Some(theirs)) if theirs > mine => Phase::Available,
    _ => Phase::UpToDate,
  }
}

/// 客户端 UA（GitHub 要求带 UA，两个客户端共用一份）。
const UA: &str = "PureVox-Desktop";
/// 下载时**空闲**多久算卡死（ms）：每收到一块就重置，不是总时长。
const IDLE_TIMEOUT_S: u64 = 30;
/// 下载最多试几次（**每次从上次断的地方续**，所以多试几次几乎总能下完）。
const DOWNLOAD_ATTEMPTS: u32 = 6;

/// 检查更新用的 HTTP 客户端：**整条请求 15 s 封顶**（检查不能拖住界面）。
fn http_client() -> Result<reqwest::Client, String> {
  reqwest::Client::builder()
    .user_agent(UA)
    .timeout(std::time::Duration::from_secs(15))
    .connect_timeout(std::time::Duration::from_secs(10))
    .build()
    .map_err(|e| format!("创建 HTTP 客户端失败：{e}"))
}

/// 下载安装包用的 HTTP 客户端：**不设总超时**。
///
/// 这条是 2026-10-18 那次「下载中断：error decoding response body」的根因：以前下载
/// 和检查共用一个 `timeout(15s)` 的客户端，而 `timeout` 是**整条请求**（含响应体）的
/// 时限 —— 安装包 50~140 MB，实测直连 GitHub 资产只有 ~41 KB/s（72.8 MB 要 30 min）、
/// 走代理 ~1.9 MB/s（也要约 40 s），**任何超过 15 s 的下载都会被拦腰掐断**；而 reqwest
/// 把「body 被掐断」归到 `Kind::Decode`，Display 正好是 `error decoding response body`
/// —— 一句既看不懂、也和真实原因（太大/太慢）无关的报错（reqwest 自带单测
/// `decode_body_timeout_is_timeout` 写的就是这条路径）。所以：
/// - 连接 15 s（连不上要快失败）；
/// - **空闲 30 s**（`read_timeout` 是「每收到一块就重置」，慢而下得动就不算超时）。
fn download_client() -> Result<reqwest::Client, String> {
  reqwest::Client::builder()
    .user_agent(UA)
    .connect_timeout(std::time::Duration::from_secs(15))
    .read_timeout(std::time::Duration::from_secs(IDLE_TIMEOUT_S))
    .build()
    .map_err(|e| format!("创建下载客户端失败：{e}"))
}

/// 下载过程中的错误 → 给人看的原因（超时与真断流分开说，数字都带上）。
fn describe_download_error(e: &reqwest::Error) -> String {
  if e.is_timeout() {
    format!("下载超时（{IDLE_TIMEOUT_S} s 没收到数据，连接在走但太慢或卡住）：{e}")
  } else {
    format!("下载中断：{e}")
  }
}

/// 一次下载尝试：把响应体**追加**写进 `file`，`total` = 已落盘的字节数（本函数内推进）。
///
/// - `total > 0` 时带 `Range: bytes=<total>-` 续传；服务器不理 `Range`（回 200 整包）时
///   **清空文件重来**，否则会把整包接在旧进度后面（文件翻倍）。
/// - 响应体流到一半断掉 → 返回 `Err`，**已经落盘的字节保留**（下次接着下）。
async fn fetch_into(
  client: &reqwest::Client,
  url: &str,
  file: &mut std::fs::File,
  total: &mut u64,
  mut on_progress: impl FnMut(u64),
) -> Result<(), String> {
  use futures_util::StreamExt;
  use std::io::{Seek, Write};

  let mut req = client.get(url);
  if *total > 0 {
    req = req.header(reqwest::header::RANGE, format!("bytes={}-", *total));
  }
  let resp = req
    .send()
    .await
    .and_then(|r| r.error_for_status())
    .map_err(|e| describe_download_error(&e))?;
  // 服务器没按 Range 回（200 = 整包）：从零来，别把整包接在旧进度后面
  if *total > 0 && resp.status() == reqwest::StatusCode::OK {
    file
      .set_len(0)
      .and_then(|_| file.rewind())
      .map_err(|e| format!("重置临时文件失败：{e}"))?;
    *total = 0;
  }
  let mut stream = resp.bytes_stream();
  while let Some(chunk) = stream.next().await {
    let chunk = chunk.map_err(|e| describe_download_error(&e))?;
    file
      .write_all(&chunk)
      .map_err(|e| format!("写文件失败：{e}"))?;
    *total += chunk.len() as u64;
    on_progress(*total);
  }
  Ok(())
}

/// 下载匹配的安装包到 `~/.purevox/update/`，带进度写回全局状态。
/// 必须在后台线程/tokio 里跑（网络 I/O）。完成置 `Ready`。
pub async fn download() -> Result<Status, String> {
  let (name, url, size) = {
    let s = status();
    if s.phase != Phase::Available && s.phase != Phase::Ready && s.phase != Phase::Failed {
      return Err("当前状态不能下载（先检查更新）".into());
    }
    if s.asset_name.is_empty() {
      return Err("该 release 没有匹配本平台的产物".into());
    }
    (s.asset_name.clone(), s.asset_url(), s.asset_size)
  };
  let dest = download_dir()?.join(&name);
  // `.part` 临时名：下载中断不留半个文件冒充完成品；**带完整产物名**（不是换扩展名 ——
  // `with_extension` 会让 `.deb` 与 `.AppImage` 撞成同一个临时名，续传会串味）。
  let tmp = dest.with_file_name(format!("{name}.part"));

  with(|s| {
    s.phase = Phase::Downloading;
    s.received = 0;
    s.reason.clear();
    s.saved_path.clear();
  });

  let client = match download_client() {
    Ok(c) => c,
    Err(e) => {
      fail(e.clone());
      return Err(e);
    }
  };
  // 断点续传：`.part` 可能是上一次（中断/超时）留下的进度，接着下
  let mut total: u64 = std::fs::metadata(&tmp).map(|m| m.len()).unwrap_or(0);
  if size > 0 && total > size {
    // 比目标还大 = 上一个版本的残留（产物名带版本号，正常情况下不会撞）
    total = 0;
    let _ = std::fs::remove_file(&tmp);
  }
  with(|s| s.received = total);

  use std::io::{Seek, Write};
  let mut last_err = String::new();
  for attempt in 1..=DOWNLOAD_ATTEMPTS {
    let mut file = match std::fs::OpenOptions::new()
      .create(true)
      .write(true)
      .truncate(false)
      .open(&tmp)
    {
      Ok(f) => f,
      Err(e) => {
        let msg = format!("写文件失败：{e}");
        fail(msg.clone());
        return Err(msg);
      }
    };
    if let Err(e) = file.seek(std::io::SeekFrom::Start(total)) {
      let msg = format!("定位临时文件失败：{e}");
      fail(msg.clone());
      return Err(msg);
    }
    let res = fetch_into(&client, &url, &mut file, &mut total, |n| {
      with(|s| s.received = n);
    })
    .await;
    let _ = file.flush();
    drop(file);
    match res {
      Ok(()) => {}
      Err(e) => last_err = e,
    }
    // 下完了（有已知大小就对齐大小，否则以「流正常结束」为准）
    if total > 0 && (size == 0 || total >= size) {
      break;
    }
    if attempt < DOWNLOAD_ATTEMPTS {
      // 退避：给网络一个喘息，然后把剩下的部分续上（不重头来）
      tokio::time::sleep(std::time::Duration::from_secs(1 << (attempt - 1).min(3))).await;
    }
  }
  if size > 0 && total != size {
    let msg = format!(
      "下载不完整（{} / {} 字节）：{}",
      total,
      size,
      if last_err.is_empty() {
        "连接反复中断".to_string()
      } else {
        last_err.clone()
      }
    );
    fail(msg.clone());
    return Err(msg);
  }
  if let Err(e) = std::fs::rename(&tmp, &dest) {
    let _ = std::fs::remove_file(&tmp);
    let msg = format!("保存安装包失败：{e}");
    fail(msg.clone());
    return Err(msg);
  }
  with(|s| {
    s.phase = Phase::Ready;
    s.received = size.max(s.received);
    s.saved_path = dest.to_string_lossy().into_owned();
  });
  Ok(status())
}

impl Status {
  /// 当前 release 里本平台产物的下载地址（`Available`/`Ready` 时有效）。
  fn asset_url(&self) -> String {
    self.asset_url.clone()
  }
}

// 平台实现分文件（AGENTS.md §4）：静默安装的方式各平台不同，
// 共享部分（查 / 下载 / 版本比较）都在本文件。
#[cfg(windows)]
#[path = "install_windows.rs"]
mod imp;
#[cfg(target_os = "linux")]
#[path = "install_linux.rs"]
mod imp;
#[cfg(not(any(windows, target_os = "linux")))]
#[path = "install_other.rs"]
mod imp;

/// 安装已下载的产物（平台实现，见 `install_*.rs`）。
///
/// **在状态锁里先把 `Ready` 推到 `Installing`**：既是「不许跳步」的守卫（连点两次安装，
/// 第二次会立刻被挡回），也让界面在 `pkexec` 授权框那几秒里显示「正在安装…」。
///
/// 最终阶段由各平台实现落下（`main.rs` 据此决定要不要退进程）：
/// - Windows → 保持 `Installing`（安装器接管、**本进程必须退出**）；
/// - Linux → `Installed`（装好了，进程继续跑，重启生效）或 `Failed`（带原因）；
/// - 其它平台 → `Unsupported`（带原因）。
pub fn install() -> Result<String, String> {
  {
    let mut s = hub().status.lock().unwrap();
    if s.phase != Phase::Ready {
      return Err(format!(
        "当前状态不能安装（{:?}）：先「检查更新」再「下载」",
        s.phase
      ));
    }
    s.phase = Phase::Installing;
    s.reason.clear();
  }
  let r = imp::install(&status());
  if let Err(e) = &r {
    // 平台实现通常已经写好 `Failed` + 原因；这里兜底「什么都没写」的那种
    // （原因就用返回的 Err 文本），免得界面永远停在「正在安装…」（不静默失败）。
    // `Unsupported` 不许被降级成 `Failed`。
    with(|s| {
      if s.phase == Phase::Installing {
        s.phase = Phase::Failed;
      }
      if s.phase == Phase::Failed && s.reason.is_empty() {
        s.reason = e.clone();
      }
    });
  }
  r
}

#[cfg(test)]
mod tests {
  use super::*;

  /// release.yml 的原文 —— 编译期嵌进来，所以 CI 改了产物命名、本仓库的测试立刻红，
  /// 不存在「忘了同步」。
  /// 产物命名规则的**唯一来源**：`tools/automation/release_stage.sh` 里的 `stage` 行。
  ///
  /// 曾经指向 release.yml —— 那段 `stage` 逻辑内联在 workflow 里，所以是 workflow
  /// 的事实来源。但**内联的脚本等于没被测过的脚本**：打第一次真 tag 才发现
  /// 「收集完文件名后 `cd ..` 回上一层 → 相对路径全失效」。搬到 .sh 之后，
  /// `run_release_stage.py` 能在本地用假产物树 + 假 gh 把它跑通。
  ///
  /// 注意 `.count()` 的断言不能省：源文件换了地方而这里忘了改的话，
  /// `lines().filter(...)` 会**一条都匹配不到**，`tpls.len() >= 6` 就是唯一
  /// 把它拦下来的东西。
  const RELEASE_STAGE_SH: &str = include_str!("../../../tools/automation/release_stage.sh");

  fn asset(name: &str) -> Asset {
    Asset {
      name: name.into(),
      browser_download_url: format!("https://example.invalid/{name}"),
      size: 1024,
    }
  }

  #[test]
  fn tag_to_stamp_strips_v_and_validates() {
    assert_eq!(
      tag_to_stamp("v2026.10.12.1430").as_deref(),
      Some("2026.10.12.1430")
    );
    // 没有 v 前缀也认（有人手工建 tag 会漏 v）
    assert_eq!(
      tag_to_stamp("2026.10.12.1430").as_deref(),
      Some("2026.10.12.1430")
    );
    assert_eq!(tag_to_stamp("v1.2.3"), None, "段数不对必须拒绝");
    assert_eq!(tag_to_stamp("v26.10.8"), None, "两段是显示版本，不是构建戳");
    assert_eq!(tag_to_stamp("v2026.13.12.1430"), None, "13 月非法");
  }

  #[test]
  fn stamp_to_version_matches_version_script_mapping() {
    // 与 tools/automation/version.py 的 major=年-2000 / minor=月 / build=日 一致。
    // 月/日都按整数取值再拼，故 09 月打成 9；version.py 那边用 int() 去前导零
    // （bash 会把 09 当八进制），两边必须逐字相同 —— 版本检测的门禁就在这儿。
    assert_eq!(stamp_to_version("2026.10.08.1430"), "26.10.8");
    assert_eq!(stamp_to_version("2026.09.30.1944"), "26.9.30");
    assert_eq!(stamp_to_version("2026.09.09.0901"), "26.9.9");
    assert_eq!(stamp_to_version("2026.01.02.0000"), "26.1.2");
    assert_eq!(
      stamp_to_version("bad"),
      "bad",
      "认不出来就原样返回，不编一个版本号"
    );
  }

  #[test]
  fn pick_asset_with_uses_suffix_priority() {
    // 后缀的先后 = 优先级：同一份清单里出现多个候选时，命中哪个只由 suffixes 的顺序
    // 决定（外层遍历 suffixes、内层 find assets），与产物在 release 里的先后无关。
    // 注意 Linux 的表现在是「一种安装方式一条」，跨格式兜底已被刻意去掉 ——
    // 见 install_linux.rs：把 AppImage 塞给 deb 用户，他装不上。
    let suffixes: &[&str] = &[".deb", ".rpm", ".appimage"];
    const V: &str = "26.10.12";
    // 干扰项：谁都不认的包 + 三个候选**倒序**放
    let assets = vec![
      asset(&format!("PureVox-{V}-other.tar.gz")),
      asset(&format!("PureVox-{V}-cand.appimage")),
      asset(&format!("PureVox-{V}-cand.rpm")),
      asset(&format!("PureVox-{V}-cand.deb")),
    ];
    let (name, url, size) = pick_asset_with(&assets, suffixes).expect("应匹配到候选产物");
    let want = format!("PureVox-{V}-cand.deb");
    assert_eq!(name, want, "应按 suffixes 的优先级选第一个");
    assert_eq!(url, format!("https://example.invalid/{want}"));
    assert_eq!(size, 1024);

    // 产物名大小写混着来（GitHub 上的名字是人起的）也得认
    let upper = [asset(&format!("PureVox-{V}-X.APPIMAGE"))];
    assert!(pick_asset_with(&upper, &[".appimage"]).is_some());
  }

  #[test]
  fn pick_asset_with_returns_none_for_foreign_assets() {
    // 安卓 apk / 其它格式不能被桌面端认领：认领了就会白下一个装不上的包
    let assets = [
      asset("PureVox-26.10.12-android-arm64.apk"),
      asset("PureVox-26.10.12-other.tar.gz"),
    ];
    for suf in [".deb", ".rpm", ".appimage", "-setup.exe"] {
      assert!(
        pick_asset_with(&assets, &[suf]).is_none(),
        "后缀 {suf} 不该匹配到别的产物"
      );
    }
    // 空表（认不出安装方式 / 其它平台）一律不挑
    assert!(pick_asset_with(&assets, &[]).is_none());
  }

  /// **每种安装方式只认自己那种包**：deb 用户不会下到 140 MB 的 AppImage（装不上），
  /// rpm 同理。查的是真实表（`install_linux.rs` 的 `suffixes_for`），不解析源码。
  ///
  /// 只在 Linux 上编译（`install_linux.rs` 本身就是 `cfg(linux)`）；`tests.yml` 的
  /// Linux 腿会跑到它 —— Windows 上永远绿的那种测试在这儿没有意义。
  #[cfg(target_os = "linux")]
  #[test]
  fn linux_install_kinds_map_to_their_own_package() {
    use super::imp::{Kind, suffixes_for};
    for (k, want) in [
      (Kind::AppImage, ".appimage"),
      (Kind::Deb, ".deb"),
      (Kind::Rpm, ".rpm"),
    ] {
      assert_eq!(
        suffixes_for(k),
        &[want][..],
        "{} 只认自己的后缀（跨格式兜底会把装不上的包下下来）",
        k.as_str()
      );
    }
    assert!(
      suffixes_for(Kind::Unknown).is_empty(),
      "认不出安装方式时不许挑任何包（宁可报原因，也不白下）"
    );
  }

  #[test]
  fn phase_serializes_snake_case_for_debug_snapshot() {
    // /debug 的 update.phase 直接用这个字符串（界面同源）
    let j = serde_json::to_string(&Phase::UpToDate).unwrap();
    assert_eq!(j, "\"up_to_date\"");
    let j = serde_json::to_string(&Phase::Available).unwrap();
    assert_eq!(j, "\"available\"");
  }

  // ── 版本检测（不联网：`decide` 是纯函数，tag 往返是纯字符串处理）──

  #[test]
  fn decide_flags_only_strictly_newer_remote() {
    // 同一个 tag → 已是最新（这条最要紧：发布完当天查一次不能反过来提示有更新）
    assert_eq!(
      decide("2026.10.14.1530", "2026.10.14.1530"),
      Phase::UpToDate
    );
    // 远端更新
    assert_eq!(
      decide("2026.10.14.1530", "2026.10.14.1531"),
      Phase::Available
    );
    assert_eq!(
      decide("2026.10.14.1530", "2026.10.15.0900"),
      Phase::Available
    );
    assert_eq!(
      decide("2026.09.30.2359", "2026.10.01.0000"),
      Phase::Available
    );
    // 远端更旧（本地是开发版以外的预发、或 release 被回退）→ 不提示
    assert_eq!(
      decide("2026.10.15.0900", "2026.10.14.1530"),
      Phase::UpToDate
    );
    assert_eq!(
      decide("2026.11.01.0000", "2026.10.31.2359"),
      Phase::UpToDate
    );
  }

  #[test]
  fn decide_treats_same_day_releases_by_minutes() {
    // 显示版本 yy.M.d 同一天会重复，靠 tag 的时分区分（version.rs 的存在理由），
    // 所以判定必须按全序而不是按显示版本。
    let early = "2026.10.14.0901";
    let late = "2026.10.14.2309";
    assert_eq!(decide(early, late), Phase::Available);
    assert_eq!(decide(late, early), Phase::UpToDate);
  }

  #[test]
  fn decide_survives_unparsable_stamps_without_panicking() {
    // tag 格式不对 / 本机构建戳缺失：不能崩，也不能谎报「有更新」
    assert_eq!(decide("2026.10.14.1530", "not-a-tag"), Phase::UpToDate);
    assert_eq!(
      decide("2026.10.14.1530", "v2026.10.14.1530"),
      Phase::UpToDate
    );
    assert_eq!(decide("", "2026.10.14.1530"), Phase::UpToDate);
  }

  #[test]
  fn release_tag_round_trips_to_the_artifact_version() {
    // 闭环：tag → stamp → 显示版本，必须等于 CI 打进包里的那个 VERSION。
    // 这条错了的表现是「界面上写 26.9.9、包名里却是 26.09.09」，用户升级时才看得见。
    for (tag, want) in [
      ("v2026.10.14.1530", "26.10.14"),
      ("v2026.09.09.0901", "26.9.9"),
      ("v2026.12.31.2359", "26.12.31"),
    ] {
      let stamp = tag_to_stamp(tag).expect("合法 tag 应能解析");
      assert_eq!(stamp_to_version(&stamp), want, "tag {tag} 的显示版本往返");
    }
  }

  #[test]
  fn ci_asset_names_match_the_tag_version() {
    // **直接读 release_stage.sh**，不把产物名硬编码在测试里 —— 之前这里写死一份名字列表，
    // 名字全是自己跟自己比，所以 CI 实际产出 `-linux-x64.deb` 而测试里写
    // `-linux-x86_64.deb` 也照样绿。改了命名规则没人会发现，用户照着 release notes
    // 去找文件才扑空。
    //
    // 断言两件最容易错的事：
    // ① 版本段来自 `${VERSION}` 而不是空串 —— release job 曾经从没跑过 version.py，
    //    产物名全叫 `PureVox--windows-x64-setup.exe`（双连字符）；
    // ② 每个平台段的**后缀**正好是本平台认领的后缀，且五个平台互不重叠。
    let tpls: Vec<&str> = RELEASE_STAGE_SH
      .lines()
      .filter(|l| l.contains("stage \"out/PureVox-"))
      .map(|l| {
        l.split("stage \"out/")
          .nth(1)
          .expect("含 stage 的行必有 out/ 前缀")
          .split('"')
          .next()
          .expect("引号必闭合")
      })
      .collect();
    assert!(
      tpls.len() >= 5,
      "release_stage.sh 里的产物模板只剩 {} 个，命名规则变了要重新审这一段",
      tpls.len()
    );

    let tag = "v2026.10.14.1530";
    let want = stamp_to_version(&tag_to_stamp(tag).expect("合法 tag"));
    let mut suffixes: Vec<String> = Vec::new();
    for tpl in &tpls {
      assert!(
        tpl.contains("${VERSION}"),
        "产物模板 {tpl} 没插版本号 → 名字里会缺一段，pick_asset 仍能按后缀认出来但用户看不懂"
      );
      let name = tpl.replace("${VERSION}", &want);
      assert!(
        !name.contains("--"),
        "产物名 {name} 有空版本段（双连字符）：多半是 env.VERSION 在该 job 里从没被赋值"
      );
      let suffix = name
        .strip_prefix(&format!("PureVox-{want}-"))
        .unwrap_or_else(|| panic!("产物名 {name} 的版本段与 tag 不一致"));
      assert!(
        !suffixes.iter().any(|s| s == suffix),
        "平台段 {suffix} 重复出现，pick_asset 会挑到别人的产物"
      );
      suffixes.push(suffix.to_string());
    }

    // 五个平台段的后缀互不为彼此的后缀（`pick_asset` 只认后缀，重叠就会挑错）
    for a in &suffixes {
      for b in &suffixes {
        if a != b {
          assert!(!a.ends_with(b), "后缀 {a} 会被 {b} 抢先命中");
        }
      }
    }
  }

  /// **CI 产出的产物名必须落在运行期某张后缀表里** —— 这条才是真正会伤到用户的那一种漂移，
  /// 而上面那个测试**抓不到**。
  ///
  /// 运行期是拿产物名去比对 `install_{linux,windows}.rs` 里的后缀字面量（"`.deb`"）。
  /// 两边各写各的，于是「把 release 里的 `-linux-x64.deb` 改名成 `-linux-x86_64.deb`」
  /// 这种改动会让上面所有断言照样全绿（它们只查内部一致性），但**从此再也匹配不到任何
  /// 产物 —— 自动更新彻底静默失效，用户不会看到任何报错，只会发现「永远有新版本」**。
  ///
  /// 所以这里把三份东西交叉比对：`release_stage.sh` 里 `stage` 出的名字、
  /// `install_*.rs` 里 `ASSET_SUFFIXES` 的字面量、以及本模块的 `pick_asset_with` 本身
  /// （构造一遍假资产列表走一遍真实函数，不只是比对字符串）。
  #[test]
  fn staged_names_are_claimable_by_pick_asset() {
    let linux_rs = include_str!("install_linux.rs");
    let windows_rs = include_str!("install_windows.rs");

    /// 从平台源码里抠出 `ASSET_SUFFIXES` 表里的全部后缀字面量。
    ///
    /// 表有两种排版：Windows 是扁平的 `&["-setup.exe"]`，Linux 是**按安装方式**分行的
    /// `(Kind::Deb, &[".deb"])`（2026-10-18 起）。这里不按排版去 split，只收这一段里的
    /// `"…"` 字面量，且只留「以 `.` 开头」或「以 `.exe` 结尾」的 —— 注释和枚举名不会被当成后缀。
    fn suffixes_of(src: &str, platform: &str) -> Vec<String> {
      let mut out = Vec::new();
      let mut in_table = false;
      for line in src.lines() {
        if !in_table {
          in_table = line.contains("ASSET_SUFFIXES:");
          if !in_table {
            continue;
          }
        }
        for lit in line.split('"').skip(1).step_by(2) {
          let s = lit.trim().to_ascii_lowercase();
          if s.starts_with('.') || s.ends_with(".exe") {
            out.push(s);
          }
        }
        if line.trim_end().ends_with("];") {
          break;
        }
      }
      assert!(
        !out.is_empty(),
        "{platform} 的 ASSET_SUFFIXES 里一个后缀都没认出来（表换了写法要同步这里）"
      );
      out
    }

    let known: Vec<String> = suffixes_of(linux_rs, "linux")
      .into_iter()
      .chain(suffixes_of(windows_rs, "windows"))
      .collect();
    assert!(
      known.len() >= 4,
      "只认出 {} 个后缀，两个平台的定义都被改了？",
      known.len()
    );
    // 三个 Linux 安装方式各一条 + Windows 一条 NSIS（结构性断言：Linux 的三种包都得有人认领）
    for want in [".deb", ".rpm", ".appimage", "-setup.exe"] {
      assert!(
        known.iter().any(|s| s == want),
        "后缀 {want} 没有任何平台认领（现在是 {known:?}）"
      );
    }

    // 走一遍真实的 stage 模板：把 ${VERSION} 换成假版本，取出后缀
    let want = stamp_to_version(&tag_to_stamp("v2026.10.14.1530").expect("合法 tag"));
    let staged: Vec<String> = RELEASE_STAGE_SH
      .lines()
      .filter(|l| l.contains("stage \"out/PureVox-"))
      .map(|l| {
        l.split("stage \"out/")
          .nth(1)
          .expect("含 stage 的行必有 out/ 前缀")
          .split('"')
          .next()
          .expect("引号必闭合")
          .replace("${VERSION}", &want)
      })
      .map(|n| n.to_ascii_lowercase())
      .collect();
    assert!(staged.len() >= 5, "只抠出 {} 个 stage 模板", staged.len());

    for name in &staged {
      // **Android 的 apk 是例外**：它由安卓客户端自己认领（本模块是桌面端更新逻辑，
      // 用户手机上装的 apk 永远不会出现在这里查的 latest release 资产列表给桌面的语境里
      // —— 真出现也认领不到，`.apk` 本来就不在 ASSET_SUFFIXES 里）。所以它必须显式跳过，
      // 否则这条断言会把「桌面端不该管安卓」当成缺陷报出来。
      if name.contains("-android-") {
        continue;
      }
      let claimed = known.iter().any(|s| name.ends_with(s.as_str()));
      assert!(
        claimed,
        "产物名 {name} 的后缀不在任何平台的 ASSET_SUFFIXES 里（{known:?}）—— \
         自动更新会静默失效：pick_asset 一个都匹配不上，用户只会看到「永远有新版本」"
      );
    }

    // 不止比字符串：用真实函数走一遍。喂的是**两个平台后缀的并集** ——
    // 单平台只认自己的格式（deb 不认 appimage），CI 跑哪个平台都能过。
    let assets: Vec<Asset> = staged
      .iter()
      .map(|n| Asset {
        name: n.clone(),
        browser_download_url: format!("https://example.invalid/{n}"),
        size: 1,
      })
      .collect();
    let refs: Vec<&str> = known.iter().map(|s| s.as_str()).collect();
    assert!(
      pick_asset_with(&assets, &refs).is_some(),
      "pick_asset_with 对这批真实产物名一个都挑不出来"
    );
  }

  // ── 下载：断点续传 / 空闲超时（2026-10-18 的「error decoding response body」）──
  //
  // 那条报错的根因是「下载共用了检查更新的 15 s **总**超时」，不是网络本身：
  // 50~140 MB 的安装包在任何慢链路上都下不完，body 被掐断后 reqwest 把它报成
  // `Kind::Decode`。修法两条 —— 下载不许有总超时（只留空闲超时）、断了能接着下。
  // 下面这组测试对着**一个真的最小 HTTP 服务器**跑 `fetch_into`：
  // 断流、服务器不理 `Range`、卡住三种真实情况各钉一条。

  /// 服务器对一次请求的动作。
  #[derive(Clone, Copy)]
  enum Act {
    /// 200 整包（不管客户端有没有带 `Range` —— 模拟不支持续传的服务器）
    Full,
    /// 206：从客户端的 `Range` 起点发到底
    From,
    /// 200 声明全长，但从 `from` 起只发 `cnt` 字节就**断开**（= 真实的中断/被掐）
    CutAt(usize),
    /// 发 `n` 字节后**卡住不关连接**（考客户端自己的空闲超时）
    StallAfter(usize),
  }

  /// 起一个最小 HTTP/1.1 服务器（只认 GET、只回一个文件），返回 (url, 收到的 Range 头列表)。
  /// `acts` 按请求次序取，用完就重复最后一个。
  async fn serve(
    payload: Vec<u8>,
    acts: Vec<Act>,
  ) -> (
    String,
    std::sync::Arc<std::sync::Mutex<Vec<Option<String>>>>,
  ) {
    use tokio::io::{AsyncReadExt, AsyncWriteExt};
    use tokio::net::TcpListener;
    let l = TcpListener::bind("127.0.0.1:0")
      .await
      .expect("监听测试端口");
    let port = l.local_addr().unwrap().port();
    let seen: std::sync::Arc<std::sync::Mutex<Vec<Option<String>>>> =
      std::sync::Arc::new(std::sync::Mutex::new(Vec::new()));
    let seen2 = seen.clone();
    let payload = std::sync::Arc::new(payload);
    tokio::spawn(async move {
      let mut n = 0usize;
      while let Ok((mut sock, _)) = l.accept().await {
        // 读请求头，抓 Range
        let mut buf = Vec::new();
        let mut tmp = [0u8; 512];
        let mut range = None;
        loop {
          match sock.read(&mut tmp).await {
            Ok(0) | Err(_) => break,
            Ok(k) => buf.extend_from_slice(&tmp[..k]),
          }
          if buf.windows(4).any(|w| w == b"\r\n\r\n") {
            let head = String::from_utf8_lossy(&buf).to_lowercase();
            range = head
              .lines()
              .find_map(|l| l.strip_prefix("range:").map(|v| v.trim().to_string()));
            break;
          }
        }
        seen2.lock().unwrap().push(range.clone());
        let from: usize = range
          .as_deref()
          .and_then(|r| r.strip_prefix("bytes="))
          .and_then(|r| r.split('-').next())
          .and_then(|s| s.parse().ok())
          .unwrap_or(0);
        let act = acts
          .get(n)
          .copied()
          .unwrap_or(*acts.last().expect("至少一个动作"));
        n += 1;
        let plen = payload.len();
        let start = from.min(plen);
        let (code, declared, body, stall) = match act {
          Act::Full => ("200 OK", plen, payload.to_vec(), false),
          Act::From => (
            "206 Partial Content",
            plen - start,
            payload[start..].to_vec(),
            false,
          ),
          Act::CutAt(cnt) => {
            let end = (start + cnt).min(plen);
            ("200 OK", plen, payload[start..end].to_vec(), false) // 声明全长、只发一段、然后断
          }
          Act::StallAfter(cnt) => {
            let end = (start + cnt).min(plen);
            ("200 OK", plen, payload[start..end].to_vec(), true)
          }
        };
        let head =
          format!("HTTP/1.1 {code}\r\nContent-Length: {declared}\r\nConnection: close\r\n\r\n");
        let _ = sock.write_all(head.as_bytes()).await;
        let _ = sock.write_all(&body).await;
        let _ = sock.flush().await;
        if stall {
          tokio::time::sleep(std::time::Duration::from_millis(1500)).await;
        }
        let _ = sock.shutdown().await;
      }
    });
    (format!("http://127.0.0.1:{port}/a.bin"), seen)
  }

  fn payload_of(n: usize) -> Vec<u8> {
    (0..n).map(|i| (i % 251) as u8).collect()
  }

  /// 临时文件路径（每个测试一份，带进程号，互不打扰）。
  fn tmp_file(tag: &str) -> std::path::PathBuf {
    let d = std::env::temp_dir().join(format!("purevox-update-test-{}", std::process::id()));
    let _ = std::fs::create_dir_all(&d);
    d.join(format!("{tag}.part"))
  }

  /// 断流之后**接着下**：整包必须逐字节正确（不重不漏），而且第二次请求真的带了 `Range`。
  #[tokio::test]
  async fn download_resumes_after_interruption() {
    use std::io::{Seek, SeekFrom, Write};
    let payload = payload_of(4096);
    let (url, seen) = serve(payload.clone(), vec![Act::CutAt(1000), Act::From]).await;
    let path = tmp_file("resume");
    let _ = std::fs::remove_file(&path);
    let client = download_client().expect("建客户端");
    let mut total = 0u64;
    let mut last = String::new();
    for _ in 0..3 {
      let mut f = std::fs::OpenOptions::new()
        .create(true)
        .write(true)
        .truncate(false)
        .open(&path)
        .expect("打开临时文件");
      f.seek(SeekFrom::Start(total)).expect("定位到断点");
      match fetch_into(&client, &url, &mut f, &mut total, |_| {}).await {
        Ok(()) => {}
        Err(e) => last = e,
      }
      if total >= payload.len() as u64 {
        break;
      }
    }
    assert_eq!(
      total,
      payload.len() as u64,
      "续传要把整包下完（最近一次错误：{last}）"
    );
    assert_eq!(
      std::fs::read(&path).expect("读回下载结果"),
      payload,
      "续传拼出来的文件必须与原包逐字节相同（不能重复也不能缺口）"
    );
    let has_range = seen
      .lock()
      .unwrap()
      .iter()
      .any(|r| r.as_deref().is_some_and(|h| h.starts_with("bytes=")));
    assert!(
      has_range,
      "断流后必须带 Range 续传（每次都从头下 = 慢网络永远下不完）"
    );
    let _ = std::fs::remove_file(&path);
    // 顺手确认写下去的是真的（避免 unwrap 静默）
    assert!(std::io::stdout().flush().is_ok());
  }

  /// 服务器**不理 `Range`**（回 200 整包）时不许把这包接在旧进度后面（否则文件翻倍）。
  #[tokio::test]
  async fn restarts_when_server_ignores_range() {
    use std::io::{Seek, SeekFrom, Write};
    let payload = payload_of(3000);
    let (url, _seen) = serve(payload.clone(), vec![Act::Full]).await;
    let path = tmp_file("ignore-range");
    // 先造出「上次下了一半」的残留
    std::fs::write(&path, vec![0xAAu8; 1000]).expect("写残留");
    let client = download_client().expect("建客户端");
    let mut total = 1000u64;
    let mut f = std::fs::OpenOptions::new()
      .create(true)
      .write(true)
      .truncate(false)
      .open(&path)
      .expect("打开临时文件");
    f.seek(SeekFrom::Start(total)).expect("定位");
    fetch_into(&client, &url, &mut f, &mut total, |_| {})
      .await
      .expect("整包响应应当成功");
    drop(f);
    assert_eq!(total, payload.len() as u64, "total 必须被归零重算");
    assert_eq!(
      std::fs::read(&path).expect("读回"),
      payload,
      "服务器不理 Range 时必须清空重来，不能把整包接在旧进度后面"
    );
    let _ = std::fs::remove_file(&path);
    assert!(std::io::stdout().flush().is_ok());
  }

  /// 卡住（在走但没数据）→ 报**下载超时**，不是那句看不懂的 `error decoding response body`；
  /// 而且已经落盘的字节要留着（下次接着下）。
  #[tokio::test]
  async fn stalled_transfer_reports_idle_timeout_and_keeps_progress() {
    use std::io::{Seek, SeekFrom};
    let payload = payload_of(4000);
    let (url, _seen) = serve(payload.clone(), vec![Act::StallAfter(800)]).await;
    let path = tmp_file("stall");
    let _ = std::fs::remove_file(&path);
    // 客户端把空闲超时收紧到 300 ms（生产值是 IDLE_TIMEOUT_S 秒，测试等不起）
    let client = reqwest::Client::builder()
      .user_agent(UA)
      .read_timeout(std::time::Duration::from_millis(300))
      .build()
      .expect("建客户端");
    let mut total = 0u64;
    let mut f = std::fs::OpenOptions::new()
      .create(true)
      .write(true)
      .truncate(false)
      .open(&path)
      .expect("打开临时文件");
    f.seek(SeekFrom::Start(0)).expect("定位");
    let err = fetch_into(&client, &url, &mut f, &mut total, |_| {})
      .await
      .expect_err("卡住必须报错");
    assert!(
      err.contains("下载超时"),
      "空闲超时必须报「下载超时」，实际：{err}"
    );
    assert_eq!(total, 800, "已经落盘的字节要留着（续传的起点）");
    let _ = std::fs::remove_file(&path);
  }
}
