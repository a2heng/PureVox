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

  // 主线程栈：MSVC 默认只留 1 MB（实测 SizeOfStackReserve=1048576）。GPUI 的窗口创建
  // 回调已经吃掉大半，再在主线程上同步 `AudioManager::start()`（静默自启路径）就溢出。
  // 后台任务是 2 MB 的 std 线程所以没事；把主线程也抬到 8 MB，保留量不值钱。
  #[cfg(windows)]
  println!("cargo:rustc-link-arg=/STACK:8388608");

  // 任务栏 / 标题栏左上角图标：GPUI 在 Windows 上用 `LoadImageW(模块, 资源 1)`
  // 取图标，所以路径必须是把 `icons/icon.ico` 编进 exe 的第 1 个图标资源。
  #[cfg(windows)]
  {
    let mut res = tauri_winres::WindowsResource::new();
    // GPUI 的 Windows 后端用 `LoadImageW(模块, 资源 1)` 取窗口图标，
    // 所以这里必须钉成 ID 1（`set_icon` 默认 32512 = IDI_APPLICATION）。
    res.set_icon_with_id("icons/icon.ico", "1");
    res.set("ProductName", "PureVox");
    res.set("FileDescription", "PureVox");
    if let Err(e) = res.compile() {
      println!("cargo:warning=嵌入 Windows 图标失败：{e}");
    }
  }
}
