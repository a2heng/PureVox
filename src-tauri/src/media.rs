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

//! 媒体库（ARCHITECTURE.md §4）：音效板 / 音乐播放器的文件**导入**与库管理。
//!
//! **入库唯一实现** [`import`]：任意来源文件（界面文件选择给出的字节）一律
//! symphonia 解码 → 下混单声道 → 重采样 48 kHz → 写 16-bit WAV 落库，
//! 配置只引用库内路径（不依赖外部原始文件）。同内容幂等复用（内容哈希进文件名）。
//!
//! 与 legacy `audio_decode.import_media` 同语义；运行时解码不再需要任何转码：
//! 音效板整段读库内 WAV（`wav::read_mono`），播放器流式读同一格式。

use std::collections::hash_map::DefaultHasher;
use std::hash::Hasher;
use std::path::{Path, PathBuf};

use symphonia::core::audio::{AudioBufferRef, Signal, SignalSpec};
use symphonia::core::codecs::DecoderOptions;
use symphonia::core::conv::FromSample;
use symphonia::core::errors::Error as SymphoniaError;
use symphonia::core::formats::FormatOptions;
use symphonia::core::io::MediaSourceStream;
use symphonia::core::meta::MetadataOptions;
use symphonia::core::probe::Hint;
use symphonia::default::{get_codecs, get_probe};

use crate::config;

/// 引擎内部采样率（库内文件恒定 48 kHz 单声道）。
const LIB_RATE: u32 = crate::audio::SAMPLE_RATE;

/// 媒体库种类：音效（整段入内存）与曲目（流式）。
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum MediaKind {
  Soundpad,
  Music,
}

impl MediaKind {
  pub fn dir_name(self) -> &'static str {
    match self {
      MediaKind::Soundpad => "soundpad",
      MediaKind::Music => "music",
    }
  }

  /// 库目录（`~/.purevox/soundpad` / `~/.purevox/music`）。
  pub fn dir(self) -> PathBuf {
    config::config_dir()
      .map(|d| d.join(self.dir_name()))
      .unwrap_or_else(|_| PathBuf::from(self.dir_name()))
  }
}

/// 导入结果（库内路径 + 展示名）。
#[derive(Clone, Debug)]
pub struct Imported {
  pub path: PathBuf,
  pub name: String,
  /// true = 库内已有同一内容（未重复解码/落盘）
  pub reused: bool,
}

/// 导入：解码 `bytes`（原始文件内容）→ 归一 → 落库，返回库内路径。
///
/// `orig_name` 只用于取名与格式提示（扩展名），不读取外部文件。
pub fn import(bytes: &[u8], orig_name: &str, kind: MediaKind) -> Result<Imported, String> {
  let stem = safe_stem(
    Path::new(orig_name)
      .file_stem()
      .and_then(|s| s.to_str())
      .unwrap_or("audio"),
  );
  let dir = kind.dir();
  std::fs::create_dir_all(&dir).map_err(|e| format!("创建媒体库目录失败：{e}"))?;
  let out = dir.join(format!("{stem}-{:016x}.wav", content_hash(bytes)));
  if out.exists() {
    return Ok(Imported {
      path: out,
      name: stem,
      reused: true,
    });
  }
  let samples = decode_to_mono_48k(bytes, orig_name)?;
  if samples.is_empty() {
    return Err("解码结果为空（文件可能没有可播放的音频）".into());
  }
  // 先写临时文件再替换：中途失败不留半个库文件
  let tmp = out.with_extension("wav.part");
  crate::wav::write_mono_16(&tmp, &samples, LIB_RATE)?;
  std::fs::rename(&tmp, &out).map_err(|e| {
    let _ = std::fs::remove_file(&tmp);
    format!("写入媒体库失败：{e}")
  })?;
  Ok(Imported {
    path: out,
    name: stem,
    reused: false,
  })
}

/// 该路径是否位于媒体库内（soundpad / music 目录之下）。
pub fn is_library_path(path: &str) -> bool {
  let p = Path::new(path);
  [MediaKind::Soundpad, MediaKind::Music]
    .iter()
    .any(|k| p.starts_with(k.dir()))
}

/// 移除库内文件。**双重护栏**（调用方已保证「仍被计划引用则不删」，这里再兜底）：
/// 只删媒体库内的文件，外部路径永不碰。
pub fn discard(path: &str) -> Result<(), String> {
  let p = Path::new(path);
  if !is_library_path(path) {
    return Err("只允许删除媒体库内的文件".into());
  }
  if !p.is_file() {
    return Ok(()); // 已不在（重复移除幂等）
  }
  std::fs::remove_file(p).map_err(|e| format!("删除 {} 失败：{e}", p.display()))
}

/// 内容哈希（64 bit，十六进制）：同内容 → 同文件名（幂等复用）。
fn content_hash(bytes: &[u8]) -> u64 {
  let mut h = DefaultHasher::new();
  h.write(bytes);
  h.finish()
}

/// 文件名净化（legacy `_safe_stem` 同规则）：丢非法/控制字符，限长 60。
fn safe_stem(raw: &str) -> String {
  const INVALID: [char; 9] = ['<', '>', ':', '"', '/', '\\', '|', '?', '*'];
  let s: String = raw
    .chars()
    .map(|ch| {
      if INVALID.contains(&ch) || (ch as u32) < 32 {
        '_'
      } else {
        ch
      }
    })
    .collect();
  let s = s
    .trim()
    .trim_end_matches('.')
    .chars()
    .take(60)
    .collect::<String>();
  if s.trim().is_empty() {
    "audio".to_string()
  } else {
    s
  }
}

/// symphonia 解码整文件 → float32 单声道 48 kHz。
fn decode_to_mono_48k(bytes: &[u8], orig_name: &str) -> Result<Vec<f32>, String> {
  let mut hint = Hint::new();
  if let Some(ext) = Path::new(orig_name).extension().and_then(|e| e.to_str()) {
    hint.with_extension(ext);
  }
  let mss = MediaSourceStream::new(
    Box::new(std::io::Cursor::new(bytes.to_vec())),
    Default::default(),
  );
  let probed = get_probe()
    .format(
      &hint,
      mss,
      &FormatOptions::default(),
      &MetadataOptions::default(),
    )
    .map_err(|e| format!("无法识别的音频格式：{e}"))?;
  let mut format = probed.format;
  let track = match format.default_track() {
    Some(t) => t,
    None => return Err("文件里没有音频轨".into()),
  };
  let track_id = track.id;
  let mut decoder = get_codecs()
    .make(&track.codec_params, &DecoderOptions::default())
    .map_err(|e| format!("不支持的音频编码：{e}"))?;

  let mut mono: Vec<f32> = Vec::new();
  let mut native_rate = 0u32;
  loop {
    let packet = match format.next_packet() {
      Ok(p) => p,
      Err(SymphoniaError::IoError(e)) if e.kind() == std::io::ErrorKind::UnexpectedEof => break,
      Err(SymphoniaError::ResetRequired) => {
        decoder.reset();
        continue;
      }
      Err(e) => {
        // 已解出内容：容忍尾段损坏（截断的下载等）；一点都没有才算失败
        if mono.is_empty() {
          return Err(format!("解码失败：{e}"));
        }
        break;
      }
    };
    if packet.track_id() != track_id {
      continue;
    }
    let decoded = match decoder.decode(&packet) {
      Ok(d) => d,
      Err(SymphoniaError::DecodeError(e)) if !mono.is_empty() => {
        let _ = e;
        break;
      }
      Err(e) => {
        if mono.is_empty() {
          return Err(format!("解码失败：{e}"));
        }
        break;
      }
    };
    let spec: SignalSpec = *decoded.spec();
    if native_rate == 0 {
      native_rate = spec.rate;
    }
    buffer_to_mono_f32(&decoded, &mut mono);
  }

  if mono.is_empty() {
    return Err("解码结果为空".into());
  }
  if native_rate == 0 {
    return Err("取不到采样率".into());
  }
  if native_rate == LIB_RATE {
    return Ok(mono);
  }
  resample_to_48k(&mono, native_rate)
}

/// 把一路解码结果下混为单声道 f32 追加到 `out`（通道数从 spec 读）。
fn buffer_to_mono_f32(buf: &AudioBufferRef, out: &mut Vec<f32>) {
  macro_rules! downmix {
    ($b:expr) => {{
      let b = $b;
      let n = b.spec().channels.count().max(1);
      let frames = b.frames();
      let base = out.len();
      out.resize(base + frames, 0.0);
      for c in 0..n {
        for (i, &s) in b.chan(c).iter().enumerate() {
          out[base + i] += f32::from_sample(s);
        }
      }
      let inv = 1.0 / n as f32;
      for x in out[base..].iter_mut() {
        *x *= inv;
      }
    }};
  }
  match buf {
    AudioBufferRef::U8(b) => downmix!(b),
    AudioBufferRef::U16(b) => downmix!(b),
    AudioBufferRef::U24(b) => downmix!(b),
    AudioBufferRef::S24(b) => downmix!(b),
    AudioBufferRef::U32(b) => downmix!(b),
    AudioBufferRef::S8(b) => downmix!(b),
    AudioBufferRef::S16(b) => downmix!(b),
    AudioBufferRef::S32(b) => downmix!(b),
    AudioBufferRef::F32(b) => downmix!(b),
    AudioBufferRef::F64(b) => downmix!(b),
  }
}

/// 离线重采样（导入文件专用）：用与实时链路同一个 [`Converter`]，但**必须冲掉 sinc
/// 的输出延迟**——首批输入只用来填重采样器内部缓冲、不出样本。实时流永远在跑，
/// 不需要补；**文件是一次性的，补不补就是少一截开头**（实测 44.1k→48k 短 480 样本
/// = 整整 1 个 hop）。末尾喂静音凑够精确目标长度，再截断。
///
/// 实时侧不要改用这个：那里靠 [`ToHops`] 保持 10 ms 网格与流式节奏。
fn resample_to_48k(input: &[f32], rate: u32) -> Result<Vec<f32>, String> {
  use crate::dsp::resampler::Converter;
  // 原生 10 ms 块，与 `ToHops` 同款（非 100 整除的采样率取整）
  let chunk = ((rate as usize) / 100).max(1);
  let mut conv = Converter::new(rate, LIB_RATE, chunk, false)?;
  if conv.is_passthrough() {
    return Ok(input.to_vec());
  }
  // 精确目标长度（按比例，不按 hop 取整，避免时长失真）
  let expected = (input.len() as u64 * LIB_RATE as u64 / rate as u64) as usize;
  let mut out: Vec<f32> = Vec::with_capacity(expected + 2 * chunk);
  let mut blk = vec![0.0f32; chunk];
  let mut rest = input;
  loop {
    let n = rest.len().min(chunk);
    blk[..n].copy_from_slice(&rest[..n]);
    blk[n..].fill(0.0); // 末块补零（Converter 要求恰好 chunk 帧）
    out.extend_from_slice(conv.process(&blk)?);
    if n < chunk {
      break; // 真实数据喂完
    }
    rest = &rest[n..];
  }
  // 冲尾：继续喂静音直到凑够 expected。64 块 ≈ 640 ms，远大于任何合理滤波延迟；
  // 仍凑不够（异常输入）就按已有的输出收尾，不静默补零。
  for _ in 0..64 {
    if out.len() >= expected {
      break;
    }
    out.extend_from_slice(conv.process(&blk)?);
  }
  out.truncate(expected);
  Ok(out)
}

#[cfg(test)]
mod tests {
  use super::*;

  fn wav_bytes(rate: u32, ch: u16, secs: f32, hz: f32) -> Vec<u8> {
    let n = (rate as f32 * secs) as usize;
    let mut pcm: Vec<f32> = Vec::with_capacity(n);
    for i in 0..n {
      pcm.push(0.5 * (2.0 * std::f32::consts::PI * hz * i as f32 / rate as f32).sin());
    }
    // 写多声道 RIFF（write_mono_16 只写单声道，这里直接手写）
    let data_len = (n * 2 * ch as usize) as u32;
    let mut out = Vec::new();
    out.extend_from_slice(b"RIFF");
    out.extend_from_slice(&(36 + data_len).to_le_bytes());
    out.extend_from_slice(b"WAVE");
    out.extend_from_slice(b"fmt ");
    out.extend_from_slice(&16u32.to_le_bytes());
    out.extend_from_slice(&1u16.to_le_bytes());
    out.extend_from_slice(&ch.to_le_bytes());
    out.extend_from_slice(&rate.to_le_bytes());
    out.extend_from_slice(&(rate * 2 * ch as u32).to_le_bytes());
    out.extend_from_slice(&(2 * ch).to_le_bytes());
    out.extend_from_slice(&16u16.to_le_bytes());
    out.extend_from_slice(b"data");
    out.extend_from_slice(&data_len.to_le_bytes());
    for &s in &pcm {
      let v = (s * 32767.0) as i16;
      for _ in 0..ch {
        out.extend_from_slice(&v.to_le_bytes());
      }
    }
    out
  }

  #[test]
  fn import_normalizes_stereo_44k_to_mono_48k() {
    let bytes = wav_bytes(44_100, 2, 0.5, 440.0);
    let im = import(&bytes, "测试 音效/<ok>:name.wav", MediaKind::Soundpad).expect("导入");
    assert!(is_library_path(im.path.to_str().unwrap()));
    assert!(im.path.exists());
    let (samples, rate) = crate::wav::read_mono(&im.path).expect("读回");
    assert_eq!(rate, 48_000);
    assert_eq!(samples.len(), 24_000, "0.5 s @48k");
    assert!(samples.iter().any(|&x| x.abs() > 0.4), "应有有效信号");
    let _ = discard(im.path.to_str().unwrap());
  }

  #[test]
  fn import_is_idempotent_for_same_content() {
    let bytes = wav_bytes(48_000, 1, 0.1, 1000.0);
    let a = import(&bytes, "same.wav", MediaKind::Music).expect("a");
    let b = import(&bytes, "other-name.wav", MediaKind::Music).expect("b");
    // 同名不同 stem 但同内容 → 哈希不同（文件名带 stem），不互相覆盖
    assert_ne!(a.path, b.path);
    let a2 = import(&bytes, "same.wav", MediaKind::Music).expect("a2");
    assert_eq!(a.path, a2.path);
    assert!(a2.reused);
    let _ = discard(a.path.to_str().unwrap());
    let _ = discard(b.path.to_str().unwrap());
  }

  #[test]
  fn import_rejects_garbage() {
    let e = import(b"not an audio file at all", "x.wav", MediaKind::Soundpad);
    assert!(e.is_err(), "垃圾数据必须报错：{e:?}");
  }

  #[test]
  fn discard_refuses_non_library_paths() {
    assert!(discard("C:\\Windows\\notepad.exe").is_err());
    assert!(discard("").is_err());
  }

  #[test]
  fn safe_stem_sanitizes() {
    assert_eq!(safe_stem("a<b>c:d\"e/f\\g|h?i*j"), "a_b_c_d_e_f_g_h_i_j");
    assert_eq!(safe_stem("   "), "audio");
    assert_eq!(safe_stem("音效 01"), "音效 01");
  }
}
