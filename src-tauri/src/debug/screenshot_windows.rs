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

//! Windows 的内置截屏：`PrintWindow(hwnd, hdc, PW_RENDERFULLCONTENT)`。
//!
//! 截的是**客户区**（不含标题栏 / 边框），与 Linux 那份（WebKitGTK snapshot）语义一致：
//! - `PW_RENDERFULLCONTENT` 会请 WebView2 把内容画进我们的 DC，所以窗口被别的窗口
//!   盖住也没关系。
//!
//! 尺寸取窗口客户区的**物理像素**：本进程是 per-monitor DPI aware，`GetClientRect` 给的就是
//! 物理尺寸（200% 缩放下 1560 逻辑 px = 3120 物理 px），所以图是完整的一张、分辨率也够。
//!
//! 千万不要为了截图去改窗口大小 —— 那是拿布局换截图（2026-10-26 走过一次回头路）。
//! 外部 DPI-unaware 工具（PowerShell `GetWindowRect`）量这个窗口会得到被虚拟化过的一半
//! 尺寸，据此截图才会「截不全」。
//!
//! 像素序：DIB 是 BGRA（自上而下用负高度），PNG 要 RGBA，逐像素换一下即可。

use std::path::Path;

use tauri::Manager as _;

use windows::Win32::Foundation::{HWND, RECT};
use windows::Win32::Graphics::Gdi::{
  BI_RGB, BITMAPINFO, BITMAPINFOHEADER, CreateCompatibleDC, CreateDIBSection, DIB_RGB_COLORS,
  DeleteDC, DeleteObject, HGDIOBJ, SelectObject,
};
use windows::Win32::Storage::Xps::{PRINT_WINDOW_FLAGS, PrintWindow};
use windows::Win32::UI::WindowsAndMessaging::GetClientRect;

/// `PW_RENDERFULLCONTENT`：让 WebView2 / DWM 把完整内容画进 DC（0x2）。
const PW_RENDERFULLCONTENT: PRINT_WINDOW_FLAGS = PRINT_WINDOW_FLAGS(0x2);

pub(super) fn capture_to(path: &Path) -> Result<(), String> {
  let app = super::APP
    .get()
    .ok_or("截屏模块还没初始化（AppHandle 未登记）")?;
  let window = app
    .get_webview_window("main")
    .ok_or("找不到主窗口（label=main）")?;
  let hwnd: HWND = window.hwnd().map_err(|e| format!("取窗口句柄失败：{e}"))?;

  let mut rect = RECT::default();
  unsafe { GetClientRect(hwnd, &mut rect) }.map_err(|e| format!("取客户区尺寸失败：{e}"))?;
  let w = rect.right - rect.left;
  let h = rect.bottom - rect.top;
  if w <= 0 || h <= 0 {
    return Err(format!("客户区尺寸异常：{w}x{h}（窗口最小化或还没画出来）"));
  }

  let (bgra, pw) = unsafe { render(hwnd, w, h)? };
  if !pw {
    return Err("PrintWindow 失败（WebView2 未就绪或窗口不可见）".into());
  }

  // BGRA（DIB 顺序）→ RGBA，并丢掉 alpha（DIB 的 alpha 常是 0，留着会全透明）
  let mut rgba = Vec::with_capacity((w * h * 4) as usize);
  for px in bgra.as_chunks::<4>().0 {
    rgba.extend_from_slice(&[px[2], px[1], px[0], 255]);
  }

  let file = std::fs::File::create(path)
    .map_err(|e| format!("创建截图文件失败（{}）：{e}", path.display()))?;
  let mut enc = png::Encoder::new(std::io::BufWriter::new(file), w as u32, h as u32);
  enc.set_color(png::ColorType::Rgba);
  enc.set_depth(png::BitDepth::Eight);
  let mut writer = enc
    .write_header()
    .map_err(|e| format!("写 PNG 头失败：{e}"))?;
  writer
    .write_image_data(&rgba)
    .map_err(|e| format!("写 PNG 数据失败：{e}"))?;
  Ok(())
}

/// 把窗口客户区画进一块 32bpp 的 DIB，返回像素（BGRA）与 `PrintWindow` 的返回值。
unsafe fn render(hwnd: HWND, w: i32, h: i32) -> Result<(Vec<u8>, bool), String> {
  unsafe {
    let screen = CreateCompatibleDC(None);
    if screen.is_invalid() {
      return Err("CreateCompatibleDC 失败".into());
    }
    let bmi = BITMAPINFO {
      bmiHeader: BITMAPINFOHEADER {
        biSize: std::mem::size_of::<BITMAPINFOHEADER>() as u32,
        biWidth: w,
        // 负高度 = 自上而下，省一次翻转
        biHeight: -h,
        biPlanes: 1,
        biBitCount: 32,
        biCompression: BI_RGB.0,
        ..Default::default()
      },
      ..Default::default()
    };
    let mut bits: *mut std::ffi::c_void = std::ptr::null_mut();
    let dib = match CreateDIBSection(Some(screen), &bmi, DIB_RGB_COLORS, &mut bits, None, 0) {
      Ok(d) => d,
      Err(e) => {
        let _ = DeleteDC(screen);
        return Err(format!("CreateDIBSection 失败：{e}"));
      }
    };
    if bits.is_null() {
      let _ = DeleteObject(HGDIOBJ(dib.0));
      let _ = DeleteDC(screen);
      return Err("CreateDIBSection 没给像素缓冲".into());
    }
    let old = SelectObject(screen, HGDIOBJ(dib.0));
    let ok = PrintWindow(hwnd, screen, PW_RENDERFULLCONTENT).as_bool();
    let len = (w as usize) * (h as usize) * 4;
    let pixels = std::slice::from_raw_parts(bits as *const u8, len).to_vec();
    SelectObject(screen, old);
    let _ = DeleteObject(HGDIOBJ(dib.0));
    let _ = DeleteDC(screen);
    Ok((pixels, ok))
  }
}
