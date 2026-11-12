// PureVox — AI 麦克风降噪工具
// Copyright (C) 2024-2026 a2heng <752848283@qq.com>
//
// PureVox is licensed under the GNU General Public License v3.0 or
// later (GPL-3.0-or-later).  See LICENSE for details.
//
// SPDX-License-Identifier: GPL-3.0-or-later

use std::time::{Duration, Instant};

use gpui_kit::PathBuilder;
use gpui_kit::component::Selectable as _;
use gpui_kit::component::slider::{Slider, SliderEvent, SliderState, SliderValue};
use gpui_kit::{AnyElement, Context, Window, canvas, div, point, prelude::*, px, rgb};

use super::theme as t;
use super::widgets::*;
use super::{DockPage, NodeModule, RecordTarget, Workspace, i18n, node_module};

impl Workspace {
  // ── 右侧信息栏（选中节点的检查器 / 全局设置）──────────────────────────

  /// 检查器本体：**只画当前选中的那一个节点**。
  ///
  /// 网络卡片 / AEC 校准跟着节点走：选中手机麦克风 / 手机输出时把网络服务卡片放进这一页，
  /// 选中 AEC 时把校准结果放进这一页。没有选中节点就只给空状态，不把其它模块堆上来。
  pub(super) fn inspector_body(
    &mut self,
    window: &mut Window,
    cx: &mut Context<Self>,
  ) -> AnyElement {
    match self.selected.and_then(|id| self.plan.node(id).cloned()) {
      Some(n) => {
        let mut body = div().flex().flex_col().gap_3();
        if node_module(&n.ptype) == NodeModule::Network {
          body = body.child(self.net_card(cx));
        }
        body
          .child(self.inspect_node(n, window, cx))
          .into_any_element()
      }
      None => div()
        .flex()
        .flex_col()
        .gap_2()
        .child(
          div()
            .text_size(px(t::TEXT_LG))
            .child(i18n::tr("没有选中节点")),
        )
        .child(hint("点画板上的节点查看参数；拖动端口连线。"))
        .into_any_element(),
    }
  }

  /// 右侧信息栏：节点检查器 **或** 全局设置，一次只显示一页。
  ///
  /// 节点页由画板选中项驱动；全局设置（更新 / 虚拟驱动 / 系统声音）从顶栏「设置」进入，
  /// 进入时清空选中，返回节点页后再点卡片即可继续。
  pub(super) fn dock(&mut self, window: &mut Window, cx: &mut Context<Self>) -> AnyElement {
    let body = match self.dock_page {
      DockPage::Node => self.inspector_body(window, cx),
      DockPage::Settings => div()
        .flex()
        .flex_col()
        .gap_3()
        .child(
          div()
            .flex()
            .items_center()
            .gap_2()
            .child(div().text_size(px(t::TEXT_LG)).child(i18n::tr("设置")))
            .child(div().flex_1())
            .child(quiet_button(cx, "dock-back", "返回").on_click(cx.listener(
              |this, _, _, cx| {
                this.dock_page = DockPage::Node;
                cx.notify();
              },
            ))),
        )
        .child(self.settings_card(cx))
        .into_any_element(),
    };
    div()
      .id("dock")
      .flex()
      .flex_col()
      .w(px(t::DOCK_W))
      .h_full()
      .bg(rgb(t::PANEL))
      .border_l_1()
      .border_color(rgb(t::BORDER))
      .child(
        div()
          .id("dock-scroll")
          .flex_1()
          .min_h_0()
          .overflow_y_scroll()
          .flex()
          .flex_col()
          .gap_3()
          .p(px(t::DOCK_PAD))
          .child(body),
      )
      .into_any_element()
  }

  /// 网络服务地址（二维码 / 复制 / 显示共用同一个）。
  pub(super) fn net_url(&self) -> String {
    let port = self
      .net_stats
      .as_ref()
      .map(|s| s.port)
      .filter(|p| *p != 0)
      .unwrap_or_else(|| crate::net::hub().port());
    let ip = if self.net_ip.is_empty() {
      "127.0.0.1".to_string()
    } else {
      self.net_ip.clone()
    };
    format!("https://{ip}:{port}/")
  }

  /// 网络卡片：服务开关 / 地址 / 二维码 / 客户端状态 / 环深度。常驻右侧。
  pub(super) fn net_card(&mut self, cx: &mut Context<Self>) -> AnyElement {
    let st = self
      .net_stats
      .clone()
      .unwrap_or_else(|| crate::net::hub().stats());
    let running = st.running;
    let dot = if running { t::OK } else { t::MUTED };
    let mut list = div().flex().flex_col().gap_2().child(
      div()
        .flex()
        .items_center()
        .gap_2()
        .child(div().w(px(8.)).h(px(8.)).rounded_full().bg(rgb(dot)))
        .child(div().text_size(px(t::TEXT_MD)).child(i18n::tr("网络")))
        .child(div().flex_1())
        .child(
          quiet_button(cx, "net-toggle", if running { "停止" } else { "启动" }).on_click(
            cx.listener(|this, _, _, cx| {
              let r = if crate::net::hub().is_running() {
                crate::net::stop();
                Ok("网络服务已停止".to_string())
              } else {
                crate::net::start()
              };
              this.status = match r {
                Ok(m) => format!("网络服务：{m}"),
                Err(e) => format!("网络服务：{e}"),
              };
              this.net_last = Instant::now() - Duration::from_secs(2);
              cx.notify();
            }),
          ),
        ),
    );
    if !running {
      return list
        .child(hint("服务未启动：启动后手机 / 网页才能连。"))
        .into_any_element();
    }
    let url = self.net_url();
    // ── 手机怎么连 ──────────────────────────────────────────────────
    list = list
      .child(section("手机连接"))
      .child(hint(
        "手机和电脑连同一个 Wi-Fi：用手机 App 扫下面的二维码；或者用浏览器直接打开地址。",
      ))
      .child(field("地址", url.clone()))
      .child(
        div()
          .flex()
          .flex_wrap()
          .gap_1()
          .child(
            quiet_button(cx, "net-copy", "复制地址").on_click(cx.listener(
              move |this, _, _, cx| {
                let url = this.net_url();
                cx.write_to_clipboard(gpui_kit::ClipboardItem::new_string(url.clone()));
                this.status = format!("已复制：{url}");
                cx.notify();
              },
            )),
          )
          .child(quiet_button(cx, "net-ca", "安装 CA").on_click(cx.listener(
            move |this, _, _, cx| {
              let url = format!("{}ca-cert", this.net_url());
              match crate::openurl::open(&url) {
                Ok(()) => this.status = format!("已打开 {url}"),
                Err(e) => this.status = format!("打开 CA 失败：{e}"),
              }
              cx.notify();
            },
          ))),
      );
    if self.net_ifaces.len() > 1 {
      let mut row = div().flex().flex_col().gap_1().w_full();
      for i in self.net_ifaces.clone() {
        let ip = i.ip.clone();
        let ip_for_click = ip.clone();
        let on = self.net_ip == ip;
        let mut card = div()
          .id(format!("net-ip:{ip}"))
          .flex()
          .flex_col()
          .gap_1()
          .w_full()
          .px(px(t::SPACE_2))
          .py(px(t::SPACE_2))
          .rounded_md()
          .border_1()
          .border_color(rgb(if on { t::ACCENT } else { t::BORDER }))
          .bg(rgb(if on { t::PANEL_HI } else { t::CONTROL }))
          .cursor_pointer()
          .tab_index(0)
          .on_click(cx.listener(move |this, _, _, cx| {
            this.net_ip = ip_for_click.clone();
            this.settings.net_ip = ip_for_click.clone();
            let _ = crate::config::save_settings(&this.settings);
            this.net_last = Instant::now() - Duration::from_secs(2);
            this.status = format!("网络地址：{ip_for_click}");
            cx.notify();
          }))
          // 第一行网卡名、第二行 IPv4、之后每个 IPv6 独占一行；窄栏里自然换行。
          .child(
            div()
              .w_full()
              .text_size(px(t::TEXT_SM))
              .text_color(rgb(t::TEXT))
              // Windows 的 AdapterName 是 `{GUID}`；网卡名用 FriendlyName（desc），
              // 没有才回退 GUID。GUID 仍保留在 `name` 里做内部标识。
              .child(if i.desc.is_empty() {
                i.name.clone()
              } else {
                i.desc.clone()
              }),
          )
          .child(
            div()
              .w_full()
              .text_size(px(t::TEXT_XS))
              .text_color(rgb(t::MUTED))
              .child(format!("IPv4  {ip}")),
          );
        for v6 in i.ipv6.clone() {
          card = card.child(
            div()
              .w_full()
              .text_size(px(t::TEXT_XS))
              .text_color(rgb(t::MUTED))
              .child(format!("IPv6  {v6}")),
          );
        }
        row = row.child(card);
      }
      list = list
        .child(hint(
          "地址要告诉手机哪一张网卡？（服务本身监听所有网卡，这里只影响二维码/复制）",
        ))
        .child(row);
    }
    match &self.net_qr {
      Some(path) => {
        list = list.child(
          div()
            .flex()
            .flex_col()
            .items_center()
            .gap_1()
            .py(px(2.))
            .child(gpui_kit::img(path.clone()).w(px(152.)).h(px(152.)))
            .child(hint("手机扫码连接")),
        );
      }
      None => {
        list = list.child(hint("二维码生成中…（手机 App 也能在局域网里自动发现本机）"));
      }
    }
    list = list.child(hint(
      "浏览器打开会提示证书不安全：点「安装 CA」，按手机提示装到系统即可永久消除。",
    ));

    // ── 连上之后发生了什么 ──────────────────────────────────────────
    list = list
      .child(section("连接状态"))
      .child(field(
        "客户端",
        if st.clients == 0 {
          "0（还没有手机连上）".to_string()
        } else {
          st.clients.to_string()
        },
      ))
      .child(field(
        "RTT",
        st.rtt_ms
          .map(|v| format!("{v:.1} ms"))
          .unwrap_or_else(|| "—（还没有 pong）".into()),
      ))
      .child(field("电脑 → 手机", probe_rate(&st.tx_packets_per_s)))
      .child(field("手机 → 电脑", probe_rate(&st.rx_packets_per_s)));
    if let Some(c) = &st.client {
      list = list.child(field(
        "手机播放环",
        format!("{:.0} / {:.0} ms（当前 / 目标）", c.level_ms, c.target_ms),
      ));
    }
    if !st.verdict.text.is_empty() {
      list = list.child(hint(format!("网络结论：{}", st.verdict.text)));
    }
    list = list.child(hint(
      "包/s：每 10 ms 一个音频包，正常约 100 包/s；为 0 表示这个方向没在传。",
    ));

    // ── 缓冲 ────────────────────────────────────────────────────────
    list = list
      .child(section("播放缓冲"))
      .child(hint(
        "手机端的抖动缓冲（10–200 ms）：越大越抗网络抖动，但延迟越高；默认 50 ms。",
      ))
      .child(self.net_ring_slider(cx));
    if st.clients == 0 {
      list = list.child(hint("扫码 / 打开地址后，这里会显示客户端与速率。"));
    }
    list.into_any_element()
  }

  /// 网络环深度滑杆（10–200 ms，10 ms 步进）。默认值在 `config::d_net_ring_ms`（50 ms）。
  pub(super) fn net_ring_slider(&mut self, cx: &mut Context<Self>) -> AnyElement {
    let cur = self.settings.net_ring_ms as f32;
    let skey = (0u32, "net_ring_ms".to_string());
    let state = match self.sliders.get(&skey) {
      Some(s) => s.clone(),
      None => {
        let state = cx.new(|_| {
          SliderState::new()
            .min(10.)
            .max(200.)
            .step(10.)
            .default_value(cur)
        });
        let sub = cx.subscribe(
          &state,
          move |this: &mut Workspace, _state, event: &SliderEvent, cx| {
            if let SliderEvent::Change(SliderValue::Single(v)) = event {
              let ms = v.round() as u32;
              crate::net::set_ring_ms(ms);
              this.settings.net_ring_ms = ms;
              this.status = format!("网络环深度 = {ms} ms");
              cx.notify();
            }
          },
        );
        self.subs.push(sub);
        self.sliders.insert(skey, state.clone());
        state
      }
    };
    div()
      .flex()
      .items_center()
      .gap_2()
      .child(Slider::new(&state).horizontal())
      .child(
        div()
          .w(px(56.))
          .text_size(px(t::TEXT_SM))
          .text_color(rgb(t::TEXT))
          .child(format!("{} ms", cur)),
      )
      .into_any_element()
  }

  /// 更新 / 虚拟驱动 / 系统声音：右侧常驻（不跟随选中节点）。
  pub(super) fn settings_card(&mut self, cx: &mut Context<Self>) -> AnyElement {
    use crate::update::Phase;
    let st = self.update_status.clone();
    let phase = match st.phase {
      Phase::Idle => "还没检查".to_string(),
      Phase::Checking => "正在检查…".to_string(),
      Phase::UpToDate => "已是最新".to_string(),
      Phase::Available => format!("发现新版本 {}", st.latest_version),
      Phase::Downloading => {
        let pct = st
          .received
          .checked_mul(100)
          .and_then(|n| n.checked_div(st.asset_size))
          .unwrap_or(0)
          .min(100);
        format!("正在下载 {pct}%")
      }
      Phase::Ready => "已下载，可安装".to_string(),
      Phase::Installing => "正在安装…".to_string(),
      Phase::Installed => "已安装，重启后生效".to_string(),
      Phase::Unsupported => st.reason.clone(),
      Phase::Failed => format!("失败：{}", st.reason),
    };

    let lang = i18n::current();
    let auto = crate::autostart::is_enabled();
    let hot = self.settings.hotkey_on;
    let cue = self.settings.cue_on;
    let hot_spec = crate::hotkey::normalize_spec(&self.settings.hotkey);
    let cue_label = |id: &str| {
      crate::cues::PRESETS
        .iter()
        .find(|(k, _)| *k == id)
        .map(|(_, l)| i18n::tr(l).to_string())
        .unwrap_or_else(|| id.to_string())
    };
    let cue_start_label = cue_label(&self.settings.cue_start);
    let cue_stop_label = cue_label(&self.settings.cue_stop);
    let cue_row = |kind: &'static str, current: String| {
      let mut row = div().flex().flex_wrap().gap_1();
      for (id, label) in crate::cues::PRESETS {
        let on = current == *id;
        let id2 = id.to_string();
        row = row.child(
          quiet_button(cx, format!("cue:{kind}:{id}"), *label)
            .when(on, |b| b.selected(true))
            .on_click(cx.listener(move |this, _, _, cx| {
              if kind == "start" {
                this.settings.cue_start = id2.clone();
              } else {
                this.settings.cue_stop = id2.clone();
              }
              this.status = match crate::config::save_settings(&this.settings) {
                Ok(()) => {
                  crate::cues::play(&id2, kind);
                  format!("提示音：{}", i18n::tr(label))
                }
                Err(e) => format!("{}{e}", i18n::tr("保存提示音设置失败：")),
              };
              cx.notify();
            })),
        );
      }
      row.into_any_element()
    };
    let lang_btn = |id: &'static str, code: &'static str, label: &'static str| {
      let on = lang == code;
      quiet_button(cx, id, label)
        .when(on, |b| b.selected(true))
        .on_click(cx.listener(move |this, _, _, cx| {
          match crate::config::save_lang(code) {
            Ok(()) => {
              i18n::set(code);
              this.status = if code == "en" {
                "Language: English".into()
              } else {
                i18n::tr("界面语言：中文").into()
              };
            }
            Err(e) => this.status = format!("{}{e}", i18n::tr("保存语言失败：")),
          }
          cx.notify();
        }))
    };

    let mut list = div()
      .flex()
      .flex_col()
      .gap_2()
      .child(section("界面语言"))
      .child(
        div()
          .flex()
          .gap_1()
          .child(lang_btn("lang-zh", "zh", "中文"))
          .child(lang_btn("lang-en", "en", "English")),
      )
      .child(section("开机自启"))
      .child(
        quiet_button(
          cx,
          "autostart",
          if auto {
            "已开启 · 点击关闭"
          } else {
            "已关闭 · 点击开启"
          },
        )
        .on_click(cx.listener(move |this, _, _, cx| {
          this.status = match crate::autostart::set(!auto) {
            Ok(()) => {
              if auto {
                "开机自启已关闭".into()
              } else {
                "开机自启已开启".into()
              }
            }
            Err(e) => format!("开机自启设置失败：{e}"),
          };
          cx.notify();
        })),
      )
      .child(section("全局热键"))
      .child(
        quiet_button(
          cx,
          "hotkey-toggle",
          if hot {
            "已开启 · 点击关闭"
          } else {
            "已关闭 · 点击开启"
          },
        )
        .on_click(cx.listener(move |this, _, _, cx| {
          this.settings.hotkey_on = !hot;
          this.status = match crate::config::save_settings(&this.settings) {
            Ok(()) => {
              this.restart_hotkeys();
              if hot {
                i18n::tr("全局热键已关闭").into()
              } else {
                i18n::tr("全局热键已开启").into()
              }
            }
            Err(e) => format!("保存热键设置失败：{e}"),
          };
          cx.notify();
        })),
      )
      .child(field(
        "当前键位",
        if hot_spec.is_empty() {
          "—".to_string()
        } else {
          hot_spec
        },
      ))
      .child(
        quiet_button(cx, "hotkey-record", "录制").on_click(cx.listener(|this, _, _, cx| {
          this.recording = Some(RecordTarget::EngineHotkey);
          this.status = i18n::tr("按下新的全局热键…（Esc 取消）").into();
          cx.notify();
        })),
      )
      .child(section("提示音"))
      .child(
        quiet_button(
          cx,
          "cue-toggle",
          if cue {
            "已开启 · 点击关闭"
          } else {
            "已关闭 · 点击开启"
          },
        )
        .on_click(cx.listener(move |this, _, _, cx| {
          this.settings.cue_on = !cue;
          this.status = match crate::config::save_settings(&this.settings) {
            Ok(()) => {
              if cue {
                i18n::tr("提示音已关闭").into()
              } else {
                i18n::tr("提示音已开启").into()
              }
            }
            Err(e) => format!("{}{e}", i18n::tr("保存提示音设置失败：")),
          };
          cx.notify();
        })),
      )
      .child(field("启动音", cue_start_label))
      .child(cue_row("start", self.settings.cue_start.clone()))
      .child(field("停止音", cue_stop_label))
      .child(cue_row("stop", self.settings.cue_stop.clone()))
      .child(section("更新"))
      .child(field(
        "当前版本",
        if st.current_stamp.is_empty() {
          format!("{}（开发构建）", st.current_version)
        } else {
          format!("{}（{}）", st.current_version, st.current_stamp)
        },
      ))
      .child(field("状态", phase));
    if !st.latest_version.is_empty() {
      list = list.child(field("最新版本", st.latest_version.clone()));
    }
    if !st.asset_name.is_empty() {
      list = list.child(field("安装包", st.asset_name.clone()));
    }
    if !st.notes.trim().is_empty() {
      let notes: String = st.notes.trim().chars().take(240).collect();
      list = list.child(hint(notes));
    }

    let mut row = div().flex().flex_wrap().gap_1();
    row = row.child(
      quiet_button(cx, "up-check", "检查更新").on_click(cx.listener(|this, _, _, cx| {
        this.status = "正在检查更新…".into();
        crate::rt::spawn(async {
          let _ = crate::update::check().await;
        });
        cx.notify();
      })),
    );
    if st.phase == Phase::Available {
      row = row.child(quiet_button(cx, "up-dl", "下载更新").on_click(cx.listener(
        |this, _, _, cx| {
          this.status = "正在下载更新…".into();
          crate::rt::spawn(async {
            let _ = crate::update::download().await;
          });
          cx.notify();
        },
      )));
    }
    if st.phase == Phase::Ready {
      row = row.child(
        quiet_button(cx, "up-inst", "安装更新").on_click(cx.listener(|this, _, _, cx| {
          this.status = match crate::update::install() {
            Ok(m) => m,
            Err(e) => format!("安装失败：{e}"),
          };
          cx.notify();
        })),
      );
    }
    if !st.url.is_empty() {
      row = row.child(
        quiet_button(cx, "up-page", "打开发布页").on_click(cx.listener(|this, _, _, cx| {
          this.status = match crate::openurl::open(&this.update_status.url) {
            Ok(()) => "已打开发布页".into(),
            Err(e) => format!("打开失败：{e}"),
          };
          cx.notify();
        })),
      );
    }
    list = list.child(row);
    if matches!(st.phase, Phase::Failed | Phase::Unsupported) {
      list = list.child(hint(st.reason.clone()));
    }

    // ── 虚拟驱动（VB-CABLE）──────────────────────────────────────────
    let vb = self
      .devices
      .iter()
      .any(|d| d.direction == "output" && d.name.contains("CABLE Input"))
      && self
        .devices
        .iter()
        .any(|d| d.direction == "input" && d.name.contains("CABLE Output"));
    list = list
      .child(section("虚拟驱动"))
      .child(field(
        "VB-CABLE",
        if vb {
          "已安装".into()
        } else {
          "未检测到".into()
        },
      ))
      .child(hint(
        "链路：麦克风 → PureVox → CABLE Input → CABLE Output → 其它软件",
      ))
      .child(
        div()
          .flex()
          .flex_wrap()
          .gap_1()
          .child(
            quiet_button(cx, "vb-download", "驱动下载").on_click(cx.listener(|this, _, _, cx| {
              let url = "https://download.vb-audio.com/Download_CABLE/VBCABLE_Driver_Pack45.zip";
              this.status = match crate::openurl::open(url) {
                Ok(()) => "已打开 VB-CABLE 驱动下载页".into(),
                Err(e) => format!("打开失败：{e}"),
              };
              cx.notify();
            })),
          )
          .child(
            quiet_button(cx, "vb-tutorial", "视频教程").on_click(cx.listener(|this, _, _, cx| {
              let url = "https://www.bilibili.com/video/BV1i2bazGEKe/";
              this.status = match crate::openurl::open(url) {
                Ok(()) => "已打开视频教程".into(),
                Err(e) => format!("打开失败：{e}"),
              };
              cx.notify();
            })),
          )
          .child(
            quiet_button(cx, "vb-panel", "VB 控制面板").on_click(cx.listener(|this, _, _, cx| {
              this.status = match crate::vb_panel::open() {
                Ok(()) => "已请求打开 VB-CABLE 控制面板".into(),
                Err(e) => format!("打开失败：{e}"),
              };
              cx.notify();
            })),
          ),
      );

    // ── 系统声音 ─────────────────────────────────────────────────────
    list = list
      .child(section("系统声音"))
      .child(
        quiet_button(cx, "open-sound", "打开系统声音设置").on_click(cx.listener(
          |this, _, _, cx| {
            this.status = match crate::sound_panel::open() {
              Ok(()) => "已打开系统声音设置".into(),
              Err(e) => format!("打开失败：{e}"),
            };
            cx.notify();
          },
        )),
      )
      .child(hint(
        "回环源在「＋ 输入 → 系统声音」里添加；这里只打开系统混音面板。",
      ));

    list.into_any_element()
  }

  /// AEC 校准：只属于**选中的那个 AEC 节点**（检查器的一部分，不常驻）。
  pub(super) fn aec_card(&self, id: u32, cx: &mut Context<Self>) -> AnyElement {
    let mut list = div().flex().flex_col().gap_2().child(section("AEC 校准"));
    let Some((ci, ri)) = self.pos(id) else {
      return list
        .child(hint("这个节点不在计划里（可能刚被删除）。"))
        .into_any_element();
    };
    list = list.child(
      div()
        .flex()
        .items_center()
        .gap_2()
        .child(
          quiet_button(cx, "aec-calib", "开始校准").on_click(cx.listener(
            move |this, _, _window, cx| {
              let mgr = this.audio.clone();
              this.spawn_job("AEC 校准", move || {
                match mgr.calibrate_aec_delay(Some((ci, ri))) {
                  Ok((msg, temp)) => {
                    mgr.finish_temp_session(temp);
                    Ok(msg)
                  }
                  Err(e) => Err(e),
                }
              });
              cx.notify();
            },
          )),
        )
        .child(hint("播放一段系统声音，让麦克风听到")),
    );
    match &self.calib_report {
      crate::debug::Probe::Ok { value } => {
        if let Some(ms) = value.delay_ms {
          list = list.child(field("远端延时", format!("{ms:.0} ms")));
        }
        if let Some(db) = value.echo_path_db {
          list = list.child(field("回声路径", format!("{db:.1} dB")));
        }
        list = list.child(hint(value.verdict_text.clone()));
        if let Some(next) = &value.next_step {
          list = list.child(hint(next.clone()));
        }
        if let Some(s) = &value.series
          && s.t_ms.len() >= 2
        {
          // 旧 Tauri 的校准曲线：近端 / 远端带内能量包络，画在同一时间轴上。
          let ts = s.t_ms.clone();
          let mic = s.mic_env_db.clone();
          let far = s.far_env_db.clone();
          list = list.child(div().h(px(96.)).w_full().child(canvas(
            |_, _, _| {},
            move |bounds, _, window, _| {
              let w = bounds.size.width.as_f32();
              let h = bounds.size.height.as_f32();
              let (t0, t1) = (ts[0], *ts.last().unwrap_or(&ts[0]));
              let span = (t1 - t0).max(1e-3);
              let (lo, hi) = (-80.0f64, 0.0f64);
              let draw = |window: &mut Window, env: &[f64], color: u32| {
                let mut pb = PathBuilder::stroke(px(1.3));
                for (i, &db) in env.iter().enumerate().take(ts.len()) {
                  let x = ((ts[i] - t0) / span) as f32 * w;
                  let v = ((db - lo) / (hi - lo)).clamp(0.0, 1.0) as f32;
                  let y = h - v * h;
                  if i == 0 {
                    pb.move_to(bounds.origin + point(px(x), px(y)));
                  } else {
                    pb.line_to(bounds.origin + point(px(x), px(y)));
                  }
                }
                if let Ok(p) = pb.build() {
                  window.paint_path(p, rgb(color));
                }
              };
              draw(window, &far, t::NET);
              draw(window, &mic, t::MIC);
            },
          )));
          list = list.child(hint("蓝：远端参考 · 绿：麦克风"));
        }
      }
      crate::debug::Probe::Pending => {
        list = list.child(hint("还没校准（结果会写进 /debug/calib）"));
      }
      crate::debug::Probe::Unavailable { reason } => {
        list = list.child(hint(reason.clone()));
      }
    }
    list.into_any_element()
  }
}
