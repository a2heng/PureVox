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

//! Linux 实现：WebKitGTK 的 `webkit_web_view_get_snapshot`。
//! 截的是 WebView 渲染表面（应用内容），不需要 Wayland/X11 截屏协议。

use std::path::Path;
use std::time::Duration;

use tauri::Manager;
use webkit2gtk::WebViewExt;

pub(super) fn capture_to(path: &Path) -> Result<(), String> {
  let app = super::APP
    .get()
    .cloned()
    .ok_or_else(|| "截屏尚未初始化（应用还没启动完成）".to_string())?;
  let path = path.to_path_buf();
  let (tx, rx) = std::sync::mpsc::channel::<Result<(), String>>();

  let app2 = app.clone();
  app
    .run_on_main_thread(move || {
      let Some(win) = app2.get_webview_window("main") else {
        let _ = tx.send(Err("找不到主窗口（main）".to_string()));
        return;
      };
      let tx2 = tx.clone();
      let path2 = path.clone();
      let res = win.with_webview(move |platform| {
        let Some(webview) = platform.downcast_ref::<tauri_runtime_wry::Webview>() else {
          let _ = tx2.send(Err("当前运行时不是 wry/WebKitGTK，无法截屏".to_string()));
          return;
        };
        let view = webview.inner();
        // 回调在 GTK 主循环里执行；写盘很小，不阻塞音频线程（它们不在主线程）。
        view.snapshot(
          webkit2gtk::SnapshotRegion::FullDocument,
          webkit2gtk::SnapshotOptions::NONE,
          None::<&webkit2gtk::gio::Cancellable>,
          move |res| {
            let out = match res {
              Ok(surface) => {
                let mut f = match std::fs::File::create(&path2) {
                  Ok(f) => f,
                  Err(e) => {
                    let _ = tx2.send(Err(format!("创建截图文件失败：{e}")));
                    return;
                  }
                };
                match surface.write_to_png(&mut f) {
                  Ok(()) => Ok(()),
                  Err(e) => Err(format!("写 PNG 失败：{e}")),
                }
              }
              Err(e) => Err(format!("WebKit 截屏失败：{e}")),
            };
            let _ = tx2.send(out);
          },
        );
      });
      if let Err(e) = res {
        let _ = tx.send(Err(format!("调度 WebView 截屏失败：{e}")));
      }
    })
    .map_err(|e| format!("调度主线程失败：{e}"))?;

  rx.recv_timeout(Duration::from_secs(8))
    .map_err(|_| "截屏超时（WebKit 没有在 8 秒内返回）".to_string())?
}
