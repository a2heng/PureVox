// PureVox — AI 麦克风降噪工具
// Copyright (C) 2024-2026 a2heng <752848283@qq.com>
//
// PureVox is licensed under the GNU General Public License v3.0 or
// later (GPL-3.0-or-later).  See LICENSE for details.
//
// SPDX-License-Identifier: GPL-3.0-or-later

//! 界面的最小展示原语。
//!
//! 所有页面都从这里取按钮 / 字段 / 提示 / 分组标题，避免同一个视觉规则在 `ui` 里
//! 复制出第二份。尺寸与颜色只引用 `theme` 的 token，不在调用点写孤立数字。

use gpui_kit::component::Sizable as _;
use gpui_kit::component::button::{Button, ButtonCustomVariant, ButtonVariants as _};
use gpui_kit::prelude::*;
use gpui_kit::{App, Div, ElementId, SharedString, div, px, rgb};

use super::i18n;
use super::theme as t;

/// 低强调按钮：工具栏、检查器里的普通命令都用它。
pub(super) fn quiet_button(
  cx: &App,
  id: impl Into<ElementId>,
  label: impl Into<SharedString>,
) -> Button {
  let label: SharedString = label.into();
  let label = SharedString::from(i18n::tr(&label));
  Button::new(id)
    .xsmall()
    .custom(
      ButtonCustomVariant::new(cx)
        .color(rgb(t::CONTROL).into())
        .foreground(rgb(t::TEXT).into())
        .hover(rgb(t::CONTROL_HI).into())
        .active(rgb(t::CONTROL_HI).into()),
    )
    .label(label)
}

/// 检查器里的说明文字：次要、可换行、不抢主控件的注意力。
pub(super) fn hint(text: impl Into<SharedString>) -> Div {
  let text: SharedString = text.into();
  div()
    .text_size(px(t::TEXT_SM))
    .text_color(rgb(t::MUTED))
    .child(SharedString::from(i18n::tr(&text)))
}

/// 一行「标签 → 值」。值右对齐并截断，长度变化时不推挤左列。
pub(super) fn field(label: &str, value: String) -> Div {
  div()
    .flex()
    .items_center()
    .justify_between()
    .gap_2()
    .text_size(px(t::TEXT_SM))
    .child(
      div()
        .text_color(rgb(t::MUTED))
        .child(i18n::tr(label).to_string()),
    )
    .child(
      div()
        .flex_1()
        .text_right()
        .truncate()
        .text_color(rgb(t::TEXT))
        .child(value),
    )
}

/// 检查器里的分组标题；`section` 只负责层级，不负责间距，间距由容器统一给。
pub(super) fn section(text: &str) -> Div {
  div()
    .mt(px(t::SPACE_1))
    .text_size(px(t::TEXT_SM))
    .text_color(rgb(t::ACCENT))
    .child(i18n::tr(text).to_string())
}

/// 速率类 `Probe<f64>` → 人类可读（采样中 / 不可用原因也照实显示，不编 0）。
pub(super) fn probe_rate(p: &crate::debug::Probe<f64>) -> String {
  match p {
    crate::debug::Probe::Ok { value } => format!("{value:.1} 包/s"),
    crate::debug::Probe::Pending => "采样中…".into(),
    crate::debug::Probe::Unavailable { reason } => reason.clone(),
  }
}
