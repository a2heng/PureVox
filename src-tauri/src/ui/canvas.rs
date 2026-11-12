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
use super::{
  CANVAS_H, CANVAS_W, DockPage, Drag, DragPayload, HEADER_H, PORT_SIZE, Pending, TERM_H, TERM_PAD,
  TERM_SLOT, Workspace, i18n,
};
use crate::plan::{RowKind, RowSpec, input_ports};

impl Workspace {
  pub(super) fn canvas(&mut self, _window: &mut Window, cx: &mut Context<Self>) -> AnyElement {
    let nodes: Vec<RowSpec> = self.plan.nodes().cloned().collect();
    let mut layer = div()
      .id("canvas")
      .relative()
      .w(px(CANVAS_W))
      .h(px(CANVAS_H))
      .bg(rgb(t::BG));

    // 连线 + 端口位置
    type WireView = (u32, u8, u32, u8, (f32, f32), (f32, f32), u32);
    type PortView = (u32, u8, bool, (f32, f32), u32);
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
        ports.push((n.id, p as u8, true, Self::port_xy(n, true, p, w, h), color));
      }
      for p in 0..out_ports {
        ports.push((
          n.id,
          p as u8,
          false,
          Self::port_xy(n, false, p, w, h),
          color,
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
        let a = (ax, ay + PORT_SIZE / 2.0);
        let b = (bx, by - PORT_SIZE / 2.0);
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
          *start_y
            + if *input {
              -PORT_SIZE / 2.0
            } else {
              PORT_SIZE / 2.0
            },
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
    for (wi, (from, fp, to, tp, a, b, color)) in wires.iter().enumerate() {
      // 平行线错开断开按钮，避免几个 × 叠在一起。
      let t = 0.5 + ((wi % 3) as f32 - 1.0) * 0.14;
      let mx = a.0 + (b.0 - a.0) * t;
      let my = a.1 + (b.1 - a.1) * t;
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
      layer = layer.child(self.node_card(n.clone(), cx));
    }

    // 端口 + 断开手柄
    for (id, port, input, (x, y), color) in &ports {
      let (id, port, input, x, y) = (*id, *port, *input, *x, *y);
      let mut p = div()
        .id(format!("port:{id}:{input}:{port}"))
        .absolute()
        .left(px(x - PORT_SIZE / 2.0))
        .top(px(y - PORT_SIZE / 2.0))
        .w(px(PORT_SIZE))
        .h(px(PORT_SIZE))
        .rounded_sm()
        .bg(rgb(*color))
        .border_1()
        .border_color(rgb(t::BG))
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
        );
      // 吸附目标加一圈白边
      if matches!(snap, Some((sid, sin, sp)) if sid == id && sin == input && sp == port) {
        p = p.border_2().border_color(rgb(0xffffff));
      }
      layer = layer.child(p);
    }

    layer.into_any_element()
  }

  pub(super) fn node_card(&self, n: RowSpec, cx: &mut Context<Self>) -> AnyElement {
    let (w, h) = Self::card_size(&n);
    let color = Self::node_color(n.kind, &n.ptype);
    let ins = input_ports(&n.ptype, n.channels);
    let outs = crate::plan::output_ports(&n.ptype, n.channels);
    // 端子只显示通道号（AEC 例外：近端 / 远端）。不写 IN/OUT 英文，
    // 上下位置已经说明了方向；号码做成方框标签，一眼能数清。
    let term = |p: usize| -> String {
      if n.ptype == "echo_cancel" {
        if p == 0 {
          "近端".into()
        } else {
          "远端".into()
        }
      } else {
        format!("{}", p + 1)
      }
    };
    let selected = self.selected == Some(n.id);
    let id = n.id;
    let label = i18n::tr(Self::node_kind_label(n.kind, &n.ptype)).to_string();
    let title = if n.kind == RowKind::Process {
      match n.ptype.as_str() {
        "gain" => n
          .params
          .get("gain_db")
          .map(|v| format!("{v} dB"))
          .unwrap_or_else(|| "增益".into()),
        "denoise" => n
          .params
          .get("model")
          .map(|m| {
            crate::infer::denoise_models()
              .iter()
              .find(|(f, _)| *f == m)
              .map(|(_, l)| i18n::tr(l).to_string())
              .unwrap_or_else(|| i18n::tr("降噪").into())
          })
          .unwrap_or_else(|| "降噪".into()),
        "echo_cancel" => "近端 + 远端".into(),
        _ => i18n::tr("处理").into(),
      }
    } else {
      self.node_title(&n)
    };
    let on = n.enabled;
    let (ci, ri) = self.pos(id).unwrap_or((0, 0));
    let peak = self.hub.row_peak(ci, ri);
    let gain = n.input_gain_db.parse::<f32>().unwrap_or(0.0);
    // 扇入是 1:1 求和，多路叠加会超过 0 dBFS —— 电平条变红就是削波提示。
    let clipping = peak >= 1.0;
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
      .when(ins > 0, |card| {
        let mut strip = div().flex().h(px(TERM_H)).pl(px(TERM_PAD)).items_end();
        for p in 0..ins {
          strip = strip.child(
            div()
              .w(px(TERM_SLOT))
              .flex()
              .justify_center()
              .pb(px(3.))
              .child(
                div()
                  .w(px(14.))
                  .h(px(12.))
                  .flex()
                  .items_center()
                  .justify_center()
                  .rounded_sm()
                  .border_1()
                  .border_color(rgb(t::MUTED))
                  .bg(rgb(t::BG))
                  .text_size(px(if ins > 9 { 8. } else { 9. }))
                  .text_color(rgb(t::TEXT))
                  .child(term(p)),
              ),
          );
        }
        card.child(strip)
      })
      .child(
        div()
          .flex()
          .items_center()
          .gap_2()
          .h(px(HEADER_H))
          .px(px(t::SPACE_2))
          .border_b_1()
          .border_color(rgb(t::BORDER))
          .child(div().w(px(6.)).h(px(6.)).rounded_full().bg(rgb(color)))
          .child(
            div()
              .text_size(px(t::TEXT_XS))
              .text_color(rgb(color))
              .child(label),
          )
          .child(div().flex_1())
          .child(
            div()
              .text_size(px(9.))
              .text_color(rgb(if on { t::MUTED } else { t::DANGER }))
              .child(if on {
                i18n::tr("启用")
              } else {
                i18n::tr("静音")
              }),
          ),
      )
      .when(n.ptype == "echo_cancel", |card| {
        card.child(div().h(px(26.)).w_full().px(px(6.)).child(canvas(
          |_, _, _| {},
          move |bounds, _, window, _| {
            let w = bounds.size.width.as_f32();
            let h = bounds.size.height.as_f32();
            let mut pb = PathBuilder::stroke(px(1.5));
            pb.move_to(bounds.origin + point(px(w * 0.30), px(2.)));
            pb.line_to(bounds.origin + point(px(w * 0.50), px(h * 0.55)));
            pb.line_to(bounds.origin + point(px(w * 0.50), px(h - 1.)));
            if let Ok(p) = pb.build() {
              window.paint_path(p, rgb(t::AEC));
            }
            let mut pb = PathBuilder::stroke(px(1.5));
            pb.move_to(bounds.origin + point(px(w * 0.70), px(2.)));
            pb.line_to(bounds.origin + point(px(w * 0.50), px(h * 0.55)));
            if let Ok(p) = pb.build() {
              window.paint_path(p, rgb(t::AEC));
            }
          },
        )))
      })
      .child(
        div()
          .flex()
          .flex_col()
          .gap_1()
          .px(px(t::SPACE_2))
          .py(px(t::SPACE_1))
          .child(
            div()
              .text_size(px(t::TEXT_BASE))
              .text_color(rgb(t::TEXT))
              .truncate()
              .child(title),
          )
          .child(
            div()
              .flex()
              .items_center()
              .gap_2()
              .child(
                div()
                  .w(px(58.))
                  .h(px(4.))
                  .rounded_sm()
                  .bg(rgb(t::BG))
                  .child(
                    div()
                      .w(px((peak.clamp(0.0, 1.2) * 58.0).min(58.0)))
                      .h(px(4.))
                      .rounded_sm()
                      .bg(rgb(if clipping { t::DANGER } else { color })),
                  ),
              )
              .child(
                div()
                  .text_size(px(9.))
                  .text_color(rgb(if clipping { t::DANGER } else { t::MUTED }))
                  .child(format!("{gain:+.1} dB")),
              )
              .child(div().flex_1())
              .child(
                div()
                  .text_size(px(9.))
                  .text_color(rgb(t::MUTED))
                  .child(format!("{}ch", n.channels.max(1))),
              ),
          ),
      )
      .when(outs > 0, |card| {
        let mut strip = div().flex().h(px(TERM_H)).pl(px(TERM_PAD)).items_start();
        for p in 0..outs {
          strip = strip.child(
            div()
              .w(px(TERM_SLOT))
              .flex()
              .justify_center()
              .pt(px(3.))
              .child(
                div()
                  .w(px(14.))
                  .h(px(12.))
                  .flex()
                  .items_center()
                  .justify_center()
                  .rounded_sm()
                  .border_1()
                  .border_color(rgb(t::MUTED))
                  .bg(rgb(t::BG))
                  .text_size(px(if outs > 9 { 8. } else { 9. }))
                  .text_color(rgb(t::TEXT))
                  .child(term(p)),
              ),
          );
        }
        card.child(strip)
      })
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
