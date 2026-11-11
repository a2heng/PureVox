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

//! 进程级 Tokio 运行时（网络服务 / 更新检查）。
//!
//! 以前这里是 `tauri::async_runtime`；去掉 Tauri 后由本模块提供**唯一**的异步运行时。
//! 只有一个多线程运行时、按需懒建：没有网络 / 更新时不会多出线程。

use std::sync::OnceLock;

use tokio::runtime::{Handle, Runtime};

static RUNTIME: OnceLock<Runtime> = OnceLock::new();

fn runtime() -> &'static Runtime {
  RUNTIME.get_or_init(|| {
    tokio::runtime::Builder::new_multi_thread()
      .worker_threads(2)
      .thread_name("purevox-async")
      .enable_all()
      .build()
      .expect("创建 Tokio 运行时失败")
  })
}

/// 运行时句柄（在异步上下文里想再 spawn 时用）。
pub fn handle() -> Handle {
  runtime().handle().clone()
}

/// 投递一个后台任务（与 `tokio::spawn` 同义）。
pub fn spawn<F>(future: F) -> tokio::task::JoinHandle<F::Output>
where
  F: std::future::Future + Send + 'static,
  F::Output: Send + 'static,
{
  runtime().spawn(future)
}

/// 在后台线程跑一个阻塞任务（数据库 / 文件 / 注册表 / 设备枚举）。
pub async fn spawn_blocking<F, R>(f: F) -> Result<R, tokio::task::JoinError>
where
  F: FnOnce() -> R + Send + 'static,
  R: Send + 'static,
{
  runtime().spawn_blocking(f).await
}
