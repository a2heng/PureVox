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

//! 网络卡片二维码：地址 → PNG 文件（GPUI `img()` 直接吃路径）。
//!
//! **不做自绘**：二维码是 20~40 行 × 列的小方格，用 div 画等于每帧上千个元素。
//! 编码成图片后 GPUI 只加载一次，URL 变了才换文件。
//!
//! 文件名带 URL 哈希：路径不变时 GPUI 会把图片当同一张缓存，改了地址必须换路径。

use std::collections::hash_map::DefaultHasher;
use std::hash::{Hash, Hasher};
use std::io::BufWriter;
use std::path::{Path, PathBuf};

/// 每个二维码模块放大多少像素（最终图片还会按控件尺寸缩放）。
const SCALE: usize = 5;
/// 四周留白（模块数）。二维码规范要求 ≥ 4。
const QUIET: usize = 4;

/// 把 `url` 编码成 `~/.purevox/net/qr-<hash>.png`，返回路径。
///
/// 同一 URL 已存在就直接复用；写失败返回原因（界面显示「二维码不可用」而不是空图）。
pub fn png_for(url: &str) -> Result<PathBuf, String> {
  let dir = crate::config::config_dir()?.join("net");
  std::fs::create_dir_all(&dir).map_err(|e| format!("创建二维码目录失败：{e}"))?;

  let mut h = DefaultHasher::new();
  url.hash(&mut h);
  let path = dir.join(format!("qr-{:016x}.png", h.finish()));
  if path.is_file() {
    return Ok(path);
  }

  let code = qrcode::QrCode::new(url.as_bytes()).map_err(|e| format!("二维码编码失败：{e}"))?;
  let w = code.width();
  let dim = (w + QUIET * 2) * SCALE;
  let mut gray = vec![255u8; dim * dim];
  let colors = code.to_colors();
  for y in 0..w {
    for x in 0..w {
      if colors[y * w + x] != qrcode::Color::Dark {
        continue;
      }
      let x0 = (x + QUIET) * SCALE;
      let y0 = (y + QUIET) * SCALE;
      for dy in 0..SCALE {
        let row = &mut gray[(y0 + dy) * dim + x0..(y0 + dy) * dim + x0 + SCALE];
        row.fill(0);
      }
    }
  }

  write_gray_png(&path, dim as u32, dim as u32, &gray)?;

  // 旧地址的图丢掉：每次改网卡 / 端口都会生成一张，不清理会一直堆。
  if let Ok(rd) = std::fs::read_dir(&dir) {
    for e in rd.flatten() {
      let p = e.path();
      if p != path && p.extension().and_then(|s| s.to_str()) == Some("png") {
        let _ = std::fs::remove_file(p);
      }
    }
  }
  Ok(path)
}

fn write_gray_png(path: &Path, w: u32, h: u32, data: &[u8]) -> Result<(), String> {
  let f = std::fs::File::create(path).map_err(|e| format!("创建二维码文件失败：{e}"))?;
  let mut enc = png::Encoder::new(BufWriter::new(f), w, h);
  enc.set_color(png::ColorType::Grayscale);
  enc.set_depth(png::BitDepth::Eight);
  let mut writer = enc
    .write_header()
    .map_err(|e| format!("写二维码 PNG 头失败：{e}"))?;
  writer
    .write_image_data(data)
    .map_err(|e| format!("写二维码 PNG 失败：{e}"))?;
  Ok(())
}
