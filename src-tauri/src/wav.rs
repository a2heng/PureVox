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

//! 最小 WAV 读写：PCM16 / IEEE float32，多声道下混单声道。
//! TSE 参考录音（10 s 48 kHz 单声道）用；不做重采样（非 48k 由调用方报错）。

use std::path::Path;

/// 读取 WAV → (单声道样本, 采样率)。支持 PCM16 与 float32（含 WAVE_FORMAT_EXTENSIBLE）。
pub fn read_mono(path: &Path) -> Result<(Vec<f32>, u32), String> {
  let data = std::fs::read(path).map_err(|e| format!("读取 {} 失败：{e}", path.display()))?;
  if data.len() < 12 || &data[0..4] != b"RIFF" || &data[8..12] != b"WAVE" {
    return Err("不是 WAV 文件（缺少 RIFF/WAVE 标识）".into());
  }

  let mut format = 0u16;
  let mut channels = 0u16;
  let mut rate = 0u32;
  let mut bits = 0u16;
  let mut body: Option<(usize, usize)> = None; // (offset, len)

  let mut pos = 12usize;
  while pos + 8 <= data.len() {
    let id = &data[pos..pos + 4];
    let size = u32::from_le_bytes(data[pos + 4..pos + 8].try_into().unwrap()) as usize;
    let start = pos + 8;
    let end = (start + size).min(data.len());
    if id == b"fmt " && size >= 16 {
      format = u16::from_le_bytes([data[start], data[start + 1]]);
      channels = u16::from_le_bytes([data[start + 2], data[start + 3]]);
      rate = u32::from_le_bytes(data[start + 4..start + 8].try_into().unwrap());
      bits = u16::from_le_bytes([data[start + 14], data[start + 15]]);
      // WAVE_FORMAT_EXTENSIBLE：真实格式在子格式 GUID 前两字节
      if format == 0xFFFE && size >= 40 {
        format = u16::from_le_bytes([data[start + 24], data[start + 25]]);
      }
    } else if id == b"data" {
      body = Some((start, end - start));
    }
    pos = end + (size & 1); // 块按偶数字节对齐
  }

  let (off, len) = body.ok_or("WAV 缺少 data 块")?;
  if channels == 0 {
    return Err("WAV 声道数为 0".into());
  }
  let ch = channels as usize;
  let raw = &data[off..off + len];

  let mono: Vec<f32> = match (format, bits) {
    (1, 16) => raw
      .chunks_exact(2 * ch)
      .map(|frame| {
        let mut s = 0.0f32;
        for c in 0..ch {
          let v = i16::from_le_bytes([frame[2 * c], frame[2 * c + 1]]);
          s += v as f32 / 32768.0;
        }
        s / ch as f32
      })
      .collect(),
    (3, 32) => raw
      .chunks_exact(4 * ch)
      .map(|frame| {
        let mut s = 0.0f32;
        for c in 0..ch {
          s += f32::from_le_bytes(frame[4 * c..4 * c + 4].try_into().unwrap());
        }
        s / ch as f32
      })
      .collect(),
    (f, b) => return Err(format!("不支持的 WAV 格式（format={f} bits={b}），仅支持 PCM16 / float32")),
  };
  Ok((mono, rate))
}

/// 写 16-bit PCM 单声道 WAV。
pub fn write_mono_16(path: &Path, samples: &[f32], rate: u32) -> Result<(), String> {
  let n = samples.len() as u32;
  let data_len = n * 2;
  let mut out = Vec::with_capacity(44 + data_len as usize);
  out.extend_from_slice(b"RIFF");
  out.extend_from_slice(&(36 + data_len).to_le_bytes());
  out.extend_from_slice(b"WAVE");
  out.extend_from_slice(b"fmt ");
  out.extend_from_slice(&16u32.to_le_bytes());
  out.extend_from_slice(&1u16.to_le_bytes()); // PCM
  out.extend_from_slice(&1u16.to_le_bytes()); // mono
  out.extend_from_slice(&rate.to_le_bytes());
  out.extend_from_slice(&(rate * 2).to_le_bytes()); // byte rate
  out.extend_from_slice(&2u16.to_le_bytes()); // block align
  out.extend_from_slice(&16u16.to_le_bytes()); // bits
  out.extend_from_slice(b"data");
  out.extend_from_slice(&data_len.to_le_bytes());
  for &s in samples {
    let v = (s.clamp(-1.0, 1.0) * 32767.0) as i16;
    out.extend_from_slice(&v.to_le_bytes());
  }
  std::fs::write(path, out).map_err(|e| format!("写入 {} 失败：{e}", path.display()))
}
