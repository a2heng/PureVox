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

//! 音乐播放器组件（ARCHITECTURE.md §4 处理行）：流式解码 + 首尾相连环流。
//!
//! 移植 legacy `MusicPlayerPlugin` 的语义：
//! - 后台解码线程 → 3 秒有界环形缓冲 → 音频线程每 hop（10 ms）取 480 样本，
//!   内存占用恒定（与文件时长无关）；
//! - 曲目视作首尾相连的环流：EOF 即回卷续播（必须循环，无循环选项）；
//! - 行启用复选框 / 暂停 = 硬旁路（本 hop 直通，缓冲与位置保留），勾回原地续播；
//! - 音量 ±10 dB；seek / 播放位置经 `resume_sec` 持久化。
//!
//! 音源唯一实现：库内 48 kHz 单声道 PCM16 WAV（`media::import` 归一保证），
//! 流式读（`WavStream`），运行时不需要任何转码。

use std::collections::VecDeque;
use std::fs::File;
use std::io::{BufReader, Read, Seek, SeekFrom};
use std::sync::atomic::{AtomicBool, Ordering::Relaxed};
use std::sync::{Arc, Condvar, Mutex};
use std::thread::JoinHandle;
use std::time::Duration;

use crate::audio::{HOP, SAMPLE_RATE};
use crate::config;
use crate::engine::media::{MediaCmd, MediaSlot};
use crate::engine::stage::{FrameContext, Stage, StageError};

/// ±10 dB（legacy 同款）。
const VOL_LO_DB: f64 = -10.0;
const VOL_HI_DB: f64 = 10.0;

/// 环形缓冲容量：3 s（48 kHz f32 = 576 KB），与 legacy 一致。
const RING_CAP: usize = SAMPLE_RATE as usize * 3;
/// 解码线程一次读入的帧数（10 ms）。
const BLOCK_FRAMES: usize = HOP;
/// 解码线程等待间隔（ring 满 / 无曲目时）。
const IDLE: Duration = Duration::from_millis(4);

fn vol_gain(db: f64) -> f32 {
  10f32.powf((db.clamp(VOL_LO_DB, VOL_HI_DB) / 20.0) as f32)
}

/// 流式 WAV 读取器：只支持库内格式（PCM16 / 单声道 / 48 kHz），其它明确报错。
struct WavStream {
  file: BufReader<File>,
  /// 剩余可读帧数
  remaining: u64,
}

impl WavStream {
  fn open(path: &std::path::Path, seek_frames: u64) -> Result<(Self, u64), String> {
    let mut file =
      BufReader::new(File::open(path).map_err(|e| format!("打开 {} 失败：{e}", path.display()))?);
    let (data_off, data_len, rate, ch, bits) = parse_header(&mut file)?;
    if rate != SAMPLE_RATE {
      return Err(format!(
        "{} 是 {rate} Hz（库内文件应为 48000 Hz，请重新导入）",
        path.display()
      ));
    }
    if ch != 1 || bits != 16 {
      return Err(format!(
        "{} 不是单声道 16-bit（ch={ch} bits={bits}，请重新导入）",
        path.display()
      ));
    }
    let total = data_len / 2; // 帧数（单声道 16-bit = 2 字节/帧）
    let seek_frames = seek_frames.min(total);
    file
      .seek(SeekFrom::Start(data_off + seek_frames * 2))
      .map_err(|e| format!("定位失败：{e}"))?;
    Ok((
      WavStream {
        file,
        remaining: total - seek_frames,
      },
      total,
    ))
  }

  /// 读一块（≤ `out.len()` 帧）为 f32；返回读到的帧数（0 = EOF）。
  fn read_block(&mut self, out: &mut [f32]) -> usize {
    let want = (out.len() as u64).min(self.remaining) as usize;
    if want == 0 {
      return 0;
    }
    let mut bytes = vec![0u8; want * 2];
    let mut got = 0usize;
    while got < bytes.len() {
      match self.file.read(&mut bytes[got..]) {
        Ok(0) => break,
        Ok(n) => got += n,
        Err(_) => break,
      }
    }
    let frames = got / 2;
    for i in 0..frames {
      out[i] = i16::from_le_bytes([bytes[2 * i], bytes[2 * i + 1]]) as f32 / 32768.0;
    }
    self.remaining -= frames as u64;
    frames
  }
}

/// 解析 RIFF/WAVE 头：返回 (data 偏移, data 长度, 采样率, 声道, 位深)。
fn parse_header(file: &mut BufReader<File>) -> Result<(u64, u64, u32, u16, u16), String> {
  let mut head = [0u8; 12];
  file
    .read_exact(&mut head)
    .map_err(|e| format!("读取 WAV 头失败：{e}"))?;
  if &head[0..4] != b"RIFF" || &head[8..12] != b"WAVE" {
    return Err("不是 WAV 文件（缺少 RIFF/WAVE 标识）".into());
  }
  let mut fmt: Option<(u32, u16, u16)> = None;
  let mut data: Option<(u64, u64)> = None;
  loop {
    let mut hdr = [0u8; 8];
    if file.read_exact(&mut hdr).is_err() {
      break;
    }
    let id = &hdr[0..4];
    let size = u32::from_le_bytes([hdr[4], hdr[5], hdr[6], hdr[7]]) as u64;
    let start = file.stream_position().unwrap_or(0);
    if id == b"fmt " && size >= 16 {
      let mut f = [0u8; 16];
      if file.read_exact(&mut f).is_ok() {
        let format = u16::from_le_bytes([f[0], f[1]]);
        let ch = u16::from_le_bytes([f[2], f[3]]);
        let rate = u32::from_le_bytes([f[4], f[5], f[6], f[7]]);
        let bits = u16::from_le_bytes([f[14], f[15]]);
        if format == 0xFFFE && size >= 40 {
          // WAVE_FORMAT_EXTENSIBLE：真实格式在子格式 GUID 前两字节
          let mut ext = [0u8; 24];
          if file.read_exact(&mut ext).is_ok() {
            let real = u16::from_le_bytes([ext[0], ext[1]]);
            if real == 1 {
              fmt = Some((rate, ch, bits));
            }
          }
        } else {
          fmt = Some((rate, ch, bits));
        }
      }
    } else if id == b"data" {
      data = Some((start, size));
    }
    let next = start + size + (size & 1);
    if file.seek(SeekFrom::Start(next)).is_err() {
      break;
    }
  }
  match (fmt, data) {
    (Some(f), Some(d)) => Ok((d.0, d.1, f.0, f.1, f.2)),
    _ => Err("WAV 缺少 fmt/data 块".into()),
  }
}

/// 解码线程与音频线程的共享状态。
struct Shared {
  /// 当前曲目路径（空 = 无曲目）
  path: Mutex<String>,
  /// 待定位（秒）；Some = 解码线程重开定位
  seek: Mutex<Option<f64>>,
  /// 曲目/定位变化标志（解码线程据此重开音源；避免持环锁再取 seek 锁的顺序反转）
  dirty: AtomicBool,
  /// 样本环（解码写、音频读）
  ring: Mutex<VecDeque<f32>>,
  cv: Condvar,
  stop: AtomicBool,
  /// 解码线程最近一次错误（进状态，不静默）
  error: Mutex<Option<String>>,
}

pub struct PlayerStage {
  slot: Arc<MediaSlot>,
  shared: Arc<Shared>,
  thread: Option<JoinHandle<()>>,
  path: String,
  volume: f32,
  vol_db: f64,
  paused: bool,
  /// 已消费样本（播放位置，音频线程推进）
  pos: u64,
  /// 曲目总帧数（0 = 未知）
  dur: u64,
}

impl PlayerStage {
  /// `path`：库内 WAV 路径（空 = 未选曲目）；`resume_sec`：断点续播位置。
  pub fn load(path: &str, vol_db: f64, resume_sec: f64, slot: Arc<MediaSlot>) -> Self {
    let shared = Arc::new(Shared {
      path: Mutex::new(String::new()),
      seek: Mutex::new(None),
      dirty: AtomicBool::new(false),
      ring: Mutex::new(VecDeque::new()),
      cv: Condvar::new(),
      stop: AtomicBool::new(false),
      error: Mutex::new(None),
    });
    let mut st = PlayerStage {
      slot,
      shared: shared.clone(),
      thread: None,
      path: String::new(),
      volume: vol_gain(vol_db),
      vol_db,
      paused: false,
      pos: 0,
      dur: 0,
    };
    if !path.trim().is_empty() {
      st.set_path(path);
    }
    if resume_sec > 0.0 {
      st.pos = (resume_sec * SAMPLE_RATE as f64) as u64;
      *st.shared.seek.lock().unwrap() = Some(resume_sec);
    }
    // 起线程前统一置位 dirty：换曲与续播定位都不会被解码线程漏掉
    st.shared.dirty.store(true, Relaxed);
    st.ensure_thread();
    st
  }

  fn ensure_thread(&mut self) {
    if self.thread.is_none() {
      let shared = self.shared.clone();
      let join = std::thread::Builder::new()
        .name("player-decode".into())
        .spawn(move || dec_loop(&shared))
        .expect("spawn player decode");
      self.thread = Some(join);
    }
  }

  /// 换曲（硬切换：位置归零、缓冲清空）；空路径 = 清曲。
  fn set_path(&mut self, path: &str) {
    self.path = path.trim().to_string();
    *self.shared.path.lock().unwrap() = self.path.clone();
    *self.shared.seek.lock().unwrap() = None;
    self.shared.dirty.store(true, Relaxed);
    self.shared.ring.lock().unwrap().clear();
    self.shared.cv.notify_all();
    self.pos = 0;
    self.dur = 0;
    *self.shared.error.lock().unwrap() = None;
  }

  fn drain_cmds(&mut self) {
    for cmd in self.slot.drain() {
      match cmd {
        MediaCmd::PlayerPlay => self.paused = false,
        MediaCmd::PlayerPause => self.paused = true,
        MediaCmd::PlayerSeek(sec) => {
          self.pos = (sec.max(0.0) * SAMPLE_RATE as f64) as u64;
          *self.shared.seek.lock().unwrap() = Some(sec.max(0.0));
          self.shared.dirty.store(true, Relaxed);
          self.shared.ring.lock().unwrap().clear();
          self.shared.cv.notify_all();
        }
        MediaCmd::Volume { db, .. } => {
          self.vol_db = db;
          self.volume = vol_gain(db);
        }
        _ => {}
      }
    }
  }

  fn publish_status(&self) {
    let err = self.shared.error.lock().unwrap().clone();
    let pos = if self.dur > 0 {
      (self.pos % self.dur) as f64 / SAMPLE_RATE as f64
    } else {
      self.pos as f64 / SAMPLE_RATE as f64
    };
    let playing = !self.path.is_empty() && !self.paused;
    self.slot.set_status(|s| {
      s.playing = playing;
      s.paused = self.paused;
      s.pos = pos;
      s.dur = self.dur as f64 / SAMPLE_RATE as f64;
      s.error = err;
    });
  }
}

/// 解码线程：流式读库内 WAV → 环缓冲；EOF 回卷@0；seek 重开定位。
///
/// 锁纪律：任何时刻最多持有一把锁（ring / seek / path 不嵌套）——`dirty` 是原子标志，
/// 「换曲/定位」经它触发重开，避免「持环锁取 seek 锁」与音频线程「持 seek 锁取环锁」的顺序反转。
fn dec_loop(shared: &Arc<Shared>) {
  let mut stream: Option<WavStream> = None;
  let mut path: String = String::new();
  let mut block = vec![0.0f32; BLOCK_FRAMES];
  loop {
    if shared.stop.load(Relaxed) {
      return;
    }
    // 1) 曲目/定位变化：重开音源
    if shared.dirty.swap(false, Relaxed) {
      stream = None;
      let seek = shared.seek.lock().unwrap().take();
      path = shared.path.lock().unwrap().clone();
      let from = seek.unwrap_or(0.0).max(0.0);
      if !path.is_empty() {
        match WavStream::open(
          std::path::Path::new(&path),
          (from * SAMPLE_RATE as f64) as u64,
        ) {
          Ok((s, _)) => {
            stream = Some(s);
            *shared.error.lock().unwrap() = None;
          }
          Err(e) => *shared.error.lock().unwrap() = Some(e),
        }
      }
    }
    // 2) 无曲目：等待（换曲/seek/stop 都会置 dirty 或唤醒）
    if path.is_empty() {
      let g = shared.path.lock().unwrap();
      let _ = shared.cv.wait_timeout(g, IDLE).unwrap();
      continue;
    }
    if stream.is_none() {
      // 打开失败：退避重试（原因已进 error）
      let g = shared.path.lock().unwrap();
      let _ = shared
        .cv
        .wait_timeout(g, Duration::from_millis(200))
        .unwrap();
      continue;
    }
    // 3) 读一块；EOF = 回卷重开（环流）
    let n = stream.as_mut().unwrap().read_block(&mut block);
    if n == 0 {
      stream = None;
      shared.dirty.store(true, Relaxed); // 回卷@0（seek=None）
      continue;
    }
    // 4) 入环：满了就等（不持锁轮询 dirty；音频线程取走会 notify）
    loop {
      let full = {
        let ring = shared.ring.lock().unwrap();
        ring.len() + n > RING_CAP
      };
      if !full {
        let mut ring = shared.ring.lock().unwrap();
        if ring.len() + n <= RING_CAP {
          ring.extend(block[..n].iter().copied());
          break;
        }
      }
      if shared.stop.load(Relaxed) {
        return;
      }
      if shared.dirty.load(Relaxed) {
        break; // 换曲/seek：丢弃本块，回步骤 1
      }
      let ring = shared.ring.lock().unwrap();
      let _ = shared
        .cv
        .wait_timeout(ring, Duration::from_millis(50))
        .unwrap();
    }
    shared.cv.notify_all();
  }
}

impl Stage for PlayerStage {
  fn name(&self) -> &'static str {
    "player"
  }

  fn process(&mut self, frame: &mut [f32], _ctx: &FrameContext) -> Result<(), StageError> {
    self.drain_cmds();
    // 无曲目 / 暂停 = 硬旁路（缓冲与位置保留）
    if self.path.is_empty() || self.paused {
      self.publish_status();
      return Ok(());
    }
    if self.dur == 0 {
      // 曲头：从文件取一次总帧数（用于进度显示与回卷）
      let p = config::expand_path(&self.path);
      if let Ok((_, total)) = WavStream::open(&p, 0) {
        self.dur = total;
      }
    }
    let got = {
      let mut ring = self.shared.ring.lock().unwrap();
      let take = ring.len().min(frame.len().min(HOP));
      for f in frame.iter_mut().take(take) {
        *f += ring.pop_front().unwrap() * self.volume;
      }
      take
    };
    if got > 0 {
      self.shared.cv.notify_all();
      self.pos += got as u64;
      if self.dur > 0 && self.pos >= self.dur {
        self.pos -= self.dur; // 与解码面回卷同步
      }
    }
    self.publish_status();
    Ok(())
  }

  fn reset(&mut self) {
    self.pos = 0;
    *self.shared.seek.lock().unwrap() = Some(0.0);
    self.shared.dirty.store(true, Relaxed);
    self.shared.ring.lock().unwrap().clear();
    self.shared.cv.notify_all();
  }

  fn status(&self) -> Option<String> {
    let pos = self.pos as f64 / SAMPLE_RATE as f64;
    let dur = self.dur as f64 / SAMPLE_RATE as f64;
    let state = if self.path.is_empty() {
      "未选曲目"
    } else if self.paused {
      "已暂停"
    } else {
      "播放中"
    };
    Some(format!(
      "{state} {}/{} s，音量 {:+.0} dB",
      pos.round() as i64,
      dur.round() as i64,
      self.vol_db
    ))
  }
}

impl Drop for PlayerStage {
  fn drop(&mut self) {
    self.shared.stop.store(true, Relaxed);
    self.shared.cv.notify_all();
    if let Some(j) = self.thread.take() {
      let _ = j.join();
    }
  }
}

#[cfg(test)]
mod tests {
  use super::*;
  use crate::engine::media::MediaHub;

  fn slot() -> Arc<MediaSlot> {
    let h = MediaHub::default();
    let g = h.begin();
    h.register(g, 9, 9, "player")
  }

  fn write_lib_wav(name: &str, secs: f32, hz: f32) -> String {
    let dir = std::env::temp_dir().join("pv_player_test");
    std::fs::create_dir_all(&dir).unwrap();
    let p = dir.join(name);
    let n = (SAMPLE_RATE as f32 * secs) as usize;
    let samples: Vec<f32> = (0..n)
      .map(|i| 0.5 * (2.0 * std::f32::consts::PI * hz * i as f32 / SAMPLE_RATE as f32).sin())
      .collect();
    crate::wav::write_mono_16(&p, &samples, SAMPLE_RATE).unwrap();
    p.to_str().unwrap().to_string()
  }

  #[test]
  fn streams_and_loops() {
    let path = write_lib_wav("loop-0.3s.wav", 0.3, 440.0);
    let mut st = PlayerStage::load(&path, 0.0, 0.0, slot());
    // 等解码线程预热
    let mut frame = [0.0f32; HOP];
    let mut got_any = false;
    for _ in 0..150 {
      st.process(&mut frame, &FrameContext::default()).unwrap();
      if frame.iter().any(|&x| x != 0.0) {
        got_any = true;
        break;
      }
      std::thread::sleep(Duration::from_millis(10));
    }
    assert!(got_any, "应能从环形缓冲取到样本");
    let s = st.slot.status();
    assert!(s.playing);
    assert!((s.dur - 0.3).abs() < 0.05, "时长应≈0.3 s：{}", s.dur);
    let _ = std::fs::remove_file(&path);
  }

  #[test]
  fn pause_freezes_position() {
    let path = write_lib_wav("pause-0.3s.wav", 0.3, 440.0);
    let mut st = PlayerStage::load(&path, 0.0, 0.0, slot());
    let mut frame = [0.0f32; HOP];
    for _ in 0..50 {
      st.process(&mut frame, &FrameContext::default()).unwrap();
    }
    st.slot.send(MediaCmd::PlayerPause);
    st.process(&mut frame, &FrameContext::default()).unwrap();
    let p1 = st.pos;
    for _ in 0..20 {
      st.process(&mut frame, &FrameContext::default()).unwrap();
    }
    assert_eq!(st.pos, p1, "暂停后位置必须冻结");
    assert!(st.slot.status().paused);
    let _ = std::fs::remove_file(&path);
  }

  #[test]
  fn seek_restarts_position() {
    let path = write_lib_wav("seek-0.3s.wav", 0.3, 440.0);
    let mut st = PlayerStage::load(&path, 0.0, 0.0, slot());
    let mut frame = [0.0f32; HOP];
    for _ in 0..30 {
      st.process(&mut frame, &FrameContext::default()).unwrap();
    }
    st.slot.send(MediaCmd::PlayerSeek(0.15));
    st.process(&mut frame, &FrameContext::default()).unwrap();
    assert_eq!(st.pos, (0.15 * SAMPLE_RATE as f64) as u64);
    let _ = std::fs::remove_file(&path);
  }

  #[test]
  fn missing_file_reports_error() {
    let mut st = PlayerStage::load("/nonexistent/pv_missing.wav", 0.0, 0.0, slot());
    let mut frame = [0.0f32; HOP];
    // 解码在后台线程开文件，30 个 hop 是一瞬间的事、赶不上它报错 → 轮询等
    // （上限 2 s），别用「跑 N 帧就应当看到」这种依赖调度的断言。
    let deadline = std::time::Instant::now() + Duration::from_secs(2);
    loop {
      st.process(&mut frame, &FrameContext::default()).unwrap();
      if st.slot.status().error.is_some() {
        break;
      }
      assert!(
        std::time::Instant::now() < deadline,
        "缺文件必须在 2 s 内报出原因，实际状态：{:?}",
        st.slot.status()
      );
      std::thread::sleep(Duration::from_millis(20));
    }
    assert!(frame.iter().all(|&x| x == 0.0), "报错期间不得混入任何音频");
  }

  #[test]
  fn no_path_passthrough() {
    let mut st = PlayerStage::load("", 0.0, 0.0, slot());
    let mut frame = [0.25f32; HOP];
    st.process(&mut frame, &FrameContext::default()).unwrap();
    assert!(frame.iter().all(|&x| x == 0.25));
    assert!(!st.slot.status().playing);
  }
}
