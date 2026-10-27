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

//! ONNX 推理（onnxruntime，`ort`）。模型契约见 `denoise.rs`。
//!
//! 模型是流式的：每 10 ms 一个 `*_hop`（480 样本）+ 一个大的一维 `cache_in`，
//! 返回处理后的 hop 与新的 `cache_out`；引擎只负责搬运波形与缓存，STFT 在模型图内。

pub mod aec;
pub mod denoise;
pub mod tse;

use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex, MutexGuard, OnceLock};
use std::time::Instant;

use ort::session::Session;

/// 打包后 Tauri 资源目录下的模型目录（由 `main` 在 setup 里 [`set_model_dir`] 写入）。
/// 开发态不设，`model_path` 靠仓库 `models/` 兜底。
static MODEL_DIR: OnceLock<PathBuf> = OnceLock::new();

/// 注册打包资源目录（`resource_dir()/models`）。只第一次生效；目录不存在时调用方不应调用。
pub fn set_model_dir(dir: PathBuf) {
  let _ = MODEL_DIR.set(dir);
}

/// 进程级 ONNX Runtime 环境（ort 2.0 rc.15 起 `Session::builder` 必须显式传它）。
fn ort_env() -> Result<&'static ort::environment::Environment, String> {
  static ENV: OnceLock<Result<ort::environment::Environment, String>> = OnceLock::new();
  ENV
    .get_or_init(|| {
      ort::environment::init()
        .with_name("PureVox")
        .build()
        .map_err(|e| format!("创建 onnxruntime 环境失败：{e}"))
    })
    .as_ref()
    .map_err(|e| e.clone())
}

/// 统一 ONNX 会话：单线程推理 + 关闭自旋等待。
///
/// ort/onnxruntime 默认按核数建线程池并忙等（allow_spinning），会占满 CPU 并和音频回调抢核，
/// 实测把 1 ms 的推理拖到 100 ms。降噪 / AEC / TSE 都走这一条路径。
pub(crate) fn build_session(path: &Path) -> Result<Session, String> {
  let path = path.to_path_buf();
  let env = ort_env()?;
  let built = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
    Session::builder(env)
      .map_err(|e| format!("创建 onnxruntime 会话失败：{e}"))?
      .with_intra_threads(1)
      .map_err(|e| format!("设置推理线程数失败：{e}"))?
      .with_inter_threads(1)
      .map_err(|e| format!("设置并行线程数失败：{e}"))?
      .with_intra_op_spinning(false)
      .map_err(|e| format!("关闭自旋等待失败：{e}"))?
      .commit_from_file(&path)
      .map_err(|e| format!("加载模型 {} 失败：{e}", path.display()))
  }));
  match built {
    Ok(v) => v,
    Err(payload) => {
      // ort 在「动态库 API 版本不匹配」这类情况下是 **panic** 而不是返回 Err。
      // 不接住它就会带走重建线程（用户看到「应用计划异常」，设备已被关掉一半）。
      let msg = payload
        .downcast_ref::<&str>()
        .map(|s| (*s).to_string())
        .or_else(|| payload.downcast_ref::<String>().cloned())
        .unwrap_or_else(|| "<non-string panic>".to_string());
      crate::debug::events::log(
        "error",
        "infer",
        format!("onnxruntime 初始化失败（模型 {}）：{msg}", path.display()),
      );
      Err(format!(
        "onnxruntime 初始化失败（模型 {}）：{msg}。请检查 libonnxruntime 版本与 ort 的 api-27 是否匹配",
        path.display()
      ))
    }
  }
}

/// 从会话输入里读某个一维缓存的元素个数（不写死维度）。
pub(crate) fn cache_dim(session: &Session, name: &str) -> Result<usize, String> {
  use ort::value::ValueType;
  session
    .inputs()
    .iter()
    .find(|i| i.name() == name)
    .and_then(|i| match i.dtype() {
      ValueType::Tensor { shape, .. } => Some(shape.num_elements()),
      _ => None,
    })
    .ok_or_else(|| format!("模型缺少 {name} 输入或不是张量"))
}

// ---------------------------------------------------------------------------
// 模型池：同一个模型文件只加载一次，多行/多次重建复用（ARCHITECTURE.md §7）
// ---------------------------------------------------------------------------

/// 每个模型文件最多缓存几个会话（够多列并发用；超出就丢弃，不再无限涨）。
const SESSIONS_PER_MODEL: usize = 8;

/// 模型池的公开统计（`/debug` 的 `audio.engine_info.models`）。
#[derive(Clone, Debug, serde::Serialize)]
pub struct ModelStat {
  /// 文件名（不是完整路径；界面上更好读）
  pub key: String,
  /// 实际从磁盘加载的次数
  pub loads: u64,
  /// 命中缓存复用的次数
  pub reused: u64,
  /// 当前被行占用的会话数
  pub in_use: u32,
  /// 最近一次加载耗时（ms）
  pub last_load_ms: f64,
  /// 累计加载耗时（ms）
  pub total_load_ms: f64,
}

#[derive(Default)]
struct StatEntry {
  key: String,
  loads: u64,
  reused: u64,
  in_use: u32,
  last_load_ms: f64,
  total_load_ms: f64,
}

struct Pool {
  /// 路径 → 空闲会话（`ort::Session::run` 需要 `&mut self`，所以每个会话一把锁；
  /// 不同行拿不同会话，推理互不串行）
  idle: Mutex<HashMap<PathBuf, Vec<Arc<Mutex<Session>>>>>,
}

fn pool() -> &'static Arc<Pool> {
  static POOL: OnceLock<Arc<Pool>> = OnceLock::new();
  POOL.get_or_init(|| {
    Arc::new(Pool {
      idle: Mutex::new(HashMap::new()),
    })
  })
}

fn stats() -> &'static Mutex<HashMap<PathBuf, StatEntry>> {
  static STATS: OnceLock<Mutex<HashMap<PathBuf, StatEntry>>> = OnceLock::new();
  STATS.get_or_init(|| Mutex::new(HashMap::new()))
}

/// 一次「借出」的模型会话：`Drop` 即归还池子（**不卸载**，下次重建直接复用）。
pub struct LeasedSession {
  key: PathBuf,
  session: Option<Arc<Mutex<Session>>>,
  pool: Arc<Pool>,
}

impl LeasedSession {
  /// 借用底层会话。锁中毒时恢复内部值（模型会话不该因为一次 panic 永久不可用）。
  pub fn lock(&self) -> MutexGuard<'_, Session> {
    self
      .session
      .as_ref()
      .expect("LeasedSession 归还后不应再使用")
      .lock()
      .unwrap_or_else(|e| e.into_inner())
  }
}

impl Drop for LeasedSession {
  fn drop(&mut self) {
    let Some(session) = self.session.take() else {
      return;
    };
    let mut idle = self.pool.idle.lock().unwrap_or_else(|e| e.into_inner());
    let slot = idle.entry(self.key.clone()).or_default();
    if slot.len() < SESSIONS_PER_MODEL {
      slot.push(session);
    }
    drop(idle);
    if let Ok(mut st) = stats().lock()
      && let Some(e) = st.get_mut(&self.key)
    {
      e.in_use = e.in_use.saturating_sub(1);
    }
  }
}

/// 借一个模型会话：命中池就复用，未命中才真正加载（并记录加载耗时）。
pub fn checkout(path: &Path) -> Result<LeasedSession, String> {
  let key = std::fs::canonicalize(path).unwrap_or_else(|_| path.to_path_buf());
  let cached = {
    let mut idle = pool().idle.lock().unwrap_or_else(|e| e.into_inner());
    idle.get_mut(&key).and_then(|v| v.pop())
  };
  let (session, reused, load_ms) = match cached {
    Some(s) => (s, true, 0.0),
    None => {
      let t0 = Instant::now();
      let session = Arc::new(Mutex::new(build_session(&key)?));
      let ms = t0.elapsed().as_secs_f64() * 1000.0;
      (session, false, ms)
    }
  };
  {
    let mut st = stats().lock().unwrap_or_else(|e| e.into_inner());
    let e = st.entry(key.clone()).or_insert_with(|| StatEntry {
      key: key
        .file_name()
        .map(|s| s.to_string_lossy().to_string())
        .unwrap_or_else(|| key.display().to_string()),
      ..StatEntry::default()
    });
    e.in_use = e.in_use.saturating_add(1);
    if reused {
      e.reused += 1;
    } else {
      e.loads += 1;
      e.last_load_ms = load_ms;
      e.total_load_ms += load_ms;
    }
  }
  Ok(LeasedSession {
    key,
    session: Some(session),
    pool: pool().clone(),
  })
}

/// 模型池统计（按文件名排序）。
pub fn models_stats() -> Vec<ModelStat> {
  let Ok(g) = stats().lock() else {
    return Vec::new();
  };
  let mut out: Vec<ModelStat> = g
    .values()
    .map(|e| ModelStat {
      key: e.key.clone(),
      loads: e.loads,
      reused: e.reused,
      in_use: e.in_use,
      last_load_ms: e.last_load_ms,
      total_load_ms: e.total_load_ms,
    })
    .collect();
  out.sort_by(|a, b| a.key.cmp(&b.key));
  out
}

/// 后台预热：把现役与可选模型都加载一遍（开销不计；换来第一次启动/重建不再卡在磁盘与 ORT 初始化）。
///
/// 只做**加载 + 归还**，不占住会话；失败的模型写事件日志（缺文件不算致命）。
pub fn preload_all() {
  std::thread::Builder::new()
    .name("model-preload".into())
    .spawn(|| {
      let mut files: Vec<String> = Vec::new();
      files.extend(denoise_models().iter().map(|(f, _)| (*f).to_string()));
      files.extend(aec::aec_models().iter().map(|(f, _)| (*f).to_string()));
      files.extend(tse::tse_models().iter().map(|(f, _)| (*f).to_string()));
      files.sort();
      files.dedup();
      for f in files {
        let Ok(path) = model_path(&f) else {
          continue;
        };
        match checkout(&path) {
          Ok(lease) => {
            drop(lease);
            crate::debug::events::log("info", "infer", format!("模型已预热：{f}"));
          }
          Err(e) => crate::debug::events::log("warn", "infer", format!("模型预热失败：{f}：{e}")),
        }
      }
    })
    .ok();
}

/// 现役降噪模型（models/ 下的文件名）。
///
/// 2026-10-26 收模型体积：只留 `202609b`（现役）与 `202606`（旧版对照），
/// `202609a` / `202609c` 已从 `models/` 删除（包里少 23 MB）。
pub const MODEL_DENOISE: &str = "purevox_denoise_202609b_ep0046.onnx";

/// 可选降噪模型 (文件名, 界面名)，供界面下拉（ARCHITECTURE.md §7）。
pub fn denoise_models() -> &'static [(&'static str, &'static str)] {
  &[
    (MODEL_DENOISE, "降噪 202609b（现役）"),
    (
      "purevox_denoise_202606_ep0014_op17.onnx",
      "降噪 202606（旧版）",
    ),
  ]
}

/// 解析模型文件路径。开发时模型在仓库 `models/`，打包后应在可执行文件旁/资源目录。
pub fn model_path(file: &str) -> Result<PathBuf, String> {
  let mut tried = Vec::new();
  let mut candidates: Vec<PathBuf> = Vec::new();
  // 打包后优先用 Tauri 注册的资源目录（Linux deb 落在 /usr/lib/<productName>/models，
  // 与可执行文件不同级，仅靠 exe 相邻目录找不到）。
  if let Some(dir) = MODEL_DIR.get() {
    candidates.push(dir.join(file));
  }
  if let Ok(dir) = std::env::var("PUREVOX_MODEL_DIR") {
    candidates.push(PathBuf::from(dir).join(file));
  }
  if let Ok(cwd) = std::env::current_dir() {
    candidates.push(cwd.join("models").join(file));
    candidates.push(cwd.join("..").join("models").join(file));
  }
  if let Ok(exe) = std::env::current_exe() {
    let dir = exe.parent().unwrap_or(Path::new("."));
    candidates.push(dir.join("models").join(file));
    // target/debug/purevox.exe → 仓库根/models
    if let Some(repo) = dir.ancestors().nth(3) {
      candidates.push(repo.join("models").join(file));
    }
    candidates.push(dir.join("resources").join("models").join(file));
  }
  for c in candidates {
    if c.is_file() {
      return Ok(c);
    }
    tried.push(c.display().to_string());
  }
  Err(format!(
    "找不到模型 {file}；可用 PUREVOX_MODEL_DIR 指定模型目录。已尝试：{}",
    tried.join("；")
  ))
}
