// PureVox — AI 麦克风降噪工具
// Copyright (C) 2024-2026 a2heng <752848283@qq.com>
//
// PureVox is licensed under the GNU General Public License v3.0 or
// later (GPL-3.0-or-later).  See LICENSE for details.
//
// SPDX-License-Identifier: GPL-3.0-or-later

//! 单实例：默认**整个用户会话只允许一个 PureVox 进程**。
//!
//! 第二个进程不会启动第二套引擎 / 调试端口 / 网络服务，而是请求已在运行的那只
//! 把窗口显示到前台，然后自己退出。
//!
//! 隔离测试需要并行实例时，显式设 `PUREVOX_ALLOW_MULTI=1`：此时互斥体名 / 锁文件
//! 按 profile（`USERPROFILE` / `HOME`）区分，临时 profile 的测试实例互不影响，
//! 也不会占住用户正常实例的名额。默认不设 = 全局单实例，用户看到两只 exe 就是 bug。

use std::hash::{Hash, Hasher};

#[cfg(windows)]
#[path = "single_instance_windows.rs"]
mod imp;

#[cfg(unix)]
#[path = "single_instance_unix.rs"]
mod imp;

#[cfg(not(any(windows, unix)))]
#[path = "single_instance_other.rs"]
mod imp;

pub use imp::{activate_message, claim};

/// 是否允许并行实例（只给隔离测试用；正常启动绝不设这个变量）。
pub(super) fn multi_allowed() -> bool {
  std::env::var_os("PUREVOX_ALLOW_MULTI")
    .map(|v| v != "0")
    .unwrap_or(false)
}

/// 单实例键：默认 0（整个用户会话唯一）；只有显式允许并行时才按 profile 分开。
///
/// `DefaultHasher::new()` 使用固定种子，跨进程结果一致；只用来给互斥体/锁文件起名，
/// 不参与任何安全判断。
pub(super) fn instance_key() -> u64 {
  if !multi_allowed() {
    return 0;
  }
  let home = std::env::var_os("USERPROFILE")
    .or_else(|| std::env::var_os("HOME"))
    .unwrap_or_default();
  let mut h = std::collections::hash_map::DefaultHasher::new();
  home.hash(&mut h);
  h.finish()
}
