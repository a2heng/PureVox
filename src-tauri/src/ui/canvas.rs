// PureVox — AI 麦克风降噪工具
// Copyright (C) 2024-2026 a2heng <752848283@qq.com>
//
// PureVox is licensed under the GNU General Public License v3.0 or
// later (GPL-3.0-or-later).  See LICENSE for details.
//
// SPDX-License-Identifier: GPL-3.0-or-later

//! 画板：节点卡片、端口、连线与选中态。只负责“画”和把指针事件翻译成命令，
//! 计划修改走 `Workspace` 的图操作（见 `mod.rs`）。

use gpui_kit::PathBuilder;
use gpui_kit::{
  AnyElement, Context, SharedString, Window, canvas, div, point, prelude::*, px, rgb, rgba,
};

use super::theme as t;
use super::widgets::{meter_bar, mini_switch, shape_text, spectrum_panel};
use super::{
  CANVAS_H, CANVAS_W, DockPage, Drag, DragPayload, PORT_H, PORT_W, PORT_W_WIDE, Pending, Workspace,
  i18n,
};
use crate::plan::{RowSpec, input_ports};

impl Workspace {
  pub(super) fn canvas(&mut self, window: &mut Window, cx: &mut Context<Self>) -> AnyElement {
    let nodes: Vec<RowSpec> = self.plan.nodes().cloned().collect();
    let mut layer = div()
      .id("canvas")
      .relative()
      .w(px(CANVAS_W))
      .h(px(CANVAS_H))
      .bg(rgb(t::BG));

    // 连线 + 端口位置
    type WireView = (u32, u8, u32, u8, (f32, f32), (f32, f32), u32);
    type PortView = (u32, u8, bool, (f32, f32), u32, String, f32);
    let mut wires: Vec<WireView> = Vec::new();
    let mut ports: Vec<PortView> = Vec::new();
    // 正在拉线时吸附到的端子（画白边）
    let snap = match &self.drag {
      Some(Drag::Wire { target, .. }) => *target,
      _ => None,
    };
    for n in &nodes {
      let (w, h) = Self::card_size(n);
      let color = Self::node_color(n.kind, &n.ptype);
      let in_ports = input_ports(&n.ptype, n.channels);
      let out_ports = crate::plan::output_ports(&n.ptype, n.channels);
      for p in 0..in_ports {
        // AEC 的近端 / 远端沿用校准图配色：近端橙、远端蓝；其它节点用角色色。
        let fill = if n.ptype == "echo_cancel" && p == 1 {
          t::NET
        } else {
          color
        };
        ports.push((
          n.id,
          p as u8,
          true,
          Self::port_xy(n, true, p, w, h),
          fill,
          Self::port_text(n, true, p),
          Self::port_width(n, true, p),
        ));
      }
      for p in 0..out_ports {
        ports.push((
          n.id,
          p as u8,
          false,
          Self::port_xy(n, false, p, w, h),
          color,
          Self::port_text(n, false, p),
          Self::port_width(n, false, p),
        ));
      }
      for e in &n.inputs {
        let Some(src) = nodes.iter().find(|x| x.id == e.from) else {
          continue;
        };
        let (sw, sh) = Self::card_size(src);
        let (ax, ay) = Self::port_xy(src, false, e.from_port as usize, sw, sh);
        let (bx, by) = Self::port_xy(n, true, e.to_port as usize, w, h);
        // 线画在端口方块下面，端点取方块外沿，视觉上正好接住端子。
        let a = (ax, ay + PORT_H / 2.0);
        let b = (bx, by - PORT_H / 2.0);
        wires.push((
          e.from,
          e.from_port,
          n.id,
          e.to_port,
          a,
          b,
          Self::node_color(src.kind, &src.ptype),
        ));
      }
    }

    let wire_data = wires.clone();
    // 正在拉的那根线：跟着鼠标（吸附时贴到目标端子外沿）。
    let temp_wire: Option<((f32, f32), (f32, f32))> = match &self.drag {
      Some(Drag::Wire {
        input,
        start_x,
        start_y,
        cur,
        ..
      }) => Some((
        (
          *start_x,
          *start_y + if *input { -PORT_H / 2.0 } else { PORT_H / 2.0 },
        ),
        *cur,
      )),
      _ => None,
    };
    let wire_layer = canvas(
      |_, _, _| {},
      move |bounds, _, window, _cx| {
        for (_from, _fp, _to, _tp, a, b, color) in &wire_data {
          let a = bounds.origin + point(px(a.0), px(a.1));
          let b = bounds.origin + point(px(b.0), px(b.1));
          let mut pb = PathBuilder::stroke(px(2.5));
          pb.move_to(a);
          // 直线：端点就是端口中心，不做贝塞尔绕行，拖动时线也不会甩。
          pb.line_to(b);
          if let Ok(path) = pb.build() {
            window.paint_path(path, rgb(*color));
          }
        }
        if let Some((a, b)) = temp_wire {
          let a = bounds.origin + point(px(a.0), px(a.1));
          let b = bounds.origin + point(px(b.0), px(b.1));
          let mut pb = PathBuilder::stroke(px(2.5));
          pb.move_to(a);
          pb.line_to(b);
          if let Ok(path) = pb.build() {
            window.paint_path(path, rgb(t::ACCENT));
          }
        }
      },
    );
    layer = layer.child(
      div()
        .absolute()
        .top_0()
        .left_0()
        .w_full()
        .h_full()
        .child(wire_layer),
    );

    // 每条线一个常驻的断开按钮（不做 hover-only：看不见的操作等于没有）
    for (from, fp, to, tp, a, b, color) in wires.iter() {
      // 断开按钮固定在上、下端子距离的正中（不再按序号错开，免得看起来偏离连线）。
      let mx = (a.0 + b.0) / 2.0;
      let my = (a.1 + b.1) / 2.0;
      let (f, ff, t2, tt) = (*from, *fp, *to, *tp);
      layer = layer.child(
        div()
          .id(format!("wire-x:{from}:{fp}:{to}:{tp}"))
          .absolute()
          .left(px(mx - 9.0))
          .top(px(my - 9.0))
          .flex()
          .items_center()
          .justify_center()
          .w(px(18.))
          .h(px(18.))
          .rounded_full()
          .bg(rgba((*color << 8) | 0xaa))
          .border_1()
          .border_color(rgb(t::BG))
          .cursor_pointer()
          .text_size(px(t::TEXT_SM))
          .text_color(rgb(0xffffff))
          .child("×")
          .on_click(cx.listener(move |this, _, _, cx| {
            this.disconnect(f, ff, t2, tt, cx);
            cx.notify();
          })),
      );
    }

    // 节点卡片
    for n in &nodes {
      layer = layer.child(self.node_card(n.clone(), window, cx));
    }

    // 端口 + 断开手柄
    for (id, port, input, (x, y), color, label, pw) in &ports {
      let (id, port, input, x, y, color, pw) = (*id, *port, *input, *x, *y, *color, *pw);
      let label = label.clone();
      let mut p = div()
        .id(format!("port:{id}:{input}:{port}"))
        .absolute()
        .left(px(x - pw / 2.0))
        .top(px(y - PORT_H / 2.0))
        .w(px(pw))
        .h(px(PORT_H))
        .flex()
        .items_center()
        .justify_center()
        .rounded_sm()
        .bg(rgb(color))
        .border_1()
        .border_color(rgb(t::BG))
        .text_size(px(if pw > PORT_W { 7. } else { 8. }))
        .text_color(rgb(t::BG))
        .cursor_pointer()
        // 输入/输出端子都是拉线起点：从上往下、从下往上都能拉。
        .on_mouse_down(
          gpui_kit::MouseButton::Left,
          cx.listener(move |this, e: &gpui_kit::MouseDownEvent, _, cx| {
            this.pending = Some(Pending::Port {
              node: id,
              port,
              input,
              pointer: e.position,
              start_x: x,
              start_y: y,
            });
            cx.stop_propagation();
          }),
        )
        .on_drag(
          DragPayload::Port {
            node: id,
            port,
            input,
          },
          |payload: &DragPayload, position, _, cx| cx.new(|_| DragGhost::new(payload, position)),
        )
        .child(label);
      // 吸附目标加一圈白边
      if matches!(snap, Some((sid, sin, sp)) if sid == id && sin == input && sp == port) {
        p = p.border_2().border_color(rgb(0xffffff));
      }
      layer = layer.child(p);
    }

    layer.into_any_element()
  }

  /// 端点上的字：普通节点写通道号，AEC 写「近端 / 远端」。号码和突起的方块
  /// 合成一个东西，不再另开编号条 —— 端点本身就是最自然的标签位置。
  pub(super) fn port_text(n: &RowSpec, input: bool, p: usize) -> String {
    if input && n.ptype == "echo_cancel" {
      i18n::tr(if p == 0 { "近端" } else { "远端" }).to_string()
    } else {
      format!("{}", p + 1)
    }
  }

  pub(super) fn port_width(n: &RowSpec, input: bool, _p: usize) -> f32 {
    if input && n.ptype == "echo_cancel" {
      PORT_W_WIDE
    } else {
      PORT_W
    }
  }

  /// 标题自适应：先按最大字号量一次，放不进首行剩余宽度再按比例缩小（不低于最小字号）。
  fn title_size(window: &Window, title: &str, lane: f32) -> f32 {
    let max = t::CARD_TITLE_MAX;
    let measured = shape_text(window, title, max, t::TEXT).width.as_f32();
    if measured <= lane {
      max
    } else {
      (max * lane / measured).clamp(t::CARD_TITLE_MIN, max)
    }
  }

  pub(super) fn node_card(
    &mut self,
    n: RowSpec,
    window: &Window,
    cx: &mut Context<Self>,
  ) -> AnyElement {
    let (w, h) = Self::card_size(&n);
    let color = Self::node_color(n.kind, &n.ptype);
    let selected = self.selected == Some(n.id);
    let id = n.id;
    // 第一行：彩色标题 + 处理前增益；第二行 VU；第三行设备 / 关键参数。
    let label = i18n::tr(Self::node_kind_label(n.kind, &n.ptype)).to_string();
    let title = label.clone();
    let title_lane =
      (w - t::CARD_PAD * 2.0 - t::CARD_SWITCH_W - t::SPACE_1 * 2.0 - t::CARD_PREGAIN_W).max(24.0);
    let title_size = Self::title_size(window, &title, title_lane);
    let mut detail = self.node_detail(&n, &label);
    if n.channels > 1 {
      if detail.is_empty() {
        detail = format!("{}ch", n.channels);
      } else {
        detail.push_str(&format!(" · {}ch", n.channels));
      }
    }
    let on = n.enabled;
    let (ci, ri) = self.pos(id).unwrap_or((0, 0));
    let peak = self.hub.row_peak(ci, ri);
    let gain = n.input_gain_db.parse::<f32>().unwrap_or(0.0);
    // 处理前增益固定占右侧一条车道：0.0 也画，避免数字左右跳。
    let gain_text = format!("{gain:+.1} dB");
    let gain_ink = if gain.abs() >= 0.05 {
      t::TEXT
    } else {
      t::MUTED
    };
    // 可视化卡片：第一行的 VU 已经继承自普通卡片，这里只补频谱。
    let viz = if n.ptype == "viz" {
      self.viz_bands(ci, ri)
    } else {
      None
    };
    let spectrum: Option<AnyElement> = if n.ptype == "viz" {
      Some(match viz {
        Some(bands) => spectrum_panel(bands, 54.),
        None => div()
          .h(px(54.))
          .w_full()
          .flex()
          .items_center()
          .justify_center()
          .rounded_md()
          .bg(rgb(t::BG))
          .border_1()
          .border_color(rgb(t::BORDER))
          .text_size(px(t::TEXT_XS))
          .text_color(rgb(t::MUTED))
          .child(i18n::tr("引擎未运行").to_string())
          .into_any_element(),
      })
    } else {
      None
    };
    div()
      .id(format!("node:{id}"))
      .absolute()
      .left(px(n.x))
      .top(px(n.y))
      .flex()
      .flex_col()
      .w(px(w))
      .h(px(h))
      .rounded_lg()
      .bg(rgb(if selected { t::PANEL_HI } else { t::PANEL }))
      .border_1()
      .border_color(rgb(if selected { t::ACCENT } else { t::BORDER }))
      .opacity(if on { 1.0 } else { 0.55 })
      .cursor_grab()
      .on_mouse_down(
        gpui_kit::MouseButton::Left,
        cx.listener(move |this, e: &gpui_kit::MouseDownEvent, _, cx| {
          this.selected = Some(id);
          this.dock_page = DockPage::Node;
          this.pending = Some(Pending::Node {
            id,
            pointer: e.position,
            start_x: this.plan.node(id).map(|n| n.x).unwrap_or(0.0),
            start_y: this.plan.node(id).map(|n| n.y).unwrap_or(0.0),
          });
          cx.notify();
        }),
      )
      .on_drag(
        DragPayload::Move { id },
        |payload: &DragPayload, position, _, cx| cx.new(|_| DragGhost::new(payload, position)),
      )
      // 卡片内容从上下端点的内侧开始：端点方块侵入卡片 7 px，留 8 px 不打架。
      .child(
        div()
          .flex()
          .flex_col()
          .flex_1()
          .min_h_0()
          .gap(px(t::SPACE_HALF))
          .pt(px(PORT_H / 2.0 + 1.0))
          .pb(px(PORT_H / 2.0 + 1.0))
          // 第一行：跳过开关 · 彩色标题 · 处理前增益。标题占用剩余宽度并自动缩字号。
          .child(
            div()
              .flex()
              .items_center()
              .gap_1()
              .h(px(14.))
              .px(px(t::CARD_PAD))
              .child(mini_switch(
                format!("skip:{id}"),
                on,
                cx.listener(move |this, _, _, cx| {
                  this.snapshot();
                  if let Some(n) = this.plan.node_mut(id) {
                    n.enabled = !n.enabled;
                  }
                  this.apply_structure(cx);
                }),
              ))
              .child(
                div()
                  .flex_1()
                  .min_w_0()
                  .truncate()
                  .text_size(px(title_size))
                  .text_color(rgb(color))
                  .child(title),
              )
              .child(
                div()
                  .w(px(t::CARD_PREGAIN_W))
                  .flex_shrink_0()
                  .text_right()
                  .text_size(px(9.))
                  .text_color(rgb(gain_ink))
                  .child(gain_text),
              ),
          )
          // 第二行：VU 铺满整行。
          .child(
            div()
              .flex()
              .items_center()
              .h(px(8.))
              .px(px(t::CARD_PAD))
              .child(meter_bar(peak, 24, 8.)),
          )
          // 第三行：小字详情（应用名 / 设备名 / 关键参数）。
          .child(
            div()
              .flex()
              .items_center()
              .h(px(12.))
              .px(px(t::CARD_PAD))
              .child(
                div()
                  .flex_1()
                  .min_w_0()
                  .truncate()
                  .text_size(px(t::TEXT_XS))
                  .text_color(rgb(t::MUTED))
                  .child(detail),
              ),
          )
          .when_some(spectrum, |card, spectrum| {
            card.child(div().px(px(t::CARD_PAD)).child(spectrum))
          }),
      )
      .into_any_element()
  }
}

struct DragGhost {
  label: SharedString,
}

impl DragGhost {
  fn new(payload: &DragPayload, _position: gpui_kit::Point<gpui_kit::Pixels>) -> Self {
    let label = match payload {
      DragPayload::Move { .. } => "",
      DragPayload::Port { .. } => "",
    };
    DragGhost {
      label: label.into(),
    }
  }
}

impl Render for DragGhost {
  fn render(&mut self, _: &mut Window, _: &mut Context<Self>) -> impl IntoElement {
    if self.label.is_empty() {
      // 拖节点本身时不要 ghost：节点就是自己的预览，多一个浮层只会拖慢手感。
      div().w(px(1.)).h(px(1.))
    } else {
      div()
        .px(px(t::SPACE_2))
        .py(px(t::SPACE_1))
        .rounded_md()
        .bg(rgb(t::ACCENT))
        .text_size(px(t::TEXT_XS))
        .text_color(rgb(0xffffff))
        .child(self.label.clone())
    }
  }
}
