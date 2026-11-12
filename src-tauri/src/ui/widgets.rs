// PureVox — AI 麦克风降噪工具
// Copyright (C) 2024-2026 a2heng <752848283@qq.com>
//
// PureVox is licensed under the GNU General Public License v3.0 or
// later (GPL-3.0-or-later).  See LICENSE for details.
//
// SPDX-License-Identifier: GPL-3.0-or-later

//! 界面的最小展示原语。
//!
//! 所有页面都从这里取控件，避免同一个视觉规则在 `ui` 里复制出第二份。
//! 尺寸与颜色只引用 `theme` 的 token，不在调用点写孤立数字。
//!
//! **层级**（不要把所有命令都画成同一种按钮）：
//! - [`add_button`]：顶栏「添加节点」，带 + 图标与强调描边；
//! - [`toolbar_icon`]：顶栏命令（撤销 / 重做 / 刷新），ghost 图标；
//! - [`quiet_button`]：检查器里的普通命令，实底 + 描边；
//! - [`choice_button`]：互斥选项（设备 / 参数值），选中态用强调色；
//! - [`danger_button`]：删除 / 断开，红色文字；
//! - [`switch_row`]：即时生效的开关（左标签 / 右开关）。

use gpui_kit::component::Sizable as _;
use gpui_kit::component::button::{Button, ButtonVariants as _};
use gpui_kit::component::switch::Switch;
use gpui_kit::component::{IconName, Size};
use gpui_kit::prelude::*;
use gpui_kit::{
  AnyElement, App, Div, ElementId, Hsla, SharedString, Stateful, Window, div, px, rgb, rgba,
};

use super::i18n;
use super::theme as t;

/// 顶栏的图标命令：没有边框、没有底色，只占一个 20 px 的点击区。
pub(super) fn toolbar_icon(
  _cx: &App,
  id: impl Into<ElementId>,
  icon: IconName,
  tip: &str,
) -> Button {
  Button::new(id)
    .xsmall()
    .ghost()
    .icon(icon)
    .text_color(rgb(t::MUTED))
    .accessibility_label(i18n::tr(tip).to_string())
    .tooltip(i18n::tr(tip).to_string())
}

/// 顶栏里的文字命令（很少用；默认优先图标）。
pub(super) fn toolbar_text(
  _cx: &App,
  id: impl Into<ElementId>,
  label: impl Into<SharedString>,
) -> Button {
  let label: SharedString = label.into();
  Button::new(id)
    .xsmall()
    .ghost()
    .label(i18n::tr(&label).to_string())
    .text_color(rgb(t::MUTED))
}

/// 顶栏主操作：添加节点。强调描边 + 加号，和普通命令区分开。
pub(super) fn add_button(
  _cx: &App,
  id: impl Into<ElementId>,
  label: impl Into<SharedString>,
  open: bool,
) -> Button {
  let label: SharedString = label.into();
  Button::new(id)
    .small()
    .icon(IconName::Plus)
    .label(i18n::tr(&label).to_string())
    .bg(rgb(if open { t::ACCENT_SOFT } else { t::PANEL_HI }))
    .text_color(rgb(t::TEXT))
    .border_1()
    .border_color(rgb(t::ACCENT))
}

/// 低强调命令：检查器里「刷新 / 停止 / 试听」这类普通动作。
///
/// 底色 / 描边显式给足，**不要**依赖 `ButtonCustomVariant::color` 的半透明混合
/// （那会把按钮压到几乎和面板同色）。
pub(super) fn quiet_button(
  _cx: &App,
  id: impl Into<ElementId>,
  label: impl Into<SharedString>,
) -> Button {
  let label: SharedString = label.into();
  Button::new(id)
    .xsmall()
    .label(i18n::tr(&label).to_string())
    .bg(rgb(t::CONTROL))
    .text_color(rgb(t::TEXT))
    .border_1()
    .border_color(rgb(t::BORDER))
}

/// 互斥选项：选中时用强调色描边 + 淡强调底，未选中是安静的面板色。
pub(super) fn choice_button(
  _cx: &App,
  id: impl Into<ElementId>,
  label: impl Into<SharedString>,
  selected: bool,
) -> Button {
  let label: SharedString = label.into();
  Button::new(id)
    .xsmall()
    .label(i18n::tr(&label).to_string())
    .bg(rgb(if selected {
      t::ACCENT_SOFT
    } else {
      t::PANEL_HI
    }))
    .text_color(rgb(if selected { t::TEXT } else { t::MUTED }))
    .border_1()
    .border_color(rgb(if selected { t::ACCENT } else { t::BORDER }))
}

/// 纯文字命令：不出边框、不抢版面（「全部停止」「刷新应用列表」）。
pub(super) fn ghost_button(
  _cx: &App,
  id: impl Into<ElementId>,
  label: impl Into<SharedString>,
) -> Button {
  let label: SharedString = label.into();
  Button::new(id)
    .xsmall()
    .ghost()
    .label(i18n::tr(&label).to_string())
    .text_color(rgb(t::MUTED))
}

/// 破坏性命令：删除节点 / 断开连线。安静，但文字是危险色。
pub(super) fn danger_button(
  _cx: &App,
  id: impl Into<ElementId>,
  label: impl Into<SharedString>,
) -> Button {
  let label: SharedString = label.into();
  Button::new(id)
    .xsmall()
    .ghost()
    .label(i18n::tr(&label).to_string())
    .text_color(rgb(t::DANGER))
}

/// 一行开关：左边标签、右边左右滑动的开关（即时生效）。
pub(super) fn switch_row(
  id: impl Into<ElementId>,
  label: &str,
  checked: bool,
  on_change: impl Fn(&bool, &mut Window, &mut App) + 'static,
) -> Div {
  let label_text = i18n::tr(label).to_string();
  div()
    .flex()
    .items_center()
    .justify_between()
    .gap_3()
    .min_h(px(24.))
    .child(
      div()
        .text_size(px(t::TEXT_SM))
        .text_color(rgb(t::TEXT))
        .child(label_text.clone()),
    )
    .child(
      Switch::new(id)
        .with_size(Size::Small)
        .checked(checked)
        .color(Hsla::from(rgb(t::OK)))
        .accessibility_label(label_text)
        .on_click(on_change),
    )
}

/// 检查器里的「跳过」开关：打开 = 红色（不处理），关闭 = 灰色。
pub(super) fn skip_row(
  id: impl Into<ElementId>,
  skip: bool,
  on_change: impl Fn(&bool, &mut Window, &mut App) + 'static,
) -> Div {
  let label = i18n::tr("跳过").to_string();
  div()
    .flex()
    .items_center()
    .justify_between()
    .gap_3()
    .min_h(px(24.))
    .child(
      div()
        .text_size(px(t::TEXT_SM))
        .text_color(rgb(t::TEXT))
        .child(label.clone()),
    )
    .child(
      Switch::new(id)
        .with_size(Size::Small)
        .checked(skip)
        .color(Hsla::from(rgb(t::DANGER)))
        .accessibility_label(label)
        .on_click(on_change),
    )
}

/// 不带标签的开关（参数行里已经有标签时用）。
pub(super) fn switch_control(
  id: impl Into<ElementId>,
  name: &str,
  checked: bool,
  on_change: impl Fn(&bool, &mut Window, &mut App) + 'static,
) -> Switch {
  Switch::new(id)
    .with_size(Size::Small)
    .checked(checked)
    .color(Hsla::from(rgb(t::OK)))
    .accessibility_label(i18n::tr(name).to_string())
    .on_click(on_change)
}

/// 节点卡片左上角的小开关：绿色 = 参与处理，红色 = 跳过。
///
/// 不写文字（卡片空间宝贵），语义由颜色 + 位置 + tooltip 承担。
pub(super) fn mini_switch(
  id: impl Into<ElementId>,
  on: bool,
  on_click: impl Fn(&gpui_kit::ClickEvent, &mut Window, &mut App) + 'static,
) -> Stateful<Div> {
  div()
    .id(id)
    .flex()
    .items_center()
    .w(px(24.))
    .h(px(12.))
    .p(px(2.))
    .rounded_full()
    .bg(rgb(if on { t::OK } else { t::DANGER }))
    .when(on, |this| this.justify_end())
    .cursor_pointer()
    .aria_label(i18n::tr(if on { "启用" } else { "跳过" }).to_string())
    .on_mouse_down(gpui_kit::MouseButton::Left, |_, _, cx| {
      cx.stop_propagation();
    })
    .child(div().w(px(8.)).h(px(8.)).rounded_full().bg(rgb(t::TEXT)))
    .on_click(on_click)
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
    .mt(px(t::SPACE_HALF))
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

/// 分段 LED 电平（卡片 mini 版 / 可视化面板共用）。
///
/// 格色按**位置**取：绿 ≤ -20 dB、黄 -20..-9、红 > -9（与 `viz.js` / legacy
/// `uitk/viz.py` 同一套阈值）；亮格数 = 当前峰值。`peak` 是线性幅度（1.0 = 0 dBFS）。
pub(super) fn seg_meter(peak: f32, cells: usize, cell_w: f32, cell_h: f32) -> AnyElement {
  let db = 20.0 * peak.max(1e-6).log10();
  let lit = (((db + 60.0) / 60.0).clamp(0.0, 1.0) * cells as f32).round() as usize;
  div()
    .flex()
    .items_end()
    .gap(px(1.))
    .h(px(cell_h))
    .children((0..cells).map(move |i| {
      let r = (i as f32 + 0.5) / cells as f32;
      let zone = if r < 40.0 / 60.0 {
        t::OK
      } else if r < 51.0 / 60.0 {
        t::AEC
      } else {
        t::DANGER
      };
      div().w(px(cell_w)).h(px(cell_h)).bg(if i < lit {
        rgb(zone)
      } else {
        rgba((zone << 8) | 0x30)
      })
    }))
    .into_any_element()
}

/// 在 canvas 里画一段文本（框内刻度 / 峰值标注共用）。
///
/// GPUI 的 canvas 没有 DOM 文本节点，这里显式 shape 一行再 paint；返回的 `ShapedLine`
/// 自带宽度，调用方可据此做右对齐 / 居中。
pub(super) fn shape_text(
  window: &Window,
  text: &str,
  size: f32,
  color: u32,
) -> gpui_kit::ShapedLine {
  let run = gpui_kit::TextRun {
    len: text.len(),
    font: window.text_style().font(),
    color: rgb(color).into(),
    background_color: None,
    underline: None,
    strikethrough: None,
  };
  window.text_system().shape_line(
    gpui_kit::SharedString::from(text.to_string()),
    px(size),
    &[run],
    None,
  )
}
