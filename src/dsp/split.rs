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

//! 交织样本 → 逐通道拆分（采集热路径）。
//!
//! 采集环是**交织**的（设备原生格式），每个通道要单独重采样。10 通道 192 kHz
//! 设备每秒要过 192 万个样本，**每样本一个分支的写法（`if active[phase]`）在
//! 10 通道只用 2 路时是纯浪费**：实测 5 ms 块从 6.5 µs 降到 0.8 µs。
//!
//! 做法：先把「环回绕」切出来的第二段对齐到帧边界，然后按通道
//! `iter().step_by(channels)` 整段 `extend`（一次预留 + 顺序写），
//! 没人订阅的通道**一次都不碰**。
//!
//! 只在采集工作线程里用（不是音频回调）；返回值无，目标缓冲由调用方复用。

/// 把 `parts`（`rtrb` 读块的连续两段，可能在帧中间回绕）按通道拆进 `scratch`。
///
/// - `channels`：设备原生通道数（≥ 1）
/// - `active[i] == false` 的通道不写（采集端只重采样有订阅者的通道）
/// - `scratch` 长度必须等于 `channels`；每个缓冲**只追加**，调用方负责 `clear`
///
/// 帧边界跨 `parts` 时按连续流处理（第一段末尾的半帧接第二段开头）。
pub fn split_active(
  parts: [&[f32]; 2],
  channels: usize,
  active: &[bool],
  scratch: &mut [Vec<f32>],
) {
  debug_assert!(channels >= 1);
  debug_assert_eq!(active.len(), channels);
  debug_assert_eq!(scratch.len(), channels);
  let mut phase = 0usize;
  for part in parts {
    let mut i = 0usize;
    while i < part.len() {
      if phase == 0 {
        let frames = (part.len() - i) / channels;
        if frames > 0 {
          let body = &part[i..i + frames * channels];
          for (c, on) in active.iter().enumerate() {
            if *on {
              // 一次预留（StepBy 是 ExactSizeIterator），顺序写
              scratch[c].extend(body[c..].iter().step_by(channels).take(frames));
            }
          }
          i += frames * channels;
          continue;
        }
      }
      // 帧中间/不足一帧的尾巴：最多 `channels - 1` 个样本
      if active[phase] {
        scratch[phase].push(part[i]);
      }
      i += 1;
      phase += 1;
      if phase == channels {
        phase = 0;
      }
    }
  }
}

#[cfg(test)]
mod tests {
  use super::*;

  /// 朴素参考实现（旧写法），用于对拍。
  fn naive(parts: [&[f32]; 2], channels: usize, active: &[bool]) -> Vec<Vec<f32>> {
    let mut out = vec![Vec::new(); channels];
    let mut phase = 0usize;
    for part in parts {
      for &v in part {
        if active[phase] {
          out[phase].push(v);
        }
        phase = (phase + 1) % channels;
      }
    }
    out
  }

  /// 任意回绕点（含**帧中间**回绕）都必须与朴素实现一致。
  #[test]
  fn any_wrap_point_matches_naive() {
    for channels in 1..=10usize {
      let n = 7 * channels + 3;
      let data: Vec<f32> = (0..n).map(|i| i as f32).collect();
      let active: Vec<bool> = (0..channels).map(|c| c % 3 == 0).collect();
      for cut in 0..=n {
        let mut scratch = vec![Vec::new(); channels];
        split_active(
          [&data[..cut], &data[cut..]],
          channels,
          &active,
          &mut scratch,
        );
        let want = naive([&data[..cut], &data[cut..]], channels, &active);
        assert_eq!(scratch, want, "channels={channels} cut={cut}");
      }
    }
  }

  /// 没订阅的通道一个样本都不写；订阅的通道按帧号顺序完整。
  #[test]
  fn inactive_channels_untouched() {
    let data: Vec<f32> = (0..40).map(|i| i as f32).collect();
    let active = [true, false, true, false];
    let mut scratch = vec![Vec::new(); 4];
    split_active([&data[..13], &data[13..]], 4, &active, &mut scratch);
    assert!(scratch[1].is_empty() && scratch[3].is_empty());
    assert_eq!(
      scratch[0],
      (0..10).map(|k| (k * 4) as f32).collect::<Vec<_>>()
    );
    assert_eq!(
      scratch[2],
      (0..10).map(|k| (k * 4 + 2) as f32).collect::<Vec<_>>()
    );
  }
}
