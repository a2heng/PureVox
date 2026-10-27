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

//! 媒体控制中枢（ARCHITECTURE.md §4）：音效板 / 播放器的**运行时命令**与状态快照。
//!
//! 播放/停止/seek/音量不是计划参数（改它们不重建会话）：界面经 Tauri 命令把
//! [`MediaCmd`] 投递到按 (列, 行) 寻址的 [`MediaSlot`]，由列工作线程里的 Stage
//! 在自己的 `process` 内 drain 执行（音频面单线程，无锁竞争）。
//! 与 [`crate::net::NetHub`] 同款全局单例；会话构建时注册、停止时清空。

use std::collections::{HashMap, VecDeque};
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex, OnceLock};

/// 媒体行的运行时命令（界面 → Stage）。serde 派生供 Tauri 命令反序列化。
#[derive(Clone, Debug, serde::Deserialize)]
pub enum MediaCmd {
  /// 音效板：播放第 `index` 个音效（重播 = 旧声部淡出、新声部从头起播）
  SoundpadPlay(usize),
  /// 音效板：停止第 `index` 个音效（淡出）
  SoundpadStop(usize),
  /// 音效板：全部停止（淡出）
  SoundpadStopAll,
  /// 播放器：播放 / 续播
  PlayerPlay,
  /// 播放器：暂停（位置与缓冲保留）
  PlayerPause,
  /// 播放器：定位到 `sec` 秒
  PlayerSeek(f64),
  /// 行内增益（dB）。音效板：`index` = 音效序号（实时更新在播声部）；
  /// 播放器：`index` 忽略（整行一个音量）。
  Volume { index: Option<usize>, db: f64 },
  /// 行启用开关变化（列工作线程在 `enabled` 翻转时转发）：音效板淡出在播声部。
  Enabled(bool),
}

/// 一个音效的运行状态（供界面显示「在播」）。
#[derive(Clone, Debug, serde::Serialize)]
pub struct PadStatus {
  pub name: String,
  pub playing: bool,
  pub volume_db: f64,
}

/// 媒体行的状态快照（界面 2 Hz 轮询 `media_status`）。
#[derive(Clone, Debug, serde::Serialize)]
pub struct MediaStatus {
  /// `soundpad` / `player`
  pub kind: &'static str,
  /// 播放器：是否正在播放（未暂停）
  pub playing: bool,
  pub paused: bool,
  /// 播放位置（秒）
  pub pos: f64,
  /// 曲目时长（秒；未知为 0）
  pub dur: f64,
  /// 行级故障原因（文件缺失/格式不符等）；None = 正常
  pub error: Option<String>,
  /// 音效板：各音效状态
  pub pads: Vec<PadStatus>,
}

impl MediaStatus {
  pub fn new(kind: &'static str) -> Self {
    MediaStatus {
      kind,
      playing: false,
      paused: false,
      pos: 0.0,
      dur: 0.0,
      error: None,
      pads: Vec::new(),
    }
  }
}

/// 一个媒体行的槽位：Stage 持有它（drain 命令、更新状态），界面经中枢投递/读取。
pub struct MediaSlot {
  ctl: Mutex<VecDeque<MediaCmd>>,
  status: Mutex<MediaStatus>,
}

impl MediaSlot {
  pub fn new(kind: &'static str) -> Self {
    MediaSlot {
      ctl: Mutex::new(VecDeque::new()),
      status: Mutex::new(MediaStatus::new(kind)),
    }
  }

  /// 界面侧：投递一条命令（不阻塞音频线程）。
  pub fn send(&self, cmd: MediaCmd) {
    self.ctl.lock().unwrap().push_back(cmd);
  }

  /// 音频侧：取走当前全部命令。
  pub fn drain(&self) -> Vec<MediaCmd> {
    std::mem::take(&mut *self.ctl.lock().unwrap())
      .into_iter()
      .collect()
  }

  /// 音频侧：更新状态快照。
  pub fn set_status(&self, f: impl FnOnce(&mut MediaStatus)) {
    f(&mut self.status.lock().unwrap());
  }

  /// 界面侧：读取状态快照。
  pub fn status(&self) -> MediaStatus {
    self.status.lock().unwrap().clone()
  }
}

/// 全局媒体中枢：(列, 行) → 槽位。
///
/// **代次（generation）**：`Session::build` 先 `begin()` 拿一个新代次，注册时带上；
/// 构建成功才 `commit(gen)`（清掉旧代次），失败/回滚则 `abort(gen)`（只删自己注册的）。
/// 这样「先建后停」时旧会话的 `Session::stop` 不会把新会话的槽位清掉
/// （从前 `stop` 里的 `clear()` 正是这个 bug：重建后音效板/播放器失灵）。
/// (代次, 槽位)：同一 (列,行) 可能同时存在旧会话与新构建的槽位。
type SlotEntry = (u64, Arc<MediaSlot>);
type SlotMap = HashMap<(usize, usize), Vec<SlotEntry>>;

pub struct MediaHub {
  /// 每个 (列,行) 保留**多代**槽位：构建中的新槽位与旧会话的槽位并存。
  /// `slot()` 取最新代（构建期间界面已能操作新行），回滚删掉新代后旧槽位还在。
  slots: Mutex<SlotMap>,
  generation: AtomicU64,
}

impl Default for MediaHub {
  fn default() -> Self {
    MediaHub {
      slots: Mutex::new(HashMap::new()),
      generation: AtomicU64::new(0),
    }
  }
}

impl MediaHub {
  /// 开始一次会话构建：返回新代次。
  pub fn begin(&self) -> u64 {
    self.generation.fetch_add(1, Ordering::Relaxed) + 1
  }

  /// 会话构建时注册。同一代重复注册覆盖；不同代并存（构建中/回滚可见）。
  pub fn register(
    &self,
    generation: u64,
    col: usize,
    row: usize,
    kind: &'static str,
  ) -> Arc<MediaSlot> {
    let slot = Arc::new(MediaSlot::new(kind));
    let mut g = self.slots.lock().unwrap_or_else(|e| e.into_inner());
    let list = g.entry((col, row)).or_default();
    list.retain(|(g, _)| *g != generation);
    list.push((generation, slot.clone()));
    list.sort_by_key(|(g, _)| *g);
    slot
  }

  /// 提交一代：清掉所有更旧的槽位（旧会话的媒体行被删除/换位置时不会留下僵尸）。
  pub fn commit(&self, generation: u64) {
    let mut g = self.slots.lock().unwrap_or_else(|e| e.into_inner());
    g.retain(|_, list| {
      list.retain(|(g, _)| *g >= generation);
      !list.is_empty()
    });
  }

  /// 放弃一代（构建失败 / 回滚）：只删这一代注册的槽位，旧会话的槽位保持可用。
  pub fn abort(&self, generation: u64) {
    let mut g = self.slots.lock().unwrap_or_else(|e| e.into_inner());
    g.retain(|_, list| {
      list.retain(|(g, _)| *g != generation);
      !list.is_empty()
    });
  }

  /// 硬清空（引擎停止）。
  pub fn clear(&self) {
    self.slots.lock().unwrap_or_else(|e| e.into_inner()).clear();
  }

  /// 界面侧按 (列,行) 取槽位：取**最新代**（构建中也能操作新行）；未注册返回 None。
  pub fn slot(&self, col: usize, row: usize) -> Option<Arc<MediaSlot>> {
    self
      .slots
      .lock()
      .unwrap_or_else(|e| e.into_inner())
      .get(&(col, row))
      .and_then(|list| list.last().map(|(_, s)| s.clone()))
  }
}

static HUB: OnceLock<Arc<MediaHub>> = OnceLock::new();

/// 全局媒体中枢（与测试音 / 网络中枢同款单例）。
pub fn hub() -> Arc<MediaHub> {
  HUB.get_or_init(|| Arc::new(MediaHub::default())).clone()
}

#[cfg(test)]
mod tests {
  use super::*;

  #[test]
  fn register_send_drain_roundtrip() {
    let h = MediaHub::default();
    let g1 = h.begin();
    let slot = h.register(g1, 1, 2, "soundpad");
    assert!(h.slot(1, 2).is_some());
    assert!(h.slot(0, 0).is_none());
    slot.send(MediaCmd::SoundpadPlay(0));
    slot.send(MediaCmd::SoundpadStopAll);
    let cmds = slot.drain();
    assert_eq!(cmds.len(), 2);
    assert!(slot.drain().is_empty(), "drain 后应取空");
    h.commit(g1);
    assert!(h.slot(1, 2).is_some(), "提交后本代槽位应保留");
    h.clear();
    assert!(h.slot(1, 2).is_none());
  }

  /// 重建时的代次语义：新会话注册后、提交前，旧会话的槽位仍然可用；
  /// 旧会话 `stop()` 不再清空新会话（旧实现正是这里把槽位清掉）。
  #[test]
  fn generation_commit_keeps_new_slots_and_abort_keeps_old() {
    let h = MediaHub::default();
    let old = h.begin();
    let _old_slot = h.register(old, 0, 1, "soundpad");
    h.commit(old);

    // 新会话构建中：旧槽位必须仍然可用
    let new = h.begin();
    let _new_slot = h.register(new, 0, 1, "soundpad");
    assert!(h.slot(0, 1).is_some(), "构建期间旧槽位不该消失");
    h.commit(new);
    assert!(h.slot(0, 1).is_some(), "提交后新槽位仍在");

    // 回滚：只删自己这一代，旧会话的槽位保留
    let failed = h.begin();
    let _ = h.register(failed, 1, 0, "player");
    h.abort(failed);
    assert!(h.slot(1, 0).is_none(), "回滚必须删掉失败的一代");
    assert!(h.slot(0, 1).is_some(), "回滚不该影响已提交的会话");

    // 同一 (列,行) 构建新行后回滚：旧槽位必须回来（否则媒体控制会永久失灵）
    let failed2 = h.begin();
    let _ = h.register(failed2, 0, 1, "soundpad");
    h.abort(failed2);
    assert!(h.slot(0, 1).is_some(), "同键回滚后旧会话的槽位必须仍然可用");
  }

  #[test]
  fn status_roundtrip() {
    let slot = MediaSlot::new("player");
    slot.set_status(|s| {
      s.playing = true;
      s.pos = 12.5;
      s.dur = 60.0;
    });
    let st = slot.status();
    assert_eq!(st.kind, "player");
    assert!(st.playing);
    assert_eq!(st.pos, 12.5);
  }
}
