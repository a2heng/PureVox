// PureVox — AI 麦克风降噪工具
// Copyright (C) 2024-2026 a2heng <752848283@qq.com>
//
// PureVox is licensed under the GNU General Public License v3.0 or
// later (GPL-3.0-or-later).  See LICENSE for details.
//
// SPDX-License-Identifier: GPL-3.0-or-later

use std::time::{Duration, Instant};

use gpui_kit::PathBuilder;
use gpui_kit::component::Sizable as _;
use gpui_kit::component::button::{Button, ButtonVariants as _};
use gpui_kit::component::slider::{Slider, SliderEvent, SliderState, SliderValue};
use gpui_kit::{
  AnyElement, Context, Entity, Window, canvas, div, point, prelude::*, px, rgb, rgba,
};

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
        let mut body = div().flex().flex_col().gap_2();
        if node_module(&n.ptype) == NodeModule::Network {
          body = body.child(self.net_card(window, cx));
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
        .gap_2()
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
      .on_mouse_up(
        gpui_kit::MouseButton::Left,
        cx.listener(|this, _, _, cx| {
          if this.menu.is_some() {
            this.menu = None;
            cx.notify();
          }
        }),
      )
      .child(
        div()
          .id("dock-scroll")
          .flex_1()
          .min_h_0()
          .overflow_y_scroll()
          .flex()
          .flex_col()
          .gap_2()
          .p(px(t::DOCK_PAD))
          .child(body),
      )
      .into_any_element()
  }

  /// 网络服务地址（二维码 / 复制 / 显示共用同一个）。
  pub(super) fn net_url(&self) -> String {
    let port = self.net_port_draft.unwrap_or_else(|| {
      self
        .net_stats
        .as_ref()
        .map(|s| s.port)
        .filter(|p| *p != 0)
        .unwrap_or_else(|| crate::net::hub().port())
    });
    let ip = if self.net_ip.is_empty() {
      "127.0.0.1".to_string()
    } else {
      self.net_ip.clone()
    };
    format!("https://{ip}:{port}/")
  }

  /// 网络卡片：服务开关 / 地址 / 二维码 / 客户端状态 / 环深度。常驻右侧。
  pub(super) fn net_card(&mut self, window: &mut Window, cx: &mut Context<Self>) -> AnyElement {
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
    let port = st.port;
    // ── 第二行：左边说明、右边二维码（横向排，省掉原来竖着堆的高度）─────────
    let mut qr_box = div()
      .flex()
      .flex_col()
      .items_center()
      .gap_1()
      .w(px(128.))
      .flex_shrink_0();
    match &self.net_qr {
      Some(path) => {
        qr_box = qr_box
          .child(gpui_kit::img(path.clone()).w(px(120.)).h(px(120.)))
          .child(
            div()
              .text_size(px(t::TEXT_XS))
              .text_color(rgb(t::MUTED))
              .child(i18n::tr("手机扫码连接")),
          );
      }
      None => {
        qr_box = qr_box.child(
          div()
            .text_size(px(t::TEXT_XS))
            .text_color(rgb(t::MUTED))
            .child(i18n::tr("二维码生成中…")),
        );
      }
    }
    list = list.child(
      div()
        .flex()
        .items_center()
        .gap_2()
        .child(
          div()
            .flex()
            .flex_1()
            .min_w_0()
            .flex_col()
            .gap_1()
            .child(hint(
              "手机和电脑连同一个 Wi-Fi：用 App 扫右边的码，或用浏览器打开下面的地址。",
            ))
            .child(hint(
              "浏览器首次访问会提示证书不安全：点「CA 说明」安装一次本地 CA。",
            )),
        )
        .child(qr_box),
    );
    // ── 第三行：地址:可编辑端口 · 复制 · CA 说明 ─────────────────────
    let port_state = match self.net_port_input.clone() {
      Some(s) => s,
      None => {
        let s = cx.new(|cx| {
          gpui_kit::component::input::InputState::new(window, cx).default_value(port.to_string())
        });
        let sub = cx.subscribe(
          &s,
          |this: &mut Workspace,
           state: Entity<gpui_kit::component::input::InputState>,
           event: &gpui_kit::component::input::InputEvent,
           cx| {
            if matches!(
              event,
              gpui_kit::component::input::InputEvent::PressEnter { .. }
                | gpui_kit::component::input::InputEvent::Blur
            ) {
              let value = state.read(cx).value().to_string();
              this.apply_net_port(&value, cx);
            } else if matches!(event, gpui_kit::component::input::InputEvent::Change) {
              let value = state.read(cx).value().to_string();
              this.on_net_port_typed(&value, cx);
            }
          },
        );
        self.subs.push(sub);
        self.net_port_input = Some(s.clone());
        s
      }
    };
    let addr_ip = if self.net_ip.is_empty() {
      "127.0.0.1".to_string()
    } else {
      self.net_ip.clone()
    };
    list = list.child(
      div()
        .flex()
        .flex_wrap()
        .items_center()
        .gap_1()
        .min_w(px(0.))
        .child(
          div()
            .flex_shrink_0()
            .text_size(px(t::TEXT_SM))
            .text_color(rgb(t::TEXT))
            .child(format!("{addr_ip}:")),
        )
        .child(
          div().w(px(64.)).flex_shrink_0().child(
            gpui_kit::component::input::Input::new(&port_state)
              .appearance(true)
              .bordered(true)
              .focus_bordered(true)
              .with_size(gpui_kit::component::Size::XSmall)
              .w(px(64.))
              .aria_label(i18n::tr("端口").to_string()),
          ),
        )
        .child(
          quiet_button(cx, "net-copy", "复制").on_click(cx.listener(move |this, _, _, cx| {
            let url = this.net_url();
            cx.write_to_clipboard(gpui_kit::ClipboardItem::new_string(url.clone()));
            this.status = format!("已复制：{url}");
            cx.notify();
          })),
        )
        .child(quiet_button(cx, "net-ca", "CA 说明").on_click(cx.listener(
          move |this, _, _, cx| {
            match crate::net::tls::write_help_page(i18n::current()) {
              Ok(page) => match crate::openurl::open(&page.to_string_lossy()) {
                Ok(()) => this.status = format!("已打开 CA 说明：{}", page.display()),
                Err(e) => this.status = format!("打开 CA 说明失败：{e}"),
              },
              Err(e) => this.status = e,
            }
            cx.notify();
          },
        ))),
    );
    // ── 第四行：紧凑的网卡选择卡片（一行一个网卡，不再铺 IPv6）──────────
    if self.net_ifaces.len() > 1 {
      let mut row = div().flex().flex_wrap().gap_1().w_full();
      for i in self.net_ifaces.clone() {
        let ip = i.ip.clone();
        let ip_for_click = ip.clone();
        let on = self.net_ip == ip;
        let name = if i.desc.is_empty() {
          i.name.clone()
        } else {
          i.desc.clone()
        };
        // IPv6 不占卡片行，鼠标悬停才显示（可能有好几条）。
        let mut tip = String::new();
        for v6 in &i.ipv6 {
          tip.push_str("IPv6  ");
          tip.push_str(v6);
          tip.push('\n');
        }
        if tip.is_empty() {
          tip = format!("IPv4  {ip}");
        }
        row = row.child(
          Button::new(format!("net-ip:{ip}"))
            .xsmall()
            .ghost()
            .flex_1()
            .min_w(px(138.))
            .flex()
            .items_center()
            .gap_1()
            .px(px(t::SPACE_2))
            .py(px(t::SPACE_1))
            .rounded_md()
            .border_1()
            .border_color(rgb(if on { t::ACCENT } else { t::BORDER }))
            .bg(rgb(if on { t::PANEL_HI } else { t::CONTROL }))
            .tooltip(tip.trim_end().to_string())
            .on_click(cx.listener(move |this, _, _, cx| {
              this.net_ip = ip_for_click.clone();
              this.settings.net_ip = ip_for_click.clone();
              let _ = crate::config::save_settings(&this.settings);
              this.net_last = Instant::now() - Duration::from_secs(2);
              this.status = format!("网络地址：{ip_for_click}");
              cx.notify();
            }))
            .child(
              div()
                .flex_1()
                .min_w(px(0.))
                .truncate()
                .text_size(px(t::TEXT_XS))
                .text_color(rgb(t::TEXT))
                .child(name),
            )
            .child(
              div()
                .flex_shrink_0()
                .text_size(px(t::TEXT_XS))
                .text_color(rgb(if on { t::ACCENT } else { t::MUTED }))
                .child(ip),
            ),
        );
      }
      list = list.child(row);
    }

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

  /// 输入框每次变化：二维码 / 复制立即跟着草稿端口走；停止输入 700 ms 后才真正重启服务。
  fn on_net_port_typed(&mut self, text: &str, cx: &mut Context<Self>) {
    self.net_port_gen = self.net_port_gen.wrapping_add(1);
    let generation = self.net_port_gen;
    match text
      .trim()
      .parse::<u16>()
      .ok()
      .filter(|p| (1024..=65535).contains(p))
    {
      Some(p) => {
        self.net_port_draft = Some(p);
        self.net_last = Instant::now() - Duration::from_secs(2);
        self.status = format!("端口 {p}（停止输入后应用）");
        let handle = cx.spawn(async move |weak, cx| {
          cx.background_executor()
            .timer(Duration::from_millis(700))
            .await;
          let _ = weak.update(cx, |this, cx| {
            if this.net_port_gen == generation {
              this.apply_net_port_value(p, cx);
            }
          });
        });
        handle.detach();
      }
      None => {
        self.net_port_draft = None;
        self.status = "端口必须是 1024–65535 的数字".into();
      }
    }
    cx.notify();
  }

  /// 端口输入框 Enter / 失焦：立即保存并重启网络服务（换端口必须重绑监听）。
  fn apply_net_port(&mut self, text: &str, cx: &mut Context<Self>) {
    let Ok(p) = text.trim().parse::<u16>() else {
      self.status = "端口必须是 1024–65535 的数字".into();
      cx.notify();
      return;
    };
    if !(1024..=65535).contains(&p) {
      self.status = "端口必须是 1024–65535 的数字".into();
      cx.notify();
      return;
    }
    self.apply_net_port_value(p, cx);
  }

  fn apply_net_port_value(&mut self, p: u16, cx: &mut Context<Self>) {
    self.net_port_draft = Some(p);
    if self.settings.net_port == p {
      return;
    }
    self.settings.net_port = p;
    let saved = crate::config::save_settings(&self.settings);
    crate::net::hub().set_port(p);
    let r = crate::net::restart();
    self.status = match (saved, r) {
      (Err(e), _) => format!("端口已应用，但设置保存失败：{e}"),
      (_, Ok(m)) => format!("端口 {p}：{m}"),
      (_, Err(e)) => format!("端口 {p} 启动失败：{e}"),
    };
    self.net_last = Instant::now() - Duration::from_secs(2);
    self.net_qr_url.clear();
    self.net_qr = None;
    cx.notify();
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
          choice_button(cx, format!("cue:{kind}:{id}"), *label, on).on_click(cx.listener(
            move |this, _, _, cx| {
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
            },
          )),
        );
      }
      row.into_any_element()
    };
    let lang_btn = |id: &'static str, code: &'static str, label: &'static str| {
      let on = lang == code;
      choice_button(cx, id, label, on).on_click(cx.listener(move |this, _, _, cx| {
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

    let entity = cx.entity();
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
      // 一个开关同时管两件事（与旧 Tauri 版的「开机自启 + 启动自运行并隐藏窗口」
      // 两个勾选合成一个）：写注册表启动项，并让下次启动自动跑引擎、收进托盘。
      .child(switch_row(
        "autostart",
        "开机自启（静默运行）",
        auto,
        {
          let entity = entity.clone();
          move |next, _, cx| {
            let on = *next;
            entity.update(cx, |this, cx| {
              this.settings.start_hidden = on;
              let auto = crate::autostart::set(on);
              let saved = crate::config::save_settings(&this.settings);
              this.status = match (auto, saved) {
                (Ok(()), Ok(())) => {
                  if on {
                    "已开启：登录后自动运行引擎并隐藏到托盘".into()
                  } else {
                    "已关闭开机自启".into()
                  }
                }
                (Err(e), _) => format!("开机自启设置失败：{e}"),
                (_, Err(e)) => format!("保存静默启动设置失败：{e}"),
              };
              cx.notify();
            });
          }
        },
      ))
      .child(hint("登录后自动运行引擎并隐藏到托盘；点托盘图标显示窗口。"))
      .child(switch_row("hotkey-toggle", "全局热键", hot, {
        let entity = entity.clone();
        move |next, _, cx| {
          let on = *next;
          entity.update(cx, |this, cx| {
            this.settings.hotkey_on = on;
            this.status = match crate::config::save_settings(&this.settings) {
              Ok(()) => {
                this.restart_hotkeys();
                if on {
                  i18n::tr("全局热键已开启").into()
                } else {
                  i18n::tr("全局热键已关闭").into()
                }
              }
              Err(e) => format!("保存热键设置失败：{e}"),
            };
            cx.notify();
          });
        }
      }))
      .child(
        div()
          .flex()
          .items_center()
          .justify_between()
          .gap_2()
          .child(
            div()
              .flex()
              .items_center()
              .gap_2()
              .text_size(px(t::TEXT_SM))
              .child(
                div()
                  .text_color(rgb(t::MUTED))
                  .child(i18n::tr("当前键位").to_string()),
              )
              .child(
                div()
                  .text_color(rgb(t::TEXT))
                  .child(if hot_spec.is_empty() {
                    "—".to_string()
                  } else {
                    hot_spec
                  }),
              ),
          )
          .child(
            quiet_button(cx, "hotkey-record", "录制").on_click(cx.listener(|this, _, _, cx| {
              this.begin_record(
                RecordTarget::EngineHotkey,
                "按下新的全局热键…（Esc 取消）",
                cx,
              );
            })),
          ),
      )
      .child(switch_row("cue-toggle", "提示音", cue, {
        let entity = entity.clone();
        move |next, _, cx| {
          let on = *next;
          entity.update(cx, |this, cx| {
            this.settings.cue_on = on;
            this.status = match crate::config::save_settings(&this.settings) {
              Ok(()) => {
                if on {
                  i18n::tr("提示音已开启").into()
                } else {
                  i18n::tr("提示音已关闭").into()
                }
              }
              Err(e) => format!("{}{e}", i18n::tr("保存提示音设置失败：")),
            };
            cx.notify();
          });
        }
      }))
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
        ghost_button(cx, "up-page", "打开发布页").on_click(cx.listener(|this, _, _, cx| {
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
            ghost_button(cx, "vb-tutorial", "视频教程").on_click(cx.listener(|this, _, _, cx| {
              let url = "https://www.bilibili.com/video/BV1i2bazGEKe/";
              this.status = match crate::openurl::open(url) {
                Ok(()) => "已打开视频教程".into(),
                Err(e) => format!("打开失败：{e}"),
              };
              cx.notify();
            })),
          )
          .child(
            ghost_button(cx, "vb-panel", "VB 控制面板").on_click(cx.listener(|this, _, _, cx| {
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
        ghost_button(cx, "open-sound", "打开系统声音设置").on_click(cx.listener(
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
        "回环源在「添加节点 → 输入 → 系统声音」里添加；这里只打开系统混音面板。",
      ));

    list.into_any_element()
  }

  /// AEC 校准：只属于**选中的那个 AEC 节点**（检查器的一部分，不常驻）。
  pub(super) fn aec_card(&self, id: u32, cx: &mut Context<Self>) -> AnyElement {
    let mut list = div().flex().flex_col().gap_1().child(section("AEC 校准"));
    if self.pos(id).is_none() {
      return list
        .child(hint("这个节点不在计划里（可能刚被删除）。"))
        .into_any_element();
    }
    list = list.child(
      div()
        .flex()
        .items_center()
        .gap_2()
        .child(
          quiet_button(cx, "aec-calib", "开始校准")
            .on_click(cx.listener(move |this, _, _window, cx| this.start_calibrate(id, cx))),
        )
        .child(hint("从远端设备播放扫频探针")),
    );
    // 按钮下面**只有一张图**：延时 / 增益这些结论已经回填到上面的参数行，
    // 这里再列一遍只会重复，而且会让校准区在校准前后高度跳动。
    let (series, mic_peak_db, far_peak_db, mic_peak_ms, far_peak_ms) = match &self.calib_report {
      crate::debug::Probe::Ok { value } => (
        value.series.clone(),
        value.mic_band_rms_db,
        value.far_band_rms_db,
        value.mic_peak_ms,
        value.far_peak_ms,
      ),
      _ => (None, None, None, None, None),
    };

    // ---- 校准曲线 ----
    // 数据来自 `CalibReport.series`（启动时从 calib_last.json 读回，不是截图）：
    // 两条带内能量包络 + 峰值横线（标数值）+ 峰值时刻竖线 + 延时差箭头，
    // 刻度全部画在框里（右侧 dB、底部时间）。
    let plot_data = series
      .as_ref()
      .filter(|s| s.t_ms.len() >= 2)
      .map(|s| (s.t_ms.clone(), s.mic_env_db.clone(), s.far_env_db.clone()));
    let has_plot = plot_data.is_some();

    // 纵轴：只看 60 dB 窗口（探针前的数字静音是 -120 dB，整段铺开会把曲线压成一条缝），
    // 上下取整到 10 dB。
    let (axis_lo, axis_hi) = plot_data
      .as_ref()
      .map(|(_, mic, far)| {
        let mut lo = f64::INFINITY;
        let mut hi = f64::NEG_INFINITY;
        for &v in mic.iter().chain(far.iter()) {
          if v.is_finite() {
            lo = lo.min(v);
            hi = hi.max(v);
          }
        }
        if !lo.is_finite() || !hi.is_finite() {
          lo = -80.0;
          hi = 0.0;
        }
        let lo = (lo.max(hi - 60.0) / 10.0).floor() * 10.0;
        let hi = ((hi / 10.0).ceil() * 10.0).max(lo + 20.0);
        (lo, hi)
      })
      .unwrap_or((-80.0, 0.0));

    const PLOT_H: f32 = 210.0;
    const AXIS_W: f32 = 36.0;
    const AXIS_H: f32 = 16.0;
    let db_ticks: Vec<f64> = {
      let mut v = Vec::new();
      let mut db = axis_hi;
      while db >= axis_lo - 0.5 {
        v.push(db);
        db -= 10.0;
      }
      v
    };
    let time_ticks: Vec<String> = match &plot_data {
      Some((ts, _, _)) => {
        let t_end = ts.last().copied().unwrap_or(0.0) / 1000.0;
        (0..5)
          .map(|i| {
            let s = t_end * i as f64 / 4.0;
            if t_end >= 1.0 {
              format!("{s:.1}s")
            } else {
              format!("{s:.0}ms")
            }
          })
          .collect()
      }
      None => vec![
        "0.0s".into(),
        "0.5s".into(),
        "1.0s".into(),
        "1.5s".into(),
        "2.0s".into(),
      ],
    };

    let plot = div()
      .relative()
      .h(px(PLOT_H))
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
            let w = (all_w - AXIS_W).max(8.0); // 曲线 + 网格区
            let h = (all_h - AXIS_H).max(8.0);
            let o = bounds.origin;
            let y_of = |db: f64| h * ((axis_hi - db) / (axis_hi - axis_lo)).clamp(0.0, 1.0) as f32;
            let grid = rgba((t::BORDER << 8) | 0x80);
            // 横向 10 dB 网格
            for &db in &db_ticks {
              let y = o.y + px(y_of(db));
              let mut pb = PathBuilder::stroke(px(1.));
              pb.move_to(point(o.x, y));
              pb.line_to(point(o.x + px(w), y));
              if let Ok(p) = pb.build() {
                window.paint_path(p, grid);
              }
            }
            // 纵向时间网格（4 等分）
            for i in 1..4 {
              let x = o.x + px(w * i as f32 / 4.0);
              let mut pb = PathBuilder::stroke(px(1.));
              pb.move_to(point(x, o.y));
              pb.line_to(point(x, o.y + px(h)));
              if let Ok(p) = pb.build() {
                window.paint_path(p, grid);
              }
            }
            // 右侧 dB 刻度（框内）
            for &db in &db_ticks {
              let y = y_of(db) - 5.0;
              if y > h - 10.0 {
                continue;
              }
              let line = shape_text(window, &format!("{db:.0}"), 9.0, t::MUTED);
              let x = all_w - 4.0 - line.width.as_f32();
              let _ = line.paint(
                point(o.x + px(x), o.y + px(y.max(0.0))),
                px(10.),
                gpui_kit::TextAlign::Left,
                None,
                window,
                cx,
              );
            }
            // 底部时间刻度（框内）
            for (i, label) in time_ticks.iter().enumerate() {
              let line = shape_text(window, label, 9.0, t::MUTED);
              let x = match i {
                0 => 2.0,
                4 => w - line.width.as_f32() - 2.0,
                _ => w * i as f32 / 4.0 - line.width.as_f32() / 2.0,
              };
              let _ = line.paint(
                point(o.x + px(x), o.y + px(h + 3.0)),
                px(10.),
                gpui_kit::TextAlign::Left,
                None,
                window,
                cx,
              );
            }

            let Some((ts, mic, far)) = plot_data.as_ref() else {
              return;
            };
            if ts.len() < 2 {
              return;
            }
            let t_end = ts.last().copied().unwrap_or(0.0).max(1e-3);
            let x_of = |t: f64| (t / t_end).clamp(0.0, 1.0) as f32 * w;
            // 虚线（峰值横线 / 峰值时刻竖线）：手画短段，省得引入 dash 支持。
            let dashed_h = |window: &mut Window, y: f32, color: gpui_kit::Rgba| {
              let mut pb = PathBuilder::stroke(px(1.));
              let mut x = 0.0f32;
              while x < w {
                let x2 = (x + 5.0).min(w);
                pb.move_to(point(o.x + px(x), o.y + px(y)));
                pb.line_to(point(o.x + px(x2), o.y + px(y)));
                x += 9.0;
              }
              if let Ok(p) = pb.build() {
                window.paint_path(p, color);
              }
            };
            let dashed_v = |window: &mut Window, x: f32, color: gpui_kit::Rgba| {
              let mut pb = PathBuilder::stroke(px(1.));
              let mut y = 0.0f32;
              while y < h {
                let y2 = (y + 5.0).min(h);
                pb.move_to(point(o.x + px(x), o.y + px(y)));
                pb.line_to(point(o.x + px(x), o.y + px(y2)));
                y += 9.0;
              }
              if let Ok(p) = pb.build() {
                window.paint_path(p, color);
              }
            };
            // 峰值横线 + 数值（蓝 = 远端，黄 = 麦克风）
            for (db, label, color, below) in [
              (far_peak_db, i18n::tr("远端峰值"), t::NET, false),
              (mic_peak_db, i18n::tr("麦克风峰值"), t::AEC, true),
            ] {
              let Some(db) = db else { continue };
              let y = y_of(db);
              dashed_h(window, y, rgba((color << 8) | 0xA0));
              let line = shape_text(window, &format!("{label} {db:.1} dBFS"), 9.0, color);
              let x = (w - 4.0 - line.width.as_f32()).max(2.0);
              let ly = if below {
                (y + 2.0).min(h - 10.0)
              } else {
                (y - 11.0).max(0.0)
              };
              let _ = line.paint(
                point(o.x + px(x), o.y + px(ly)),
                px(10.),
                gpui_kit::TextAlign::Left,
                None,
                window,
                cx,
              );
            }
            // 峰值时刻竖线 + 时刻（两条线的水平距离就是延时差）
            for (t, label, color, top) in [
              (far_peak_ms, i18n::tr("远端"), t::NET, 2.0),
              (mic_peak_ms, i18n::tr("麦克风"), t::AEC, 14.0),
            ] {
              let Some(t) = t else { continue };
              let x = x_of(t);
              dashed_v(window, x, rgba((color << 8) | 0x80));
              let line = shape_text(window, &format!("{label} {:.2}s", t / 1000.0), 9.0, color);
              let mut tx = x + 3.0;
              if tx + line.width.as_f32() > w - 2.0 {
                tx = (x - 3.0 - line.width.as_f32()).max(2.0);
              }
              let _ = line.paint(
                point(o.x + px(tx), o.y + px(top)),
                px(10.),
                gpui_kit::TextAlign::Left,
                None,
                window,
                cx,
              );
            }
            // 延时差：两根峰值竖线之间的双向箭头 + Δ 数值
            if let (Some(a), Some(b)) = (far_peak_ms, mic_peak_ms) {
              let (x0, x1) = (x_of(a), x_of(b));
              let y_arrow = h * 0.62;
              let mut pb = PathBuilder::stroke(px(1.));
              pb.move_to(point(o.x + px(x0), o.y + px(y_arrow)));
              pb.line_to(point(o.x + px(x1), o.y + px(y_arrow)));
              pb.move_to(point(o.x + px(x0), o.y + px(y_arrow)));
              pb.line_to(point(o.x + px(x0 + 4.0), o.y + px(y_arrow - 3.0)));
              pb.move_to(point(o.x + px(x0), o.y + px(y_arrow)));
              pb.line_to(point(o.x + px(x0 + 4.0), o.y + px(y_arrow + 3.0)));
              pb.move_to(point(o.x + px(x1), o.y + px(y_arrow)));
              pb.line_to(point(o.x + px(x1 - 4.0), o.y + px(y_arrow - 3.0)));
              pb.move_to(point(o.x + px(x1), o.y + px(y_arrow)));
              pb.line_to(point(o.x + px(x1 - 4.0), o.y + px(y_arrow + 3.0)));
              if let Ok(p) = pb.build() {
                window.paint_path(p, rgba((t::TEXT << 8) | 0xB0));
              }
              let delta = (b - a).abs();
              let label = format!("Δ {delta:.0} ms");
              let line = shape_text(window, &label, 9.0, t::TEXT);
              let gap = (x1 - x0).abs();
              let mut lx = (x0 + x1) / 2.0 - line.width.as_f32() / 2.0;
              if gap < line.width.as_f32() + 8.0 {
                lx = (x0.max(x1) + 4.0).min(w - line.width.as_f32() - 2.0);
              }
              let ly = (y_arrow - 13.0).max(0.0);
              let _ = line.paint(
                point(o.x + px(lx.max(2.0)), o.y + px(ly)),
                px(10.),
                gpui_kit::TextAlign::Left,
                None,
                window,
                cx,
              );
            }

            let draw = |window: &mut Window, env: &[f64], color: u32| {
              let mut pb = PathBuilder::stroke(px(1.6));
              let mut pen = false;
              for (i, &db) in env.iter().enumerate().take(ts.len()) {
                if !db.is_finite() {
                  pen = false;
                  continue;
                }
                let p = o + point(px(x_of(ts[i])), px(y_of(db)));
                if pen {
                  pb.line_to(p);
                } else {
                  pb.move_to(p);
                  pen = true;
                }
              }
              if let Ok(p) = pb.build() {
                window.paint_path(p, rgb(color));
              }
            };
            draw(window, far, t::NET);
            draw(window, mic, t::AEC);
          },
        )
        .w_full()
        .h_full(),
      )
      .when(!has_plot, |this| {
        this.child(
          div()
            .absolute()
            .inset_0()
            .flex()
            .items_center()
            .justify_center()
            .child(hint("校准后显示远端 / 麦克风包络")),
        )
      });

    list = list.child(plot).child(
      div()
        .flex()
        .items_center()
        .gap_1()
        .text_size(px(t::TEXT_XS))
        .text_color(rgb(t::MUTED))
        .child(div().size(px(6.)).rounded_full().bg(rgb(t::NET)))
        .child(i18n::tr("远端参考"))
        .child(div().size(px(6.)).rounded_full().bg(rgb(t::AEC)))
        .child(i18n::tr("麦克风")),
    );
    list.into_any_element()
  }
}
