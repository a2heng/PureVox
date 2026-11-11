// PureVox — AI 麦克风降噪工具
// Copyright (C) 2024-2026 a2heng <752848283@qq.com>
//
// PureVox is licensed under the GNU General Public License v3.0 or
// later (GPL-3.0-or-later).  See LICENSE for details.
//
// SPDX-License-Identifier: GPL-3.0-or-later

//! 混音台界面配色与尺寸常量。

/// 窗口背景
pub const BG: u32 = 0x0d1015;
/// 面板背景
pub const PANEL: u32 = 0x141922;
/// 抬高的面板 / 输入框
pub const PANEL_HI: u32 = 0x1b2230;
/// 普通控件底色（按钮 / 下拉触发器）
pub const CONTROL: u32 = 0x222b39;
/// 控件悬停底色
pub const CONTROL_HI: u32 = 0x2c3747;
/// 分隔线
pub const BORDER: u32 = 0x283244;
/// 主文字
pub const TEXT: u32 = 0xdfe6ef;
/// 次文字
pub const MUTED: u32 = 0x8794a7;
/// 强调色
pub const ACCENT: u32 = 0x4f8cff;
/// 播放中 / 正常
pub const OK: u32 = 0x3ecf8e;
/// 录制中 / 错误
pub const DANGER: u32 = 0xe05561;
/// 连线（中性）
pub const WIRE: u32 = 0x3d4c66;
/// 麦克风 / 音频输入
pub const MIC: u32 = 0x3ecf8e;
/// 手机 / 网络输入
pub const NET: u32 = 0x4f8cff;
/// 系统声音（回环）
pub const LOOPBACK: u32 = 0xa06bff;
/// 回声消除（两路输入）
pub const AEC: u32 = 0xe0a35a;
/// 顶栏高度（像素）。
pub const TOP_H: f32 = 52.0;
/// 底栏高度。
pub const STATUS_H: f32 = 28.0;
/// 右侧设备面板宽度。
pub const SIDE_W: f32 = 300.0;
