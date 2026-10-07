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

//! 信号源扇出：一个源（输入采集 / 测试音）把 48 kHz hop 分发给多个订阅者（输出流）。
//! 只在工作线程之间使用（不在实时回调里），加锁可接受。

use std::sync::Mutex;
use std::sync::atomic::{AtomicBool, Ordering};

use rtrb::{Consumer, Producer, RingBuffer};

use super::SAMPLE_RATE;

/// 每个订阅者的 48 kHz 环形缓冲容量：1 s（输出侧封顶逻辑远小于此）。
const SUBSCRIBER_CAPACITY: usize = SAMPLE_RATE as usize;

pub struct Fanout {
  name: String,
  subscribers: Mutex<Vec<(String, Producer<f32>)>>,
  closed: AtomicBool,
}

impl Fanout {
  pub fn new(name: impl Into<String>) -> Self {
    Fanout {
      name: name.into(),
      subscribers: Mutex::new(Vec::new()),
      closed: AtomicBool::new(false),
    }
  }

  /// 源已停止（不会再有数据）。
  pub fn close(&self) {
    self.closed.store(true, Ordering::SeqCst);
  }

  pub fn is_closed(&self) -> bool {
    self.closed.load(Ordering::SeqCst)
  }

  /// 源的显示名（输出流调试信息里展示）。
  pub fn name(&self) -> &str {
    &self.name
  }

  /// 订阅：返回本订阅者的 48 kHz 读端。同一 id 重复订阅会替换旧的。
  pub fn subscribe(&self, id: &str) -> Consumer<f32> {
    let (prod, cons) = RingBuffer::<f32>::new(SUBSCRIBER_CAPACITY);
    let mut subs = self.subscribers.lock().unwrap();
    subs.retain(|(sid, _)| sid != id);
    subs.push((id.to_string(), prod));
    cons
  }

  pub fn unsubscribe(&self, id: &str) {
    self
      .subscribers
      .lock()
      .unwrap()
      .retain(|(sid, _)| sid != id);
  }

  /// 分发一个 hop；订阅者缓冲满（其工作线程停摆）时丢弃该订阅者这一 hop 的多余部分。
  pub fn push_hop(&self, hop: &[f32]) {
    let mut subs = self.subscribers.lock().unwrap();
    // 读端已被丢弃（输出流已停止）的订阅者顺手清掉
    subs.retain(|(_, p)| !p.is_abandoned());
    for (_, prod) in subs.iter_mut() {
      for &x in hop {
        if prod.push(x).is_err() {
          break;
        }
      }
    }
  }
}
