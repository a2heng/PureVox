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
use super::widgets::{mini_switch, seg_meter};
use super::{
  CANVAS_H, CANVAS_W, DockPage, Drag, DragPayload, PORT_SIZE, Pending, TERM_H, TERM_PAD, TERM_SLOT,
  Workspace, i18n,
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
          i18n::tr("近端").to_string()
        } else {
          i18n::tr("远端").to_string()
        }
      } else {
        format!("{}", p + 1)
      }
    };
    // AEC 的两个端子用校准图里的颜色：近端（麦克风）= 橙、远端（参考）= 蓝。
    let aec_term = n.ptype == "echo_cancel";
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
    // 没有具体副标题（默认模型 / 纯处理）时不要写「处理」凑数。
    let title = if title == "处理" || title == "降噪" {
      String::new()
    } else {
      title
    };
    let on = n.enabled;
    let (ci, ri) = self.pos(id).unwrap_or((0, 0));
    let peak = self.hub.row_peak(ci, ri);
    let gain = n.input_gain_db.parse::<f32>().unwrap_or(0.0);
    // 扇入是 1:1 求和，多路叠加会超过 0 dBFS —— 电平条变红就是削波提示。
    let clipping = peak >= 1.0;
    // 一行读完：类型 · 主标题。标题和类型重复时只写一次。
    let primary = if title.is_empty() || title == label || label.contains(&title) {
      label
    } else {
      format!("{label} · {title}")
    };
    // 0.0 dB / 1ch 是默认值，画出来只是噪音；但**位置要保留**，否则各卡片的
    // 电平条和数字会左右跳。右侧三条固定车道：电平 34 / 数值 32 / 通道 16。
    let show_db = gain.abs() >= 0.05;
    let show_ch = n.channels > 1;
    let db_text = if clipping {
      "CLIP".to_string()
    } else if show_db {
      format!("{gain:+.1}")
    } else {
      String::new()
    };
    let ch_text = if show_ch {
      format!("{}ch", n.channels.max(1))
    } else {
      String::new()
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
      .when(ins > 1, |card| {
        let mut strip = div().flex().h(px(TERM_H)).pl(px(TERM_PAD)).items_end();
        for p in 0..ins {
          // AEC 的「近端 / 远端」是两个字，标签要比通道号宽一点、并换成校准图同色，
          // 否则文字会溢出小方框，看起来像底下多了一个圈。
          let ink = if aec_term {
            if p == 0 { t::AEC } else { t::NET }
          } else {
            t::TEXT
          };
          let edge = if aec_term { ink } else { t::MUTED };
          strip = strip.child(
            div()
              .w(px(TERM_SLOT))
              .flex()
              .justify_center()
              .when(aec_term, |el| el.pb(px(0.)))
              .when(!aec_term, |el| el.pb(px(3.)))
              .child(
                div()
                  .when(aec_term, |el| el.w(px(21.)).h(px(11.)).px(px(1.)))
                  .when(!aec_term, |el| el.w(px(13.)).h(px(10.)))
                  .flex()
                  .items_center()
                  .justify_center()
                  .rounded_sm()
                  .border_1()
                  .border_color(rgb(edge))
                  .bg(rgb(t::BG))
                  .text_size(px(if aec_term {
                    8.
                  } else if ins > 9 {
                    7.
                  } else {
                    8.
                  }))
                  .text_color(rgb(ink))
                  .child(term(p)),
              ),
          );
        }
        card.child(strip)
      })
      // 一行读完一张卡：跳过开关、节点类型、主标题、电平、增益、通道。
      // 没有用的信息（0.0 dB / 1ch）不占位置；端子编号只在多通道时出现。
      .child(
        div()
          .flex()
          .flex_1()
          .min_h_0()
          .items_center()
          .gap_1()
          .px(px(6.))
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
          .child(div().w(px(5.)).h(px(5.)).rounded_full().bg(rgb(color)))
          .child(
            div()
              .flex_1()
              .min_w_0()
              .truncate()
              .text_size(px(t::TEXT_SM))
              .text_color(rgb(t::TEXT))
              .child(primary),
          )
          .child(
            div()
              .flex()
              .items_center()
              .gap_1()
              .flex_shrink_0()
              .child(seg_meter(peak, 12, 2., 7.))
              .child(
                div()
                  .w(px(32.))
                  .text_right()
                  .text_size(px(9.))
                  .text_color(rgb(if clipping { t::DANGER } else { t::MUTED }))
                  .child(db_text),
              )
              .child(
                div()
                  .w(px(16.))
                  .text_right()
                  .text_size(px(9.))
                  .text_color(rgb(t::MUTED))
                  .child(ch_text),
              ),
          ),
      )
      .when(outs > 1, |card| {
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
                  .w(px(13.))
                  .h(px(10.))
                  .flex()
                  .items_center()
                  .justify_center()
                  .rounded_sm()
                  .border_1()
                  .border_color(rgb(t::MUTED))
                  .bg(rgb(t::BG))
                  .text_size(px(if outs > 9 { 7. } else { 8. }))
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
