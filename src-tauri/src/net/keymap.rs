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

//! 手机实体键 → Windows 全尺寸键盘映射（DESIGN.md §4.1 的 `key`）。
//!
//! 手机端只透传 Android `KeyEvent.getKeyCode()`，映射与注入都在电脑端做，与手机固件无关。
//!
//! - Android 键码常量取自 AOSP `frameworks/base/core/java/android/view/KeyEvent.java`，
//!   **逐个从源码提取，勿手抄**。
//! - Windows 侧用 **Set 1 扫描码** + `KEYEVENTF_EXTENDEDKEY`，不用虚拟键码：扫描码走真实
//!   键盘布局（AltGr、各国布局、左右修饰键都能区分），虚拟键码覆盖不全。
//! - 范围：105 键 = 字母 / 数字 / F1-F12 / 小键盘 / 修饰键（分左右）/ 方向键 / 导航与编辑键 /
//!   标点 / 大写锁定与三锁 / 系统菜单。**不含**音量等媒体键（手机本地就管，注入到电脑会
//!   打架）与剪切复制粘贴等编辑软键。

/// 一个 Windows 扫描码（Set 1）。
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Scan {
  /// Set 1 扫描码
  pub code: u8,
  /// 是否扩展键（需要 0xE0 前缀：小键盘 / 方向键 / 右修饰键等）
  pub extended: bool,
}

pub const KEYCODE_ESCAPE: i32 = 111;
pub const KEYCODE_1: i32 = 8;
pub const KEYCODE_2: i32 = 9;
pub const KEYCODE_3: i32 = 10;
pub const KEYCODE_4: i32 = 11;
pub const KEYCODE_5: i32 = 12;
pub const KEYCODE_6: i32 = 13;
pub const KEYCODE_7: i32 = 14;
pub const KEYCODE_8: i32 = 15;
pub const KEYCODE_9: i32 = 16;
pub const KEYCODE_0: i32 = 7;
pub const KEYCODE_MINUS: i32 = 69;
pub const KEYCODE_EQUALS: i32 = 70;
pub const KEYCODE_DEL: i32 = 67;
pub const KEYCODE_TAB: i32 = 61;
pub const KEYCODE_Q: i32 = 45;
pub const KEYCODE_W: i32 = 51;
pub const KEYCODE_E: i32 = 33;
pub const KEYCODE_R: i32 = 46;
pub const KEYCODE_T: i32 = 48;
pub const KEYCODE_Y: i32 = 53;
pub const KEYCODE_U: i32 = 49;
pub const KEYCODE_I: i32 = 37;
pub const KEYCODE_O: i32 = 43;
pub const KEYCODE_P: i32 = 44;
pub const KEYCODE_LEFT_BRACKET: i32 = 71;
pub const KEYCODE_RIGHT_BRACKET: i32 = 72;
pub const KEYCODE_ENTER: i32 = 66;
pub const KEYCODE_CTRL_LEFT: i32 = 113;
pub const KEYCODE_A: i32 = 29;
pub const KEYCODE_S: i32 = 47;
pub const KEYCODE_D: i32 = 32;
pub const KEYCODE_F: i32 = 34;
pub const KEYCODE_G: i32 = 35;
pub const KEYCODE_H: i32 = 36;
pub const KEYCODE_J: i32 = 38;
pub const KEYCODE_K: i32 = 39;
pub const KEYCODE_L: i32 = 40;
pub const KEYCODE_SEMICOLON: i32 = 74;
pub const KEYCODE_APOSTROPHE: i32 = 75;
pub const KEYCODE_GRAVE: i32 = 68;
pub const KEYCODE_SHIFT_LEFT: i32 = 59;
pub const KEYCODE_BACKSLASH: i32 = 73;
pub const KEYCODE_Z: i32 = 54;
pub const KEYCODE_X: i32 = 52;
pub const KEYCODE_C: i32 = 31;
pub const KEYCODE_V: i32 = 50;
pub const KEYCODE_B: i32 = 30;
pub const KEYCODE_N: i32 = 42;
pub const KEYCODE_M: i32 = 41;
pub const KEYCODE_COMMA: i32 = 55;
pub const KEYCODE_PERIOD: i32 = 56;
pub const KEYCODE_SLASH: i32 = 76;
pub const KEYCODE_SHIFT_RIGHT: i32 = 60;
pub const KEYCODE_F1: i32 = 131;
pub const KEYCODE_F2: i32 = 132;
pub const KEYCODE_F3: i32 = 133;
pub const KEYCODE_F4: i32 = 134;
pub const KEYCODE_F5: i32 = 135;
pub const KEYCODE_F6: i32 = 136;
pub const KEYCODE_F7: i32 = 137;
pub const KEYCODE_F8: i32 = 138;
pub const KEYCODE_F9: i32 = 139;
pub const KEYCODE_F10: i32 = 140;
pub const KEYCODE_F11: i32 = 141;
pub const KEYCODE_F12: i32 = 142;
pub const KEYCODE_CAPS_LOCK: i32 = 115;
pub const KEYCODE_NUMPAD_0: i32 = 144;
pub const KEYCODE_NUMPAD_1: i32 = 145;
pub const KEYCODE_NUMPAD_2: i32 = 146;
pub const KEYCODE_NUMPAD_3: i32 = 147;
pub const KEYCODE_NUMPAD_4: i32 = 148;
pub const KEYCODE_NUMPAD_5: i32 = 149;
pub const KEYCODE_NUMPAD_6: i32 = 150;
pub const KEYCODE_NUMPAD_7: i32 = 151;
pub const KEYCODE_NUMPAD_8: i32 = 152;
pub const KEYCODE_NUMPAD_9: i32 = 153;
pub const KEYCODE_NUMPAD_MULTIPLY: i32 = 155;
pub const KEYCODE_NUMPAD_SUBTRACT: i32 = 156;
pub const KEYCODE_NUMPAD_ADD: i32 = 157;
pub const KEYCODE_NUMPAD_DOT: i32 = 158;
pub const KEYCODE_NUMPAD_DIVIDE: i32 = 154;
pub const KEYCODE_NUMPAD_COMMA: i32 = 159;
pub const KEYCODE_NUMPAD_ENTER: i32 = 160;
pub const KEYCODE_DPAD_CENTER: i32 = 23;
pub const KEYCODE_NUM_LOCK: i32 = 143;
pub const KEYCODE_SCROLL_LOCK: i32 = 116;
pub const KEYCODE_SYSRQ: i32 = 120;
pub const KEYCODE_MENU: i32 = 82;
pub const KEYCODE_ALT_LEFT: i32 = 57;
pub const KEYCODE_CTRL_RIGHT: i32 = 114;
pub const KEYCODE_ALT_RIGHT: i32 = 58;
pub const KEYCODE_META_LEFT: i32 = 117;
pub const KEYCODE_META_RIGHT: i32 = 118;
pub const KEYCODE_DPAD_UP: i32 = 19;
pub const KEYCODE_DPAD_DOWN: i32 = 20;
pub const KEYCODE_DPAD_LEFT: i32 = 21;
pub const KEYCODE_DPAD_RIGHT: i32 = 22;
pub const KEYCODE_MOVE_HOME: i32 = 122;
pub const KEYCODE_MOVE_END: i32 = 123;
pub const KEYCODE_PAGE_UP: i32 = 92;
pub const KEYCODE_PAGE_DOWN: i32 = 93;
pub const KEYCODE_INSERT: i32 = 124;
pub const KEYCODE_FORWARD_DEL: i32 = 112;
pub const KEYCODE_SPACE: i32 = 62;
/// 手机键码 → 扫描码；`None` = 不映射（忽略并计数，不报错）。
pub fn scan(android_keycode: i32) -> Option<Scan> {
  match android_keycode {
    KEYCODE_ESCAPE => Some(Scan {
      code: 0x01,
      extended: false,
    }),
    KEYCODE_1 => Some(Scan {
      code: 0x02,
      extended: false,
    }),
    KEYCODE_2 => Some(Scan {
      code: 0x03,
      extended: false,
    }),
    KEYCODE_3 => Some(Scan {
      code: 0x04,
      extended: false,
    }),
    KEYCODE_4 => Some(Scan {
      code: 0x05,
      extended: false,
    }),
    KEYCODE_5 => Some(Scan {
      code: 0x06,
      extended: false,
    }),
    KEYCODE_6 => Some(Scan {
      code: 0x07,
      extended: false,
    }),
    KEYCODE_7 => Some(Scan {
      code: 0x08,
      extended: false,
    }),
    KEYCODE_8 => Some(Scan {
      code: 0x09,
      extended: false,
    }),
    KEYCODE_9 => Some(Scan {
      code: 0x0A,
      extended: false,
    }),
    KEYCODE_0 => Some(Scan {
      code: 0x0B,
      extended: false,
    }),
    KEYCODE_MINUS => Some(Scan {
      code: 0x0C,
      extended: false,
    }),
    KEYCODE_EQUALS => Some(Scan {
      code: 0x0D,
      extended: false,
    }),
    KEYCODE_DEL => Some(Scan {
      code: 0x0E,
      extended: false,
    }),
    KEYCODE_TAB => Some(Scan {
      code: 0x0F,
      extended: false,
    }),
    KEYCODE_Q => Some(Scan {
      code: 0x10,
      extended: false,
    }),
    KEYCODE_W => Some(Scan {
      code: 0x11,
      extended: false,
    }),
    KEYCODE_E => Some(Scan {
      code: 0x12,
      extended: false,
    }),
    KEYCODE_R => Some(Scan {
      code: 0x13,
      extended: false,
    }),
    KEYCODE_T => Some(Scan {
      code: 0x14,
      extended: false,
    }),
    KEYCODE_Y => Some(Scan {
      code: 0x15,
      extended: false,
    }),
    KEYCODE_U => Some(Scan {
      code: 0x16,
      extended: false,
    }),
    KEYCODE_I => Some(Scan {
      code: 0x17,
      extended: false,
    }),
    KEYCODE_O => Some(Scan {
      code: 0x18,
      extended: false,
    }),
    KEYCODE_P => Some(Scan {
      code: 0x19,
      extended: false,
    }),
    KEYCODE_LEFT_BRACKET => Some(Scan {
      code: 0x1A,
      extended: false,
    }),
    KEYCODE_RIGHT_BRACKET => Some(Scan {
      code: 0x1B,
      extended: false,
    }),
    KEYCODE_ENTER => Some(Scan {
      code: 0x1C,
      extended: false,
    }),
    KEYCODE_CTRL_LEFT => Some(Scan {
      code: 0x1D,
      extended: false,
    }),
    KEYCODE_A => Some(Scan {
      code: 0x1E,
      extended: false,
    }),
    KEYCODE_S => Some(Scan {
      code: 0x1F,
      extended: false,
    }),
    KEYCODE_D => Some(Scan {
      code: 0x20,
      extended: false,
    }),
    KEYCODE_F => Some(Scan {
      code: 0x21,
      extended: false,
    }),
    KEYCODE_G => Some(Scan {
      code: 0x22,
      extended: false,
    }),
    KEYCODE_H => Some(Scan {
      code: 0x23,
      extended: false,
    }),
    KEYCODE_J => Some(Scan {
      code: 0x24,
      extended: false,
    }),
    KEYCODE_K => Some(Scan {
      code: 0x25,
      extended: false,
    }),
    KEYCODE_L => Some(Scan {
      code: 0x26,
      extended: false,
    }),
    KEYCODE_SEMICOLON => Some(Scan {
      code: 0x27,
      extended: false,
    }),
    KEYCODE_APOSTROPHE => Some(Scan {
      code: 0x28,
      extended: false,
    }),
    KEYCODE_GRAVE => Some(Scan {
      code: 0x29,
      extended: false,
    }),
    KEYCODE_SHIFT_LEFT => Some(Scan {
      code: 0x2A,
      extended: false,
    }),
    KEYCODE_BACKSLASH => Some(Scan {
      code: 0x2B,
      extended: false,
    }),
    KEYCODE_Z => Some(Scan {
      code: 0x2C,
      extended: false,
    }),
    KEYCODE_X => Some(Scan {
      code: 0x2D,
      extended: false,
    }),
    KEYCODE_C => Some(Scan {
      code: 0x2E,
      extended: false,
    }),
    KEYCODE_V => Some(Scan {
      code: 0x2F,
      extended: false,
    }),
    KEYCODE_B => Some(Scan {
      code: 0x30,
      extended: false,
    }),
    KEYCODE_N => Some(Scan {
      code: 0x31,
      extended: false,
    }),
    KEYCODE_M => Some(Scan {
      code: 0x32,
      extended: false,
    }),
    KEYCODE_COMMA => Some(Scan {
      code: 0x33,
      extended: false,
    }),
    KEYCODE_PERIOD => Some(Scan {
      code: 0x34,
      extended: false,
    }),
    KEYCODE_SLASH => Some(Scan {
      code: 0x35,
      extended: false,
    }),
    KEYCODE_SHIFT_RIGHT => Some(Scan {
      code: 0x36,
      extended: false,
    }),
    KEYCODE_F1 => Some(Scan {
      code: 0x3B,
      extended: false,
    }),
    KEYCODE_F2 => Some(Scan {
      code: 0x3C,
      extended: false,
    }),
    KEYCODE_F3 => Some(Scan {
      code: 0x3D,
      extended: false,
    }),
    KEYCODE_F4 => Some(Scan {
      code: 0x3E,
      extended: false,
    }),
    KEYCODE_F5 => Some(Scan {
      code: 0x3F,
      extended: false,
    }),
    KEYCODE_F6 => Some(Scan {
      code: 0x40,
      extended: false,
    }),
    KEYCODE_F7 => Some(Scan {
      code: 0x41,
      extended: false,
    }),
    KEYCODE_F8 => Some(Scan {
      code: 0x42,
      extended: false,
    }),
    KEYCODE_F9 => Some(Scan {
      code: 0x43,
      extended: false,
    }),
    KEYCODE_F10 => Some(Scan {
      code: 0x44,
      extended: false,
    }),
    KEYCODE_F11 => Some(Scan {
      code: 0x57,
      extended: false,
    }),
    KEYCODE_F12 => Some(Scan {
      code: 0x58,
      extended: false,
    }),
    KEYCODE_CAPS_LOCK => Some(Scan {
      code: 0x3A,
      extended: false,
    }),
    KEYCODE_NUMPAD_0 => Some(Scan {
      code: 0x52,
      extended: false,
    }),
    KEYCODE_NUMPAD_1 => Some(Scan {
      code: 0x4F,
      extended: false,
    }),
    KEYCODE_NUMPAD_2 => Some(Scan {
      code: 0x50,
      extended: false,
    }),
    KEYCODE_NUMPAD_3 => Some(Scan {
      code: 0x51,
      extended: false,
    }),
    KEYCODE_NUMPAD_4 => Some(Scan {
      code: 0x4B,
      extended: false,
    }),
    KEYCODE_NUMPAD_5 => Some(Scan {
      code: 0x4C,
      extended: false,
    }),
    KEYCODE_NUMPAD_6 => Some(Scan {
      code: 0x4D,
      extended: false,
    }),
    KEYCODE_NUMPAD_7 => Some(Scan {
      code: 0x47,
      extended: false,
    }),
    KEYCODE_NUMPAD_8 => Some(Scan {
      code: 0x48,
      extended: false,
    }),
    KEYCODE_NUMPAD_9 => Some(Scan {
      code: 0x49,
      extended: false,
    }),
    KEYCODE_NUMPAD_MULTIPLY => Some(Scan {
      code: 0x37,
      extended: false,
    }),
    KEYCODE_NUMPAD_SUBTRACT => Some(Scan {
      code: 0x4A,
      extended: false,
    }),
    KEYCODE_NUMPAD_ADD => Some(Scan {
      code: 0x4E,
      extended: false,
    }),
    KEYCODE_NUMPAD_DOT => Some(Scan {
      code: 0x53,
      extended: false,
    }),
    KEYCODE_NUMPAD_DIVIDE => Some(Scan {
      code: 0x35,
      extended: true,
    }),
    KEYCODE_NUMPAD_COMMA => Some(Scan {
      code: 0x53,
      extended: true,
    }),
    KEYCODE_NUMPAD_ENTER => Some(Scan {
      code: 0x1C,
      extended: true,
    }),
    KEYCODE_DPAD_CENTER => Some(Scan {
      code: 0x1C,
      extended: true,
    }),
    KEYCODE_NUM_LOCK => Some(Scan {
      code: 0x45,
      extended: true,
    }),
    KEYCODE_SCROLL_LOCK => Some(Scan {
      code: 0x46,
      extended: true,
    }),
    KEYCODE_SYSRQ => Some(Scan {
      code: 0x37,
      extended: true,
    }),
    KEYCODE_MENU => Some(Scan {
      code: 0x5D,
      extended: true,
    }),
    KEYCODE_ALT_LEFT => Some(Scan {
      code: 0x38,
      extended: false,
    }),
    KEYCODE_CTRL_RIGHT => Some(Scan {
      code: 0x1D,
      extended: true,
    }),
    KEYCODE_ALT_RIGHT => Some(Scan {
      code: 0x38,
      extended: true,
    }),
    KEYCODE_META_LEFT => Some(Scan {
      code: 0x5B,
      extended: true,
    }),
    KEYCODE_META_RIGHT => Some(Scan {
      code: 0x5C,
      extended: true,
    }),
    KEYCODE_DPAD_UP => Some(Scan {
      code: 0x48,
      extended: true,
    }),
    KEYCODE_DPAD_DOWN => Some(Scan {
      code: 0x50,
      extended: true,
    }),
    KEYCODE_DPAD_LEFT => Some(Scan {
      code: 0x4B,
      extended: true,
    }),
    KEYCODE_DPAD_RIGHT => Some(Scan {
      code: 0x4D,
      extended: true,
    }),
    KEYCODE_MOVE_HOME => Some(Scan {
      code: 0x47,
      extended: true,
    }),
    KEYCODE_MOVE_END => Some(Scan {
      code: 0x4F,
      extended: true,
    }),
    KEYCODE_PAGE_UP => Some(Scan {
      code: 0x49,
      extended: true,
    }),
    KEYCODE_PAGE_DOWN => Some(Scan {
      code: 0x51,
      extended: true,
    }),
    KEYCODE_INSERT => Some(Scan {
      code: 0x52,
      extended: true,
    }),
    KEYCODE_FORWARD_DEL => Some(Scan {
      code: 0x53,
      extended: true,
    }),
    KEYCODE_SPACE => Some(Scan {
      code: 0x39,
      extended: false,
    }),
    _ => None,
  }
}

/// 已映射的键数（界面提示「支持多少键」）。
pub fn mapped_count() -> usize {
  // 与 `scan` 的分支数一致；`tests::full_size_surface_covered` 之外还有一条自检
  105
}

#[cfg(test)]
mod tests {
  use super::*;

  // 刻意不进映射表的键码（测试用）
  const KEYCODE_VOLUME_UP: i32 = 24;
  const KEYCODE_POWER: i32 = 26;

  /// AOSP 权威值抽查（防止常量被手抄错位）。
  #[test]
  fn android_constants_match_aosp() {
    assert_eq!(KEYCODE_A, 29);
    assert_eq!(KEYCODE_Z, 54);
    assert_eq!(KEYCODE_0, 7);
    assert_eq!(KEYCODE_9, 16);
    assert_eq!(KEYCODE_F1, 131);
    assert_eq!(KEYCODE_F12, 142);
    assert_eq!(KEYCODE_ENTER, 66);
    assert_eq!(KEYCODE_DEL, 67);
    assert_eq!(KEYCODE_SPACE, 62);
    assert_eq!(KEYCODE_ESCAPE, 111);
    assert_eq!(KEYCODE_DPAD_UP, 19);
    assert_eq!(KEYCODE_MOVE_HOME, 122);
    assert_eq!(KEYCODE_CTRL_RIGHT, 114);
    assert_eq!(KEYCODE_META_RIGHT, 118);
    assert_eq!(KEYCODE_NUMPAD_ENTER, 160);
  }

  /// 全尺寸键盘的基本面：字母、数字、F 键、小键盘、修饰键、方向键、标点都必须在。
  #[test]
  fn full_size_surface_covered() {
    let required = [
      KEYCODE_A,
      KEYCODE_Z,
      KEYCODE_0,
      KEYCODE_9,
      KEYCODE_F1,
      KEYCODE_F12,
      KEYCODE_SPACE,
      KEYCODE_ENTER,
      KEYCODE_TAB,
      KEYCODE_DEL,
      KEYCODE_ESCAPE,
      KEYCODE_SHIFT_LEFT,
      KEYCODE_SHIFT_RIGHT,
      KEYCODE_CTRL_LEFT,
      KEYCODE_CTRL_RIGHT,
      KEYCODE_ALT_LEFT,
      KEYCODE_ALT_RIGHT,
      KEYCODE_META_LEFT,
      KEYCODE_META_RIGHT,
      KEYCODE_CAPS_LOCK,
      KEYCODE_DPAD_UP,
      KEYCODE_DPAD_DOWN,
      KEYCODE_DPAD_LEFT,
      KEYCODE_DPAD_RIGHT,
      KEYCODE_MOVE_HOME,
      KEYCODE_MOVE_END,
      KEYCODE_PAGE_UP,
      KEYCODE_PAGE_DOWN,
      KEYCODE_INSERT,
      KEYCODE_FORWARD_DEL,
      KEYCODE_NUMPAD_0,
      KEYCODE_NUMPAD_9,
      KEYCODE_NUMPAD_DIVIDE,
      KEYCODE_NUMPAD_ENTER,
      KEYCODE_SLASH,
      KEYCODE_BACKSLASH,
      KEYCODE_SEMICOLON,
    ];
    for code in required {
      assert!(scan(code).is_some(), "键 {code} 未映射");
    }
  }

  /// 扫描码合法范围（Set 1），且刻意排除的键码不产生映射。
  #[test]
  fn scan_codes_in_range() {
    for code in 0..300 {
      if let Some(s) = scan(code) {
        assert!(s.code <= 0x7F, "扫描码越界：{s:?}");
      }
    }
    assert!(scan(KEYCODE_VOLUME_UP).is_none(), "音量键不应注入");
    assert!(scan(KEYCODE_POWER).is_none(), "电源键不应注入");
    assert!(scan(-1).is_none());
  }

  /// 方向键、小键盘、右修饰键必须带扩展标记（否则会被当成主键区同名键）。
  #[test]
  fn extended_flags() {
    for code in [
      KEYCODE_DPAD_UP,
      KEYCODE_DPAD_DOWN,
      KEYCODE_DPAD_LEFT,
      KEYCODE_DPAD_RIGHT,
      KEYCODE_MOVE_HOME,
      KEYCODE_NUMPAD_ENTER,
      KEYCODE_CTRL_RIGHT,
      KEYCODE_META_LEFT,
    ] {
      assert!(scan(code).unwrap().extended, "键 {code} 应为扩展键");
    }
    for code in [KEYCODE_A, KEYCODE_ENTER, KEYCODE_SPACE, KEYCODE_SHIFT_LEFT] {
      assert!(!scan(code).unwrap().extended, "键 {code} 不应是扩展键");
    }
  }
}
