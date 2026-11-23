// PureVox — AI 麦克风降噪工具
// Copyright (C) 2024-2026 a2heng <752848283@qq.com>
//
// PureVox is licensed under the GNU General Public License v3.0 or
// later (GPL-3.0-or-later).  See LICENSE for details.
//
// SPDX-License-Identifier: GPL-3.0-or-later

use gpui_kit::component::Sizable as _;
use gpui_kit::component::checkbox::Checkbox;
use gpui_kit::component::input::{Input, InputEvent, InputState};
use gpui_kit::component::slider::{Slider, SliderEvent, SliderScale, SliderState, SliderValue};
use gpui_kit::{
  AnyElement, Context, Div, Focusable, PathBuilder, Window, canvas, div, fill, point, prelude::*,
  px, rgb, rgba,
};

use super::theme as t;
use super::widgets::*;
use super::{
  EQ_SLOTS, EqDrag, EqGeom, NodeModule, PendingMedia, RecordTarget, Workspace, i18n, node_module,
};
use crate::engine::components::eq31::{CENTER_HZ, response_db};
use crate::engine::hot::EqSettings;
use crate::plan::{RowKind, RowSpec};

/// 秒 → `m:ss`（进度条旁的时间文字；超过一小时也只显示分钟数）。
fn fmt_time(sec: f64) -> String {
  let s = sec.max(0.0) as u64;
  format!("{}:{:02}", s / 60, s % 60)
}

/// EQ 画图的边距：左侧 dB 轴宽、顶部频率刻度高、底部数值行高。
const EQ_AXIS_W: f32 = 28.0;
const EQ_TOP: f32 = 20.0;
const EQ_BOTTOM: f32 = 16.0;

/// 图形 EQ 的 8 个预设（key, 上屏标签），和插槽 S1–S8 分开。
const EQ_PRESETS: [(&str, &str); 8] = [
  ("flat", "默认平直"),
  ("clear", "清晰透亮"),
  ("warm", "温暖饱满"),
  ("deep", "低沉有力"),
  ("deess", "减少齿音"),
  ("denasal", "减少鼻音"),
  ("unmuddy", "消除沉闷"),
  ("presence", "增强临场"),
];

/// 鼠标 x → 最近的频段（列宽 = 1/3 倍频程；圆点在列中心）。
fn eq_band_at(x: f32, g: EqGeom) -> usize {
  let t = ((x - g.x) / g.w).clamp(0.0, 1.0);
  (t * 31.0).floor().clamp(0.0, 30.0) as usize
}

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
        let (pos, dur) = self
          .pos(id)
          .and_then(|(ci, ri)| crate::engine::media::hub().slot(ci, ri))
          .map(|s| {
            let st = s.status();
            (st.pos, st.dur)
          })
          .unwrap_or((0.0, 0.0));
        let resume = n
          .params
          .get("resume_sec")
          .and_then(|v| v.parse::<f64>().ok())
          .unwrap_or(0.0);
        let shown_pos = if dur > 0.0 { pos } else { resume };
        let vol = n
          .params
          .get("volume_db")
          .and_then(|v| v.parse::<f64>().ok())
          .unwrap_or(0.0);
        list = list.child(field("当前曲目", shown));
        // **进度 = 续播**：同一个滑块，拖动即定位并写入 `resume_sec`（不再单独一条参数）。
        list = list.child(
          div()
            .flex()
            .flex_col()
            .gap_1()
            .child(
              div()
                .flex()
                .items_center()
                .justify_between()
                .gap_2()
                .child(
                  div()
                    .text_size(px(t::TEXT_XS))
                    .text_color(rgb(t::MUTED))
                    .child(format!(
                      "{} / {}",
                      fmt_time(shown_pos),
                      if dur > 0.0 {
                        fmt_time(dur)
                      } else {
                        "--:--".to_string()
                      }
                    )),
                )
                .child(
                  div()
                    .text_size(px(t::TEXT_XS))
                    .text_color(rgb(t::MUTED))
                    .child(if dur > 0.0 {
                      i18n::tr("拖动定位，同时记住续播").to_string()
                    } else {
                      i18n::tr("启动引擎后可拖动").to_string()
                    }),
                ),
            )
            .child(self.player_progress_slider(id, dur, shown_pos, window, cx)),
        );
        list = list.child(
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
        // 音量条放在播放按钮下面（整行一个音量，拖动实时、松手落盘）。
        list = list.child(
          div()
            .flex()
            .items_center()
            .gap_2()
            .child(
              div()
                .w(px(28.))
                .text_size(px(t::TEXT_XS))
                .text_color(rgb(t::MUTED))
                .child(i18n::tr("音量").to_string()),
            )
            .child(self.player_volume_slider(id, vol, window, cx))
            .child(
              div()
                .w(px(46.))
                .text_size(px(t::TEXT_XS))
                .text_color(rgb(t::MUTED))
                .child(format!("{vol:+.1} dB")),
            ),
        );
        // 播放 / 暂停热键：全局生效（窗口在后台也行），录制完立即重注册。
        let hk = n.params.get("hotkey").cloned().unwrap_or_default();
        let recording = matches!(self.recording, Some(RecordTarget::Player { node }) if node == id);
        let mut actions = vec![
          quiet_button(
            cx,
            format!("pkey:{id}"),
            if recording { "按键…" } else { "录制" },
          )
          .on_click(cx.listener(move |this, _, _, cx| {
            if recording {
              this.end_record();
              this.status = i18n::tr("已取消录制热键").into();
            } else {
              this.begin_record(
                RecordTarget::Player { node: id },
                "按下播放/暂停热键…（Esc 取消）",
                cx,
              );
            }
            cx.notify();
          }))
          .into_any_element(),
        ];
        if !hk.is_empty() && !recording {
          actions.push(
            ghost_button(cx, format!("pkeyclear:{id}"), "清除")
              .on_click(cx.listener(move |this, _, _, cx| {
                this.set_node_hotkey(id, String::new(), cx);
              }))
              .into_any_element(),
          );
        }
        list = list.child(hotkey_row(i18n::tr("播放 / 暂停"), &hk, recording, actions));
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
          let volume = pad.get("volume_db").and_then(|v| v.as_f64()).unwrap_or(0.0);
          let enabled = pad.get("enabled").and_then(|v| v.as_bool()).unwrap_or(true);
          let recording = matches!(
            self.recording,
            Some(RecordTarget::Pad { node, index }) if node == id && index == i
          );
          let entity = cx.entity();
          let toggle = switch_control(
            format!("paden:{id}:{i}"),
            "启用",
            enabled,
            move |next, _, cx| {
              let on = *next;
              entity.update(cx, |this, cx| this.set_pad_enabled(id, i, on, cx));
            },
          );
          list = list.child(
            div()
              .flex()
              .flex_col()
              .gap_1()
              .p(px(t::SPACE_2))
              .rounded(px(t::RADIUS_MD))
              .border_1()
              .border_color(rgb(t::BORDER))
              .bg(rgb(t::PANEL))
              .child(
                div()
                  .flex()
                  .items_center()
                  .gap_2()
                  .child(toggle)
                  .child(
                    div()
                      .flex_1()
                      .min_w_0()
                      .overflow_hidden()
                      .text_size(px(t::TEXT_SM))
                      .text_color(rgb(if enabled { t::TEXT } else { t::MUTED }))
                      .child(if enabled {
                        name.clone()
                      } else {
                        format!("{name}{}", i18n::tr("（已关闭）"))
                      }),
                  )
                  .child(
                    quiet_button(cx, format!("pad:{id}:{i}"), "▶").on_click(cx.listener(
                      move |this, _, _, cx| {
                        if !enabled {
                          this.status = i18n::tr("音效已关闭").into();
                          cx.notify();
                          return;
                        }
                        if let Some((ci, ri)) = this.pos(id)
                          && let Some(s) = crate::engine::media::hub().slot(ci, ri)
                        {
                          s.send(crate::engine::media::MediaCmd::SoundpadPlay(i));
                          this.status = format!("播放音效：{name}");
                        } else {
                          this.status = "引擎未运行：先启动再播放".into();
                        }
                        cx.notify();
                      },
                    )),
                  )
                  .child(
                    danger_button(cx, format!("paddel:{id}:{i}"), "删除").on_click(cx.listener(
                      move |this, _, _, cx| {
                        this.remove_pad(id, i, cx);
                      },
                    )),
                  ),
              )
              .child(
                div()
                  .flex()
                  .items_center()
                  .gap_2()
                  .child(
                    div()
                      .w(px(28.))
                      .text_size(px(t::TEXT_XS))
                      .text_color(rgb(t::MUTED))
                      .child(i18n::tr("音量").to_string()),
                  )
                  .child(self.pad_volume_slider(id, i, volume, window, cx))
                  .child(
                    div()
                      .w(px(46.))
                      .text_size(px(t::TEXT_XS))
                      .text_color(rgb(t::MUTED))
                      .child(format!("{volume:+.1} dB")),
                  ),
              )
              .child({
                let mut actions = vec![
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
                  }))
                  .into_any_element(),
                ];
                if !hotkey.is_empty() && !recording {
                  actions.push(
                    ghost_button(cx, format!("padkeyclear:{id}:{i}"), "清除")
                      .on_click(cx.listener(move |this, _, _, cx| {
                        this.set_pad_hotkey(id, i, String::new(), cx);
                      }))
                      .into_any_element(),
                  );
                }
                hotkey_row(i18n::tr("热键"), &hotkey, recording, actions)
              }),
          );
        }
      }
    }

    // 测试音：随包素材多选（敲桌子 / 鼠标 / 键盘 / 正弦波）+ 重复间隙。
    if n.ptype == "tone" {
      let selected = crate::audio::tone::selected(&n.params);
      list = list.child(section("测试素材"));
      let mut clip_rows: Vec<(String, &'static str)> =
        vec![(crate::audio::tone::SINE_ID.to_string(), "1 kHz 正弦")];
      for c in crate::audio::tone::CLIPS {
        clip_rows.push((c.id.to_string(), c.label));
      }
      for (cid, label) in clip_rows {
        let on = selected.contains(&cid);
        let cid2 = cid.clone();
        list = list.child(
          Checkbox::new(format!("clip:{id}:{cid}"))
            .checked(on)
            .label(i18n::tr(label).to_string())
            .on_click(cx.listener(move |this, checked, _, cx| {
              let mut cur = this
                .plan
                .node(id)
                .map(|n| crate::audio::tone::selected(&n.params))
                .unwrap_or_default();
              if *checked {
                if !cur.contains(&cid2) {
                  cur.push(cid2.clone());
                }
              } else {
                cur.retain(|s| *s != cid2);
              }
              this.snapshot();
              if let Some(n) = this.plan.node_mut(id) {
                n.params
                  .insert("clips".into(), serde_json::json!(cur).to_string());
              }
              this.status = "测试素材已更新".into();
              this.apply_structure(cx);
            })),
        );
      }
      let gap = n
        .params
        .get("gap_ms")
        .and_then(|v| v.parse::<f64>().ok())
        .unwrap_or(crate::audio::tone::DEFAULT_GAP_MS as f64);
      list = list
        .child(section("重复间隙"))
        .child(self.param_slider(id, "tone", "gap_ms", 50.0, 5000.0, 50.0, gap, window, cx))
        .child(hint(
          "勾选多个素材会叠加播放；间隙是每次瞬态之间的静音。改完重建一次会话。",
        ));
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

    // 只有**物理输出**节点才选设备：`remote_speaker`（手机 / 客户端）也是「没有输出端口」，
    // 但它是网络汇，右侧再列一遍扬声器既没有意义，也会让人以为声音会从本机设备出。
    if n.ptype == "audio_output" {
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
        .child(self.params_editor(id, window, cx))
        .child(self.aec_card(id, cx));
    } else if n.ptype == "eq31" {
      list = list
        .child(section("均衡"))
        .child(self.eq_editor(id, window, cx));
    } else if n.ptype == "viz" {
      list = list
        .child(section("可视化"))
        .child(self.params_editor(id, window, cx))
        .child(self.viz_editor(id, cx));
    } else if n.kind != RowKind::Output
      // 媒体节点（音效板 / 播放器）的参数全部由上面的专属面板接管：
      // 文件、每个音效的开关/音量/热键、播放器进度/音量/热键，不再有通用参数块。
      && !matches!(n.ptype.as_str(), "soundpad" | "player")
      && crate::engine::registry::get_spec(&n.ptype)
        .map(|s| !s.params.is_empty())
        .unwrap_or(false)
    {
      list = list
        .child(section("参数"))
        .child(self.params_editor(id, window, cx));
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

    // 状态徽标：流存在就按 stream 状态，否则按引擎 / 旁路状态；健康结论单独一行。
    let stream = self.stream_for(&n, ci, ri);
    let native = self.native_for(&n);
    let health = stream.as_ref().and_then(|s| match &s.health {
      crate::debug::Probe::Ok { value } => Some((value.level, value.text.clone())),
      _ => None,
    });
    let (state_text, state_level, state_detail) = match &stream {
      Some(s) => match &s.state {
        crate::debug::Probe::Ok { .. } => ("运行中", "ok", String::new()),
        crate::debug::Probe::Pending => ("预热中", "warn", "等待设备回调".to_string()),
        crate::debug::Probe::Unavailable { reason } => ("不可用", "bad", reason.clone()),
      },
      None if n.kind == RowKind::Process => {
        if !enabled {
          ("已旁路", "idle", String::new())
        } else if self.audio.is_running() {
          ("运行中", "ok", String::new())
        } else {
          ("未启动", "idle", "引擎未运行".to_string())
        }
      }
      None => (
        "未启动",
        "idle",
        if self.audio.is_running() {
          "这一行没有独立设备流".to_string()
        } else {
          "引擎未运行".to_string()
        },
      ),
    };
    let db = if peak > 1e-9 {
      20.0 * peak.log10()
    } else {
      -120.0
    };
    let level_color = if peak >= 0.98 { t::DANGER } else { t::TEXT };
    let mut info = div()
      .flex()
      .flex_col()
      .gap_2()
      .child(
        div()
          .flex()
          .items_center()
          .gap_2()
          .child(status_chip(state_text, state_level))
          .child(
            div()
              .flex_1()
              .min_w_0()
              .text_size(px(t::TEXT_XS))
              .text_color(rgb(t::MUTED))
              .truncate()
              .child(state_detail),
          ),
      )
      .when_some(health, |el, (level, text)| {
        el.child(field_color(
          "健康",
          text,
          if level == "ok" { t::OK } else { t::AEC },
        ))
      })
      .child(
        div()
          .flex()
          .flex_col()
          .gap_1()
          .child(
            div()
              .flex()
              .items_center()
              .justify_between()
              .child(
                div()
                  .text_size(px(t::TEXT_SM))
                  .text_color(rgb(t::MUTED))
                  .child(i18n::tr("电平").to_string()),
              )
              .child(
                div()
                  .text_size(px(t::TEXT_SM))
                  .text_color(rgb(level_color))
                  .child(format!("{db:.1} dBFS")),
              ),
          )
          .child(meter_bar(peak, 24, 8.)),
      )
      .child(field("延迟", latency));

    if let Some((host, native)) = &native {
      info = info.child(section("设备")).child(field(
        "设备格式",
        format!(
          "{host} · {} Hz · {} ch · {}",
          native.sample_rate, native.channels, native.sample_format
        ),
      ));
    }

    if let Some(s) = &stream {
      let fmt_text = if s.sample_format.contains('@') {
        s.sample_format.clone()
      } else {
        format!(
          "{} · {} ch · {} Hz",
          s.sample_format, s.channels, s.sample_rate
        )
      };
      info = info
        .child(section("流"))
        .child(field("设备", s.device_name.clone()))
        .child(field("流格式", fmt_text));
      if let crate::debug::Probe::Ok { value } = &s.denoise {
        info = info.child(field("降噪", value.clone()));
      }
      if let crate::debug::Probe::Ok { value } = &s.inference_ms_avg {
        let max = match &s.inference_ms_max {
          crate::debug::Probe::Ok { value } => format!(" / 峰值 {value:.1} ms"),
          _ => String::new(),
        };
        info = info.child(field("推理", format!("{value:.1} ms{max}")));
      }
      if let crate::debug::Probe::Ok { value: cf } = &s.callback_frames {
        info = info.child(field(
          "回调",
          format!(
            "{:.2} ms · {} 帧（间隔 {:.1} / 峰值 {:.1} ms）",
            cf.last as f64 * 1000.0 / s.sample_rate.max(1) as f64,
            cf.last,
            cf.last_gap_ms,
            cf.max_gap_ms
          ),
        ));
      }
      let dev_buf = match &s.device_buffer_ms {
        crate::debug::Probe::Ok { value } => format!("{value:.1} ms"),
        crate::debug::Probe::Pending => "采样中".into(),
        crate::debug::Probe::Unavailable { .. } => "—".into(),
      };
      info = info.child(field(
        "缓冲",
        format!("{:.1} ms（源）· {dev_buf}（设备）", s.buffer_level_ms),
      ));
      info = info.child(field(
        "时钟",
        format!(
          "{} → {}",
          hz_text(&s.measured_input_rate),
          hz_text(&s.measured_output_rate)
        ),
      ));
      if !s.resampler.is_empty() {
        info = info.child(field(
          "重采样",
          format!("{} · {:.1} ms", s.resampler, s.resampler_delay_ms),
        ));
      }
      if let crate::debug::Probe::Ok { value } = &s.latency_ms {
        info = info.child(field("流延迟", format!("{value:.1} ms")));
      }
      if let crate::debug::Probe::Ok { value } = &s.asrc_adjust_ppm {
        info = info.child(field("ASRC", format!("{value:+.1} ppm")));
      }
      let anomaly = format!(
        "欠载 {} · 过载 {} · 重同步 {} · 错误 {}",
        per_s(&s.underrun_per_s),
        per_s(&s.overrun_per_s),
        per_s(&s.resync_per_s),
        per_s(&s.stream_error_per_s)
      );
      let bad = rate_positive(&s.underrun_per_s)
        || rate_positive(&s.overrun_per_s)
        || rate_positive(&s.resync_per_s)
        || rate_positive(&s.stream_error_per_s);
      info = info.child(field_color(
        "异常",
        anomaly,
        if bad { t::DANGER } else { t::TEXT },
      ));
      if let Some(src) = &s.source {
        info = info.child(field("源", src.clone()));
      }
    } else if n.kind == RowKind::Process {
      info = info.child(field("类型", label));
    }

    if has_inputs {
      info = info.child(section("入线")).child(conns);
    }

    list.child(section("信息")).child(info).into_any_element()
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

  /// 31 段 EQ 编辑器（图形 EQ）：一条平滑曲线 + 31 个频段圆点。
  ///
  /// 参照 Peace / Equalizer APO 一类图形 EQ 的画法：纵向每个频段一条网格线，
  /// 圆点、网格、频率刻度共用同一个 x 坐标；顶部是频率刻度，底部是调整过的段的值；
  /// 左侧是 dB 轴。点圆点选中后可精确输入 dB，双击归零。
  pub(super) fn eq_editor(
    &mut self,
    id: u32,
    window: &mut Window,
    cx: &mut Context<Self>,
  ) -> AnyElement {
    let eq = self
      .plan
      .node(id)
      .and_then(|n| n.params.get("settings"))
      .and_then(|s| EqSettings::from_json(s).ok())
      .unwrap_or_default();
    let bands = eq.bands.clone();
    let curve_src = eq.clone();
    let slots = self.eq_slots_of(id);
    let active_slot = slots.active;
    let geom = self.eq_geom.clone();
    let low_on = eq.low_cut_on;
    let high_on = eq.high_cut_on;
    let sel = self
      .eq_band
      .filter(|(n, _)| *n == id)
      .map(|(_, b)| b)
      .unwrap_or(17);
    let hover = self.eq_hover.filter(|(n, _)| *n == id).map(|(_, b)| b);
    let sel_db = bands.get(sel).copied().unwrap_or(0.0);
    let sel_hz = CENTER_HZ.get(sel).copied().unwrap_or(1_000.0);

    let canvas_el = canvas(
      {
        let geom = geom.clone();
        move |bounds, _, _| {
          let w = bounds.size.width.as_f32();
          let h = bounds.size.height.as_f32();
          // 只记录绘图区（去掉左侧 dB 轴、顶部频率轴、底部数值行）。
          geom.set(EqGeom {
            x: bounds.origin.x.as_f32() + EQ_AXIS_W,
            y: bounds.origin.y.as_f32() + EQ_TOP,
            w: (w - EQ_AXIS_W).max(8.0),
            h: (h - EQ_TOP - EQ_BOTTOM).max(8.0),
          });
        }
      },
      move |bounds, _, window, cx| {
        let w = bounds.size.width.as_f32();
        let h = bounds.size.height.as_f32();
        let ox = bounds.origin.x.as_f32();
        let oy = bounds.origin.y.as_f32();
        let plot_x = ox + EQ_AXIS_W;
        let plot_y = oy + EQ_TOP;
        let plot_w = (w - EQ_AXIS_W).max(8.0);
        let plot_h = (h - EQ_TOP - EQ_BOTTOM).max(8.0);
        let mid = plot_y + plot_h / 2.0;
        let span = (plot_h / 2.0).max(1.0);
        let y_of = |db: f32| mid - (db / 24.0).clamp(-1.0, 1.0) * span;
        let step = plot_w / 31.0;
        // 频段 i 的中心 x；网格线、圆点、顶部刻度、底部数值都用它，保证对得齐。
        let x_of_band = |i: usize| plot_x + (i as f32 + 0.5) * step;
        // 按 1/3 倍频程位置画曲线：频段中心正好落在 (i + 0.5) 上。
        let x_of_pos = |pos: f32| plot_x + (pos + 0.5) / 31.0 * plot_w;

        // 横向 dB 网格 + 左侧刻度（每 8 dB 一条，0 线更亮）。
        for db in [24.0f32, 16.0, 8.0, 0.0, -8.0, -16.0, -24.0] {
          let y = y_of(db);
          let zero = db == 0.0;
          let mut pb = PathBuilder::stroke(px(if zero { 1.0 } else { 0.5 }));
          pb.move_to(point(px(plot_x), px(y)));
          pb.line_to(point(px(plot_x + plot_w), px(y)));
          if let Ok(p) = pb.build() {
            window.paint_path(p, rgba((t::BORDER << 8) | if zero { 0xE0 } else { 0x60 }));
          }
          let text = if zero {
            "0".to_string()
          } else {
            format!("{db:+.0}")
          };
          let line = shape_text(window, &text, 9.0, t::MUTED);
          let lx = (plot_x - 5.0 - line.width.as_f32()).max(2.0);
          let ly = (y - 5.0).clamp(oy, oy + h - 10.0);
          let _ = line.paint(
            point(px(lx), px(ly)),
            px(10.),
            gpui_kit::TextAlign::Left,
            None,
            window,
            cx,
          );
        }

        // 纵向网格：每个频段一条，和圆点严格对齐。
        for i in 0..31usize {
          let x = x_of_band(i);
          let mut pb = PathBuilder::stroke(px(1.0));
          pb.move_to(point(px(x), px(plot_y)));
          pb.line_to(point(px(x), px(plot_y + plot_h)));
          if let Ok(p) = pb.build() {
            window.paint_path(p, rgba((t::BORDER << 8) | 0x45));
          }
        }

        // 顶部频率刻度：每 2 段一个（20 / 31.5 / 50 … 20k），与竖线对齐。
        for i in (0..31usize).step_by(2) {
          let line = shape_text(window, &hz_label(CENTER_HZ[i]), 9.0, t::MUTED);
          let lw = line.width.as_f32();
          let lx = (x_of_band(i) - lw / 2.0).clamp(ox + 2.0, ox + w - lw - 2.0);
          let _ = line.paint(
            point(px(lx), px(oy + 2.0)),
            px(10.),
            gpui_kit::TextAlign::Left,
            None,
            window,
            cx,
          );
        }

        // 平滑响应曲线（含低切 / 高切）；采样点落在频段中心之间，曲线经过每个圆点。
        const SAMPLES: usize = 124;
        let freqs: Vec<f32> = (0..=SAMPLES)
          .map(|k| {
            let pos = -0.5 + k as f32 / SAMPLES as f32 * 31.0;
            20.0 * 2f32.powf(pos / 3.0)
          })
          .collect();
        let dbs = response_db(&curve_src, &freqs);
        let mut pb = PathBuilder::stroke(px(1.8));
        for (k, db) in dbs.iter().enumerate() {
          let pos = -0.5 + k as f32 / SAMPLES as f32 * 31.0;
          let x = x_of_pos(pos);
          let y = y_of(*db);
          if k == 0 {
            pb.move_to(point(px(x), px(y)));
          } else {
            pb.line_to(point(px(x), px(y)));
          }
        }
        if let Ok(p) = pb.build() {
          window.paint_path(p, rgb(t::ACCENT));
        }

        // 频段圆点：平坦段是小灰点，调整过的段是实心蓝点（正负同色，靠位置表示方向）。
        for (i, &db) in bands.iter().enumerate().take(31) {
          let x = x_of_band(i);
          let y = y_of(db);
          let active = i == sel || hover == Some(i);
          if db.abs() < 0.05 && !active {
            paint_dot(window, x, y, 2.8, t::WIRE, 0xFF);
            continue;
          }
          if active {
            paint_dot(window, x, y, 7.0, t::ACCENT, 0x38);
            paint_dot(window, x, y, 4.5, t::ACCENT, 0xFF);
          } else {
            paint_dot(window, x, y, 3.8, t::ACCENT, 0xFF);
          }
        }

        // 底部数值：调整过的段在圆点正下方写 dB，和顶部频率刻度一样与竖线对齐。
        let value_y = oy + h - 12.0;
        for (i, &db) in bands.iter().enumerate().take(31) {
          if db.abs() < 0.05 {
            continue;
          }
          let text = db_label(db);
          let line = shape_text(
            window,
            &text,
            9.0,
            if i == sel { t::TEXT } else { t::MUTED },
          );
          let lw = line.width.as_f32();
          let lx = (x_of_band(i) - lw / 2.0).clamp(ox + 2.0, ox + w - lw - 2.0);
          let _ = line.paint(
            point(px(lx), px(value_y)),
            px(10.),
            gpui_kit::TextAlign::Left,
            None,
            window,
            cx,
          );
        }

        // 悬停气泡：频率 + 一位小数 dB，跟着圆点走。
        if let Some(hovered) = hover
          && let Some(&db) = bands.get(hovered)
        {
          let text = format!("{}  {db:+.1} dB", hz_label(CENTER_HZ[hovered]));
          let line = shape_text(window, &text, 9.0, t::TEXT);
          let bubble_w = line.width.as_f32() + 8.0;
          let bx = (x_of_band(hovered) - bubble_w / 2.0).clamp(plot_x, plot_x + plot_w - bubble_w);
          let cap = y_of(db);
          let by = if db >= 0.0 {
            (cap - 18.0).max(oy + 1.0)
          } else {
            (cap + 4.0).min(plot_y + plot_h - 14.0)
          };
          window.paint_quad(fill(
            gpui_kit::bounds(
              point(px(bx), px(by - 1.0)),
              gpui_kit::size(px(bubble_w), px(13.0)),
            ),
            rgb(t::PANEL_HI),
          ));
          let _ = line.paint(
            point(px(bx + 4.0), px(by)),
            px(10.),
            gpui_kit::TextAlign::Left,
            None,
            window,
            cx,
          );
        }
      },
    )
    .w_full()
    .h_full();

    let hit = div()
      .id(format!("eq-hit:{id}"))
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
          let band = eq_band_at(e.position.x.as_f32(), g);
          this.eq_band = Some((id, band));
          this.eq_hover = Some((id, band));
          // 单击只选中；双击归零；拖动才改值（见 `graph.rs::on_mouse_move`）。
          if e.click_count >= 2 {
            this.eq_drag = Some(EqDrag {
              node: id,
              band,
              start_y: e.position.y.as_f32(),
            });
            this.eq_set_band(id, band, 0.0, false, cx);
          } else {
            this.eq_drag = Some(EqDrag {
              node: id,
              band,
              start_y: e.position.y.as_f32(),
            });
            cx.notify();
          }
        }),
      )
      .on_mouse_move(
        cx.listener(move |this, e: &gpui_kit::MouseMoveEvent, _, cx| {
          let g = this.eq_geom.get();
          if g.w <= 0.0 || g.h <= 0.0 {
            return;
          }
          let band = eq_band_at(e.position.x.as_f32(), g);
          if this.eq_hover != Some((id, band)) {
            this.eq_hover = Some((id, band));
            cx.notify();
          }
        }),
      )
      .on_hover(
        cx.listener(move |this: &mut Workspace, hovered: &bool, _, cx| {
          if !*hovered && this.eq_hover.is_some_and(|(n, _)| n == id) {
            this.eq_hover = None;
            cx.notify();
          }
        }),
      );

    let mut slot_row = div().flex().flex_1().min_w_0().gap_1();
    for i in 0..EQ_SLOTS {
      slot_row = slot_row.child(
        eq_cell(
          format!("eqslot:{id}:{i}"),
          format!("S{}", i + 1),
          i == active_slot,
        )
        .on_click(cx.listener(move |this, _, _, cx| this.eq_select_slot(id, i, cx))),
      );
    }
    let mut preset_row = div().flex().flex_1().min_w_0().gap_1();
    for (key, label) in EQ_PRESETS {
      let preset = eq_preset(key);
      preset_row = preset_row.child(
        eq_cell(format!("eqpreset:{id}:{key}"), label, false)
          .on_click(cx.listener(move |this, _, _, cx| this.eq_apply(id, &preset, true, cx))),
      );
    }
    let slot_line = div()
      .flex()
      .items_center()
      .gap_2()
      .child(eq_row_label("插槽"))
      .child(slot_row);
    let preset_line = div()
      .flex()
      .items_center()
      .gap_2()
      .child(eq_row_label("预设"))
      .child(preset_row);

    let band_input = self.number_field(
      format!("eq:{id}"),
      sel_db as f64,
      1,
      66.0,
      window,
      cx,
      move |this, v, cx| {
        let band = this
          .eq_band
          .filter(|(n, _)| *n == id)
          .map(|(_, b)| b)
          .unwrap_or(17);
        this.eq_set_band(id, band, v.clamp(-24.0, 24.0) as f32, true, cx);
      },
    );
    let zero =
      quiet_button(cx, format!("eqz:{id}"), "归零").on_click(cx.listener(move |this, _, _, cx| {
        let band = this
          .eq_band
          .filter(|(n, _)| *n == id)
          .map(|(_, b)| b)
          .unwrap_or(sel);
        this.eq_set_band(id, band, 0.0, true, cx);
      }));

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
          .h(px(220.0))
          .rounded_lg()
          .bg(rgb(t::BG))
          .border_1()
          .border_color(rgb(t::BORDER))
          .child(canvas_el)
          .child(hit),
      )
      .child(
        div()
          .flex()
          .items_center()
          .gap_2()
          .child(
            div()
              .w(px(76.))
              .h(px(20.))
              .flex_shrink_0()
              .truncate()
              .flex()
              .items_center()
              .justify_center()
              .rounded_full()
              .bg(rgb(t::ACCENT_SOFT))
              .border_1()
              .border_color(rgb(t::ACCENT))
              .text_size(px(t::TEXT_XS))
              .text_color(rgb(t::TEXT))
              .child(format!("{} {}", i18n::tr("选中"), hz_label(sel_hz))),
          )
          .child(band_input)
          .child(
            div()
              .text_size(px(t::TEXT_XS))
              .text_color(rgb(t::MUTED))
              .child("dB"),
          )
          .child(zero),
      )
      .child(hint(
        "拖动圆点调 ±24 dB；点圆点选中后可精确输入；双击圆点归零；松手落盘。",
      ))
      .child(slot_line)
      .child(preset_line)
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
  /// 可精确输入的数字框：回车 / 失焦时把解析后的值交给 `commit`。
  ///
  /// 输入框按 `key` 缓存在 `Workspace.inputs`，每次渲染只在**没有焦点**且显示值
  /// 与计划值不一致时回写，避免打断正在打字（滑杆/加减按钮改了值也会同步过来）。
  #[allow(clippy::too_many_arguments)]
  pub(super) fn number_field(
    &mut self,
    key: String,
    value: f64,
    decimals: usize,
    width: f32,
    window: &mut Window,
    cx: &mut Context<Self>,
    commit: impl Fn(&mut Self, f64, &mut Context<Self>) + 'static,
  ) -> AnyElement {
    let shown = format!("{value:.decimals$}");
    let state = match self.inputs.get(&key) {
      Some(s) => s.clone(),
      None => {
        let initial = shown.clone();
        let state = cx.new(|cx| InputState::new(window, cx).default_value(initial));
        let sub = cx.subscribe(
          &state,
          move |this: &mut Workspace, state, event: &InputEvent, cx| {
            if matches!(event, InputEvent::PressEnter { .. } | InputEvent::Blur) {
              let text = state.read(cx).value().to_string();
              match parse_number(&text) {
                Some(v) => commit(this, v, cx),
                // 不是数字：下次渲染把显示值写回计划值。
                None => cx.notify(),
              }
            }
          },
        );
        self.subs.push(sub);
        self.inputs.insert(key.clone(), state.clone());
        state
      }
    };
    let current = state.read(cx).value().to_string();
    let focused = state.read(cx).focus_handle(cx).is_focused(window);
    if !focused && current != shown {
      state.update(cx, |state, cx| state.set_value(shown.clone(), window, cx));
    }
    Input::new(&state)
      .appearance(true)
      .bordered(true)
      .focus_bordered(true)
      .with_size(gpui_kit::component::Size::XSmall)
      .w(px(width))
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
    // 滑杆负责快速扫，数字框负责精确落值；两边共用同一份计划参数。
    let sv = state.read(cx).value().end();
    if (sv - value).abs() > 0.01 {
      state.update(cx, |state, cx| state.set_value(value, window, cx));
    }
    let field = self.number_field(
      format!("gain:{id}:{key}"),
      value as f64,
      1,
      58.0,
      window,
      cx,
      move |this, v, cx| {
        let v = (v as f32).clamp(min, max);
        let text = format!("{v:.1}");
        if let Some((ci, ri)) = this.pos(id) {
          let _ = this.audio.set_row_param(ci, ri, key, &text);
        }
        if let Some(n) = this.plan.node_mut(id) {
          if key == "input_gain_db" {
            n.input_gain_db = text.clone();
          } else {
            n.params.insert(key.into(), text.clone());
          }
        }
        let _ = this.audio.persist_plan(this.plan.clone());
        this.status = format!("{key} = {text}");
        cx.notify();
      },
    );
    div()
      .flex()
      .items_center()
      .gap_2()
      .child(
        Slider::new(&state)
          .horizontal()
          .bg(rgb(t::ACCENT))
          .text_color(rgb(t::TEXT)),
      )
      .child(field)
      .child(
        div()
          .text_size(px(t::TEXT_XS))
          .text_color(rgb(t::MUTED))
          .child("dB"),
      )
      .into_any_element()
  }

  /// 参数滑杆 + 精确输入：拖动实时显示，非热参数松手才重建一次会话。
  ///
  /// 数字框和滑杆共用同一个值；第一次 Change 记一次撤销快照，避免拖动时刷爆撤销栈。
  #[allow(clippy::too_many_arguments)]
  pub(super) fn param_slider(
    &mut self,
    id: u32,
    ptype: &str,
    key: &'static str,
    lo: f64,
    hi: f64,
    step: f64,
    value: f64,
    window: &mut Window,
    cx: &mut Context<Self>,
  ) -> AnyElement {
    let decimals = decimals_for_step(step);
    let skey = (id, format!("param:{key}"));
    let drag_key = format!("param:{id}:{key}");
    let state = match self.sliders.get(&skey) {
      Some(s) => s.clone(),
      None => {
        // 时间类参数（attack/release/speed，跨两个数量级以上）用对数刻度，
        // 否则 6 ms 会挤在 1–500 ms 滑杆的 1% 处。
        let scale = if lo > 0.0 && hi / lo >= 50.0 {
          SliderScale::Logarithmic
        } else {
          SliderScale::Linear
        };
        let state = cx.new(|_| {
          SliderState::new()
            .min(lo as f32)
            .max(hi as f32)
            .step(step as f32)
            .scale(scale)
            .default_value(value as f32)
        });
        let hot = crate::engine::registry::is_hot(ptype, key);
        let key_s = key.to_string();
        let dk = drag_key.clone();
        let sub = cx.subscribe(
          &state,
          move |this: &mut Workspace, _state, event: &SliderEvent, cx| {
            if let SliderEvent::Change(SliderValue::Single(v)) = event {
              if this.slider_drag.as_deref() != Some(&dk) {
                this.snapshot();
                this.slider_drag = Some(dk.clone());
              }
              let text = format!("{v:.decimals$}");
              if hot {
                this.set_param(id, &key_s, text, cx);
              } else {
                if let Some(n) = this.plan.node_mut(id) {
                  n.params.insert(key_s.clone(), text.clone());
                }
                this.pending_param = Some((std::time::Instant::now(), id, key_s.clone(), text));
                this.status = format!("{key_s} = {v:.decimals$}（停手后应用）");
                cx.notify();
              }
            } else if let SliderEvent::Release(SliderValue::Single(v)) = event {
              this.slider_drag = None;
              if !hot {
                this.pending_param = None;
                this.set_param(id, &key_s, format!("{v:.decimals$}"), cx);
              }
            }
          },
        );
        self.subs.push(sub);
        self.sliders.insert(skey.clone(), state.clone());
        state
      }
    };
    // 计划值被输入框 / 撤销 / 预设改动后，滑杆位置跟着回来。
    let current = state.read(cx).value().end();
    if (current - value as f32).abs() > (step as f32 / 2.0).max(0.000_001) {
      state.update(cx, |state, cx| state.set_value(value as f32, window, cx));
    }
    let exact = self.number_field(
      format!("num:{id}:{key}"),
      value,
      decimals,
      66.0,
      window,
      cx,
      move |this, nv, cx| {
        let nv = nv.clamp(lo, hi);
        this.snapshot();
        this.slider_drag = None;
        this.set_param(id, key, format!("{nv:.decimals$}"), cx);
      },
    );
    let unit = param_unit(key);
    div()
      .flex()
      .flex_1()
      .min_w_0()
      .items_center()
      .gap_2()
      .child(
        Slider::new(&state)
          .horizontal()
          .bg(rgb(t::ACCENT))
          .text_color(rgb(t::TEXT)),
      )
      .child(exact)
      .when(!unit.is_empty(), |row| {
        row.child(
          div()
            .text_size(px(t::TEXT_XS))
            .text_color(rgb(t::MUTED))
            .child(unit),
        )
      })
      .into_any_element()
  }

  /// 音效板里单个音效的音量滑杆（±10 dB）：拖动实时改在播声部，松手落盘。
  pub(super) fn pad_volume_slider(
    &mut self,
    node: u32,
    index: usize,
    value: f64,
    window: &mut Window,
    cx: &mut Context<Self>,
  ) -> AnyElement {
    let skey = (node, format!("padvol:{index}"));
    let drag_key = format!("padvol:{node}:{index}");
    let state = match self.sliders.get(&skey) {
      Some(s) => s.clone(),
      None => {
        let state = cx.new(|_| {
          SliderState::new()
            .min(-10.0)
            .max(10.0)
            .step(1.0)
            .scale(SliderScale::Linear)
            .default_value(value as f32)
        });
        let dk = drag_key.clone();
        let sub = cx.subscribe(
          &state,
          move |this: &mut Workspace, _state, event: &SliderEvent, cx| match event {
            SliderEvent::Change(SliderValue::Single(v)) => {
              if this.slider_drag.as_deref() != Some(&dk) {
                this.snapshot();
                this.slider_drag = Some(dk.clone());
              }
              this.set_pad_volume(node, index, *v as f64, false, cx);
            }
            SliderEvent::Release(SliderValue::Single(v)) => {
              this.slider_drag = None;
              this.set_pad_volume(node, index, *v as f64, true, cx);
            }
            _ => {}
          },
        );
        self.subs.push(sub);
        self.sliders.insert(skey, state.clone());
        state
      }
    };
    let current = state.read(cx).value().end();
    if (current - value as f32).abs() > 0.01 {
      state.update(cx, |state, cx| state.set_value(value as f32, window, cx));
    }
    div()
      .flex_1()
      .min_w_0()
      .child(
        Slider::new(&state)
          .horizontal()
          .bg(rgb(t::ACCENT))
          .text_color(rgb(t::TEXT)),
      )
      .into_any_element()
  }

  /// 播放器进度条：**进度展示 + 定位 + 断点续播二合一**。
  ///
  /// 拖动时实时 `PlayerSeek`（听感立刻跟手），同时把秒数写进节点参数 `resume_sec`；
  /// 松手才落盘。滑杆状态按「时长」分键，换曲/换文件后最大值自动跟着变。
  pub(super) fn player_progress_slider(
    &mut self,
    node: u32,
    dur: f64,
    pos: f64,
    window: &mut Window,
    cx: &mut Context<Self>,
  ) -> AnyElement {
    let disabled = dur <= 0.0;
    let skey = (node, format!("playerpos:{:.1}", dur.max(0.0)));
    let drag_key = format!("playerpos:{node}");
    let state = match self.sliders.get(&skey) {
      Some(s) => s.clone(),
      None => {
        let state = cx.new(|_| {
          SliderState::new()
            .min(0.0)
            .max(dur.max(1.0) as f32)
            .step(0.1)
            .scale(SliderScale::Linear)
            .default_value(pos as f32)
        });
        let dk = drag_key.clone();
        let sub = cx.subscribe(
          &state,
          move |this: &mut Workspace, _state, event: &SliderEvent, cx| match event {
            SliderEvent::Change(SliderValue::Single(v)) => {
              this.slider_drag = Some(dk.clone());
              this.set_player_seek(node, *v as f64, false, cx);
            }
            SliderEvent::Release(SliderValue::Single(v)) => {
              this.slider_drag = None;
              this.set_player_seek(node, *v as f64, true, cx);
            }
            _ => {}
          },
        );
        self.subs.push(sub);
        self.sliders.insert(skey, state.clone());
        state
      }
    };
    // 播放推进时让滑块跟着走；用户拖动期间不要抢位置。
    if !disabled && self.slider_drag.as_deref() != Some(&drag_key) {
      let cur = state.read(cx).value().end();
      if (cur - pos as f32).abs() > 0.2 {
        state.update(cx, |state, cx| state.set_value(pos as f32, window, cx));
      }
    }
    div()
      .flex_1()
      .min_w_0()
      .child(
        Slider::new(&state)
          .horizontal()
          .disabled(disabled)
          .bg(rgb(t::ACCENT))
          .text_color(rgb(t::TEXT)),
      )
      .into_any_element()
  }

  /// 播放器音量（±10 dB）：拖动实时改在播音量，松手落盘；不重建会话。
  pub(super) fn player_volume_slider(
    &mut self,
    node: u32,
    value: f64,
    window: &mut Window,
    cx: &mut Context<Self>,
  ) -> AnyElement {
    let skey = (node, "playervol".to_string());
    let drag_key = format!("playervol:{node}");
    let state = match self.sliders.get(&skey) {
      Some(s) => s.clone(),
      None => {
        let state = cx.new(|_| {
          SliderState::new()
            .min(-10.0)
            .max(10.0)
            .step(1.0)
            .scale(SliderScale::Linear)
            .default_value(value as f32)
        });
        let dk = drag_key.clone();
        let sub = cx.subscribe(
          &state,
          move |this: &mut Workspace, _state, event: &SliderEvent, cx| match event {
            SliderEvent::Change(SliderValue::Single(v)) => {
              this.slider_drag = Some(dk.clone());
              this.set_player_volume(node, *v as f64, false, cx);
            }
            SliderEvent::Release(SliderValue::Single(v)) => {
              this.slider_drag = None;
              this.set_player_volume(node, *v as f64, true, cx);
            }
            _ => {}
          },
        );
        self.subs.push(sub);
        self.sliders.insert(skey, state.clone());
        state
      }
    };
    let current = state.read(cx).value().end();
    if (current - value as f32).abs() > 0.01 {
      state.update(cx, |state, cx| state.set_value(value as f32, window, cx));
    }
    div()
      .flex_1()
      .min_w_0()
      .child(
        Slider::new(&state)
          .horizontal()
          .bg(rgb(t::ACCENT))
          .text_color(rgb(t::TEXT)),
      )
      .into_any_element()
  }

  pub(super) fn params_editor(
    &mut self,
    id: u32,
    window: &mut Window,
    cx: &mut Context<Self>,
  ) -> Div {
    let Some(n) = self.plan.node(id).cloned() else {
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
      // `far_device` 自己选（AEC 卡片）；`hotkey` 是热键参数，用录制行编辑，不当文本显示。
      if p.key == "far_device" || p.key == "hotkey" {
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
          self.param_slider(id, &ptype, p.key, lo, hi, step, v, window, cx)
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
            // 一行一个：模型名较长（purevox_denoise_202605a），横排会换行错位
            let mut row = div().flex().flex_col().items_start().gap_1();
            for (value, label) in choices {
              let v = value.clone();
              let k = key.clone();
              let on = cur == value;
              row = row.child(
                choice_button(cx, format!("pm:{id}:{}:{}", p.key, value), label, on)
                  .w_full()
                  .on_click(cx.listener(move |this, _, _window, cx| {
                    this.snapshot();
                    this.set_param(id, &k, v.clone(), cx);
                  })),
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

  /// 选中节点对应的设备流（只有真正打开设备的那几类节点才有）。
  ///
  /// 流 id 由 `engine::session` / `device_pool` 决定：池化输入输出用设备 id，
  /// 回环 / AEC 的独立采集用 `c{col}r{row}`；不要在这里猜别的形式。
  fn stream_for(&self, n: &RowSpec, ci: usize, ri: usize) -> Option<crate::debug::StreamBrief> {
    let id = match (n.kind, n.ptype.as_str()) {
      (RowKind::Input, "audio_input") => format!("input:{}", n.device.as_deref()?),
      (RowKind::Input, "loopback") => format!("input:c{ci}r{ri}"),
      (RowKind::Process, "echo_cancel") => format!("input:c{ci}r{ri}m"),
      (RowKind::Output, _) => format!("output:{}", n.device.as_deref()?),
      _ => return None,
    };
    self.hub.stream_brief(&id)
  }

  /// 节点绑定设备的枚举信息（原生格式 + host）；回环等不在枚举里的返回 None。
  fn native_for(&self, n: &RowSpec) -> Option<(String, crate::devices::NativeFormat)> {
    let id = n.device.as_deref()?;
    let dir = if n.kind == RowKind::Output {
      "output"
    } else {
      "input"
    };
    let d = self
      .devices
      .iter()
      .find(|d| d.id == id && d.direction == dir)?;
    match &d.native {
      crate::debug::Probe::Ok { value } => Some((d.host.clone(), value.clone())),
      _ => None,
    }
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

/// 速率类 `Probe<f64>` → 检查器里的短文本。
fn hz_text(p: &crate::debug::Probe<f64>) -> String {
  match p {
    crate::debug::Probe::Ok { value } => format!("{value:.0} Hz"),
    crate::debug::Probe::Pending => "采样中".into(),
    crate::debug::Probe::Unavailable { .. } => "—".into(),
  }
}

fn per_s(p: &crate::debug::Probe<f64>) -> String {
  match p {
    crate::debug::Probe::Ok { value } => format!("{value:.2}/s"),
    crate::debug::Probe::Pending => "采样中".into(),
    crate::debug::Probe::Unavailable { .. } => "—".into(),
  }
}

fn rate_positive(p: &crate::debug::Probe<f64>) -> bool {
  matches!(p, crate::debug::Probe::Ok { value } if *value > 0.05)
}

/// 参数单位（只用于上屏；注册表里没有单位字段，按 key 归类）。
fn param_unit(key: &str) -> &'static str {
  match key {
    "threshold" | "output_gain" => "dB",
    _ if key.ends_with("_db") => "dB",
    _ if key.ends_with("_time") || key.ends_with("_ms") => "ms",
    "ratio" => ":1",
    _ => "",
  }
}

/// 按步进挑显示精度：阈值 0.1 用两位、AGC 跟随速度 0.0001 用四位。
fn decimals_for_step(step: f64) -> usize {
  if step >= 0.1 {
    2
  } else if step >= 0.01 {
    3
  } else {
    4
  }
}

/// 数字框的解析：容忍全角负号、首尾空白和「3 dB」「-24ms」这类带单位的粘贴。
fn parse_number(text: &str) -> Option<f64> {
  let cleaned = text.replace(['−', '–'], "-");
  let cleaned = cleaned.trim();
  if let Ok(v) = cleaned.parse::<f64>() {
    return Some(v);
  }
  let end = cleaned
    .find(|c: char| !(c.is_ascii_digit() || matches!(c, '-' | '+' | '.' | 'e' | 'E')))
    .unwrap_or(cleaned.len());
  cleaned[..end].trim().parse::<f64>().ok()
}

/// 中心频率的上屏写法：小于 1 kHz 用整数（31.5 保留一位小数），以上用 k。
fn hz_label(hz: f32) -> String {
  if hz >= 1_000.0 {
    let k = hz / 1_000.0;
    if (k - k.round()).abs() < 0.001 {
      format!("{k:.0}k")
    } else if (k * 10.0 - (k * 10.0).round()).abs() < 0.01 {
      format!("{k:.1}k")
    } else {
      format!("{k:.2}k")
    }
  } else if (hz - hz.round()).abs() < 0.001 {
    format!("{hz:.0}")
  } else {
    format!("{hz:.1}")
  }
}

/// 圆点旁的 dB：整数就不带小数（+3），半格才写一位（-2.5）。
fn db_label(db: f32) -> String {
  if (db - db.round()).abs() < 0.05 {
    format!("{:+}", db.round() as i32)
  } else {
    format!("{db:+.1}")
  }
}

/// 内置 EQ 预设。31 段是 ISO 1/3 倍频程 20 Hz~20 kHz；按频率找最近的段。
fn eq_preset(name: &str) -> EqSettings {
  let mut s = EqSettings::default();
  let idx = |f: f32| (((f / 20.0).log2() * 3.0).round() as usize).min(30);
  match name {
    "clear" => {
      s.bands[idx(250.0)] -= 1.0;
      for f in [2_000.0, 3_150.0, 5_000.0, 8_000.0] {
        s.bands[idx(f)] += 2.0;
      }
    }
    "warm" => {
      for f in [100.0, 160.0, 250.0] {
        s.bands[idx(f)] += 2.5;
      }
      s.bands[idx(3_150.0)] -= 1.0;
    }
    "deep" => {
      for f in [40.0, 63.0, 80.0, 100.0] {
        s.bands[idx(f)] += 4.0;
      }
      s.bands[idx(315.0)] -= 1.0;
    }
    "deess" => {
      for f in [5_000.0, 6_300.0, 8_000.0] {
        s.bands[idx(f)] -= 4.0;
      }
    }
    "denasal" => {
      for f in [250.0, 315.0, 400.0] {
        s.bands[idx(f)] -= 3.0;
      }
    }
    "unmuddy" => {
      for f in [160.0, 250.0, 400.0] {
        s.bands[idx(f)] -= 2.0;
      }
      for f in [2_500.0, 4_000.0, 6_300.0] {
        s.bands[idx(f)] += 2.0;
      }
    }
    "presence" => {
      for f in [2_000.0, 3_150.0, 5_000.0] {
        s.bands[idx(f)] += 3.0;
      }
    }
    // "flat" 以及未知 key 都返回平直曲线。
    _ => {}
  }
  s
}

/// 插槽 / 预设行的左侧标签：固定宽度，保证两行格子起点对齐。
fn eq_row_label(text: &'static str) -> Div {
  div()
    .w(px(30.))
    .flex_shrink_0()
    .text_size(px(t::TEXT_XS))
    .text_color(rgb(t::MUTED))
    .child(i18n::tr(text).to_string())
}

#[cfg(test)]
mod tests {
  use super::{db_label, eq_preset, hz_label, parse_number};

  #[test]
  fn eq_presets_are_well_formed() {
    for (key, _) in super::EQ_PRESETS {
      let s = eq_preset(key);
      assert_eq!(s.bands.len(), 31, "{key}");
      assert!(s.bands.iter().all(|b| (-24.0..=24.0).contains(b)), "{key}");
    }
    assert!(eq_preset("flat").bands.iter().all(|b| *b == 0.0));
    assert!(eq_preset("deess").bands.iter().any(|b| *b < 0.0));
  }

  #[test]
  fn eq_axis_labels_are_compact() {
    assert_eq!(hz_label(20.0), "20");
    assert_eq!(hz_label(31.5), "31.5");
    assert_eq!(hz_label(800.0), "800");
    assert_eq!(hz_label(1_000.0), "1k");
    assert_eq!(hz_label(1_250.0), "1.25k");
    assert_eq!(hz_label(3_150.0), "3.15k");
    assert_eq!(hz_label(20_000.0), "20k");
    assert_eq!(db_label(3.0), "+3");
    assert_eq!(db_label(-2.5), "-2.5");
  }

  #[test]
  fn numeric_input_accepts_units_and_unicode_minus() {
    assert_eq!(parse_number("-24"), Some(-24.0));
    assert_eq!(parse_number("−24"), Some(-24.0));
    assert_eq!(parse_number(" 3 dB "), Some(3.0));
    assert_eq!(parse_number("-0.1"), Some(-0.1));
    assert_eq!(parse_number("abc"), None);
    assert_eq!(parse_number(""), None);
  }

  #[test]
  fn eq_slots_default_and_normalize() {
    use crate::engine::hot::EqSettings;
    let mut current = EqSettings::default();
    current.bands[17] = 3.0;
    let mut slots = super::super::EqSlots::with_current(&current);
    assert_eq!(slots.active, 0);
    assert_eq!(slots.slots.len(), super::super::EQ_SLOTS);
    assert_eq!(slots.slots[0].bands[17], 3.0);
    slots.active = 99;
    slots.slots.truncate(2);
    slots.normalize();
    assert_eq!(slots.active, super::super::EQ_SLOTS - 1);
    assert_eq!(slots.slots.len(), super::super::EQ_SLOTS);
  }
}
