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
  AnyElement, App, Div, ElementId, Hsla, PathBuilder, SharedString, Stateful, Window, bounds,
  canvas, div, fill, point, px, rgb, rgba,
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
    .w(px(t::CARD_SWITCH_W))
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

/// 卡片里的分段 LED 电平：占满剩余宽度（`flex_1`），格宽由容器决定。
///
/// 格色按**位置**取：绿 ≤ -20 dB、黄 -20..-9、红 > -9（与 `viz.js` / legacy
/// `uitk/viz.py` 同一套阈值）；亮格数 = 当前峰值。`peak` 是线性幅度（1.0 = 0 dBFS）。
/// 卡片第一行只留开关和标题，其余全部交给电平条，所以这里不写死格宽。
pub(super) fn meter_bar(peak: f32, cells: usize, cell_h: f32) -> AnyElement {
  let db = 20.0 * peak.max(1e-6).log10();
  let lit = (((db + 60.0) / 60.0).clamp(0.0, 1.0) * cells as f32).round() as usize;
  div()
    .flex()
    .flex_1()
    .min_w_0()
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
      div().flex_1().min_w_0().h_full().bg(if i < lit {
        rgb(zone)
      } else {
        rgba((zone << 8) | 0x30)
      })
    }))
    .into_any_element()
}

/// 频谱面板：对数频率轴（20 Hz–20 kHz）+ dBFS 纵轴（0…-90），按**电平**分三色。
///
/// 与电平条同一套阈值（绿 ≤ -45、黄 -45..-30、红 > -30），刻度全部画在框里；
/// 61 段是 ISO 1/6 倍频程，所以「等宽柱子 = 对数轴」，低频不会挤成一条线。
/// `height` 由调用方给：检查器用大图，画板卡片用紧凑图（同一份绘制，不另算频谱）。
pub(super) fn spectrum_panel(bands: Vec<f32>, height: f32) -> AnyElement {
  const MIN_DB: f32 = -90.0;
  const AXIS_W: f32 = 34.0;
  const AXIS_H: f32 = 15.0;
  const F_LO: f32 = 20.0;
  const F_HI: f32 = 20480.0;
  /// 显示下限：低于 ~60 Hz 的前几段只覆盖 1–2 个 FFT bin，插值会画出左右翘的锯齿，
  /// 对麦克风降噪也没有意义。**只影响显示**：分析仍然是 20 Hz 起，数组不改。
  const DISPLAY_LO: f32 = 60.0;
  let n = bands.len().max(1);
  div()
    .relative()
    .h(px(height))
    .w_full()
    .rounded_md()
    .bg(rgb(t::BG))
    .border_1()
    .border_color(rgb(t::BORDER))
    .overflow_hidden()
    .child(
      canvas(
        |_, _, _| {},
        move |bounds, _, window, cx| {
          let all_w = bounds.size.width.as_f32();
          let all_h = bounds.size.height.as_f32();
          let w = (all_w - AXIS_W).max(8.0);
          let h = (all_h - AXIS_H).max(8.0);
          let o = bounds.origin;
          let y_of = |db: f32| (-db / -MIN_DB).clamp(0.0, 1.0) * h;
          let grid = rgba((t::BORDER << 8) | 0x80);
          // 横向 dB 网格 + 右侧刻度
          for db in [0.0f32, -20.0, -40.0, -60.0, -80.0] {
            let y = y_of(db);
            let mut pb = PathBuilder::stroke(px(1.));
            pb.move_to(point(o.x, o.y + px(y)));
            pb.line_to(point(o.x + px(w), o.y + px(y)));
            if let Ok(p) = pb.build() {
              window.paint_path(p, grid);
            }
            let line = shape_text(window, &format!("{db:.0}"), 9.0, t::MUTED);
            let lx = all_w - 4.0 - line.width.as_f32();
            let _ = line.paint(
              point(o.x + px(lx), o.y + px((y - 5.0).max(0.0))),
              px(10.),
              gpui_kit::TextAlign::Left,
              None,
              window,
              cx,
            );
          }
          // 只画 >= 60 Hz 的段；前面的极低频段 bin 太少，画出来就是左右翘的锯齿。
          let first = (0..n)
            .find(|i| F_LO * 2f32.powf(*i as f32 / 6.0) >= DISPLAY_LO)
            .unwrap_or(0);
          let vis: Vec<f32> = bands.iter().skip(first).copied().collect();
          let n_vis = vis.len().max(1);
          let f_lo = F_LO * 2f32.powf(first as f32 / 6.0);
          // 纵向频率网格（对数轴）+ 底部刻度
          let x_of = |f: f32| ((f / f_lo).ln() / (F_HI / f_lo).ln()) * w;
          for (f, label) in [(100.0f32, "100"), (1000.0, "1k"), (10000.0, "10k")] {
            let x = x_of(f);
            let mut pb = PathBuilder::stroke(px(1.));
            pb.move_to(point(o.x + px(x), o.y));
            pb.line_to(point(o.x + px(x), o.y + px(h)));
            if let Ok(p) = pb.build() {
              window.paint_path(p, grid);
            }
            let line = shape_text(window, label, 9.0, t::MUTED);
            let lx = (x - line.width.as_f32() / 2.0).clamp(0.0, w - line.width.as_f32());
            let _ = line.paint(
              point(o.x + px(lx), o.y + px(h + 3.0)),
              px(10.),
              gpui_kit::TextAlign::Left,
              None,
              window,
              cx,
            );
          }
          // 柱子：底部 → 顶部按电平分三段着色（填充矩形，不是描线）
          let step = w / n_vis as f32;
          let bw = (step - 1.0).max(1.0);
          for (i, &raw) in vis.iter().enumerate() {
            let db = raw.clamp(MIN_DB, 0.0);
            if db <= MIN_DB + 0.5 {
              continue;
            }
            let x = i as f32 * step;
            // 绿：-90 → min(db, -45)
            let green_top = db.min(-45.0);
            if green_top > MIN_DB + 0.01 {
              paint_bar(
                window,
                o.x.as_f32() + x,
                o.y.as_f32(),
                bw,
                y_of(green_top),
                y_of(MIN_DB),
                t::OK,
              );
            }
            // 黄：-45 → min(db, -30)
            if db > -45.0 {
              let top = db.min(-30.0);
              paint_bar(
                window,
                o.x.as_f32() + x,
                o.y.as_f32(),
                bw,
                y_of(top),
                y_of(-45.0),
                t::AEC,
              );
            }
            // 红：-30 → db
            if db > -30.0 {
              paint_bar(
                window,
                o.x.as_f32() + x,
                o.y.as_f32(),
                bw,
                y_of(db),
                y_of(-30.0),
                t::DANGER,
              );
            }
          }
        },
      )
      .w_full()
      .h_full(),
    )
    .into_any_element()
}

/// 画一根频谱柱的一段（`y0..y1` 是框内坐标，左上为 0）。
fn paint_bar(window: &mut Window, x: f32, y_base: f32, w: f32, y0: f32, y1: f32, color: u32) {
  if y1 <= y0 {
    return;
  }
  window.paint_quad(fill(
    bounds(
      point(px(x), px(y_base + y0)),
      gpui_kit::size(px(w), px(y1 - y0)),
    ),
    rgb(color),
  ));
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
