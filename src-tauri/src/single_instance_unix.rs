// PureVox — AI 麦克风降噪工具
// Copyright (C) 2024-2026 a2heng <752848283@qq.com>
//
// PureVox is licensed under the GNU General Public License v3.0 or
// later (GPL-3.0-or-later).  See LICENSE for details.
//
// SPDX-License-Identifier: GPL-3.0-or-later

//! Unix 单实例：对 profile 目录下的锁文件做非阻塞 `flock`。
//!
//! 锁随文件描述符存在，进程退出（包括崩溃）时内核自动释放；文件本身可以留着，
//! 不需要 pid 清理，也不会出现「上次崩溃后永远起不来」。
//! Linux 托盘没有统一的「显示已有窗口」协议，第二个进程只退出，不假装能激活窗口。

use std::os::unix::io::AsRawFd;
use std::path::{Path, PathBuf};

fn lock_path() -> PathBuf {
  let dir = std::env::var_os("XDG_RUNTIME_DIR")
    .map(PathBuf::from)
    .or_else(|| std::env::var_os("HOME").map(|h| PathBuf::from(h).join(".purevox")))
    .unwrap_or_else(|| PathBuf::from("."));
  dir.join(format!("instance-{:016x}.lock", super::instance_key()))
}

fn try_lock(path: &Path) -> bool {
  if let Some(dir) = path.parent() {
    let _ = std::fs::create_dir_all(dir);
  }
  let Ok(file) = std::fs::OpenOptions::new()
    .create(true)
    .read(true)
    .write(true)
    .truncate(false)
    .open(path)
  else {
    // 锁文件都建不出来时不要把自己锁死：宁可多开，也不静默起不来。
    return true;
  };
  let rc = unsafe { libc::flock(file.as_raw_fd(), libc::LOCK_EX | libc::LOCK_NB) };
  if rc == 0 {
    // 锁跟着 fd 走：故意泄漏到进程结束，系统自动释放。
    std::mem::forget(file);
    true
  } else {
    false
  }
}

/// 占住单实例；返回 false = 已有同 profile 的实例，调用方直接退出。
pub fn claim() -> bool {
  try_lock(&lock_path())
}

/// Unix 没有统一的窗口激活协议：返回 0（Windows 托盘 WndProc 才用这个 id）。
pub fn activate_message() -> u32 {
  0
}
