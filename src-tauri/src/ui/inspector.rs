// PureVox — AI 麦克风降噪工具
// Copyright (C) 2024-2026 a2heng <752848283@qq.com>
//
// PureVox is licensed under the GNU General Public License v3.0 or
// later (GPL-3.0-or-later).  See LICENSE for details.
//
// SPDX-License-Identifier: GPL-3.0-or-later

use gpui_kit::component::slider::{Slider, SliderEvent, SliderState, SliderValue};
use gpui_kit::{
  AnyElement, Context, Div, PathBuilder, Window, canvas, div, point, prelude::*, px, rgb,
};

use super::theme as t;
use super::widgets::*;
use super::{EqDrag, EqGeom, NodeModule, PendingMedia, RecordTarget, Workspace, i18n, node_module};
use crate::engine::hot::EqSettings;
use crate::plan::{RowKind, RowSpec};

impl Workspace {
  // ── 节点检查器：只渲染当前选中的节点 ─────────────────────────────────

  pub(super) fn inspect_node(
    &mut self,
    n: RowSpec,
    window: &mut Window,
    cx: &mut Context<Self>,
  ) -> AnyElement {
    let id = n.id;
    let color = Self::node_color(n.kind, &n.ptype);
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
    let enabled = n.enabled;
    let (ci, ri) = self.pos(id).unwrap_or((0, 0));
    let peak = self.hub.row_peak(ci, ri);
    let latency = self.latency_short(ci);

    // ── 操作（上）────────────────────────────────────────────────────
    let entity = cx.entity();
    let mut list = div()
      .flex()
      .flex_col()
      .gap_2()
      .child(
        div()
          .flex()
          .min_w_0()
          .items_center()
          .justify_between()
          .child(
            div()
              .flex()
              .flex_1()
              .min_w_0()
              .items_center()
              .gap_2()
              .child(div().w(px(8.)).h(px(8.)).rounded_full().bg(rgb(color)))
              .child(
                div()
                  .flex_1()
                  .min_w_0()
                  .truncate()
                  .text_size(px(t::TEXT_LG))
                  .child(title),
              ),
          )
          .child(
            danger_button(cx, format!("del:{id}"), "删除").on_click(cx.listener(
              move |this, _, _window, cx| {
                this.delete_node(id, cx);
              },
            )),
          ),
      )
      .child(section("操作"))
      .child(skip_row(
        format!("skip:{id}"),
        !enabled,
        move |next, _, cx| {
          let skip = *next;
          entity.update(cx, |this, cx| {
            this.snapshot();
            if let Some(n) = this.plan.node_mut(id) {
              n.enabled = !skip;
            }
            this.apply_structure(cx);
          });
        },
      ));

    // 媒体源（音效板 / 播放器）：文件选择 → 库内 WAV → 重建。
    if n.ptype == "soundpad" || n.ptype == "player" {
      let kind = if n.ptype == "soundpad" {
        crate::media::MediaKind::Soundpad
      } else {
        crate::media::MediaKind::Music
      };
      list = list.child(section("媒体"));
      list = list.child(
        quiet_button(cx, format!("pick:{id}"), "选择音频文件…").on_click(cx.listener(
          move |this, _, _window, cx| {
            let slot = this.media_pick.clone();
            let node = id;
            std::thread::spawn(move || {
              let picked = rfd::FileDialog::new()
                .add_filter("音频文件", &["wav", "mp3", "flac", "ogg", "m4a", "aac"])
                .pick_file();
              let Some(p) = picked else {
                return;
              };
              let fname = p
                .file_name()
                .and_then(|s| s.to_str())
                .unwrap_or("audio")
                .to_string();
              let result = match std::fs::read(&p) {
                Ok(bytes) => crate::media::import(&bytes, &fname, kind),
                Err(e) => Err(format!("读取文件失败：{e}")),
              };
              *slot.lock().unwrap_or_else(|e| e.into_inner()) =
                Some(PendingMedia { node, kind, result });
            });
            this.status = "选择音频文件…".into();
            cx.notify();
          },
        )),
      );
      if n.ptype == "player" {
        let cur = n.params.get("path").cloned().unwrap_or_default();
        let shown = if cur.is_empty() {
          "（未选择）".to_string()
        } else {
          std::path::Path::new(&cur)
            .file_name()
            .and_then(|s| s.to_str())
            .unwrap_or(&cur)
            .to_string()
        };
        list = list.child(field("当前曲目", shown)).child(
          div()
            .flex()
            .gap_1()
            .child(
              quiet_button(cx, format!("pplay:{id}"), "播放").on_click(cx.listener(
                move |this, _, _, cx| {
                  if let Some((ci, ri)) = this.pos(id)
                    && let Some(s) = crate::engine::media::hub().slot(ci, ri)
                  {
                    s.send(crate::engine::media::MediaCmd::PlayerPlay);
                    this.status = "播放器：播放".into();
                  }
                  cx.notify();
                },
              )),
            )
            .child(
              quiet_button(cx, format!("ppause:{id}"), "暂停").on_click(cx.listener(
                move |this, _, _, cx| {
                  if let Some((ci, ri)) = this.pos(id)
                    && let Some(s) = crate::engine::media::hub().slot(ci, ri)
                  {
                    s.send(crate::engine::media::MediaCmd::PlayerPause);
                    this.status = "播放器：暂停".into();
                  }
                  cx.notify();
                },
              )),
            ),
        );
      } else {
        let pads: Vec<serde_json::Value> = n
          .params
          .get("pads")
          .and_then(|s| serde_json::from_str(s).ok())
          .unwrap_or_default();
        if pads.is_empty() {
          list = list.child(hint("还没有音效：点上面的按钮导入。"));
        }
        for (i, pad) in pads.iter().enumerate() {
          let name = pad
            .get("name")
            .and_then(|v| v.as_str())
            .unwrap_or("音效")
            .to_string();
          let hotkey = pad
            .get("hotkey")
            .and_then(|v| v.as_str())
            .unwrap_or("")
            .to_string();
          let recording = matches!(
            self.recording,
            Some(RecordTarget::Pad { node, index }) if node == id && index == i
          );
          list = list.child(
            div()
              .flex()
              .flex_col()
              .gap_1()
              .child(
                quiet_button(cx, format!("pad:{id}:{i}"), format!("▶ {name}")).on_click(
                  cx.listener(move |this, _, _, cx| {
                    if let Some((ci, ri)) = this.pos(id)
                      && let Some(s) = crate::engine::media::hub().slot(ci, ri)
                    {
                      s.send(crate::engine::media::MediaCmd::SoundpadPlay(i));
                      this.status = format!("播放音效：{name}");
                    } else {
                      this.status = "引擎未运行：先启动再播放".into();
                    }
                    cx.notify();
                  }),
                ),
              )
              .child(
                div()
                  .flex()
                  .items_center()
                  .gap_1()
                  .child(
                    div()
                      .flex_1()
                      .text_size(px(t::TEXT_XS))
                      .text_color(rgb(t::MUTED))
                      .child(if hotkey.is_empty() {
                        i18n::tr("热键 —").to_string()
                      } else {
                        format!("{} {hotkey}", i18n::tr("热键"))
                      }),
                  )
                  .child(
                    quiet_button(
                      cx,
                      format!("padkey:{id}:{i}"),
                      if recording { "按键…" } else { "录制" },
                    )
                    .on_click(cx.listener(move |this, _, _, cx| {
                      if recording {
                        this.end_record();
                        this.status = i18n::tr("已取消录制热键").into();
                        cx.notify();
                      } else {
                        this.begin_record(
                          RecordTarget::Pad { node: id, index: i },
                          "按下这个音效的热键…（Esc 取消）",
                          cx,
                        );
                      }
                    })),
                  ),
              ),
          );
        }
        if !pads.is_empty() {
          list = list.child(
            ghost_button(cx, format!("padstop:{id}"), "全部停止").on_click(cx.listener(
              move |this, _, _, cx| {
                if let Some((ci, ri)) = this.pos(id)
                  && let Some(s) = crate::engine::media::hub().slot(ci, ri)
                {
                  s.send(crate::engine::media::MediaCmd::SoundpadStopAll);
                  this.status = "音效板：全部停止".into();
                }
                cx.notify();
              },
            )),
          );
        }
      }
    }

    // 源头 / 出口：当前设备只在标题里显示一次，这里是唯一的**选择**入口。
    if crate::plan::input_ports(&n.ptype, n.channels) == 0 {
      match n.ptype.as_str() {
        "audio_input" => {
          list = list.child(section("输入源"));
          for d in self
            .devices
            .iter()
            .filter(|d| d.direction == "input")
            .cloned()
          {
            let on = n.device.as_deref() == Some(d.id.as_str());
            let did = d.id.clone();
            let dname = d.name.clone();
            list = list.child(
              choice_button(cx, format!("dev:{id}:{}", d.id), d.name, on).on_click(cx.listener(
                move |this, _, _window, cx| {
                  this.snapshot();
                  if let Some(n) = this.plan.node_mut(id) {
                    n.device = Some(did.clone());
                    n.device_name = Some(dname.clone());
                  }
                  this.apply_structure(cx);
                },
              )),
            );
          }
        }
        "app_capture" => {
          list = list.child(section("应用音频"));
          list = list.child(
            ghost_button(cx, format!("applist:{id}"), "刷新应用列表").on_click(cx.listener(
              |this, _, _window, cx| {
                match crate::audio::app_capture::list_apps() {
                  Ok(v) => {
                    this.status = format!("发现 {} 个正在出声的应用", v.len());
                    this.app_list = v;
                  }
                  Err(e) => this.status = e,
                }
                cx.notify();
              },
            )),
          );
          if self.app_list.is_empty() {
            list = list.child(hint(
              "只列出正在出声的应用（Windows 10 2004+）；点「刷新应用列表」。",
            ));
          }
          for (pid, name) in self.app_list.clone() {
            let dev = format!("app:{pid}");
            let on = n.device.as_deref() == Some(dev.as_str());
            list = list.child(
              choice_button(
                cx,
                format!("app:{id}:{pid}"),
                format!("{name}  ·  PID {pid}"),
                on,
              )
              .on_click(cx.listener(move |this, _, _window, cx| {
                this.snapshot();
                if let Some(n) = this.plan.node_mut(id) {
                  n.device = Some(dev.clone());
                  n.device_name = Some(name.clone());
                }
                this.apply_structure(cx);
              })),
            );
          }
          if let Some(dev) = &n.device
            && dev.starts_with("app:")
          {
            list = list.child(field("当前应用", n.device_name.clone().unwrap_or_default()));
          }
        }
        "loopback" => {
          list = list.child(section("系统声音"));
          let on = n.device.as_deref().map(|d| d == "loopback").unwrap_or(true);
          list = list.child(
            choice_button(cx, format!("loop:{id}"), "系统默认输出", on).on_click(cx.listener(
              move |this, _, _window, cx| {
                this.snapshot();
                if let Some(n) = this.plan.node_mut(id) {
                  n.device = Some("loopback".into());
                  n.device_name = Some("系统声音（默认输出）".into());
                }
                this.apply_structure(cx);
              },
            )),
          );
          for t in crate::devices::loopback_targets() {
            let on = n.device.as_deref() == Some(t.id.as_str());
            let did = t.id.clone();
            let dname = t.name.clone();
            list = list.child(
              choice_button(cx, format!("loop:{id}:{}", t.id), t.name, on).on_click(cx.listener(
                move |this, _, _window, cx| {
                  this.snapshot();
                  if let Some(n) = this.plan.node_mut(id) {
                    n.device = Some(did.clone());
                    n.device_name = Some(dname.clone());
                  }
                  this.apply_structure(cx);
                },
              )),
            );
          }
        }
        _ => {}
      }
    }

    if crate::plan::output_ports(&n.ptype, n.channels) == 0 {
      list = list.child(section("输出设备"));
      for d in self
        .devices
        .iter()
        .filter(|d| d.direction == "output")
        .cloned()
      {
        let on = n.device.as_deref() == Some(d.id.as_str());
        let did = d.id.clone();
        let dname = d.name.clone();
        list = list.child(
          choice_button(cx, format!("odev:{id}:{}", d.id), d.name, on).on_click(cx.listener(
            move |this, _, _window, cx| {
              this.snapshot();
              if let Some(n) = this.plan.node_mut(id) {
                n.device = Some(did.clone());
                n.device_name = Some(dname.clone());
              }
              this.apply_structure(cx);
            },
          )),
        );
      }
    }

    list = list
      .child(section("通道"))
      .child(
        div()
          .flex()
          .items_center()
          .gap_2()
          .child(
            quiet_button(cx, format!("ch-:{id}"), "−").on_click(cx.listener(
              move |this, _, _window, cx| {
                this.set_channels(id, -1, cx);
              },
            )),
          )
          .child(
            div()
              .text_size(px(t::TEXT_BASE))
              .child(format!("{} 通道", n.channels.max(1))),
          )
          .child(
            quiet_button(cx, format!("ch+:{id}"), "+").on_click(cx.listener(
              move |this, _, _window, cx| {
                this.set_channels(id, 1, cx);
              },
            )),
          )
          .child(
            div()
              .text_size(px(t::TEXT_XS))
              .text_color(rgb(t::MUTED))
              .child(format!("上限 {}", self.node_max_channels(&n))),
          ),
      )
      .child(section("输入微调"))
      .child(self.db_slider(
        id,
        "input_gain_db",
        -24.0,
        24.0,
        n.input_gain_db.clone(),
        window,
        cx,
      ));

    if n.ptype == "gain" {
      let cur = n
        .params
        .get("gain_db")
        .cloned()
        .unwrap_or_else(|| "0.0".into());
      list = list
        .child(section("增益"))
        .child(self.db_slider(id, "gain_db", -40.0, 24.0, cur, window, cx));
    } else if node_module(&n.ptype) == NodeModule::Aec {
      list = list
        .child(section("AEC"))
        .child(self.params_editor(id, cx))
        .child(self.aec_card(id, cx));
    } else if n.ptype == "eq31" {
      list = list.child(section("均衡")).child(self.eq_editor(id, cx));
    } else if n.ptype == "viz" {
      list = list
        .child(section("可视化"))
        .child(self.params_editor(id, cx))
        .child(self.viz_editor(id, cx));
    } else if n.kind != RowKind::Output
      && crate::engine::registry::get_spec(&n.ptype)
        .map(|s| !s.params.is_empty())
        .unwrap_or(false)
    {
      list = list
        .child(section("参数"))
        .child(self.params_editor(id, cx));
    }

    if n.ptype == "tse" {
      let path = crate::config::default_tse_reference();
      let exists = path.exists();
      let rec = self.audio.recorder_status();
      let busy = matches!(
        rec.phase,
        crate::recorder::RecorderPhase::Recording
          | crate::recorder::RecorderPhase::Finishing
          | crate::recorder::RecorderPhase::Encoding
      );
      let label = if busy {
        "录制中…"
      } else if exists {
        "重新录制 10 秒参考"
      } else {
        "录制 10 秒参考"
      };
      let mut rec_btn = quiet_button(cx, format!("rec:{id}"), label);
      if !busy {
        rec_btn = rec_btn.on_click(cx.listener(move |this, _, _, cx| {
          let mgr = this.audio.clone();
          match mgr.record_reference(Some(id)) {
            Ok(p) => this.status = format!("开始录制 TSE 参考，完成后覆盖 {p}"),
            Err(e) => this.status = e,
          }
          cx.notify();
        }));
      }
      list = list.child(section("参考语音")).child(rec_btn);
      if busy {
        let fraction = if rec.phase == crate::recorder::RecorderPhase::Encoding {
          1.0
        } else if rec.total_s > 0.0 {
          (rec.elapsed_s / rec.total_s) as f32
        } else {
          0.0
        };
        let text = match rec.phase {
          crate::recorder::RecorderPhase::Recording if rec.elapsed_s <= 0.0 => {
            i18n::tr("准备中…（0.5 s 后开始计数）").to_string()
          }
          crate::recorder::RecorderPhase::Recording => format!(
            "{} {:.1}/{:.0} s",
            i18n::tr("录制中"),
            rec.elapsed_s,
            rec.total_s
          ),
          crate::recorder::RecorderPhase::Finishing => i18n::tr("收尾中…").to_string(),
          crate::recorder::RecorderPhase::Encoding => i18n::tr("正在写入参考文件…").to_string(),
          _ => String::new(),
        };
        list = list
          .child(progress_bar(
            fraction,
            if rec.phase == crate::recorder::RecorderPhase::Encoding {
              t::ACCENT
            } else {
              t::OK
            },
          ))
          .child(hint(text));
      } else {
        list = list.child(hint(if exists {
          format!(
            "{}{}{}",
            i18n::tr("当前参考："),
            path.display(),
            i18n::tr("（重新录制会覆盖）")
          )
        } else {
          i18n::tr("还没录（TSE 直通）").to_string()
        }));
      }
      list = list.child(hint(
        "录的是进入 TSE 节点、经过节点输入增益之后的信号，不是 TSE 的输出。",
      ));
    }

    // ── 信息（下）────────────────────────────────────────────────────
    let mut conns = div().flex().flex_col().gap_1();
    let has_inputs = crate::plan::input_ports(&n.ptype, n.channels) > 0;
    if has_inputs && n.inputs.is_empty() {
      conns = conns.child(hint("入线：无（输出静音）"));
    }
    for e in &n.inputs {
      let src = self
        .plan
        .node(e.from)
        .map(|x| self.node_title(x))
        .unwrap_or_else(|| "(已删除)".into());
      let (from, fp, tp) = (e.from, e.from_port, e.to_port);
      conns = conns.child(
        div()
          .flex()
          .items_center()
          .gap_2()
          .child(
            div()
              .flex_1()
              .text_size(px(t::TEXT_SM))
              .truncate()
              .child(format!("{src} · {fp} → {tp}")),
          )
          .child(
            danger_button(cx, format!("disc:{id}:{from}:{fp}:{tp}"), "断开").on_click(cx.listener(
              move |this, _, _, cx| {
                this.disconnect(from, fp, id, tp, cx);
              },
            )),
          ),
      );
    }

    list
      .child(section("信息"))
      // 源/输出的名称已经在标题和设备选择里，这里不再重复「类型」。
      .when(n.kind == RowKind::Process, |el| {
        el.child(field("类型", label))
      })
      .child(field("电平", format!("{peak:.3}")))
      .child(field("延迟", latency))
      .when(has_inputs, |el| el.child(conns))
      .into_any_element()
  }

  /// 可视化节点：画板卡片已经显示频谱；这里只给大图，不再重复 VU。
  pub(super) fn viz_editor(&mut self, id: u32, _cx: &mut Context<Self>) -> AnyElement {
    let Some((ci, ri)) = self.pos(id) else {
      return hint("这个节点不在计划里。").into_any_element();
    };
    match self.viz_bands(ci, ri) {
      Some(bands) => spectrum_panel(bands, 150.).into_any_element(),
      None => hint("引擎未运行，或这一行还没有可视化数据。").into_any_element(),
    }
  }

  /// 31 段 EQ 编辑器：拖柱子改增益（热参数），预设 / 低切高切一键写整份设置。
  pub(super) fn eq_editor(&mut self, id: u32, cx: &mut Context<Self>) -> AnyElement {
    let eq = self
      .plan
      .node(id)
      .and_then(|n| n.params.get("settings"))
      .and_then(|s| EqSettings::from_json(s).ok())
      .unwrap_or_default();
    let bands = eq.bands.clone();
    let geom = self.eq_geom.clone();
    let low_on = eq.low_cut_on;
    let high_on = eq.high_cut_on;

    let canvas_el = canvas(
      {
        let geom = geom.clone();
        move |bounds, _, _| {
          geom.set(EqGeom {
            x: bounds.origin.x.as_f32(),
            y: bounds.origin.y.as_f32(),
            w: bounds.size.width.as_f32(),
            h: bounds.size.height.as_f32(),
          });
        }
      },
      move |bounds, _, window, _| {
        let w = bounds.size.width.as_f32();
        let h = bounds.size.height.as_f32();
        let mid = h / 2.0;
        // 0 dB 基线
        let mut pb = PathBuilder::stroke(px(1.0));
        pb.move_to(bounds.origin + point(px(0.0), px(mid)));
        pb.line_to(bounds.origin + point(px(w), px(mid)));
        if let Ok(p) = pb.build() {
          window.paint_path(p, rgb(t::WIRE));
        }
        for (i, &db) in bands.iter().enumerate().take(31) {
          let x = (i as f32 + 0.5) / 31.0 * w;
          let y = mid - (db / 24.0).clamp(-1.0, 1.0) * mid;
          let mut pb = PathBuilder::stroke(px(2.0));
          pb.move_to(bounds.origin + point(px(x), px(mid)));
          pb.line_to(bounds.origin + point(px(x), px(y)));
          if let Ok(p) = pb.build() {
            window.paint_path(p, rgb(if db >= 0.0 { t::ACCENT } else { t::DANGER }));
          }
        }
      },
    )
    .w_full()
    .h_full();

    let hover = div()
      .absolute()
      .top_0()
      .left_0()
      .w_full()
      .h_full()
      .cursor_ns_resize()
      .on_mouse_down(
        gpui_kit::MouseButton::Left,
        cx.listener(move |this, e: &gpui_kit::MouseDownEvent, _, cx| {
          let g = this.eq_geom.get();
          if g.w <= 0.0 || g.h <= 0.0 {
            return;
          }
          let x = e.position.x.as_f32();
          let band = (((x - g.x) / g.w) * 31.0).floor().clamp(0.0, 30.0) as usize;
          let half = (g.h / 2.0).max(1.0);
          let db = (((g.y + half - e.position.y.as_f32()) / half) * 24.0).clamp(-24.0, 24.0);
          let db = (db * 2.0).round() / 2.0;
          this.eq_drag = Some(EqDrag { node: id, band });
          this.eq_set_band(id, band, db, false, cx);
        }),
      );

    let mut presets = div().flex().flex_wrap().gap_1();
    for (key, label) in [
      ("flat", "平坦"),
      ("vocal", "人声"),
      ("podcast", "播客"),
      ("bass", "低音"),
      ("treble", "高音"),
      ("loud", "响度"),
    ] {
      let preset = eq_preset(key);
      presets = presets.child(
        ghost_button(cx, format!("eqp:{id}:{key}"), label)
          .on_click(cx.listener(move |this, _, _, cx| this.eq_apply(id, &preset, true, cx))),
      );
    }

    let eq_entity = cx.entity();
    let toggle_cut = |key: &'static str, label: &'static str, on: bool| {
      let eq = eq.clone();
      let entity = eq_entity.clone();
      switch_row(
        format!("eqcut:{id}:{key}"),
        label,
        on,
        move |next, _, cx| {
          let on = *next;
          let mut eq = eq.clone();
          if key == "low" {
            eq.low_cut_on = on;
          } else {
            eq.high_cut_on = on;
          }
          entity.update(cx, |this, cx| {
            this.eq_apply(id, &eq, true, cx);
          });
        },
      )
    };

    div()
      .flex()
      .flex_col()
      .gap_2()
      .child(
        div()
          .relative()
          .w_full()
          .h(px(128.0))
          .rounded_md()
          .bg(rgb(t::BG))
          .border_1()
          .border_color(rgb(t::BORDER))
          .child(canvas_el)
          .child(hover),
      )
      .child(
        div()
          .flex()
          .justify_between()
          .text_size(px(t::TEXT_XS))
          .text_color(rgb(t::MUTED))
          .child("20 Hz")
          .child("200")
          .child("2k")
          .child("20k"),
      )
      .child(hint("拖动柱子调 ±24 dB；拖动实时生效，松手落盘。"))
      .child(presets)
      .child(
        div()
          .flex()
          .flex_col()
          .gap_2()
          .child(toggle_cut("low", "低切 80 Hz", low_on))
          .child(toggle_cut("high", "高切 16 kHz", high_on)),
      )
      .into_any_element()
  }

  #[allow(clippy::too_many_arguments)]
  pub(super) fn db_slider(
    &mut self,
    id: u32,
    key: &'static str,
    min: f32,
    max: f32,
    cur: String,
    window: &mut Window,
    cx: &mut Context<Self>,
  ) -> AnyElement {
    let value: f32 = cur.parse().unwrap_or(0.0);
    let skey = (id, key.to_string());
    let state = match self.sliders.get(&skey) {
      Some(s) => s.clone(),
      None => {
        let state = cx.new(|_| {
          SliderState::new()
            .min(min)
            .max(max)
            .step(0.5)
            .default_value(value)
        });
        let sub = cx.subscribe(
          &state,
          move |this: &mut Workspace, _state, event: &SliderEvent, cx| {
            if let SliderEvent::Change(SliderValue::Single(v)) = event {
              if let Some((ci, ri)) = this.pos(id) {
                let _ = this.audio.set_row_param(ci, ri, key, &format!("{v:.1}"));
              }
              if let Some(n) = this.plan.node_mut(id) {
                if key == "input_gain_db" {
                  n.input_gain_db = format!("{v:.1}");
                } else {
                  n.params.insert(key.into(), format!("{v:.1}"));
                }
              }
              let _ = this.audio.persist_plan(this.plan.clone());
              cx.notify();
            }
          },
        );
        self.subs.push(sub);
        self.sliders.insert(skey.clone(), state.clone());
        state
      }
    };
    let _ = window;
    let _ = &state;
    div()
      .flex()
      .items_center()
      .gap_2()
      .child(Slider::new(&state).horizontal())
      .child(
        div()
          .w(px(54.))
          .text_size(px(t::TEXT_SM))
          .text_color(rgb(t::TEXT))
          .child(format!("{value:+.1} dB")),
      )
      .into_any_element()
  }

  pub(super) fn params_editor(&self, id: u32, cx: &mut Context<Self>) -> Div {
    let Some(n) = self.plan.node(id) else {
      return div();
    };
    let ptype = n.ptype.clone();
    let spec = crate::engine::registry::get_spec(&ptype);
    let entity = cx.entity();
    let mut list = div().flex().flex_col().gap_1();
    let Some(spec) = spec else {
      return list;
    };
    for p in spec.params {
      if p.key == "far_device" {
        continue;
      }
      let cur = n
        .params
        .get(p.key)
        .cloned()
        .unwrap_or_else(|| p.default_text());
      let label = div()
        .w(px(76.))
        .flex_shrink_0()
        .text_size(px(t::TEXT_SM))
        .text_color(rgb(t::MUTED))
        .child(i18n::tr(p.label).to_string());
      let ptype = ptype.clone();
      let control: AnyElement = match p.kind {
        crate::engine::registry::ParamKind::Number {
          lo,
          hi,
          default,
          step,
        } => {
          let v: f64 = cur.parse().unwrap_or(default);
          let key = p.key.to_string();
          let k1 = key.clone();
          let k2 = key.clone();
          let (l, h, s) = (lo, hi, step);
          div()
            .flex()
            .items_center()
            .gap_2()
            .child(
              quiet_button(cx, format!("nm:{id}:{}", p.key), "−").on_click(cx.listener(
                move |this, _, _window, cx| {
                  let nv = (v - s).clamp(l, h);
                  this.snapshot();
                  this.set_param(id, &k1, format!("{nv:.2}"), cx);
                },
              )),
            )
            .child(
              div()
                .w(px(76.))
                .text_size(px(t::TEXT_SM))
                .child(format!("{v:.2}")),
            )
            .child(
              quiet_button(cx, format!("np:{id}:{}", p.key), "+").on_click(cx.listener(
                move |this, _, _window, cx| {
                  let nv = (v + s).clamp(l, h);
                  this.snapshot();
                  this.set_param(id, &k2, format!("{nv:.2}"), cx);
                },
              )),
            )
            .into_any_element()
        }
        crate::engine::registry::ParamKind::Bool { default } => {
          let on = match cur.as_str() {
            "1" | "true" | "on" => true,
            "0" | "false" | "off" => false,
            _ => default,
          };
          let key = p.key.to_string();
          let entity = entity.clone();
          switch_control(
            format!("pb:{id}:{}", p.key),
            p.label,
            on,
            move |next, _, cx| {
              let on = *next;
              let key = key.clone();
              entity.update(cx, |this, cx| {
                this.snapshot();
                this.set_param(id, &key, if on { "1" } else { "0" }.into(), cx);
              });
            },
          )
          .into_any_element()
        }
        crate::engine::registry::ParamKind::Choice { options, default } => {
          let cur = if cur.is_empty() {
            default
          } else {
            cur.as_str()
          };
          let mut row = div().flex().flex_wrap().gap_1();
          for (value, label) in options {
            let key = p.key.to_string();
            let v = (*value).to_string();
            row = row.child(
              choice_button(
                cx,
                format!("pc:{id}:{}:{value}", p.key),
                *label,
                cur == *value,
              )
              .on_click(cx.listener(move |this, _, _window, cx| {
                this.snapshot();
                this.set_param(id, &key, v.clone(), cx);
              })),
            );
          }
          row.into_any_element()
        }
        crate::engine::registry::ParamKind::Text { default: _ } => {
          if p.key == "model" {
            let choices: Vec<(String, String)> = match ptype.as_str() {
              "denoise" => crate::infer::denoise_models()
                .iter()
                .map(|(f, l)| ((*f).to_string(), i18n::tr(l).to_string()))
                .collect(),
              "tse" => crate::infer::tse::tse_models()
                .iter()
                .map(|(f, l)| ((*f).to_string(), i18n::tr(l).to_string()))
                .collect(),
              "echo_cancel" => crate::infer::aec::aec_models()
                .iter()
                .map(|(f, l)| ((*f).to_string(), i18n::tr(l).to_string()))
                .collect(),
              _ => Vec::new(),
            };
            let key = p.key.to_string();
            let mut row = div().flex().flex_wrap().gap_1();
            for (value, label) in choices {
              let v = value.clone();
              let k = key.clone();
              let on = cur == value;
              row = row.child(
                choice_button(cx, format!("pm:{id}:{}:{}", p.key, value), label, on).on_click(
                  cx.listener(move |this, _, _window, cx| {
                    this.snapshot();
                    this.set_param(id, &k, v.clone(), cx);
                  }),
                ),
              );
            }
            row.into_any_element()
          } else {
            div()
              .text_size(px(t::TEXT_SM))
              .text_color(rgb(t::TEXT))
              .child(cur)
              .into_any_element()
          }
        }
      };
      list = list.child(
        div()
          .flex()
          .items_center()
          .gap_2()
          .min_h(px(22.))
          .child(label)
          .child(div().flex_1().min_w_0().child(control)),
      );
    }
    list
  }

  pub(super) fn latency_short(&self, col: usize) -> String {
    match &self.latency {
      crate::debug::Probe::Ok { value } => value
        .columns
        .iter()
        .find(|c| c.column == col)
        .map(|c| {
          c.measured_ms
            .map(|m| format!("{m:.0} ms"))
            .unwrap_or_else(|| "测量中".into())
        })
        .unwrap_or_else(|| "测量中".into()),
      crate::debug::Probe::Pending => "采集中".into(),
      crate::debug::Probe::Unavailable { reason } => reason.clone(),
    }
  }
}

/// 内置 EQ 预设。31 段是 ISO 1/3 倍频程 20 Hz~20 kHz；按频率找最近的段。
fn eq_preset(name: &str) -> EqSettings {
  let mut s = EqSettings::default();
  let idx = |f: f32| (((f / 20.0).log2() * 3.0).round() as usize).min(30);
  match name {
    "vocal" => {
      s.bands[idx(250.0)] -= 2.0;
      s.bands[idx(400.0)] -= 1.0;
      for f in [1_000.0, 2_000.0, 4_000.0] {
        s.bands[idx(f)] += 3.0;
      }
    }
    "podcast" => {
      s.low_cut_on = true;
      s.bands[idx(200.0)] -= 1.0;
      for f in [2_000.0, 3_000.0, 5_000.0] {
        s.bands[idx(f)] += 2.0;
      }
    }
    "bass" => {
      for f in [40.0, 60.0, 80.0, 120.0] {
        s.bands[idx(f)] += 4.0;
      }
      s.bands[idx(300.0)] -= 1.0;
    }
    "treble" => {
      for f in [6_000.0, 8_000.0, 12_000.0] {
        s.bands[idx(f)] += 3.0;
      }
    }
    "loud" => {
      s.bands[idx(60.0)] += 3.0;
      s.bands[idx(3_000.0)] += 2.0;
      s.bands[idx(10_000.0)] += 2.0;
      s.bands[idx(500.0)] -= 2.0;
    }
    _ => {}
  }
  s
}

#[cfg(test)]
mod tests {
  use super::eq_preset;

  #[test]
  fn eq_presets_are_well_formed() {
    for key in ["flat", "vocal", "podcast", "bass", "treble", "loud"] {
      let s = eq_preset(key);
      assert_eq!(s.bands.len(), 31, "{key}");
      assert!(s.bands.iter().all(|b| (-24.0..=24.0).contains(b)), "{key}");
    }
    assert!(eq_preset("flat").bands.iter().all(|b| *b == 0.0));
    assert!(eq_preset("podcast").low_cut_on);
  }
}
