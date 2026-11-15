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

//! Linux 全局热键：XDG 桌面门户 `org.freedesktop.portal.GlobalShortcuts`。
//!
//! 为什么不是 `XGrabKey`：GNOME 50 的会话是 **Wayland**，应用看不到全局按键，也没有
//! X11 的抓键接口。Wayland 上「把某个键交给应用」只有门户这一条路 —— 由合成器持有
//! 快捷键、按下时通知我们（`Activated` 信号），这也是 GNOME 设置里那些快捷键的同一套机制。
//!
//! ## 三条与 Windows 的实质差异（都如实上报，不静默）
//!
//! 1. **需要桌面应用标识（app id）**：门户按 systemd 用户单元名 `app-<app-id>-<pid>.scope`
//!    认应用身份，再由 `<app-id>.desktop` 找到桌面项。只有**从应用菜单 / 桌面图标启动**
//!    才有这个身份；终端里直接跑（`cargo run` / `target/debug/purevox`）没有 app id，
//!    门户会以 `NotAllowed: An app id is required` 拒绝。此时这里给出可执行的提示。
//! 2. **首次绑定会弹一次系统对话框**：由用户确认（并可改键）。确认结果（
//!    `trigger_description`）写进调试事件，用户能看到最终绑的是哪个键。
//! 3. **合成器可能改键**：`preferred_trigger` 只是「建议」，被占用时合成器会换成别的或
//!    直接不绑。以门户返回的 `shortcuts` 列表为准 —— 没在列表里的就是不生效。
//!
//! 实现走 zbus（阻塞版在线程里跑一个 current-thread tokio 运行时），不引入 C 依赖。

use std::collections::HashMap;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, AtomicU32, Ordering};
use std::thread::JoinHandle;
use std::time::Duration;

use futures_util::StreamExt;
use zbus::zvariant::{OwnedObjectPath, OwnedValue, Value};

use crate::debug::SharedHub;

const DEST: &str = "org.freedesktop.portal.Desktop";
const PORTAL_PATH: &str = "/org/freedesktop/portal/desktop";
const GS_IFACE: &str = "org.freedesktop.portal.GlobalShortcuts";
const REQUEST_IFACE: &str = "org.freedesktop.portal.Request";
const SESSION_IFACE: &str = "org.freedesktop.portal.Session";

/// 建会话 / 绑快捷键等待用户确认的上限。绑定要等用户在系统对话框里点确认，给足 2 分钟。
const CREATE_TIMEOUT: Duration = Duration::from_secs(15);
const BIND_TIMEOUT: Duration = Duration::from_secs(120);

static SEQ: AtomicU32 = AtomicU32::new(0);

/// 热键回调（与共享层同一个签名）。
type Cb = Arc<dyn Fn() + Send + Sync>;
/// 快捷键 id → 回调（门户实际绑上的那些）。
type Bound = HashMap<String, Cb>;
/// 快捷键 id → (规范键位串, 回调)（我们登记的那些，用于对账与报错）。
type Pending<'a> = HashMap<String, (&'a str, Cb)>;

/// 启动监听线程；线程本身就是「门户会话的生命周期」——停止时关会话、退出线程。
pub(super) fn spawn(
  bindings: Vec<super::Resolved>,
  hub: SharedHub,
  stop: Arc<AtomicBool>,
) -> Option<JoinHandle<()>> {
  let h = hub.clone();
  std::thread::Builder::new()
    .name("hotkey".into())
    .spawn(move || {
      let rt = match tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
      {
        Ok(rt) => rt,
        Err(e) => {
          hub.push_ui("error", format!("全局热键：运行时起不来（{e}）"));
          return;
        }
      };
      rt.block_on(run(bindings, hub, stop));
    })
    .map_err(|e| h.push_ui("error", format!("全局热键线程起不来：{e}")))
    .ok()
}

async fn run(bindings: Vec<super::Resolved>, hub: SharedHub, stop: Arc<AtomicBool>) {
  let conn = match zbus::Connection::session().await {
    Ok(c) => c,
    Err(e) => {
      hub.push_ui("error", format!("全局热键：连不上会话总线（{e}）"));
      return;
    }
  };
  let proxy = match zbus::Proxy::new(&conn, DEST, PORTAL_PATH, GS_IFACE).await {
    Ok(p) => p,
    Err(e) => {
      hub.push_ui("unavailable", explain(e));
      return;
    }
  };

  let session = match open_session(&conn, &proxy, &hub).await {
    Ok(s) => s,
    Err(reason) => {
      hub.push_ui("unavailable", reason);
      return;
    }
  };
  let callbacks = match bind(&conn, &proxy, &session, &bindings, &hub).await {
    Ok(m) => m,
    Err(reason) => {
      close_session(&conn, &session).await;
      hub.push_ui("error", reason);
      return;
    }
  };
  if callbacks.is_empty() {
    close_session(&conn, &session).await;
    return;
  }

  // 监听激活：门户只在用户按下**已绑定**的键时发 Activated。
  let mut activated = match proxy.receive_signal("Activated").await {
    Ok(s) => s,
    Err(e) => {
      close_session(&conn, &session).await;
      hub.push_ui("error", explain(e));
      return;
    }
  };
  let want = session.to_string();
  loop {
    tokio::select! {
      _ = tokio::time::sleep(Duration::from_millis(200)) => {
        if stop.load(Ordering::Relaxed) { break; }
      }
      msg = activated.next() => {
        let Some(msg) = msg else { break };
        let Ok((sid, id, _, _)) = msg.body().deserialize::<(String, String, u64, HashMap<String, OwnedValue>)>() else {
          continue;
        };
        if sid != want { continue; }
        if let Some(cb) = callbacks.get(&id) {
          let cb = cb.clone();
          // 回调里会启动/停止引擎（几百毫秒级）：别堵住门户消息循环。
          tokio::task::spawn_blocking(move || cb());
        }
      }
    }
  }
  close_session(&conn, &session).await;
}

/// 建门户会话，返回会话对象路径。
async fn open_session(
  conn: &zbus::Connection,
  proxy: &zbus::Proxy<'_>,
  hub: &SharedHub,
) -> Result<OwnedObjectPath, String> {
  let token = format!(
    "purevox_{}_{}",
    std::process::id(),
    SEQ.fetch_add(1, Ordering::Relaxed)
  );
  // 先订阅再调用：`handle_token` 决定请求对象路径，先挂上监听才不会漏掉 Response。
  let mut rx = request_stream(conn, &token).await?;
  let mut opts: HashMap<&str, Value<'_>> = HashMap::new();
  opts.insert("handle_token", Value::from(token.as_str()));
  opts.insert("session_handle_token", Value::from(token.as_str()));
  let handle: OwnedObjectPath = proxy
    .call("CreateSession", &(opts,))
    .await
    .map_err(explain)?;
  let (code, results) = wait_response(&mut rx, CREATE_TIMEOUT, &handle).await?;
  if code != 0 {
    return Err(format!("创建全局热键会话被拒绝（门户响应码 {code}）"));
  }
  let raw = results
    .get("session_handle")
    .and_then(|v| String::try_from(v.clone()).ok())
    .ok_or_else(|| "门户没有返回 session_handle".to_string())?;
  let path = OwnedObjectPath::try_from(raw).map_err(|e| format!("session_handle 非法：{e}"))?;
  hub.push_ui("info", "全局热键：门户会话已建立".to_string());
  Ok(path)
}

/// 绑定快捷键；返回「门户实际绑上的 id → 回调」。没被绑上的会逐条报出来。
async fn bind(
  conn: &zbus::Connection,
  proxy: &zbus::Proxy<'_>,
  session: &OwnedObjectPath,
  bindings: &[super::Resolved],
  hub: &SharedHub,
) -> Result<Bound, String> {
  let token = format!(
    "purevox_{}_{}",
    std::process::id(),
    SEQ.fetch_add(1, Ordering::Relaxed)
  );
  let mut rx = request_stream(conn, &token).await?;

  // (id, {description, preferred_trigger})
  let mut shortcuts: Vec<(String, HashMap<String, Value<'_>>)> = Vec::new();
  let mut meta: Pending<'_> = HashMap::new();
  for (i, (spec, _bits, _vk, cb)) in bindings.iter().enumerate() {
    let id = format!("hotkey{i}");
    let mut opt: HashMap<String, Value<'_>> = HashMap::new();
    opt.insert("description".into(), Value::from(spec.as_str()));
    match trigger_of(spec) {
      Some(t) => {
        opt.insert("preferred_trigger".into(), Value::from(t));
      }
      None => {
        hub.push_ui(
          "error",
          format!("热键键位无法翻译成桌面触发键，跳过：{spec}"),
        );
        continue;
      }
    }
    shortcuts.push((id.clone(), opt));
    meta.insert(id, (spec.as_str(), cb.clone()));
  }
  if shortcuts.is_empty() {
    return Ok(HashMap::new());
  }

  let mut opts: HashMap<String, Value<'_>> = HashMap::new();
  opts.insert("handle_token".into(), Value::from(token.as_str()));
  let handle: OwnedObjectPath = proxy
    .call("BindShortcuts", &(session, &shortcuts, "", opts))
    .await
    .map_err(explain)?;
  let (code, results) = wait_response(&mut rx, BIND_TIMEOUT, &handle).await?;
  if code != 0 {
    return Err(format!("全局热键绑定被取消或拒绝（门户响应码 {code}）"));
  }

  // 以门户返回的列表为准：没返回的键就是没绑上（被占用 / 用户删掉了）。
  let mut bound: Bound = HashMap::new();
  let list: Vec<(String, HashMap<String, OwnedValue>)> = results
    .get("shortcuts")
    .and_then(|v| Vec::<(String, HashMap<String, OwnedValue>)>::try_from(v.clone()).ok())
    .unwrap_or_default();
  for (id, props) in list {
    let Some((spec, cb)) = meta.get(&id) else {
      continue;
    };
    let desc = props
      .get("trigger_description")
      .and_then(|v| String::try_from(v.clone()).ok())
      .unwrap_or_else(|| "?".into());
    hub.push_ui("info", format!("热键已注册：{spec}（{desc}）"));
    bound.insert(id, cb.clone());
  }
  for (id, (spec, _)) in &meta {
    if !bound.contains_key(id) {
      hub.push_ui(
        "unavailable",
        format!("热键未生效（被占用或被用户取消）：{spec}"),
      );
    }
  }
  Ok(bound)
}

/// 在请求对象上挂 `Response` 监听。`handle_token` 就是请求路径的最后一段。
async fn request_stream(
  conn: &zbus::Connection,
  token: &str,
) -> Result<zbus::proxy::SignalStream<'static>, String> {
  let sender = conn
    .unique_name()
    .map(|n| n.as_str().trim_start_matches(':').replace('.', "_"))
    .unwrap_or_default();
  let path = format!("/org/freedesktop/portal/desktop/request/{sender}/{token}");
  let proxy = zbus::Proxy::new(conn, DEST, path.as_str(), REQUEST_IFACE)
    .await
    .map_err(explain)?;
  proxy.receive_signal("Response").await.map_err(explain)
}

/// 等 `Response` 信号 → (响应码, 结果字典)。超时明确报错。
async fn wait_response(
  rx: &mut zbus::proxy::SignalStream<'static>,
  timeout: Duration,
  handle: &OwnedObjectPath,
) -> Result<(u32, HashMap<String, OwnedValue>), String> {
  let wait = async {
    while let Some(msg) = rx.next().await {
      // 只认我们这次请求的对象（同一连接可能同时有别的请求）
      if msg.header().path().map(|p| p.as_str()) != Some(handle.as_str()) {
        continue;
      }
      if let Ok((code, results)) = msg
        .body()
        .deserialize::<(u32, HashMap<String, OwnedValue>)>()
      {
        return Ok((code, results));
      }
    }
    Err("门户在给出结果前断开了连接".to_string())
  };
  match tokio::time::timeout(timeout, wait).await {
    Ok(r) => r,
    Err(_) => Err("等待门户响应超时（系统对话框没有确认？）".to_string()),
  }
}

/// 关闭会话（门户会释放占用的快捷键）。
async fn close_session(conn: &zbus::Connection, session: &OwnedObjectPath) {
  if let Ok(p) = zbus::Proxy::new(conn, DEST, session.as_str(), SESSION_IFACE).await {
    let _ = p.call::<_, _, ()>("Close", &()).await;
  }
}

/// 把规范键位串（`Ctrl+Alt+1` / `Alt+.` / `F8`）翻成门户要的 `CTRL+ALT+1`
/// （修饰键名 + xkbcommon keysym 名，见 freedesktop.org「Shortcuts Specification」）。
///
/// 键位**合法性**已由共享的 [`super::parse_spec`] 校验过，这里只做「Windows 词表 →
/// xkb keysym 名」的翻译；翻译不了就返回 None（调用方跳过并报错，不静默）。
fn trigger_of(spec: &str) -> Option<String> {
  let mut mods: Vec<&str> = Vec::new();
  let mut key: Option<String> = None;
  for raw in spec.split('+') {
    let p = raw.trim().to_ascii_lowercase();
    if p.is_empty() {
      continue;
    }
    match p.as_str() {
      "ctrl" | "control" => mods.push("CTRL"),
      "alt" => mods.push("ALT"),
      "shift" => mods.push("SHIFT"),
      "win" | "super" | "meta" => mods.push("LOGO"),
      _ => key = Some(p),
    }
  }
  let keysym = keysym_of(&key?)?;
  let mut out = mods.join("+");
  if !out.is_empty() {
    out.push('+');
  }
  out.push_str(keysym);
  Some(out)
}

/// Windows 词表的 token → xkbcommon keysym 名（不带 `XKB_KEY_` 前缀）。
fn keysym_of(token: &str) -> Option<&'static str> {
  let t = token.trim();
  // 单字符：字母 / 数字 / 标点
  if t.len() == 1 {
    let c = t.as_bytes()[0];
    if c.is_ascii_lowercase() || c.is_ascii_digit() {
      return Some(match c {
        b'0' => "0",
        b'1' => "1",
        b'2' => "2",
        b'3' => "3",
        b'4' => "4",
        b'5' => "5",
        b'6' => "6",
        b'7' => "7",
        b'8' => "8",
        b'9' => "9",
        b'a' => "a",
        b'b' => "b",
        b'c' => "c",
        b'd' => "d",
        b'e' => "e",
        b'f' => "f",
        b'g' => "g",
        b'h' => "h",
        b'i' => "i",
        b'j' => "j",
        b'k' => "k",
        b'l' => "l",
        b'm' => "m",
        b'n' => "n",
        b'o' => "o",
        b'p' => "p",
        b'q' => "q",
        b'r' => "r",
        b's' => "s",
        b't' => "t",
        b'u' => "u",
        b'v' => "v",
        b'w' => "w",
        b'x' => "x",
        b'y' => "y",
        b'z' => "z",
        _ => unreachable!(),
      });
    }
    return Some(match c {
      b'`' => "grave",
      b'-' => "minus",
      b'=' => "equal",
      b'[' => "bracketleft",
      b']' => "bracketright",
      b'\\' => "backslash",
      b';' => "semicolon",
      b'\'' => "apostrophe",
      b',' => "comma",
      b'.' => "period",
      b'/' => "slash",
      _ => return None,
    });
  }
  // F1–F24
  if let Some(n) = t.strip_prefix('f')
    && let Ok(i) = n.parse::<u32>()
  {
    return Some(
      [
        "F1", "F2", "F3", "F4", "F5", "F6", "F7", "F8", "F9", "F10", "F11", "F12", "F13", "F14",
        "F15", "F16", "F17", "F18", "F19", "F20", "F21", "F22", "F23", "F24",
      ]
      .get((i as usize).checked_sub(1)?)?,
    );
  }
  // 小键盘 num0–num9
  if let Some(n) = t.strip_prefix("num")
    && let Ok(i) = n.parse::<usize>()
  {
    return Some(
      [
        "KP_0", "KP_1", "KP_2", "KP_3", "KP_4", "KP_5", "KP_6", "KP_7", "KP_8", "KP_9",
      ]
      .get(i)?,
    );
  }
  Some(match t {
    "backspace" => "BackSpace",
    "tab" => "Tab",
    "enter" => "Return",
    "esc" => "Escape",
    "space" => "space",
    "pageup" => "Page_Up",
    "pagedown" => "Page_Down",
    "end" => "End",
    "home" => "Home",
    "left" => "Left",
    "up" => "Up",
    "right" => "Right",
    "down" => "Down",
    "insert" => "Insert",
    "delete" => "Delete",
    _ => return None,
  })
}

/// 把 zbus 的失败翻译成用户能照做的说明（**不静默**，也不甩一坨英文堆栈）。
fn explain(e: zbus::Error) -> String {
  let s = e.to_string();
  if s.contains("app id") || s.contains("AppID") {
    return "全局热键需要桌面应用标识：请从「应用菜单 / 桌面图标」启动 PureVox\
            （终端里直接运行没有 app id，GNOME 门户会拒绝）"
      .to_string();
  }
  if s.contains("ServiceUnknown") || s.contains("not provided") || s.contains("UnknownMethod") {
    return "系统没有可用的全局快捷键门户（org.freedesktop.portal.GlobalShortcuts）；\
            需要 xdg-desktop-portal + GNOME/KDE 的对应实现"
      .to_string();
  }
  format!("全局热键不可用：{s}")
}
