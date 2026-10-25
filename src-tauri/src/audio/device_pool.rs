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

//! 设备池（DESIGN.md §6.5）：**每个 `(设备 ID, 角色)` 只打开一次**，按引用计数持有。
//!
//! 从前「打开设备」长在**逻辑行**里（输入行 `capture::spawn`、输出行 `playback::spawn`），
//! 于是逻辑结构与硬件独占绑死，三条后果都是实测过的：① 同一 `(设备, 方向)` 出现在两列/两行 →
//! Linux 上第二次打开报 busy；②「一个输入设备喂多列」「多列混到一个输出设备」做不到；
//! ③ 重建要关掉全部设备再重开，「加一列」也会打断别的列（实测单次重建 7.25 s）。
//!
//! 现在：设备由**这一层**持有（引用计数归零才关），逻辑行只拿一个**租约**：
//! - 输入行 → [`InputLease::subscribe`] 拿到该设备扇出的一个订阅端（`Fanout` 本来就是
//!   「一个源、N 个订阅者」，多列订阅同一个采集流天然成立）；
//! - 输出行 → [`OutputLease`]（内含该设备**唯一**那条混音的源扇出，见 [`super::mixer`]）。
//!
//! 于是「同一设备被多列用」不再报 busy（第二次是订阅/混音，不是打开），**会话重建也不再关设备**
//! —— 重建从「全量重启」变成「差异重启」（引用没归零的设备原样留着，`started_at` 都不变）。
//!
//! 纪律（原来散在 `AudioManager::release_session` 的注释里，现在收口在这一层）：
//! - 打开动作**不持锁**（设备可能几秒才打开）；
//! - 真关是**同步**的（`WorkerHandle::stop` 会 join 工作线程，cpal 的 `snd_pcm_close` 在它自己的
//!   线程上）—— 所以引用归零后**当拍就关完了**，下一个 `acquire` 不会撞上「上一次还没关」；
//! - 角色只有**输入 / 输出**两种键：同一张全双工声卡同时做输入与输出**是合法的**（两个 PCM）。

use std::collections::HashMap;
use std::sync::{Arc, Mutex, OnceLock};

use super::WorkerHandle;
use super::capture::Capture;
use super::fanout::Fanout;
use super::mixer::{MixPort, Mixer, SourceStat};
use super::playback;

struct InputEntry {
  cap: Capture,
  refs: usize,
}

struct OutputEntry {
  playback: WorkerHandle,
  mixer: Mixer,
  refs: usize,
}

/// 设备池（进程级单例，与 `net::hub()` / `media::hub()` 同款）。
///
/// 内部用 `Arc<Inner>`：**租约要记住自己属于哪个池**（Drop 时把引用还回同一个池，而不是
/// 想当然调全局单例）—— 单测用的是自己的池，这条不能靠假设。
pub struct Pool {
  inner: Arc<Inner>,
}

struct Inner {
  inputs: Mutex<HashMap<String, InputEntry>>,
  outputs: Mutex<HashMap<String, OutputEntry>>,
}

impl Default for Pool {
  fn default() -> Self {
    Pool::new()
  }
}

impl Pool {
  pub fn new() -> Self {
    Pool {
      inner: Arc::new(Inner {
        inputs: Mutex::new(HashMap::new()),
        outputs: Mutex::new(HashMap::new()),
      }),
    }
  }
}

static HUB: OnceLock<Pool> = OnceLock::new();

pub fn hub() -> &'static Pool {
  HUB.get_or_init(Pool::new)
}

/// 输入租约（行持有）：持有期间设备保证开着；**Drop 即放掉这一份引用**。
pub struct InputLease {
  device: String,
  fanout: Arc<Fanout>,
  pool: Arc<Inner>,
}

impl InputLease {
  /// 订阅这一路（同一设备的多个订阅者各拿一条订阅端）。
  pub fn subscribe(&self, id: &str) -> rtrb::Consumer<f32> {
    self.fanout.subscribe(id)
  }
}

impl Drop for InputLease {
  fn drop(&mut self) {
    release_input(&self.pool, &self.device);
  }
}

/// 输出租约（行持有）：`push_hop` 这一列的 hop；**Drop 即注销这一路源 + 放掉引用**。
pub struct OutputLease {
  device: String,
  /// 行推 hop 用的扇出（与 [`MixPort`] 同一条）
  pub out: Arc<Fanout>,
  /// 这一路源的句柄：**只为它的 Drop 而持有**（Drop 即从混音注销）
  _port: MixPort,
  pool: Arc<Inner>,
}

impl Drop for OutputLease {
  fn drop(&mut self) {
    // _port 先 Drop（注销源），再放引用
    release_output(&self.pool, &self.device);
  }
}

/// 引用 -1，归零才真关（**真关是同步的**：`WorkerHandle::stop` 会 join 工作线程，
/// cpal 的 `snd_pcm_close` 在它自己的线程上）。
fn release_input(inner: &Inner, device: &str) {
  let mut g = inner.inputs.lock().unwrap();
  let Some(e) = g.get_mut(device) else {
    return;
  };
  e.refs = e.refs.saturating_sub(1);
  if e.refs > 0 {
    return;
  }
  if let Some(InputEntry { cap, .. }) = g.remove(device) {
    cap.handle.stop();
  }
}

fn release_output(inner: &Inner, device: &str) {
  let mut g = inner.outputs.lock().unwrap();
  let Some(e) = g.get_mut(device) else {
    return;
  };
  e.refs = e.refs.saturating_sub(1);
  if e.refs > 0 {
    return;
  }
  if let Some(OutputEntry {
    playback, mixer, ..
  }) = g.remove(device)
  {
    drop(mixer);
    playback.stop();
  }
}

/// 某设备在池里的即时状态（进 `/debug/devices`）。
#[derive(Clone, Debug, Default, serde::Serialize)]
pub struct PoolStat {
  /// 有几个逻辑路在用（0 = 池没持有）
  pub refs: usize,
  /// 输出设备：逐源（这一路有没有在出声、有没有在丢）
  pub sources: Vec<SourceStat>,
  /// 输入设备：逐订阅者（哪一列没在取数据、丢了多少）
  pub subscribers: Vec<super::fanout::FanoutStat>,
}

impl Pool {
  /// 取输入设备：命中就 `refs += 1` 复用（**不碰设备**）；未命中才真打开。
  pub fn acquire_input(
    &self,
    hub: crate::debug::SharedHub,
    device_id: &str,
  ) -> Result<InputLease, String> {
    let tag = tag_for(device_id);
    self.acquire_input_with(device_id, |id| {
      super::capture::spawn(hub.clone(), id.to_string(), tag)
    })
  }

  /// 同上，但打开动作可注入（单测传真源 —— 不开真设备就能钉住「只开一次 + 引用计数」）。
  pub fn acquire_input_with<F>(&self, device_id: &str, open: F) -> Result<InputLease, String>
  where
    F: FnOnce(&str) -> Result<Capture, String>,
  {
    if let Some(e) = self.inner.inputs.lock().unwrap().get_mut(device_id) {
      e.refs += 1;
      return Ok(InputLease {
        device: device_id.to_string(),
        fanout: e.cap.fanout.clone(),
        pool: self.inner.clone(),
      });
    }
    // 打开可能几秒（`with_device_retry`），**不持锁**
    let cap = open(device_id)?;
    let fanout = cap.fanout.clone();
    let mut g = self.inner.inputs.lock().unwrap();
    if let Some(e) = g.get_mut(device_id) {
      // 竞态：这中间别人先开好了 —— 用别人的，把刚开的收掉
      e.refs += 1;
      let Capture { handle, .. } = cap;
      handle.stop();
    } else {
      g.insert(device_id.to_string(), InputEntry { cap, refs: 1 });
    }
    Ok(InputLease {
      device: device_id.to_string(),
      fanout,
      pool: self.inner.clone(),
    })
  }

  /// 取输出设备：命中就 `refs += 1` 并复用那条混音（**不碰设备**）。
  pub fn acquire_output(
    &self,
    hub: crate::debug::SharedHub,
    device_id: &str,
  ) -> Result<OutputLease, String> {
    let tag = tag_for(device_id);
    self.acquire_output_with(device_id, |id, mix_out| {
      let mixer = Mixer::spawn(mix_out.clone(), tag.clone())?;
      let playback = playback::spawn(hub.clone(), id.to_string(), mix_out, tag.clone())?;
      Ok((playback, mixer))
    })
  }

  /// 同上，但打开动作可注入（单测用假 playback + 真混音线程）。
  pub fn acquire_output_with<F>(&self, device_id: &str, open: F) -> Result<OutputLease, String>
  where
    F: FnOnce(&str, Arc<Fanout>) -> Result<(WorkerHandle, Mixer), String>,
  {
    if let Some(e) = self.inner.outputs.lock().unwrap().get_mut(device_id) {
      e.refs += 1;
      let port = e.mixer.register(format!("{device_id}#{}", e.refs));
      return Ok(OutputLease {
        device: device_id.to_string(),
        out: port.fanout.clone(),
        _port: port,
        pool: self.inner.clone(),
      });
    }
    // 混音产出 → 混音扇出 → playback（playback 像订阅任何源一样订阅它）
    let mix_out = Arc::new(Fanout::new(format!("混音：{device_id}")));
    let (playback, mixer) = open(device_id, mix_out)?;
    let port = mixer.register(format!("{device_id}#1"));
    let mut g = self.inner.outputs.lock().unwrap();
    match g.get_mut(device_id) {
      Some(e) => {
        // 竞态：别人先开好了 —— 用别人的，把刚开的收掉
        e.refs += 1;
        drop(mixer);
        playback.stop();
        let port = e.mixer.register(format!("{device_id}#{}", e.refs));
        Ok(OutputLease {
          device: device_id.to_string(),
          out: port.fanout.clone(),
          _port: port,
          pool: self.inner.clone(),
        })
      }
      None => {
        g.insert(
          device_id.to_string(),
          OutputEntry {
            playback,
            mixer,
            refs: 1,
          },
        );
        Ok(OutputLease {
          device: device_id.to_string(),
          out: port.fanout.clone(),
          _port: port,
          pool: self.inner.clone(),
        })
      }
    }
  }

  /// 某设备的池状态（`/debug/devices` 的 `refs` / 逐源）。
  pub fn stat(&self, device_id: &str) -> PoolStat {
    if let Some(e) = self.inner.inputs.lock().unwrap().get(device_id) {
      return PoolStat {
        refs: e.refs,
        sources: Vec::new(),
        subscribers: e.cap.fanout.stats(),
      };
    }
    if let Some(e) = self.inner.outputs.lock().unwrap().get(device_id) {
      return PoolStat {
        refs: e.refs,
        sources: e.mixer.stats(),
        subscribers: Vec::new(),
      };
    }
    PoolStat::default()
  }
}

/// 流标识随**设备**走（不再带列/行）：设备层跨重建存活、流与列已经解耦 ——
/// `input:c0r0` 这种带列行的 id 会在「加一列」之后指着一列不存在的行。
fn tag_for(device_id: &str) -> String {
  device_id.to_string()
}

#[cfg(test)]
mod tests {
  use super::*;
  use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
  use std::time::Duration;

  /// 假采集：一个扇出 + 一个「停过了」的句柄 + 关流计数。
  fn fake_capture(opened: Arc<AtomicUsize>, closed: Arc<AtomicUsize>) -> Capture {
    opened.fetch_add(1, Ordering::Relaxed);
    let stop = Arc::new(AtomicBool::new(false));
    let fanout = Arc::new(Fanout::new("假采集".to_string()));
    let s2 = closed.clone();
    let stop2 = stop.clone();
    let join = std::thread::Builder::new()
      .spawn(move || {
        while !stop2.load(Ordering::Relaxed) {
          std::thread::sleep(Duration::from_millis(2));
        }
        s2.fetch_add(1, Ordering::Relaxed);
      })
      .unwrap();
    Capture {
      handle: WorkerHandle::new(stop, join),
      fanout,
    }
  }

  /// **同一设备只打开一次**：两次 `acquire` → 一次打开、`refs = 2`；归零才关。
  /// 这条就是「一个输入设备喂多列」的地基（从前第二次打开在 Linux 上直接 busy）。
  #[test]
  fn one_open_per_device_refcounted() {
    let pool = Pool::new();
    let (opened, closed) = (Arc::new(AtomicUsize::new(0)), Arc::new(AtomicUsize::new(0)));
    let (o1, c1) = (opened.clone(), closed.clone());
    let a = match pool.acquire_input_with("dev-A", move |_: &str| -> Result<Capture, String> {
      Ok(fake_capture(o1, c1))
    }) {
      Ok(v) => v,
      Err(e) => panic!("第一次就该成功：{e}"),
    };
    let (o2, c2) = (opened.clone(), closed.clone());
    let b = match pool.acquire_input_with("dev-A", move |_: &str| -> Result<Capture, String> {
      Ok(fake_capture(o2, c2))
    }) {
      Ok(v) => v,
      Err(e) => panic!("第二次应当复用：{e}"),
    };
    assert_eq!(opened.load(Ordering::Relaxed), 1, "只许打开一次");
    assert_eq!(pool.stat("dev-A").refs, 2, "两个逻辑路在用");
    // 两条订阅都能拿到（扇出本来就多订阅者）
    let _s1 = a.subscribe("col1row0");
    let _s2 = b.subscribe("col2row0");
    drop(a);
    assert_eq!(closed.load(Ordering::Relaxed), 0, "还有人用就不许关");
    assert_eq!(pool.stat("dev-A").refs, 1);
    drop(b);
    assert_eq!(closed.load(Ordering::Relaxed), 1, "归零必须关掉");
    assert_eq!(pool.stat("dev-A").refs, 0, "关掉后池里不该再持有");
  }

  /// 打开失败要**带上原因**（不静默），并且不留下半条记录。
  #[test]
  fn open_failure_reports_reason() {
    let pool = Pool::new();
    let e = match pool.acquire_input_with("dev-busy", |_: &str| -> Result<Capture, String> {
      Err("设备忙：另一个进程在用".to_string())
    }) {
      Ok(_) => panic!("打不开就必须报错，不能静默成功"),
      Err(e) => e,
    };
    assert!(e.contains("设备忙"), "实际：{e}");
    assert_eq!(pool.stat("dev-busy").refs, 0, "失败不该留下池记录");
  }

  /// 不同设备 / 不同角色各自成键（同一张全双工声卡做输入又做输出是合法的）。
  #[test]
  fn different_devices_are_separate_keys() {
    let pool = Pool::new();
    let (a, b) = (Arc::new(AtomicUsize::new(0)), Arc::new(AtomicUsize::new(0)));
    let (a1, b1) = (a.clone(), b.clone());
    let _x = match pool.acquire_input_with("dev-A", move |_: &str| -> Result<Capture, String> {
      Ok(fake_capture(a1, b1))
    }) {
      Ok(v) => v,
      Err(e) => panic!("{e}"),
    };
    let (a2, b2) = (a.clone(), b.clone());
    let _y = match pool.acquire_input_with("dev-B", move |_: &str| -> Result<Capture, String> {
      Ok(fake_capture(a2, b2))
    }) {
      Ok(v) => v,
      Err(e) => panic!("{e}"),
    };
    assert_eq!(a.load(Ordering::Relaxed), 2, "两台设备各开一次");
    assert_eq!(pool.stat("dev-A").refs, 1);
    assert_eq!(pool.stat("dev-B").refs, 1);
  }

  /// 输出侧：同一设备的多路源登记在同一条混音里（逐源可见），引用归零才关。
  #[test]
  fn output_shares_one_mix_per_device() {
    let pool = Pool::new();
    let streams = Arc::new(AtomicUsize::new(0));
    let closer = Arc::new(AtomicUsize::new(0));
    let (s1, c1) = (streams.clone(), closer.clone());
    let a = pool
      .acquire_output_with(
        "dev-O",
        move |_id: &str, out: Arc<Fanout>| -> Result<(WorkerHandle, Mixer), String> {
          s1.fetch_add(1, Ordering::Relaxed);
          let stop = Arc::new(AtomicBool::new(false));
          let c = c1.clone();
          let st = stop.clone();
          let join = std::thread::Builder::new()
            .spawn(move || {
              while !st.load(Ordering::Relaxed) {
                std::thread::sleep(Duration::from_millis(2));
              }
              c.fetch_add(1, Ordering::Relaxed);
            })
            .unwrap();
          Ok((
            WorkerHandle::new(stop, join),
            Mixer::spawn(out, "测试混音".into()).expect("混音线程"),
          ))
        },
      )
      .expect("第一路");
    a.out.push_hop(&[0.1; super::super::HOP]);
    let (s2, c2) = (streams.clone(), closer.clone());
    let b = pool
      .acquire_output_with(
        "dev-O",
        move |_id: &str, out: Arc<Fanout>| -> Result<(WorkerHandle, Mixer), String> {
          s2.fetch_add(1, Ordering::Relaxed);
          let stop = Arc::new(AtomicBool::new(false));
          let c = c2.clone();
          let st = stop.clone();
          let join = std::thread::Builder::new()
            .spawn(move || {
              while !st.load(Ordering::Relaxed) {
                std::thread::sleep(Duration::from_millis(2));
              }
              c.fetch_add(1, Ordering::Relaxed);
            })
            .unwrap();
          Ok((
            WorkerHandle::new(stop, join),
            Mixer::spawn(out, "测试混音".into()).expect("混音线程"),
          ))
        },
      )
      .expect("第二路应当复用同一条混音");
    assert_eq!(streams.load(Ordering::Relaxed), 1, "一个设备只许一条输出流");
    let st = pool.stat("dev-O");
    assert_eq!(st.refs, 2);
    assert_eq!(
      st.sources.len(),
      2,
      "两路源都登记在同一条混音里（逐源可见）"
    );
    drop(a);
    assert_eq!(closer.load(Ordering::Relaxed), 0, "还有一路在用");
    drop(b);
    assert_eq!(closer.load(Ordering::Relaxed), 1, "归零才关");
  }
}
