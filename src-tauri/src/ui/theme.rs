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
/// 右侧信息栏宽度。
pub const DOCK_W: f32 = 340.0;
/// 右侧信息栏内边距。
pub const DOCK_PAD: f32 = 12.0;

// ── 间距梯度：以 4 px 为基准，界面里不要再随手写 3/6/10 这类孤立数字 ──
/// 4 px：紧邻元素（标签与值、图标与文字）。
pub const SPACE_1: f32 = 4.0;
/// 8 px：同一组内的控件间距。
pub const SPACE_2: f32 = 8.0;
/// 12 px：面板内边距 / 分组间距。
pub const SPACE_3: f32 = 12.0;
/// 16 px：区块之间。
pub const SPACE_4: f32 = 16.0;
/// 24 px：页面级留白。
pub const SPACE_5: f32 = 24.0;

// ── 字号梯度：桌面端以 11 / 12 为正文，10 只给徽标，14 给检查器标题 ──
/// 10 px：徽标 / 端子号。
pub const TEXT_XS: f32 = 10.0;
/// 11 px：正文、字段标签、提示。
pub const TEXT_SM: f32 = 11.0;
/// 12 px：正文强调。
pub const TEXT_BASE: f32 = 12.0;
/// 13 px：分组标题 / 卡片标题。
pub const TEXT_MD: f32 = 13.0;
/// 14 px：检查器标题、空状态标题。
pub const TEXT_LG: f32 = 14.0;
/// 15 px：品牌字。
pub const TEXT_XL: f32 = 15.0;

// ── 圆角梯度 ──
/// 4 px：小控件 / 端子。
pub const RADIUS_SM: f32 = 4.0;
/// 6 px：按钮 / 输入框。
pub const RADIUS_MD: f32 = 6.0;
/// 8 px：卡片 / 面板。
pub const RADIUS_LG: f32 = 8.0;
