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

// release 版不弹控制台窗口（Windows）
#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

mod audio;
// 打开链接的平台实现分文件（AGENTS.md §4）。
mod config;
mod cues;
mod debug;
mod devices;
mod dsp;
mod engine;
mod hotkey;
mod infer;
mod media;
mod net;
#[cfg(windows)]
#[path = "openurl_windows.rs"]
mod openurl;
#[cfg(target_os = "linux")]
#[path = "openurl_linux.rs"]
mod openurl;
#[cfg(not(any(windows, target_os = "linux")))]
#[path = "openurl_other.rs"]
mod openurl;
mod plan;
mod recorder;
#[cfg(windows)]
#[path = "sound_panel_windows.rs"]
mod sound_panel;
#[cfg(target_os = "linux")]
#[path = "sound_panel_linux.rs"]
mod sound_panel;
#[cfg(not(any(windows, target_os = "linux")))]
#[path = "sound_panel_other.rs"]
mod sound_panel;
mod update;
#[cfg(windows)]
#[path = "vb_panel_windows.rs"]
mod vb_panel;
#[cfg(not(windows))]
#[path = "vb_panel_other.rs"]
mod vb_panel;
mod version;
mod wav;
mod winstate;

use std::sync::{Arc, Mutex};

use audio::AudioManager;
use debug::{DebugHub, DebugSnapshot, SharedHub};
use plan::Plan;
use tauri::Manager;

/// 热键宿主（可重启）：设置变更时先停旧的再起新的。
struct HotkeyHost(Mutex<hotkey::Hotkeys>);

/// UI 调试面板取数：与 HTTP 接口同一个快照。
#[tauri::command]
fn debug_snapshot(hub: tauri::State<'_, SharedHub>) -> DebugSnapshot {
  hub.snapshot()
}

/// 刷新设备列表（后台执行，立即返回一句状态给界面显示）。
///
/// **不静默跳过**：正在刷就如实说正在刷，失败就带原因——以前这里是个静默的
/// 「已有刷新在进行时直接 return」，界面上按钮点了跟没点一样，只会觉得被卡住。
#[tauri::command]
fn refresh_devices(hub: tauri::State<'_, SharedHub>) -> String {
  devices::spawn_refresh(hub.inner().clone())
}

/// 设备刷新状态（正在刷 / 上次成功于 / 上次耗时 / 上次失败原因）。
#[tauri::command]
fn device_refresh_state() -> devices::RefreshState {
  devices::refresh_state()
}

/// 界面自报错误/状态，追加进调试快照（`/debug` 的 `ui` 数组），便于无控制台时定位前端问题。
#[tauri::command]
fn ui_report(kind: String, message: String, hub: tauri::State<'_, SharedHub>) {
  hub.push_ui(&kind, message);
}

/// 打开 WebView2 开发者工具（JS 交互式调试）。
#[tauri::command]
fn open_devtools(window: tauri::WebviewWindow) {
  window.open_devtools();
}

/// 打开调试面板（**系统浏览器**，独立进程）：页面由本机只读调试接口提供
/// （`http://127.0.0.1:47821/debug/ui`），它只 `fetch('/debug')` 渲染，不经过 WebView2 / IPC。
///
/// **不要再退回「第二个 WebView2 窗口」**：Tauri 3 alpha 在本机建第二个 webview 时页面
/// 根本不会导航（CDP 目标停在 `about:blank`、`Chrome_WidgetWin_0` 是 0×0，整窗纯白，
/// 时好时坏；2026-10-26 实测）。面板只是渲染只读数据，交给浏览器是天然解耦的独立进程，
/// 也不会再把主界面拖下水（白窗卡死过一次）。
///
/// 解耦的代价：面板**截不到主窗口的图**（跨进程），所以面板里没有「截屏」按钮；
/// 内置截屏已整体移除（用户 2026-10-26 决定），抓界面截图用系统截图工具。
#[tauri::command]
fn open_debug_window(hub: tauri::State<'_, SharedHub>) -> Result<(), String> {
  let url = format!("http://127.0.0.1:{}/debug/ui", debug::DEBUG_HTTP_PORT);
  match &hub.snapshot().app.debug_http {
    debug::Probe::Ok { .. } => openurl::open(&url),
    debug::Probe::Pending => Err(format!("调试接口还没起来（{url}）")),
    debug::Probe::Unavailable { reason } => Err(format!(
      "调试接口不可用：{reason}（页面在 {url}，接口没起来就打不开）"
    )),
  }
}

/// 开机自启开关：走官方插件 `tauri-plugin-autostart`（见 `main` 里的 `.plugin(...)`）。
///
/// Windows 上是 `auto-launch`：先试 HKLM（非管理员会失败，**不弹 UAC**），再回退 HKCU，
/// 同时写 `Explorer\StartupApproved\Run` 的「已启用」标记 —— 少了这个标记，任务管理器
/// 的「启动」页会把条目当成被禁用（手写 Run 键踩过这个坑，2026-10-26）。
#[tauri::command]
async fn set_autostart(app: tauri::AppHandle, on: bool) -> Result<(), String> {
  use tauri_plugin_autostart::ManagerExt;
  // 注册表/文件读写是阻塞的：别放在主线程（async 命令跑在运行时线程池）。
  let m = app.autostart();
  if on { m.enable() } else { m.disable() }.map_err(|e| format!("开机自启设置失败：{e}"))
}

#[tauri::command]
fn get_autostart(app: tauri::AppHandle) -> bool {
  use tauri_plugin_autostart::ManagerExt;
  app.autostart().is_enabled().unwrap_or(false)
}

/// 读取当前会话计划。
#[tauri::command]
fn get_plan(mgr: tauri::State<'_, Arc<AudioManager>>) -> Plan {
  mgr.plan()
}

/// 应用新计划（结构性变更 → 重建会话）。返回逐行问题（非致命）。
///
/// 顺便**重注册音效热键**：热键跟着计划走（`plan::pad_hotkeys`），增删音效后
/// 必须重新收口，否则界面看着有键、按下去没反应。
#[tauri::command]
async fn apply_plan(
  plan: Plan,
  mgr: tauri::State<'_, Arc<AudioManager>>,
  app: tauri::AppHandle,
) -> Result<Vec<String>, String> {
  let mgr = mgr.inner().clone();
  let problems = tauri::async_runtime::spawn_blocking(move || mgr.apply_plan(plan))
    .await
    .map_err(|e| format!("应用计划异常：{e}"))??;
  start_hotkey(&app, &app.state::<HotkeyHost>());
  Ok(problems)
}

/// 改一个**行级热参数**（与模型无关的标量：增益 / 音量）：**不重建会话**，下一 hop 生效。
/// 界面按注册表的 `is_hot` 决定走这条路还是 `apply_plan`（见 `engine/hot.rs` 的说明）。
#[tauri::command]
async fn set_row_param(
  col: usize,
  row: usize,
  key: String,
  value: String,
  mgr: tauri::State<'_, Arc<AudioManager>>,
) -> Result<(), String> {
  let mgr = mgr.inner().clone();
  tauri::async_runtime::spawn_blocking(move || mgr.set_row_param(col, row, &key, &value))
    .await
    .map_err(|e| format!("热参数写入异常：{e}"))?
}

/// 热参数落盘（不重建会话）：音量 / 续播位置等改动用它 + 对应的媒体运行时命令
/// （DESIGN.md §5：热参数下发给运行中的实例，不重启）。
#[tauri::command]
async fn persist_plan(plan: Plan, mgr: tauri::State<'_, Arc<AudioManager>>) -> Result<(), String> {
  let mgr = mgr.inner().clone();
  tauri::async_runtime::spawn_blocking(move || mgr.persist_plan(plan))
    .await
    .map_err(|e| format!("计划落盘异常：{e}"))?
}

#[derive(serde::Serialize)]
struct ImportedInfo {
  path: String,
  name: String,
  reused: bool,
}

/// 媒体库导入（音效板 / 播放器）：界面文件选择读出字节 → base64 → 解码归一 → 落库。
/// `kind`：`soundpad` / `music`。
///
/// 走 base64 文本而非字节数组：`<input type=file>` 给的是 `Uint8Array`，逐字节过
/// JSON 会让一个 10 MB 的曲子变成几千万个数字，编码成一个字符串小一个量级。
#[tauri::command]
async fn media_import(
  data_b64: String,
  name: String,
  kind: String,
) -> Result<ImportedInfo, String> {
  tauri::async_runtime::spawn_blocking(move || {
    use base64::Engine;
    let kind = match kind.as_str() {
      "soundpad" => media::MediaKind::Soundpad,
      "music" => media::MediaKind::Music,
      other => return Err(format!("未知媒体类型 {other}")),
    };
    let bytes = base64::engine::general_purpose::STANDARD
      .decode(data_b64.as_bytes())
      .map_err(|e| format!("文件内容解码失败：{e}"))?;
    let im = media::import(&bytes, &name, kind)?;
    Ok(ImportedInfo {
      path: im.path.to_string_lossy().into_owned(),
      name: im.name,
      reused: im.reused,
    })
  })
  .await
  .map_err(|e| format!("媒体导入异常：{e}"))?
}

/// 移除库内媒体文件。双重护栏：只删媒体库内路径；仍被计划引用（任一行的参数里
/// 出现过该路径）则拒绝——同一文件可被多个音效引用。
#[tauri::command]
async fn media_discard(
  path: String,
  mgr: tauri::State<'_, Arc<AudioManager>>,
) -> Result<(), String> {
  let mgr = mgr.inner().clone();
  tauri::async_runtime::spawn_blocking(move || {
    let plan = mgr.plan();
    let referenced = plan
      .columns
      .iter()
      .flat_map(|c| c.rows.iter())
      .any(|r| r.params.values().any(|v| v.contains(&path)));
    if referenced {
      return Err("该文件仍被计划引用，不能删除".into());
    }
    media::discard(&path)
  })
  .await
  .map_err(|e| format!("媒体删除异常：{e}"))?
}

/// 媒体行运行时命令（音效板 / 播放器）。(col, row) 寻址；会话未运行时静默忽略
/// （槽位不存在）。
#[tauri::command]
fn media_command(col: usize, row: usize, cmd: engine::media::MediaCmd) {
  if let Some(slot) = engine::media::hub().slot(col, row) {
    slot.send(cmd);
  }
}

/// 媒体行状态快照（界面 2 Hz 轮询）；未运行 / 非媒体行返回 null。
#[tauri::command]
fn media_status(col: usize, row: usize) -> Option<engine::media::MediaStatus> {
  engine::media::hub().slot(col, row).map(|s| s.status())
}

#[derive(serde::Serialize)]
struct ModelInfo {
  file: String,
  label: String,
}

/// 某类处理节点的可选模型（供界面下拉）。
#[tauri::command]
fn list_models(ptype: String) -> Vec<ModelInfo> {
  let list: &[(&str, &str)] = match ptype.as_str() {
    "tse" => infer::tse::tse_models(),
    "echo_cancel" => infer::aec::aec_models(),
    _ => infer::denoise_models(),
  };
  list
    .iter()
    .map(|(f, l)| ModelInfo {
      file: (*f).into(),
      label: (*l).into(),
    })
    .collect()
}

#[derive(serde::Serialize)]
struct ParamInfo {
  key: String,
  label: String,
  kind: String,
  default: String,
  #[serde(skip_serializing_if = "Option::is_none")]
  min: Option<f64>,
  #[serde(skip_serializing_if = "Option::is_none")]
  max: Option<f64>,
  #[serde(skip_serializing_if = "Option::is_none")]
  step: Option<f64>,
  /// Choice 参数的选项（value, label）；其它类型为空。
  #[serde(skip_serializing_if = "Vec::is_empty")]
  options: Vec<(String, String)>,
  /// 是否**热参数**（改它不重建会话，见 `engine/hot.rs`）：界面照它选
  /// `set_row_param` 还是 `apply_plan`。
  hot: bool,
}

#[derive(serde::Serialize)]
struct NodeInfo {
  ptype: String,
  label: String,
  kind: String,
  params: Vec<ParamInfo>,
}

/// 注册表里的节点类型（供界面构建下拉与参数编辑，单一来源）。
#[tauri::command]
fn list_nodes() -> Vec<NodeInfo> {
  engine::registry::all_specs()
    .iter()
    .map(|s| NodeInfo {
      ptype: s.ptype.into(),
      label: s.label.into(),
      kind: s.kind.as_str().into(),
      params: s
        .params
        .iter()
        .map(|p| {
          let (min, max, step) = match p.kind {
            engine::registry::ParamKind::Number { lo, hi, step, .. } => {
              (Some(lo), Some(hi), Some(step))
            }
            _ => (None, None, None),
          };
          let options = match p.kind {
            engine::registry::ParamKind::Choice { options, .. } => options
              .iter()
              .map(|(v, l)| ((*v).to_string(), (*l).to_string()))
              .collect(),
            _ => Vec::new(),
          };
          ParamInfo {
            key: p.key.into(),
            label: p.label.into(),
            kind: p.kind_str().into(),
            default: p.default_text(),
            min,
            max,
            step,
            options,
            hot: engine::registry::is_hot(s.ptype, p.key),
          }
        })
        .collect(),
    })
    .collect()
}

/// 录制 TSE 参考：录「降噪后、TSE 前」的信号 `seconds` 秒，**原样**写
/// `~/.purevox/tse_reference.wav`；进度见 `/debug` 的 `recorder` 字段。
#[tauri::command]
async fn record_tse_reference(
  seconds: f64,
  mgr: tauri::State<'_, Arc<AudioManager>>,
) -> Result<String, String> {
  let mgr = mgr.inner().clone();
  tauri::async_runtime::spawn_blocking(move || mgr.record_reference(seconds))
    .await
    .map_err(|e| format!("录制异常：{e}"))?
}

/// TSE 参考文件的当前状态（界面卡片只显示录制时间；其余字段留给调试 / 脚本）。
#[derive(serde::Serialize)]
struct TseRefInfo {
  path: String,
  exists: bool,
  size: u64,
  modified_ms: u64,
  duration_s: Option<f64>,
}

/// 读 TSE 参考状态（唯一路径：`~/.purevox/tse_reference.wav`）。
#[tauri::command]
async fn tse_reference_info() -> Result<TseRefInfo, String> {
  let path = config::default_tse_reference();
  tauri::async_runtime::spawn_blocking(move || {
    let meta = std::fs::metadata(&path).ok();
    let exists = meta.is_some();
    let size = meta.as_ref().map(|m| m.len()).unwrap_or(0);
    let modified_ms = meta
      .as_ref()
      .and_then(|m| m.modified().ok())
      .and_then(|t| t.duration_since(std::time::UNIX_EPOCH).ok())
      .map(|d| d.as_millis() as u64)
      .unwrap_or(0);
    let duration_s = if exists {
      crate::wav::read_mono(&path)
        .ok()
        .map(|(s, r)| s.len() as f64 / r.max(1) as f64)
    } else {
      None
    };
    Ok(TseRefInfo {
      path: path.display().to_string(),
      exists,
      size,
      modified_ms,
      duration_s,
    })
  })
  .await
  .map_err(|e| format!("读取参考信息异常：{e}"))?
}

/// 打开 TSE 参考语音所在文件夹（资源管理器 / 文件管理器里定位 `tse_reference.wav`）。
#[tauri::command]
async fn tse_open_reference_folder() -> Result<(), String> {
  let path = config::default_tse_reference();
  if let Some(dir) = path.parent() {
    std::fs::create_dir_all(dir).map_err(|e| format!("创建参考语音文件夹失败：{e}"))?;
  }
  openurl::reveal(&path)
}

/// 测 AEC 延时：采集 1.6 s（最大搜索 1000 ms），结果见 `/debug` 的 `calib` 字段。
///
/// **不必先手动启动**：会话未运行时自动临时启动一个，测完（含失败 / 超时）由看门狗线程
/// 停回原状；期间用户自己启停过就不动他的会话（`AudioManager::finish_temp_session`）。
#[tauri::command]
async fn calibrate_aec_delay(mgr: tauri::State<'_, Arc<AudioManager>>) -> Result<String, String> {
  let mgr = mgr.inner().clone();
  let mgr2 = mgr.clone();
  let (msg, temp_epoch) = tauri::async_runtime::spawn_blocking(move || mgr2.calibrate_aec_delay())
    .await
    .map_err(|e| format!("校准异常：{e}"))??;
  if temp_epoch.is_some() {
    spawn_calib_watchdog(mgr, temp_epoch);
  }
  Ok(msg)
}

/// 校准看门狗：轮询到校准不在进行中就收尾；硬超时兜底，保证临时会话一定被停掉。
/// 间隔 100 ms、上限 30 s（`calib` 内部 deadline 是采集时长 ×3，这里留足余量）。
fn spawn_calib_watchdog(mgr: Arc<AudioManager>, temp_epoch: Option<u64>) {
  std::thread::Builder::new()
    .name("calib-watchdog".into())
    .spawn(move || {
      const TICK: std::time::Duration = std::time::Duration::from_millis(100);
      const LIMIT: std::time::Duration = std::time::Duration::from_secs(30);
      let t0 = std::time::Instant::now();
      while t0.elapsed() < LIMIT && mgr.calib_in_flight() {
        std::thread::sleep(TICK);
      }
      mgr.finish_temp_session(temp_epoch);
    })
    .ok();
}

/// 启动网络服务（手机 ⇄ 电脑，端口 59123）。幂等。
#[tauri::command]
async fn net_start() -> Result<String, String> {
  tauri::async_runtime::spawn_blocking(net::start)
    .await
    .map_err(|e| format!("启动网络服务异常：{e}"))?
}

/// 停网络服务，并强制关掉远程输入。
#[tauri::command]
async fn net_stop() -> Result<String, String> {
  tauri::async_runtime::spawn_blocking(|| {
    net::stop();
    Ok::<_, String>("已停止".into())
  })
  .await
  .map_err(|e| format!("停止网络服务异常：{e}"))?
}

/// 重启网络服务：先真的停（abort 监听）再起。改端口后、连不上、或服务假死都靠它。
#[tauri::command]
async fn net_restart() -> Result<String, String> {
  tauri::async_runtime::spawn_blocking(net::restart)
    .await
    .map_err(|e| format!("重启网络服务异常：{e}"))?
}

/// 网络状态（界面展示；明细见 `/debug` 的 `net` 字段）。
#[tauri::command]
fn net_status() -> String {
  net::status()
}

/// 服务是否在跑（界面按钮用布尔，别去 `net_status` 的中文串里匹配）。
#[tauri::command]
fn net_running() -> bool {
  net::hub().is_running()
}

/// 远程输入总开关：手机输入法打字 + 手机实体键当全尺寸键盘。**默认关**。
#[tauri::command]
fn net_set_remote_input(on: bool) -> Result<String, String> {
  net::hub().set_remote_input(on);
  Ok(if on {
    "已开启远程输入：同网段的手机现在可以向这台电脑打字和发按键".to_string()
  } else {
    "已关闭远程输入".to_string()
  })
}

/// 本机 CA 证书 PEM（界面「下载证书」按钮 / 调试接口用）。
///
/// 服务没启动时是 `None`：那时证书还没准备好，界面该说「先启动服务」，而不是给一个点
/// 下去下载到空文件的按钮。
#[tauri::command]
fn net_ca_pem() -> Option<String> {
  net::hub().ca_pem()
}

/// 网络行卡片展示用：本机在局域网的 IP 与**网页版客户端地址**（`https://<ip>:<port>/`）。
///
/// IP 用 `net::advertised_ip()`：卡片上选的那张，没有（或已经失效）就用默认路由那张。
/// 端口用 `net::hub().port()`：用户改过的那个，不是写死的 59123。
/// 取不到时 `ip` 为空、`reason` 说明原因（界面显示「不可用 + 原因」，不显示伪造的 0）。
/// 注意给的是 `http://` 不是 `ws://`：二维码要能扫出**能打开的网页**。
#[derive(serde::Serialize)]
struct NetLocalAddr {
  ip: String,
  port: u16,
  url: String,
  reason: String,
}

#[tauri::command]
fn net_local_addr() -> NetLocalAddr {
  let port = net::hub().port();
  let fail = |reason: String| NetLocalAddr {
    ip: String::new(),
    port,
    url: String::new(),
    reason,
  };
  let ip = net::advertised_ip();
  if ip.is_empty() {
    return fail("取本机地址失败：本机没有可直连局域网的网卡（Wi-Fi / 有线都没连？）".into());
  }
  NetLocalAddr {
    // 给人看的地址是 **https://**：网页版客户端的麦克风靠 `getUserMedia`，而它**只在安全
    // 上下文里可用** —— `http://192.168.x.x` 不是安全上下文，浏览器会直接拒绝麦克风请求，
    // 而且没有任何绕过办法。所以必须是 https（自签证书，见 `net::tls`）。
    //
    // 不是 `ws://`：`ws://` 只在网页版客户端内部用（它自己拼 `/ws`）。二维码指向 ws://
    // 的话手机会当普通文本，扫码后什么也不会发生。
    url: format!("https://{ip}:{port}/"),
    ip,
    port,
    reason: String::new(),
  }
}

/// 列出可供局域网直连的网卡（卡片上的子网下拉）。
///
/// **不过滤网卡类型**：USB 共享、蓝牙共享、Wi-Fi 热点共享、有线、VPN、虚拟网卡全都是
/// 能直连的可用网卡，按 `IF_TYPE` 过滤反而会把有用的滤掉。只滤掉回环与链路本地 ——
/// 那两种客户端连不上，列出来只是让人去试一个必然失败的地址。
#[tauri::command]
fn net_ifaces() -> Vec<net::addrs::NetIface> {
  net::addrs::list()
}

/// 选一张网卡对外广告（只换二维码 / 显示 / 复制的内容，**不重启服务**）。
///
/// 服务绑的是 `0.0.0.0:port`，所有网卡本来就都在服务里；选子网只是决定**把哪个地址
/// 告诉客户端**。为这个重启会把已连着的客户端全踢掉，而实际什么都没变
/// （DESIGN.md §4.1.1）。
#[tauri::command]
fn net_select_ip(ip: String) -> Result<String, String> {
  let ip = ip.trim();
  if ip.is_empty() {
    net::set_advertised_ip("")?;
    return Ok("已改为自动选择（默认路由那张网卡）".into());
  }
  let ok = ip.parse::<std::net::Ipv4Addr>().ok().filter(|a| {
    let s = a.to_string();
    !s.starts_with("127.") && !s.starts_with("169.254.")
  });
  if ok.is_none() {
    return Err(format!(
      "{ip} 不是可用的局域网 IPv4 地址（回环与链路本地都不行）"
    ));
  }
  net::set_advertised_ip(ip)?;
  Ok(format!(
    "已改为通过 {ip} 对外（服务不用重启，本来就监听所有网卡）"
  ))
}

/// 改监听端口。**端口是绑定那一刻定死的，所以这里直接重启服务**，省得用户再点一次。
/// 服务没在跑就只存下，下次启动生效。范围限 1024~65535：1024 以下在 Unix 上要 root，
/// 而且 47821 是调试接口、别来抢。
#[tauri::command]
fn net_set_port(port: u16) -> Result<String, String> {
  if port < 1024 {
    return Err(format!(
      "端口 {port} 太小（要 ≥ 1024，低端口在 Linux 上需要管理员权限）"
    ));
  }
  if port == debug::DEBUG_HTTP_PORT {
    return Err(format!("端口 {port} 是本机调试接口占着的，换一个"));
  }
  let was = net::hub().port();
  net::set_port(port)?;
  net::hub().set_port(port);
  if was == port {
    return Ok(format!("端口已经是 {port}，没变"));
  }
  if net::hub().is_running() {
    let r = net::restart()?;
    Ok(format!(
      "端口 {was} → {port}，{r}（已连着的客户端需要重新连）"
    ))
  } else {
    Ok(format!("端口 {was} → {port}（服务没在跑，下次启动生效）"))
  }
}
#[tauri::command]
fn virtual_mic_status() -> debug::Probe<audio::virtual_mic::Status> {
  audio::virtual_mic::status_probe()
}

/// 创建 Linux 虚拟麦克风（幂等），返回创建后的状态。
#[tauri::command]
async fn virtual_mic_create(
  hub: tauri::State<'_, SharedHub>,
) -> Result<audio::virtual_mic::Status, String> {
  let hub = hub.inner().clone();
  tauri::async_runtime::spawn_blocking(move || {
    let s = audio::virtual_mic::create()?;
    hub.set_virtual_mic(debug::Probe::ok(s.clone()));
    Ok(s)
  })
  .await
  .map_err(|e| format!("创建虚拟麦克风异常：{e}"))?
}

/// 移除 Linux 虚拟麦克风（幂等），返回移除后的状态。
#[tauri::command]
async fn virtual_mic_remove(
  hub: tauri::State<'_, SharedHub>,
) -> Result<audio::virtual_mic::Status, String> {
  let hub = hub.inner().clone();
  tauri::async_runtime::spawn_blocking(move || {
    let s = audio::virtual_mic::remove()?;
    hub.set_virtual_mic(debug::Probe::ok(s.clone()));
    Ok(s)
  })
  .await
  .map_err(|e| format!("移除虚拟麦克风异常：{e}"))?
}

/// AEC 远端可选的回环目标（某个输出/sink 的 monitor；「系统默认输出」由界面加）。
#[tauri::command]
async fn list_loopback_targets() -> Vec<devices::LoopbackTarget> {
  // 可能会碰 pactl / cpal（Windows），绝不能在主线程做：放到阻塞线程，
  // 界面首屏不再等它（见 `ui/columns.js` 的 init）。
  tauri::async_runtime::spawn_blocking(devices::loopback_targets)
    .await
    .unwrap_or_default()
}

/// 当前平台标识（界面「驱动」页据此切换平台实现）：`linux` / `windows` / `macos`。
#[tauri::command]
fn get_platform() -> String {
  std::env::consts::OS.to_string()
}

// ── 更新（GitHub Releases，见 `update` 模块）──

/// 更新状态（界面轮询；与 `/debug` 的 `update` 段同源）。
#[tauri::command]
fn update_status() -> update::Status {
  update::status()
}

/// 检查更新（拉 GitHub Releases 的 `latest`）。网络 I/O 放后台，不卡界面。
#[tauri::command]
async fn update_check() -> Result<update::Status, String> {
  update::check().await
}

/// 下载已匹配到的本平台安装包（带进度，见 `update_status`）。
#[tauri::command]
async fn update_download() -> Result<update::Status, String> {
  update::download().await
}

/// 安装已下载的产物。Windows = NSIS 静默（拉起安装器后本进程退出并由安装器重启）；
/// Linux = AppImage 覆盖自身 / deb、rpm 走 `pkexec` 包管理器（装完**不退出**，重启生效）。
///
/// **必须异步 + `spawn_blocking`**：deb / rpm 的 `pkexec` 会等到用户在 polkit 授权框里
/// 点完、包管理器装完（几秒到几十秒）才返回，同步命令会卡住主线程（界面与状态轮询一起停）。
#[tauri::command]
async fn update_install(app: tauri::AppHandle) -> Result<String, String> {
  let msg = tokio::task::spawn_blocking(update::install)
    .await
    .map_err(|e| format!("安装任务中断：{e}"))??;
  // 最终阶段是 `Installing` = 安装器已接管、**本进程必须退出**（NSIS 安装器开头的
  // `CheckIfAppIsRunning` 会拦下正在运行的 exe）；Linux 装完是 `Installed`，
  // 进程继续跑、下次启动就是新版 —— 不打断用户正在用的会话。
  if update::status().phase == update::Phase::Installing {
    // 引擎先停，避免占着音频设备不退。
    app.state::<Arc<AudioManager>>().stop();
    // 给安装器一点时间看到进程已退出（安装器侧也有 CheckIfAppIsRunning 兜底）。
    std::thread::spawn(move || {
      std::thread::sleep(std::time::Duration::from_millis(1200));
      app.exit(0);
    });
  }
  Ok(msg)
}

/// 打开 release 页面（手动下载的出口；不自动更新时界面引导到这）。
#[tauri::command]
fn update_open_page(url: String) -> Result<(), String> {
  openurl::open(&url)
}

/// 用系统默认浏览器打开链接（驱动下载 / 教程）。
#[tauri::command]
fn open_url(url: String) -> Result<(), String> {
  if !(url.starts_with("https://") || url.starts_with("http://")) {
    return Err("只允许打开 http(s) 链接".into());
  }
  openurl::open(&url)
}

/// 打开系统声音控制面板（Windows = mmsys.cpl；Linux = pavucontrol / 系统设置）。
#[tauri::command]
fn open_sound_panel() -> Result<(), String> {
  sound_panel::open()
}

/// 以管理员权限打开 VB-CABLE 控制面板（仅 Windows；其它平台明确不可用）。
#[tauri::command]
fn open_vb_panel() -> Result<(), String> {
  vb_panel::open()
}

fn main() {
  // panic 也进事件日志：release 没有控制台，后台线程 panic 不能「只消失不报」。
  debug::events::install_panic_hook();
  let hub = DebugHub::new();
  debug::system::spawn_sampler(hub.clone());
  debug::http::spawn(hub.clone());
  devices::spawn_refresh(hub.clone());
  let audio = Arc::new(AudioManager::new(hub.clone()));
  let hotkey_host = HotkeyHost(Mutex::new(hotkey::Hotkeys::start(Vec::new(), hub.clone())));

  tauri::Builder::default()
    .runtime(tauri_runtime_wry::Wry::default())
    // 开机自启（官方插件；应用名取 package_info，路径取当前 exe）
    .plugin(tauri_plugin_autostart::Builder::new().build())
    .manage(hub)
    .manage(audio)
    .manage(hotkey_host)
    .setup(|app| {
      // 打包后模型位于 Tauri 资源目录（Windows：安装目录/models；Linux deb：/usr/lib/<productName>/models），
      // 注册给模型查找（开发态该目录不存在，走仓库 models/）。
      if let Ok(res) = app.path().resource_dir() {
        let models = res.join("models");
        if models.is_dir() {
          infer::set_model_dir(models);
        }
      }
      // 模型预热（后台）：开销不计，换来首次启动/重建不用等磁盘与 ORT 初始化。
      infer::preload_all();
      // 窗口标题带版本（与包版本同源，见 `version.rs`）。
      let start_hidden = config::load_settings().start_hidden;
      if let Some(w) = app.get_webview_window("main") {
        let _ = w.set_title(&format!("PureVox {}", version::VERSION));
        // 位置/大小记忆：恢复上次的几何，再决定要不要显示（config 里主窗口是 visible:false，
        // 这样不会先按默认尺寸闪一下再跳到记住的位置）。
        winstate::restore(&w);
        if !start_hidden {
          let _ = w.show();
          let _ = w.set_focus();
        }
      }
      // 运行中把窗口几何写回（拖动/缩放后 1.5 s 内落盘，见 `winstate`）。
      winstate::spawn_saver(app.handle().clone());
      // 托盘：**失败不能致命**。`org.kde.StatusNotifierWatcher` 在 GNOME 上由 appindicator
      // 扩展提供，扩展没启用（精简安装 / 关掉扩展）时总线上根本没有这个名字，ksni 直接返回
      // 硬错误。引擎与主窗口跟托盘没关系，没有它照样能用 —— 报出原因继续跑。原来这里用 `?`
      // 传播，结果是**启动即 panic**：用户连界面都看不到，更不会知道是托盘的问题
      // （2026-10-18 在本机 GNOME + 扩展 INACTIVE 的会话里实测）。
      if let Err(e) = setup_tray(app) {
        let msg = format!("托盘不可用（{e}）——窗口与引擎照常，只是少了托盘的启动/停止/退出菜单");
        eprintln!("PureVox: {msg}");
        app.state::<SharedHub>().inner().push_ui("error", msg);
      }
      // 读回上次选定的网络端口 / 广告地址。不读的话用户填的端口每次启动都白填
      // （hub 默认 59123），必须在第一次 `net_start` 之前。
      net::load_settings();
      // 调试钩子（不设环境变量则完全不生效）：`PUREVOX_AUTONET=1` 时自动启动网络服务。
      // 服务启动只发生在界面点「启动」之后，无人值守的验证（脚本 / CI / 回归排查）
      // 就没法确认它到底起没起 —— 加了这个钩子就能纯靠 `/debug` 的 `net` 段判断。
      if std::env::var("PUREVOX_AUTONET").is_ok() {
        let hub = app.state::<SharedHub>().inner().clone();
        match net::start() {
          Ok(m) => hub.push_ui("info", format!("AUTONET 启动网络服务：{m}")),
          Err(e) => hub.push_ui("error", format!("AUTONET 启动网络服务失败：{e}")),
        }
      }
      start_hotkey(app.handle(), &app.state::<HotkeyHost>());
      // 初始状态同步窗口图标
      sync_tray(app.handle(), app.state::<Arc<AudioManager>>().is_running());
      // 启动自运行并隐藏窗口（收到托盘）
      if config::load_settings().start_hidden {
        let mgr = app.state::<Arc<AudioManager>>().inner().clone();
        let _ = mgr.start();
        sync_tray(app.handle(), true);
        if let Some(w) = app.get_webview_window("main") {
          let _ = w.hide();
        }
      }
      // 调试钩子（不设环境变量则完全不生效）：`PUREVOX_AUTOCALIB_MS=<毫秒>` 时自动启动会话，
      // 并在该延时后跑一次 AEC 延时校准——用于无人点击界面时验证 AEC（结果进 /debug 的 `calib` 与 `ui`）。
      if let Some(ms) = std::env::var("PUREVOX_AUTOCALIB_MS")
        .ok()
        .and_then(|s| s.parse::<u64>().ok())
      {
        let mgr = app.state::<Arc<AudioManager>>().inner().clone();
        let hub = app.state::<SharedHub>().inner().clone();
        std::thread::spawn(move || {
          if let Err(e) = mgr.start() {
            hub.push_ui("error", format!("AUTOCALIB 启动会话失败：{e}"));
            return;
          }
          std::thread::sleep(std::time::Duration::from_millis(ms));
          match mgr.calibrate_aec_delay() {
            // 这个钩子是自己先 start() 的，所以返回 None（不是临时会话，别停）
            Ok((t, _)) => hub.push_ui("info", format!("AUTOCALIB：{t}")),
            Err(e) => hub.push_ui("error", format!("AUTOCALIB：{e}")),
          }
        });
      }
      // 调试钩子：`PUREVOX_AUTODEBUG=1` 自动打开独立调试窗口（无人值守验证用）。
      // 窗口创建必须回主线程（Linux 的 GTK 尤其如此）。
      if std::env::var("PUREVOX_AUTODEBUG").is_ok() {
        let handle = app.handle().clone();
        std::thread::spawn(move || {
          std::thread::sleep(std::time::Duration::from_millis(800));
          let h = handle.clone();
          let _ = handle.run_on_main_thread(move || {
            if let Err(e) = open_debug_window(h.state::<SharedHub>()) {
              eprintln!("AUTODEBUG 打开调试面板失败：{e}");
            }
          });
        });
      }
      Ok(())
    })
    // 最小化 / 关闭都收到托盘（不退出）
    .on_window_event(|window, event| match event {
      tauri::WindowEvent::CloseRequested { api, .. } => {
        // 调试窗口正常关闭；只有主窗口收到托盘（AGENTS §1.1：调试是独立窗口）
        if window.label() != "main" {
          return;
        }
        api.prevent_close();
        hide_to_tray(window);
      }
      // 最小化：Windows 下最小化会先 `Resized(0x0)` 再 `is_minimized()==true`，
      // 两个判据都要看（只看 `is_minimized()` 实测会漏，窗口留在任务栏上 —— 2026-10-26）。
      tauri::WindowEvent::Resized(size)
        if window.label() == "main"
          && (window.is_minimized().unwrap_or(false) || size.width == 0 || size.height == 0) =>
      {
        hide_to_tray(window);
      }
      _ => {}
    })
    .invoke_handler(tauri::generate_handler![
      debug_snapshot,
      refresh_devices,
      device_refresh_state,
      ui_report,
      open_devtools,
      open_debug_window,
      set_autostart,
      get_autostart,
      set_running,
      get_running,
      set_language,
      get_settings,
      set_settings,
      list_cues,
      get_plan,
      apply_plan,
      persist_plan,
      set_row_param,
      media_import,
      media_discard,
      media_command,
      media_status,
      list_models,
      list_nodes,
      record_tse_reference,
      tse_reference_info,
      tse_open_reference_folder,
      calibrate_aec_delay,
      net_start,
      net_stop,
      net_restart,
      net_status,
      net_running,
      net_set_remote_input,
      net_local_addr,
      net_ca_pem,
      net_ifaces,
      net_select_ip,
      net_set_port,
      virtual_mic_status,
      virtual_mic_create,
      virtual_mic_remove,
      list_loopback_targets,
      get_platform,
      update_status,
      update_check,
      update_download,
      update_install,
      update_open_page,
      open_url,
      open_sound_panel,
      open_vb_panel
    ])
    .run(tauri::generate_context!())
    .expect("error while running tauri application");
}

/// 系统托盘：启动/停止、显示/隐藏主窗口、退出；图标随运行状态变化。
fn setup_tray(app: &tauri::App) -> Result<(), Box<dyn std::error::Error>> {
  use tauri::tray::TrayIconBuilder;

  let mgr = app.state::<Arc<AudioManager>>().inner().clone();
  let menu = build_tray_menu(app, &config::load_lang(), mgr.is_running())?;
  let mgr2 = mgr.clone();
  let app_ev = app.handle().clone();
  let mut builder = TrayIconBuilder::with_id("main")
    .tooltip("PureVox")
    .menu(&menu)
    // 左键单击 = 显隐窗口；右键 = 弹菜单
    .show_menu_on_left_click(false)
    .on_menu_event(move |app, event| match event.id.as_ref() {
      "run" => toggle_running(app, &mgr2),
      "show" => toggle_window(app),
      "quit" => app.exit(0),
      _ => {}
    })
    .on_tray_icon_event(move |_tray, event| {
      if let tauri::tray::TrayIconEvent::Click {
        button: tauri::tray::MouseButton::Left,
        button_state: tauri::tray::MouseButtonState::Up,
        ..
      } = event
      {
        toggle_window(&app_ev);
      }
    });
  if let Some(icon) = tray_icon(mgr.is_running()) {
    builder = builder.icon(icon);
  }
  builder.build(app)?;
  Ok(())
}

const TRAY_RUN_PNG: &[u8] = include_bytes!("../icons/tray_running.png");
const TRAY_STOP_PNG: &[u8] = include_bytes!("../icons/tray_stopped.png");

fn tray_icon(running: bool) -> Option<tauri::image::Image<'static>> {
  tauri::image::Image::from_bytes(if running { TRAY_RUN_PNG } else { TRAY_STOP_PNG }).ok()
}

/// 显隐主窗口（托盘左键 / 菜单「显示/隐藏」共用）。
///
/// **每一步都要有结果可看**（AGENTS §1 调试优先）：找不到窗口、hide/show 失败都写进
/// `/debug` 的 `ui`，否则用户只会看到「点了没反应」。
fn toggle_window(app: &tauri::AppHandle) {
  let Some(w) = app.get_webview_window("main") else {
    push_ui(app, "error", "托盘：找不到主窗口（label=main）".to_string());
    return;
  };
  // 最小化时 `is_visible()` 仍是 true —— 先按「最小化也算隐藏」处理，
  // 否则点第一下会被当成「已显示」去 hide，看起来毫无反应（2026-10-26 实测）。
  let minimized = w.is_minimized().unwrap_or(false);
  let visible = w.is_visible().unwrap_or(false);
  if visible && !minimized {
    match w.hide() {
      Ok(()) => push_ui(app, "info", "托盘：已隐藏主窗口".to_string()),
      Err(e) => push_ui(app, "error", format!("托盘：隐藏窗口失败：{e}")),
    }
  } else {
    let r = w
      .unminimize()
      .and_then(|()| w.show())
      .and_then(|()| w.set_focus());
    match r {
      Ok(()) => push_ui(app, "info", "托盘：已显示主窗口".to_string()),
      Err(e) => push_ui(app, "error", format!("托盘：显示窗口失败：{e}")),
    }
  }
}

/// 「收到托盘」= 隐藏窗口（最小化 / 关窗都走这里）。
fn hide_to_tray(window: &tauri::Window) {
  let app = window.app_handle();
  match window.hide() {
    Ok(()) => push_ui(
      app,
      "info",
      "窗口已收到托盘（最小化 / 关闭都不退出）".to_string(),
    ),
    Err(e) => push_ui(app, "error", format!("隐藏窗口失败：{e}")),
  }
}

/// 往 `/debug` 的 `ui` 消息流写一行（带日志的场景没有 `app` 之外的东西）。
fn push_ui(app: &tauri::AppHandle, level: &str, msg: String) {
  app.state::<SharedHub>().inner().push_ui(level, msg);
}

/// 托盘菜单（随语言与运行状态重建）。
fn build_tray_menu<R: tauri::Runtime, M: Manager<R>>(
  mgr: &M,
  lang: &str,
  running: bool,
) -> tauri::Result<tauri::menu::Menu<R>> {
  use tauri::menu::{Menu, MenuItem};
  let en = lang == "en";
  let run = if running {
    if en { "Stop" } else { "停止" }
  } else if en {
    "Start"
  } else {
    "启动"
  };
  let show = if en {
    "Show / Hide window"
  } else {
    "显示 / 隐藏窗口"
  };
  let quit = if en { "Quit PureVox" } else { "退出 PureVox" };
  let run = MenuItem::with_id(mgr, "run", run, true, None::<&str>)?;
  let show = MenuItem::with_id(mgr, "show", show, true, None::<&str>)?;
  let quit = MenuItem::with_id(mgr, "quit", quit, true, None::<&str>)?;
  Menu::with_items(mgr, &[&run, &show, &quit])
}

/// 启动 / 停止：改引擎状态 + 提示音 + 同步托盘图标与菜单。
fn apply_running(app: &tauri::AppHandle, mgr: &AudioManager, on: bool) {
  if on {
    let _ = mgr.start();
  } else {
    mgr.stop();
  }
  let s = config::load_settings();
  if s.cue_on {
    cues::play(
      if on { &s.cue_start } else { &s.cue_stop },
      if on { "start" } else { "stop" },
    );
  }
  sync_tray(app, on);
}

/// 按当前设置 + 当前计划（音效热键）重启全局热键。
///
/// 绑定来源两处：① 设置里的启停热键；② 每个 `soundpad` 处理行里逐个音效录的键
/// （跟着计划走，增删音效即重注册）。计划是结构性变更（`apply_plan`）、
/// 设置是保存动作——两处都调这里，不搞两套注册逻辑。
fn start_hotkey(app: &tauri::AppHandle, host: &HotkeyHost) {
  let s = config::load_settings();
  let hub = app.state::<SharedHub>().inner().clone();
  let app2 = app.clone();
  let mgr = app.state::<Arc<AudioManager>>().inner().clone();
  let mut bindings: Vec<hotkey::Binding> = Vec::new();

  if s.hotkey_on && !s.hotkey.trim().is_empty() {
    let a = app2.clone();
    let m = mgr.clone();
    bindings.push((
      s.hotkey.clone(),
      Arc::new(move || toggle_running(&a, &m)) as Arc<dyn Fn() + Send + Sync>,
    ));
  }
  bindings.extend(plan::pad_hotkeys(&mgr.plan(), hub.clone()));

  let hk = hotkey::Hotkeys::start(bindings, hub);
  let mut guard = host.0.lock().unwrap();
  guard.stop();
  *guard = hk;
}

#[tauri::command]
fn get_settings() -> config::AppSettings {
  config::load_settings()
}

#[derive(serde::Serialize)]
struct CueInfo {
  id: String,
  label: String,
}

/// 可选提示音预设（供设置面板下拉）。
#[tauri::command]
fn list_cues() -> Vec<CueInfo> {
  cues::PRESETS
    .iter()
    .map(|(id, label)| CueInfo {
      id: (*id).into(),
      label: (*label).into(),
    })
    .collect()
}

/// 保存设置：归一化热键 → 落盘 → 重启热键 → 刷新托盘菜单（语言）。
#[tauri::command]
fn set_settings(mut settings: config::AppSettings, app: tauri::AppHandle) -> Result<(), String> {
  settings.hotkey = hotkey::normalize_spec(&settings.hotkey);
  config::save_settings(&settings)?;
  start_hotkey(&app, &app.state::<HotkeyHost>());
  let running = app.state::<Arc<AudioManager>>().is_running();
  if let Some(tray) = app.tray_by_id("main")
    && let Ok(menu) = build_tray_menu(&app, &settings.lang, running)
  {
    let _ = tray.set_menu(Some(menu));
  }
  Ok(())
}

fn toggle_running(app: &tauri::AppHandle, mgr: &AudioManager) {
  apply_running(app, mgr, !mgr.is_running());
}

fn sync_tray(app: &tauri::AppHandle, running: bool) {
  // 托盘图标 + 窗口（标题栏/任务栏）图标都随运行状态切换
  if let Some(icon) = tray_icon(running) {
    if let Some(tray) = app.tray_by_id("main") {
      let _ = tray.set_icon(Some(icon.clone()));
    }
    if let Some(w) = app.get_webview_window("main") {
      let _ = w.set_icon(icon);
    }
  }
  if let Ok(menu) = build_tray_menu(app, &config::load_lang(), running)
    && let Some(tray) = app.tray_by_id("main")
  {
    let _ = tray.set_menu(Some(menu));
  }
}

/// 启动 / 停止音频引擎（界面按钮、托盘、快捷键共用）。
#[tauri::command]
async fn set_running(
  on: bool,
  app: tauri::AppHandle,
  mgr: tauri::State<'_, Arc<AudioManager>>,
) -> Result<(), String> {
  let mgr = mgr.inner().clone();
  tauri::async_runtime::spawn_blocking(move || apply_running(&app, &mgr, on))
    .await
    .map_err(|e| format!("启动/停止异常：{e}"))
}

#[tauri::command]
fn get_running(mgr: tauri::State<'_, Arc<AudioManager>>) -> bool {
  mgr.is_running()
}

/// 切换界面语言：保存 + 更新托盘菜单（界面文案由前端 i18n 处理）。
#[tauri::command]
fn set_language(lang: String, app: tauri::AppHandle) -> Result<(), String> {
  let _ = config::save_lang(&lang);
  let running = app.state::<Arc<AudioManager>>().is_running();
  if let Some(tray) = app.tray_by_id("main") {
    let menu = build_tray_menu(&app, &lang, running).map_err(|e| e.to_string())?;
    tray.set_menu(Some(menu)).map_err(|e| e.to_string())?;
  }
  Ok(())
}
