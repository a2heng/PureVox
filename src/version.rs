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

//! 构建版本号（**版本号 = 年.月.日**）。
//!
//! 打包/发版时由 `tools/automation/version.py`（跨平台唯一实现）从 tag
//! （`v<yyyy.MM.dd.HHmm>`）或当前 UTC 时间推导，产出两个编译期常量：
//! - [`VERSION`] = `yy.M.d`（如 `26.10.8`）：**显示用**，也是 Tauri 打包的包版本。
//!   必须三段（Tauri 要 semver）。之所以不是 `2026.10.8`：历史上还要出 MSI，而
//!   MSI ProductVersion 限 major≤255 / minor≤255 / build≤65535，`2026.10.8` 直译会报
//!   major > 255；**现在 Windows 只发 NSIS（exe）**，但这个 `yy.M.d` 映射保持不变 ——
//!   产品的显示版本、产物文件名、更新说明都按它走，改格式只会平白漂移。
//! - [`STAMP`] = `yyyy.MM.dd.HHmm`（如 `2026.10.08.1430`）：**比较更新用**，按时间全序。
//!   显示版本同日重复时（同一天发两次），靠 STAMP 的时分区分新旧——更新检查比较它，
//!   不比较 VERSION。
//!
//! 开发态两者都没注入：VERSION 回退 Cargo 包版本，STAMP 为空串（`is_dev()` = true，
//! 更新检查据此同时报「开发构建」而不是拿 0.1.0 去比）。

/// 显示版本：CI 注入的 `PUREVOX_BUILD_VERSION`（`yy.M.d`），否则 Cargo 包版本。
pub const VERSION: &str = match option_env!("PUREVOX_BUILD_VERSION") {
  Some(v) => v,
  None => env!("CARGO_PKG_VERSION"),
};

/// 完整构建时间戳：CI 注入的 `PUREVOX_BUILD_STAMP`（`yyyy.MM.dd.HHmm`），开发态为空。
pub const STAMP: &str = match option_env!("PUREVOX_BUILD_STAMP") {
  Some(v) => v,
  None => "",
};

/// 是否为开发构建（无 CI 注入的时间戳）——更新检查据此不给出「已是最新」的误导结论。
pub fn is_dev() -> bool {
  STAMP.is_empty()
}

/// `yyyy.MM.dd.HHmm` → 可比较的整数元组（年, 月, 日, 当日分钟数）；不合法返回 None。
///
/// **更新比较的唯一依据**：显示版本 `yy.M.d` 同日重复，靠这里的时分区分新旧
/// （见 [`crate::update`]；那边解析 release tag 也用它）。`HHmm` 折成当日分钟数，
/// 元组按时间全序，可直接用 `>` 比较。
pub fn parse_stamp(s: &str) -> Option<(u32, u32, u32, u32)> {
  let p: Vec<&str> = s.split('.').collect();
  if p.len() != 4 {
    return None;
  }
  let y: u32 = p[0].parse().ok()?;
  let mo: u32 = p[1].parse().ok()?;
  let d: u32 = p[2].parse().ok()?;
  // 末段是 `HHmm`（4 位，如 1430 / 0930），不是 `HH.mm`
  if p[3].len() != 4 {
    return None;
  }
  let hh: u32 = p[3][..2].parse().ok()?;
  let mi: u32 = p[3][2..].parse().ok()?;
  (y >= 2000 && (1..=12).contains(&mo) && (1..=31).contains(&d) && hh < 24 && mi < 60).then_some((
    y,
    mo,
    d,
    hh * 60 + mi,
  ))
}

#[cfg(test)]
mod tests {
  use super::*;

  #[test]
  fn parse_stamp_accepts_full_and_rejects_junk() {
    // (年, 月, 日, 当日分钟数)
    assert_eq!(
      parse_stamp("2026.10.08.1430"),
      Some((2026, 10, 8, 14 * 60 + 30))
    );
    assert_eq!(parse_stamp("26.10.8"), None, "两段不是构建时间戳");
    assert_eq!(parse_stamp("2026.13.08.1430"), None, "13 月非法");
    assert_eq!(parse_stamp("2026.10.32.1430"), None, "32 日非法");
    assert_eq!(parse_stamp("2026.10.08.2500"), None, "25 点非法");
    assert_eq!(
      parse_stamp("2026.10.08.14.30"),
      None,
      "四段格式，末段不是 HHmm"
    );
    assert_eq!(parse_stamp("2026.10.08.143"), None, "末段必须 4 位");
  }

  #[test]
  fn parse_stamp_is_time_ordered() {
    let a = parse_stamp("2026.10.08.1430").unwrap();
    let b = parse_stamp("2026.10.08.1431").unwrap();
    let c = parse_stamp("2026.10.09.0001").unwrap();
    assert!(a < b && b < c, "同日时分、次日都必须全序");
  }
}
