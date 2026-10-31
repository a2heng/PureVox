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

//! ONNX 推理（onnxruntime，`ort`）。模型契约见 `denoise.rs` / `aec.rs` / `tse.rs`。
//!
//! 模型是流式的：每 10 ms 一帧 `(1,2,481)` 复数谱 + 一个一维 `cache_in`，
//! 返回增强谱与新的 `cache_out`。**傅里叶在图外**（2026-10-29 换代）：波形↔谱、OLA
//! 与 1 hop 滞后全在 [`spec`]，三个模型共用同一份 DSP。

pub mod aec;
pub mod denoise;
pub mod spec;
pub mod tse;

/// `spec` 的 hop 常量必须与音频引擎的 10 ms 网格一致（spec 不引 `crate::audio`，
/// 好让 `examples/` 直接复用同一份 DSP）。
const _: () = assert!(spec::HOP == crate::audio::HOP);

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

/// 读某个输入的形状（不写死维度；换一代模型不用改代码）。
pub(crate) fn input_shape(session: &Session, name: &str) -> Result<Vec<usize>, String> {
  use ort::value::ValueType;
  session
    .inputs()
    .iter()
    .find(|i| i.name() == name)
    .and_then(|i| match i.dtype() {
      ValueType::Tensor { shape, .. } => Some(shape.iter().map(|&d| d.max(0) as usize).collect()),
      _ => None,
    })
    .ok_or_else(|| format!("模型缺少 {name} 输入或不是张量"))
}

/// 校验谱接口：`(1,2,481)`（= 960 点 FFT / 480 hop，2026-10 起的图外傅里叶契约）。
///
/// 旧模型（`mix_hop (1,480)`，STFT 在图内）在这里直接报错而不是悄悄跑出垃圾：
/// 两代模型同名参数不同，静默错会表现为「降噪没效果」，比启动失败难查得多。
pub(crate) fn check_spec(session: &Session, name: &str) -> Result<(), String> {
  let shape = input_shape(session, name)?;
  if shape != [1, 2, spec::BINS] {
    return Err(format!(
      "模型接口不是谱接口：{name} 的形状是 {shape:?}，期望 [1, 2, {}] \
       （2026-10 起模型只吃/吐复数谱，傅里叶由应用侧做；旧波形契约的模型不能直接用）",
      spec::BINS
    ));
  }
  Ok(())
}

/// 找**谱输入**：形状 `(1,2,481)` 的那个张量输入（名字随代变：`spec_in` / `mix_spec`）。
///
/// TSE 这类「一代一个输入名」的线用它，别写死名字；必须**恰好一个**命中，否则报错。
pub(crate) fn find_spec_input(session: &Session) -> Result<String, String> {
  use ort::value::ValueType;
  let mut hit: Vec<String> = Vec::new();
  for i in session.inputs() {
    if let ValueType::Tensor { shape, .. } = i.dtype() {
      let dims: Vec<usize> = shape.iter().map(|&d| d.max(0) as usize).collect();
      if dims == [1, 2, spec::BINS] {
        hit.push(i.name().to_string());
      }
    }
  }
  match hit.len() {
    1 => Ok(hit.remove(0)),
    0 => Err(format!(
      "模型没有 (1,2,{}) 复数谱输入（2026-10 起模型只吃/吐复数谱，旧波形契约的模型不能直接用）",
      spec::BINS
    )),
    _ => Err(format!("模型有多个 (1,2,{}) 输入 {hit:?}，无法判断哪个是谱", spec::BINS)),
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

/// 计划里**当前还引用**的模型文件名（小写不敏感无所谓，文件名本来就固定）。
///
/// `None` = 还没设置过（启动预热阶段）⇒ 不限制。`Some` 之后，归还到池子的会话若不在
/// 名单里就**直接丢弃**（连 ORT 的线程一起释放）——这就是「模型不再列中」的回收钩子。
fn allowed() -> &'static Mutex<Option<std::collections::HashSet<String>>> {
  static ALLOWED: OnceLock<Mutex<Option<std::collections::HashSet<String>>>> = OnceLock::new();
  ALLOWED.get_or_init(|| Mutex::new(None))
}

/// 把已经 free 掉的大块堆**还给系统**。
///
/// 光 drop 掉 `ort::Session` 不够：ORT 的权重是 malloc 出来的大块，glibc 只把它们留在
/// 自己的 arena 里，RSS 一点不降（2026-10-31 实测：drop 4 个会话后 79 MB 纹丝不动，
/// trim 之后 46 MB）。Windows 的 CRT 会把大块还给系统，不需要这一步。
fn trim_heap() {
  #[cfg(target_os = "linux")]
  unsafe {
    libc::malloc_trim(0)
  };
}

fn is_allowed(key: &Path) -> bool {
  let Ok(g) = allowed().lock() else {
    return true;
  };
  let Some(list) = g.as_ref() else {
    return true;
  };
  key
    .file_name()
    .map(|n| list.contains(&*n.to_string_lossy()))
    .unwrap_or(true)
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
    if !is_allowed(&self.key) {
      // 计划里已经没有这个模型了：不回收进池子，`session` 在这里 drop ⇒ ORT 会话、
      // 权重内存与它的线程一起释放（2026-10-31「删除就回收」）。顺手还堆给系统。
      drop(session);
      trim_heap();
      return;
    }
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

/// 把「计划里还引用的模型」设为白名单，并**释放白名单之外的会话**（内存 + ORT 线程）。
///
/// 返回释放掉的空闲会话数。正在被行占用的会话不在池子里，等它们归还时由
/// [`LeasedSession::drop`] 按同一份白名单丢弃 —— 所以调用时机不必卡在旧会话停掉之后。
pub fn retain_models(keep: &[String]) -> usize {
  let set: std::collections::HashSet<String> = keep.iter().cloned().collect();
  if let Ok(mut g) = allowed().lock() {
    *g = Some(set);
  }
  // 正在被行占用的模型：名字集合。它们若还**额外**躺着一份空闲副本，那份是纯重复——
  // 每次重建都是「新会话先加载（旧会话还攥着它那份）→ 旧会话再归还」，稳态就变成
  // 一份在用 + 一份空闲（降噪实测 +40 MB）。这里把重复副本丢掉，稳态回到每个模型一份。
  let in_use: std::collections::HashSet<String> = stats()
    .lock()
    .map(|st| {
      st.iter()
        .filter(|(_, e)| e.in_use > 0)
        .filter_map(|(k, _)| k.file_name().map(|n| n.to_string_lossy().into_owned()))
        .collect()
    })
    .unwrap_or_default();
  let dropped = {
    let mut idle = pool().idle.lock().unwrap_or_else(|e| e.into_inner());
    let before: usize = idle.values().map(|v| v.len()).sum();
    idle.retain(|k, _| {
      is_allowed(k)
        && !k
          .file_name()
          .map(|n| in_use.contains(&*n.to_string_lossy()))
          .unwrap_or(false)
    });
    let after: usize = idle.values().map(|v| v.len()).sum();
    before - after
  };
  if dropped > 0 {
    if let Ok(mut st) = stats().lock() {
      st.retain(|k, _| is_allowed(k));
    }
    trim_heap();
    crate::debug::events::log(
      "info",
      "infer",
      format!("模型回收：释放 {dropped} 个不再使用的会话（保留 {keep:?}）"),
    );
  }
  dropped
}

/// 计划里**还留着**的模型文件名（回收白名单的唯一来源）。
///
/// 口径是「**删除才回收，暂停不动内存**」（2026-10-31 用户要求）：只要行还在计划里
/// （哪怕停用了、整列暂停了），它的模型就留在池子里；只有行/列被**删掉**才回收。
/// 停用只是挂起链路，重新启用时不该再花 40 ms 重新加载 + 抖一次内存。
///
/// - `denoise` 行 → 计划里选的降噪模型
/// - `tse` 行 → 主图 + 配套注册编码器（成对，缺一不可）
/// - `echo_cancel` 行 / 勾了 AEC 的 `remote_mic` 行 → AEC 模型
pub fn models_in_plan(plan: &crate::plan::Plan) -> Vec<String> {
  let mut out: Vec<String> = Vec::new();
  for c in plan.columns.iter() {
    for r in c.rows.iter() {
      let model = r.params.get("model").map(|s| s.as_str()).unwrap_or("");
      match r.ptype.as_str() {
        "denoise" => out.push(known_model(denoise_models(), model).to_string()),
        "tse" => {
          out.push(known_model(tse::tse_models(), model).to_string());
          out.push(tse::MODEL_TSE_REF.to_string());
        }
        "echo_cancel" => out.push(aec::MODEL_AEC.to_string()),
        "remote_mic" if r.params.get("aec").map(|v| v == "1").unwrap_or(false) => {
          out.push(aec::MODEL_AEC.to_string())
        }
        _ => {}
      }
    }
  }
  out.sort();
  out.dedup();
  out
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
      // 注册编码器不是「可选模型」，但同样要预热（每句只跑一次，慢一次就够了）
      files.push(tse::MODEL_TSE_REF.to_string());
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

/// 现役降噪模型（`models/` 下的文件名）。
///
/// **2026-10-29 起文件名不带版本号**：应用侧代码只认这四个固定名字，
/// 换模型 = 换文件（模型标识记在 ONNX metadata `model` 与 `models/README.md`）。
/// 谱接口见 [`spec`]。
pub const MODEL_DENOISE: &str = "purevox_denoise.onnx";

/// 可选降噪模型 (文件名, 界面名)，供界面下拉（ARCHITECTURE.md §7）。
/// 界面名只是**显示用**，不重复版本号：真正的标识在 ONNX metadata `model`。
pub fn denoise_models() -> &'static [(&'static str, &'static str)] {
  &[(MODEL_DENOISE, "降噪（现役）")]
}

/// 计划里存的模型名 → 现役模型（不在列表里就回落到第一个，并记一条事件日志）。
///
/// 为什么需要：2026-10-29 起模型文件名**不带版本号**（换模型 = 换文件），
/// 用户在旧计划里存的是带版本号的旧文件名，文件已经不在包里。
/// 直接按名字找会得到「找不到模型」的行，不如收敛到现役默认值（界面下拉也只是提示）。
pub fn known_model(list: &'static [(&'static str, &'static str)], requested: &str) -> &'static str {
  match list.iter().find(|(f, _)| *f == requested) {
    Some((f, _)) => f,
    None => {
      crate::debug::events::log(
        "warn",
        "infer",
        format!("计划里的模型 {requested} 不在现役列表 → 用 {}", list[0].0),
      );
      list[0].0
    }
  }
}

/// 解析模型文件路径。开发时模型在仓库 `models/`，打包后应在可执行文件旁/资源目录。
pub fn model_path(file: &str) -> Result<PathBuf, String> {
  let mut tried = Vec::new();
  let mut candidates: Vec<PathBuf> = Vec::new();
  // **开发期覆盖优先**（2026-10-30）：`PUREVOX_MODEL_DIR` 放在资源目录之前，
  // 这样频繁换模型时只改仓库 `models/` + 重启应用即可，不必重新打 deb。
  // （打包后的正常启动不设这个环境变量，仍然用资源目录。）
  if let Ok(dir) = std::env::var("PUREVOX_MODEL_DIR") {
    candidates.push(PathBuf::from(dir).join(file));
  }
  // 打包后优先用 Tauri 注册的资源目录（Linux deb 落在 /usr/lib/<productName>/models，
  // 与可执行文件不同级，仅靠 exe 相邻目录找不到）。
  if let Some(dir) = MODEL_DIR.get() {
    candidates.push(dir.join(file));
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

#[cfg(test)]
mod tests {
  use super::*;
  use crate::plan::{ColumnSpec, Plan, RowKind, RowSpec};
  use std::collections::BTreeMap;

  fn row(kind: RowKind, ptype: &str, enabled: bool) -> RowSpec {
    RowSpec {
      kind,
      ptype: ptype.into(),
      device: None,
      enabled,
      params: BTreeMap::new(),
    }
  }

  fn rss_kb() -> u64 {
    // /proc/self/status 的 VmRSS（Linux）；其它平台回 0（这条探针只在 Linux 上跑）
    std::fs::read_to_string("/proc/self/status")
      .ok()
      .and_then(|t| {
        t.lines()
          .find(|l| l.starts_with("VmRSS:"))
          .and_then(|l| l.split_whitespace().nth(1).map(|v| v.parse().ok()))
          .flatten()
      })
      .unwrap_or(0)
  }

  /// 内存是否**真的**退回去了：把三个模型都借出来（求值一次）→ 归还 → 回收 → 看 RSS。
  /// 手动跑：`cargo test --release -- --ignored --nocapture memory_release`
  #[test]
  #[ignore = "真要加载 ONNX + 读 RSS，手动跑"]
  fn memory_release_probe() {
    let files = [MODEL_DENOISE, tse::MODEL_TSE, tse::MODEL_TSE_REF, aec::MODEL_AEC];
    let base = rss_kb();
    for f in files {
      let _ = f;
      let path = model_path(f).expect(f);
      let lease = checkout(&path).expect(f);
      // 真跑一次，别只加载（ORT 首次会分配 arena）
      let mut g = lease.lock();
      let shape: Vec<i64> = g
        .inputs()
        .iter()
        .find(|i| i.name() == "cache_in")
        .and_then(|i| match i.dtype() {
          ort::value::ValueType::Tensor { shape, .. } => Some(shape.to_vec()),
          _ => None,
        })
        .unwrap_or_default();
      drop(g);
      drop(lease);
      println!("  加载 {f}: RSS {} KB", rss_kb());
      let _ = shape;
    }
    let loaded = rss_kb();
    let freed = retain_models(&[]);
    let trimmed = rss_kb();
    println!(
      "  基线 {base} KB → 全加载 {loaded} KB（+{} KB）→ 回收 {freed} 个会话 {trimmed} KB（-{} KB）",
      loaded.saturating_sub(base),
      loaded.saturating_sub(trimmed)
    );
    assert!(trimmed < loaded, "回收之后 RSS 没降：{loaded} → {trimmed} KB");

    // 模拟一次**重建**：旧行还攥着自己在用的那份，新会话只能再加载一份 → 两两并存。
    retain_models(&[MODEL_DENOISE.to_string()]);
    let dn = model_path(MODEL_DENOISE).expect("denoise");
    let old_row = checkout(&dn).expect("denoise");
    let new_row = checkout(&dn).expect("denoise"); // 池子空 ⇒ 从磁盘再加载一份
    drop(old_row); // 旧会话（旧行 stop 之后才会归还）
    let dual = rss_kb();
    let n = retain_models(&[MODEL_DENOISE.to_string()]);
    let one = rss_kb();
    println!(
      "  重建瞬间（新旧两份并存）{dual} KB → 回收 {n} 个重复副本 {one} KB（-{} KB）",
      dual.saturating_sub(one)
    );
    drop(new_row);
    retain_models(&[]);

    // 再来两轮：残留在进程里的那部分是不是**一次性**的（ORT runtime / glibc arena），
    // 还是会随每次加载-回收越涨越高（真泄漏）。
    for round in 2..=3 {
      for f in files {
        let path = model_path(f).expect(f);
        drop(checkout(&path).expect(f));
      }
      let up = rss_kb();
      let n = retain_models(&[]);
      println!(
        "  第 {round} 轮：加载后 {up} KB → 回收 {n} 个会话 {s} KB",
        s = rss_kb()
      );
    }
  }

  /// 白名单口径：**删除才回收，停用/暂停不回收**（2026-10-31）。
  #[test]
  fn models_in_plan_keeps_paused_rows() {
    let off = Plan {
      columns: vec![ColumnSpec {
        enabled: true,
        rows: vec![
          row(RowKind::Input, "audio_input", true),
          row(RowKind::Process, "denoise", false),
          row(RowKind::Process, "gain", true),
          row(RowKind::Output, "audio_output", true),
        ],
      }],
    };
    assert!(
      models_in_plan(&off).contains(&MODEL_DENOISE.to_string()),
      "停用（还在计划里）的降噪行必须保留模型，暂停不动内存"
    );

    let on = Plan {
      columns: vec![ColumnSpec {
        enabled: true,
        rows: vec![
          row(RowKind::Input, "audio_input", true),
          row(RowKind::Process, "denoise", true),
          row(RowKind::Process, "tse", true),
          row(RowKind::Process, "echo_cancel", true),
          row(RowKind::Output, "audio_output", true),
        ],
      }],
    };
    let got = models_in_plan(&on);
    assert!(got.contains(&MODEL_DENOISE.to_string()), "{got:?}");
    assert!(got.contains(&tse::MODEL_TSE.to_string()), "{got:?}");
    assert!(got.contains(&tse::MODEL_TSE_REF.to_string()), "{got:?}");
    assert!(got.contains(&aec::MODEL_AEC.to_string()), "{got:?}");

    // 整列暂停：行还在 → 模型照样留着
    let col_off = Plan {
      columns: vec![ColumnSpec { enabled: false, rows: on.columns[0].rows.clone() }],
    };
    assert_eq!(models_in_plan(&col_off), got, "停用的列也不该回收模型");

    // 只有增益这类纯 DSP 行：一根 ONNX 都不涉及（增益不占模型内存，也不进白名单）
    let dsp_only = Plan {
      columns: vec![ColumnSpec {
        enabled: true,
        rows: vec![
          row(RowKind::Input, "audio_input", true),
          row(RowKind::Process, "gain", true),
          row(RowKind::Process, "eq31", true),
          row(RowKind::Output, "audio_output", true),
        ],
      }],
    };
    assert!(models_in_plan(&dsp_only).is_empty(), "纯 DSP 行不该出现任何模型");

    // 真删掉（计划里没有这些行）= 回收
    let deleted = Plan {
      columns: vec![ColumnSpec {
        enabled: true,
        rows: vec![row(RowKind::Input, "audio_input", true), row(RowKind::Output, "audio_output", true)],
      }],
    };
    assert!(models_in_plan(&deleted).is_empty(), "删掉模型行之后必须回收");
  }
}
