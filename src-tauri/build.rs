// PureVox — AI 麦克风降噪工具
// Copyright (C) 2024-2026 a2heng <752848283@qq.com>
//
// PureVox is licensed under the GNU General Public License v3.0 or
// later (GPL-3.0-or-later).  See LICENSE for details.
//
// SPDX-License-Identifier: GPL-3.0-or-later

fn main() {
  // onnxruntime.lib（ort 的预编译包）里有 telemetry.cc.obj，引用 shell32 的
  // `CommandLineToArgvW`。主程序恰好从别的依赖拿到 shell32，但 `examples/*`
  // 是独立目标，`cargo test` 会一起链 —— 在这里显式声明，别靠偶然。
  #[cfg(windows)]
  println!("cargo:rustc-link-lib=shell32");

  // 任务栏 / 标题栏左上角图标：GPUI 在 Windows 上用 `LoadImageW(模块, 资源 1)`
  // 取图标，所以路径必须是把 `icons/icon.ico` 编进 exe 的第 1 个图标资源。
  #[cfg(windows)]
  {
    let mut res = tauri_winres::WindowsResource::new();
    res.set_icon("icons/icon.ico");
    res.set("ProductName", "PureVox");
    res.set("FileDescription", "PureVox");
    if let Err(e) = res.compile() {
      println!("cargo:warning=嵌入 Windows 图标失败：{e}");
    }
  }
}
