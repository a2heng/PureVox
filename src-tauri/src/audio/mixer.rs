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

//! 输出设备的混音（DESIGN.md §6.5）：**N 个源 → 一个设备**。
//!
//! 设备侧（打开设备 / 唯一主时钟 / 设备环 / 输出流的伺服与丢弃）全部交给既有的
//! [`super::playback`]：混音器把「多源相加后的 48k hop 流」推进一条 [`Fanout`]，`playback`
//! 像订阅任何源一样订阅它。于是**单源路径一字未改**，多源只是在这一层相加。
//!
//! 每个源一条独立链路（**不是**把多路直接写进同一条环 —— 那样各路会互相错位，环也会按源数
//! 成倍上涨）：
//!
//! ```text
//! 行 push_hop → 源扇出（1 s，满则丢最旧）→ Converter（48k→48k，比例可调）
//!             → 累计器（VecDeque，吸收小数样本）→ 逐 hop 相加 → 混音扇出 → playback
//! ```
//!
//! **为什么每源都要一路可调比例重采样**：各源的时钟（各自的采集设备 / 列节拍）相对设备时钟各有
//! ppm 级偏差。若直接相加，某个源会一路涨或一路空，最后只能靠「丢最旧 / 补静音」收拾 —— 那就是
//! 把时钟差变成听得见的周期性咔哒。这里每源一个 [`Converter`]（比例由**累计器水位**的 PI 伺服
//! 给，限幅 ±3%），把偏差在**小数样本**层面吸收掉；源扇出本身按「目标 + 硬顶、超限丢最旧」管，
//! 丢的量与累计器补零都逐源计数进 `/debug`（谁在丢、谁没喂上，一眼可见）。

use std::collections::VecDeque;
use std::sync::atomic::{AtomicBool, AtomicU32, AtomicU64, Ordering::Relaxed};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use rtrb::Consumer;

use super::fanout::Fanout;
use super::playback::Servo;
use super::{HOP, SAMPLE_RATE};
use crate::dsp::resampler::Converter;

/// 一拍（与列工作线程同一套 10 ms 网格）。
const GRID: Duration = Duration::from_millis(10);
/// 每个源的环目标水位（ms）：源时钟与设备时钟的粗对齐点。
/// 三格：吸收列线程与混音线程的相位差（两边都是 10 ms 定时器，谁先谁后不固定），
/// 又不会把延迟攒起来。
const SRC_TARGET_MS: usize = 30;
/// 每源环硬顶（ms）：超过丢最旧到目标 —— 源停摆 / 某列卡住时不让水位无限涨。
const SRC_CAP_MS: usize = 100;
/// 累计器目标水位（ms）：转换器比例要维持的小数级余量（约一格，够吸收一次调度抖动）。
const ACC_TARGET_MS: f64 = 10.0;
/// 累计器硬顶（样本）：超过丢最旧（同样是「过时的样本」）。
const ACC_CAP: usize = SAMPLE_RATE as usize / 1000 * 300;

fn ms_to_samples(ms: usize) -> usize {
  SAMPLE_RATE as usize / 1000 * ms
}

/// 一个源的即时统计（逐源进 `/debug`）。
#[derive(Default)]
pub struct SourceStats {
  /// 累计器不够一格、补零的**样本数**（源停摆或供不上）
  pub starved: AtomicU64,
  /// 源环超硬顶丢掉的最旧样本数
  pub dropped: AtomicU64,
  /// 这一路参与相加的峰值（dBFS，缓慢回落）—— 一眼看出有没有在出声
  peak_dbfs: AtomicU32,
}

impl SourceStats {
  fn peak(&self, db: f32) {
    let cur = f32::from_bits(self.peak_dbfs.load(Relaxed));
    self
      .peak_dbfs
      .store(if db > cur { db } else { cur - 0.2 }.to_bits(), Relaxed);
  }

  fn peak_dbfs(&self) -> f32 {
    f32::from_bits(self.peak_dbfs.load(Relaxed))
  }
}

/// 一路源接进混音的句柄（行持有它推 hop）；**Drop 即注销**。
pub struct MixPort {
  /// 行把这一列的 hop 推进它（`playback` 那套 `push_hop` 接口不变）
  pub fanout: Arc<Fanout>,
  /// 注销用的**唯一令牌**（不是 `id`！见 [`Mixer::register`] 的注释）
  token: u64,
  mixer: Arc<MixerInner>,
}

impl Drop for MixPort {
  fn drop(&mut self) {
    self.mixer.unregister(self.token);
  }
}

/// 逐源统计（进 `/debug`）。
#[derive(Clone, Debug, serde::Serialize)]
pub struct SourceStat {
  pub id: String,
  /// 源环水位（ms）：这一路攒了多少 = 相对设备时钟的富余
  pub level_ms: f32,
  /// 累计器补零的样本数（这一路没喂上）
  pub starved: u64,
  /// 源环丢最旧的样本数
  pub dropped: u64,
  pub peak_dbfs: f32,
}

/// 混音内部状态（`MixPort` 与混音线程共享）。
struct MixerInner {
  sources: Mutex<Vec<Source>>,
  stop: AtomicBool,
  /// 每次 [`Mixer::register`] 递增的一次性令牌：**注销按令牌、不按 id**。
  ///
  /// 2026-10-26 实测的哑巴故障：会话重建是「先建新会话、再停旧会话」，两次注册用的是
  /// 同一个 id（`c{列}r{行}`，行号不变），而旧的 `MixPort` 在 Drop 时按 id 注销 ——
  /// 它把**新注册的那一路**一起删掉了，于是混音器从此 0 个源、输出恒为 -160 dBFS
  /// （界面全绿、波形是直线，重启才恢复）。令牌让「谁的句柄谁注销」，新旧重叠期也安全。
  next_token: AtomicU64,
}

impl MixerInner {
  fn unregister(&self, token: u64) {
    if let Ok(mut g) = self.sources.lock() {
      g.retain(|s| s.token != token);
    }
  }
}

/// 一路源的运行状态（混音线程独占）。
struct Source {
  token: u64,
  id: String,
  ring: Consumer<f32>,
  conv: Converter,
  acc: VecDeque<f32>,
  /// 内环：累计器水位 → 转换器比例（保持约一格小数余量）
  servo: Servo,
  /// 小数格预算（Bresenham）：让「偶尔吃两格」平滑发生，而不是 2× 猛拉
  pull_budget: f64,
  stats: Arc<SourceStats>,
  last_ratio: Instant,
}

impl Source {
  /// 一拍：源环 → 转换器 → 累计器 → 相加进 `mixed`。
  fn pull(&mut self, mixed: &mut [f32; HOP], dt_s: f64) {
    // ① 源环硬顶：超过就丢最旧到目标（过时样本不慢慢放出去 —— DESIGN.md §8.3）
    let cap = ms_to_samples(SRC_CAP_MS);
    let target = ms_to_samples(SRC_TARGET_MS);
    let lv = self.ring.slots();
    if lv > cap {
      let drop = lv - target;
      if let Ok(c) = self.ring.read_chunk(drop) {
        c.commit_all();
        self.stats.dropped.fetch_add(drop as u64, Relaxed);
      }
    } else if lv > target + 2 * HOP {
      // ② 中等积压（> 目标 + 2 格）：跳到目标附近，但只按整 hop 丢，保持相位。
      let drop = (lv - target) / HOP * HOP;
      if drop > 0
        && let Ok(c) = self.ring.read_chunk(drop)
      {
        c.commit_all();
        self.stats.dropped.fetch_add(drop as u64, Relaxed);
      }
    }

    // ③ 外环：源环水位 → 每拍吃几格（1.0 = 实时；只在 ±3% 内微调）。
    //    **不是**「超过目标就 2× 猛拉」：那会把积压搬进累计器，延迟换了个地方存。
    //    带 1 格死区：列线程与混音线程都是 10 ms 定时器，相位不固定，水位在
    //    「目标 ± 1 格」内是正常抖动，不许因此把速率压到 0.97 造成周期欠载。
    let level_ms = self.ring.slots() as f64 * 1000.0 / SAMPLE_RATE as f64;
    let err_ms = level_ms - SRC_TARGET_MS as f64;
    let corr = if err_ms.abs() <= 10.0 {
      0.0
    } else {
      (err_ms / 100.0).clamp(-0.03, 0.03)
    };
    self.pull_budget = (self.pull_budget + 1.0 + corr).clamp(0.0, 2.5);
    let mut hops = self.pull_budget.floor() as usize;
    if hops > 2 {
      hops = 2;
    }
    self.pull_budget -= hops as f64;
    for _ in 0..hops {
      if self.ring.slots() < HOP {
        // 这一拍源没喂上：不喂转换器（零输入只是空转），累计器自然被抽空 → 下一拍补零
        break;
      }
      let mut hop = [0.0f32; HOP];
      if let Ok(c) = self.ring.read_chunk(HOP) {
        let (a, b) = c.as_slices();
        hop[..a.len()].copy_from_slice(a);
        hop[a.len()..a.len() + b.len()].copy_from_slice(b);
        c.commit_all();
      }
      if let Ok(out) = self.conv.process(&hop) {
        self.acc.extend(out.iter().copied());
      }
    }
    if self.acc.len() > ACC_CAP {
      let n = self.acc.len() - ms_to_samples(ACC_TARGET_MS as usize);
      self.acc.drain(..n);
      self.stats.dropped.fetch_add(n as u64, Relaxed);
    }
    // 相加：缺的补零 = 这一路静音，不拖慢别的源
    let mut peak = 0.0f32;
    let mut starved = 0u64;
    for slot in mixed.iter_mut() {
      if let Some(v) = self.acc.pop_front() {
        *slot += v;
        let a = v.abs();
        if a > peak {
          peak = a;
        }
      } else {
        starved += 1;
      }
    }
    if starved > 0 {
      self.stats.starved.fetch_add(starved, Relaxed);
    }
    let db = if peak > 1e-9 {
      20.0 * peak.log10()
    } else {
      -120.0
    };
    self.stats.peak(db);
    // ④ 内环：按**累计器**水位调这一路的比例（累计器才是小数样本的积分器）
    let acc_ms = self.acc.len() as f64 * 1000.0 / SAMPLE_RATE as f64;
    self.servo.update(acc_ms, dt_s.max(1e-4));
    if self.last_ratio.elapsed() >= Duration::from_millis(50) {
      self.last_ratio = Instant::now();
      let _ = self.conv.set_relative_ratio(1.0 - self.servo.adj);
    }
  }
}

/// 一个输出设备的混音（由设备池按引用计数持有）。
pub struct Mixer {
  inner: Arc<MixerInner>,
  join: Option<std::thread::JoinHandle<()>>,
}

impl Mixer {
  /// 起一个混音线程：每 10 ms 把「多源相加」的一个 hop 推进 `out`（`playback` 订阅它）。
  pub fn spawn(out: Arc<Fanout>, tag: String) -> Result<Mixer, String> {
    let inner = Arc::new(MixerInner {
      sources: Mutex::new(Vec::new()),
      stop: AtomicBool::new(false),
      next_token: AtomicU64::new(1),
    });
    let (i2, out2) = (inner.clone(), out.clone());
    let join = std::thread::Builder::new()
      .name(format!("mix-{tag}"))
      .spawn(move || {
        let token = crate::debug::threads::ThreadToken::new(format!("mix:{tag}"));
        let mut mixed = [0.0f32; HOP];
        let mut next = Instant::now();
        let mut last = Instant::now();
        while !i2.stop.load(Relaxed) {
          token.beat();
          // 绝对时刻对齐 10 ms 网格（AGENTS：别用 `sleep(10ms - 已用)`，那只会跑到 ~64 拍/s）
          next += GRID;
          let now = Instant::now();
          if next > now {
            std::thread::sleep(next - now);
          } else if now.duration_since(next) > GRID * 4 {
            next = now + GRID;
          }
          let dt = now.duration_since(last).as_secs_f64().max(1e-4);
          last = now;
          mixed.fill(0.0);
          if let Ok(mut g) = i2.sources.lock() {
            for s in g.iter_mut() {
              s.pull(&mut mixed, dt);
            }
          }
          out2.push_hop(&mixed);
        }
      })
      .map_err(|e| format!("起混音线程失败：{e}"))?;
    Ok(Mixer {
      inner,
      join: Some(join),
    })
  }

  /// 登记一路源：返回给行推 hop 的扇出（行照旧 `push_hop`）。
  pub fn register(&self, id: String) -> MixPort {
    let token = self.inner.next_token.fetch_add(1, Relaxed);
    let fanout = Arc::new(Fanout::new(format!("混音源：{id}")));
    let ring = fanout.subscribe(&id);
    let conv = Converter::new(SAMPLE_RATE, SAMPLE_RATE, HOP, true)
      .expect("48k→48k 转换器（内部参数固定，不该失败）");
    if let Ok(mut g) = self.inner.sources.lock() {
      g.push(Source {
        token,
        id: id.clone(),
        ring,
        conv,
        acc: VecDeque::with_capacity(4096),
        servo: Servo::new(ACC_TARGET_MS),
        pull_budget: 1.0,
        stats: Arc::new(SourceStats::default()),
        last_ratio: Instant::now(),
      });
    }
    MixPort {
      fanout,
      token,
      mixer: self.inner.clone(),
    }
  }

  /// 逐源统计（进 `/debug`）。
  pub fn stats(&self) -> Vec<SourceStat> {
    let Ok(g) = self.inner.sources.lock() else {
      return Vec::new();
    };
    g.iter()
      .map(|s| SourceStat {
        id: s.id.clone(),
        level_ms: s.ring.slots() as f32 * 1000.0 / SAMPLE_RATE as f32,
        starved: s.stats.starved.load(Relaxed),
        dropped: s.stats.dropped.load(Relaxed),
        peak_dbfs: s.stats.peak_dbfs(),
      })
      .collect()
  }
}

impl Drop for Mixer {
  fn drop(&mut self) {
    self.inner.stop.store(true, Relaxed);
    if let Some(j) = self.join.take() {
      let _ = j.join();
    }
  }
}

#[cfg(test)]
mod tests {
  use super::*;

  /// 一路「像真行那样每拍推一格」的源：环 + 混音源句柄。
  struct Fixture {
    fan: Arc<Fanout>,
    src: Source,
  }

  fn fixture(id: &str) -> Fixture {
    let fan = Arc::new(Fanout::new(format!("测试源 {id}")));
    let ring = fan.subscribe(id);
    Fixture {
      fan,
      src: Source {
        token: 0,
        id: id.into(),
        ring,
        conv: Converter::new(SAMPLE_RATE, SAMPLE_RATE, HOP, true).unwrap(),
        acc: VecDeque::new(),
        servo: Servo::new(ACC_TARGET_MS),
        pull_budget: 1.0,
        stats: Arc::new(SourceStats::default()),
        last_ratio: Instant::now(),
      },
    }
  }

  /// 两路源相加：各自恒定 0.25 → 混音输出 0.5（**相加，不平均、不限幅**）。
  /// 这条钉的是「多列混到一个输出设备」的语义：同一信号两路 = +6 dB，不许被悄悄平均掉。
  #[test]
  fn two_sources_sum() {
    let mut a = fixture("a");
    let mut b = fixture("b");
    let mut out = [0.0f32; HOP];
    let mut settled = 0.0f32;
    for _ in 0..40 {
      a.fan.push_hop(&[0.25; HOP]);
      b.fan.push_hop(&[0.25; HOP]);
      out.fill(0.0);
      a.src.pull(&mut out, 0.01);
      b.src.pull(&mut out, 0.01);
      settled = out[HOP / 2];
    }
    assert!(
      (settled - 0.5).abs() < 0.02,
      "两路 0.25 相加应当约等于 0.5，实测 {settled}"
    );
  }

  /// 源停摆（环空）：这一路补零、别的路照旧 —— 一个源的欠载不许拖慢混音。
  #[test]
  fn stalled_source_does_not_block_others() {
    let mut live = fixture("live");
    let mut dead = fixture("dead"); // 一直不推：模拟这一路停摆
    let mut out = [0.0f32; HOP];
    for _ in 0..40 {
      live.fan.push_hop(&[0.5; HOP]);
      out.fill(0.0);
      live.src.pull(&mut out, 0.01);
      dead.src.pull(&mut out, 0.01);
    }
    assert!(
      (out[HOP / 2] - 0.5).abs() < 0.02,
      "活着的那一路照常出声，实测 {}",
      out[HOP / 2]
    );
    assert!(
      dead.src.stats.starved.load(Relaxed) > 0,
      "停摆的那一路要计数（逐源可见）"
    );
  }

  /// 源环的实时纪律：**推一格吃一格**时水位不涨（延迟不累积）；停推即抽干并计数。
  #[test]
  fn ring_does_not_accumulate_when_paced() {
    let mut a = fixture("a");
    let mut out = [0.0f32; HOP];
    for _ in 0..200 {
      a.fan.push_hop(&[0.1; HOP]);
      out.fill(0.0);
      a.src.pull(&mut out, 0.01);
    }
    let lv_ms = a.src.ring.slots() as f32 * 1000.0 / SAMPLE_RATE as f32;
    assert!(
      lv_ms <= SRC_TARGET_MS as f32 + 12.0,
      "每拍推一格就不该攒水位，实测 {lv_ms} ms"
    );
  }
  /// 会话重建的顺序是「**先建新会话、再停旧会话**」，两边的源用的是同一个 id
  /// （`c{列}r{行}`，行号不变）。旧句柄 Drop 时若按 id 注销，会把新注册的那一路一起
  /// 删掉 —— 混音器 0 源、输出恒为 -160 dBFS（2026-10-26 实测的哑巴故障）。
  #[test]
  fn rebuild_keeps_new_source() {
    let out = Arc::new(Fanout::new("测试混音输出"));
    let mixer = Mixer::spawn(out, "test".into()).unwrap();
    let old = mixer.register("c0r4".into());
    let new = mixer.register("c0r4".into());
    assert_eq!(mixer.stats().len(), 2, "重建重叠期两路都在（旧的还没停）");
    drop(old);
    let stats = mixer.stats();
    assert_eq!(stats.len(), 1, "旧句柄注销后，新的那一路必须还在");
    drop(new);
    assert_eq!(mixer.stats().len(), 0);
  }
}
