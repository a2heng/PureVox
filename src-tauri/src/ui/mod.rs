// PureVox — AI 麦克风降噪工具
// Copyright (C) 2024-2026 a2heng <752848283@qq.com>
//
// PureVox is licensed under the GNU General Public License v3.0 or
// later (GPL-3.0-or-later).  See LICENSE for details.
//
// SPDX-License-Identifier: GPL-3.0-or-later

//! Session V2 界面：Element 风格节点画板 + 每节点检查器。
//!
//! 画板只负责「节点 / 端口 / 线」；所有参数在右侧检查器，且**只显示选中的那一个节点**。

mod canvas;
mod dock;
mod graph;
mod i18n;
mod inspector;
mod qr;
mod theme;
mod widgets;

use widgets::*;

use std::cell::Cell;
use std::rc::Rc;
use std::sync::Arc;
use std::time::{Duration, Instant};

use gpui_kit::component::button::{Button, ButtonVariants as _};
use gpui_kit::component::slider::SliderState;
use gpui_kit::component::{IconName, Sizable as _};
use gpui_kit::{
  AnyElement, Context, Entity, FocusHandle, KeyBinding, Subscription, Window, div, prelude::*, px,
  rgb,
};

use crate::audio::AudioManager;
use crate::debug::{SharedHub, UiCommand};
use crate::plan::{Plan, RowKind, RowSpec, input_ports};
use theme as t;

gpui_kit::actions!([ToggleEngine, RefreshDevices, Undo, Redo, DeleteNode]);

pub fn key_bindings() -> Vec<KeyBinding> {
  vec![
    KeyBinding::new("space", ToggleEngine, None),
    KeyBinding::new("ctrl-r", RefreshDevices, None),
    KeyBinding::new("ctrl-z", Undo, None),
    KeyBinding::new("ctrl-shift-z", Redo, None),
    KeyBinding::new("delete", DeleteNode, None),
  ]
}

type Job = Arc<std::sync::Mutex<Option<Result<String, String>>>>;

const HEADER_H: f32 = 30.0;
const TERM_H: f32 = 24.0;

/// 端子只占**一排**：号码 + 边缘上的接线凸起，通道再多也不加厚。
fn term_rows(n: usize) -> usize {
  if n == 0 { 0 } else { 1 }
}
const PORT_SIZE: f32 = 12.0;
/// 端子固定槽位：第 1 路永远在同一 x，第 2 路永远在 +22，和卡片类型/通道数无关。
const TERM_SLOT: f32 = 22.0;
const TERM_PAD: f32 = 8.0;
const GRID: f32 = 8.0;
const CANVAS_W: f32 = 4200.0;
const CANVAS_H: f32 = 2600.0;

#[derive(Clone, Copy)]
enum DragPayload {
  Move { id: u32 },
  Port { node: u32, port: u8, input: bool },
}

/// 鼠标按下时记下的真实起点。`on_drag` 的构造参数是**拖动视图的左上角**，
/// 不是指针位置 —— 拿它当锚点会让卡片整体偏移一个抓取距离（看起来就是「飞出去」）。
#[derive(Clone, Copy)]
enum Pending {
  Node {
    id: u32,
    pointer: gpui_kit::Point<gpui_kit::Pixels>,
    start_x: f32,
    start_y: f32,
  },
  Port {
    node: u32,
    port: u8,
    input: bool,
    pointer: gpui_kit::Point<gpui_kit::Pixels>,
    start_x: f32,
    start_y: f32,
  },
}

/// 正在进行的拖动。位置一律用**绝对锚点**：起点 + (当前指针 − 起点指针)，
/// 不逐事件累加 delta（事件被合并时会永久丢位移）。
#[derive(Clone, Copy)]
enum Drag {
  Node {
    id: u32,
    start_x: f32,
    start_y: f32,
    pointer: gpui_kit::Point<gpui_kit::Pixels>,
  },
  Wire {
    node: u32,
    port: u8,
    input: bool,
    start_x: f32,
    start_y: f32,
    pointer: gpui_kit::Point<gpui_kit::Pixels>,
    cur: (f32, f32),
    target: Option<(u32, bool, u8)>,
  },
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum Menu {
  Source,
  Ai,
  Process,
  Content,
  Output,
}

/// 右侧信息栏当前展示的内容。
///
/// 只有两种目的地：选中节点的检查器（画板点哪张卡就显示哪张卡）和全局设置。
/// 网络 / AEC 校准不再常驻 —— 它们跟着对应的节点走（发 `remote_*` / `echo_cancel` 节点时
/// 出现在该节点的检查器里）。这样右侧永远只有「当前选中对象」的一页，不会堆成一条长列表。
#[derive(Clone, Copy, PartialEq, Eq)]
enum DockPage {
  Node,
  Settings,
}

/// 节点检查器里的「专属模块」。右侧只画当前节点需要的那一块：
/// 网络节点带服务卡片，AEC 节点带校准；其它节点什么也不额外加。
/// 正在等待下一个键位组合的目标（设置页的启停热键 / 某个音效）。
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum RecordTarget {
  EngineHotkey,
  Pad { node: u32, index: usize },
}

/// EQ 画布在窗口里的逻辑像素矩形（prepaint 时写入，鼠标事件读）。
#[derive(Clone, Copy, Default)]
struct EqGeom {
  x: f32,
  y: f32,
  w: f32,
  h: f32,
}

/// 正在拖动的 EQ 段：节点 + 段号（横向拖动不换段）。
#[derive(Clone, Copy)]
struct EqDrag {
  node: u32,
  band: usize,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum NodeModule {
  None,
  Network,
  Aec,
}

fn node_module(ptype: &str) -> NodeModule {
  match ptype {
    "remote_mic" | "remote_speaker" => NodeModule::Network,
    "echo_cancel" => NodeModule::Aec,
    _ => NodeModule::None,
  }
}

struct PendingMedia {
  node: u32,
  kind: crate::media::MediaKind,
  result: Result<crate::media::Imported, String>,
}

pub struct Workspace {
  focus_handle: FocusHandle,
  audio: Arc<AudioManager>,
  hub: SharedHub,
  plan: Plan,
  devices: Vec<crate::devices::DeviceInfo>,
  last_refresh: Instant,
  selected: Option<u32>,
  /// 右侧信息栏的页面：节点检查器 / 全局设置（见 `DockPage`）
  dock_page: DockPage,
  status: String,
  job: Option<Job>,
  latency: crate::debug::Probe<crate::engine::latency::LatencyReport>,
  drag: Option<Drag>,
  pending: Option<Pending>,
  menu: Option<Menu>,
  undo: Vec<Plan>,
  redo: Vec<Plan>,
  sliders: std::collections::HashMap<(u32, String), Entity<SliderState>>,
  subs: Vec<Subscription>,
  /// 设置缓存（网络端口 / 抖动缓冲等；右侧网络卡片直接改它并落盘）
  settings: crate::config::AppSettings,
  /// 网络服务快照（每 1 s 刷一次；卡片常驻在右侧）
  net_stats: Option<crate::net::hub::NetStats>,
  net_ifaces: Vec<crate::net::addrs::NetIface>,
  net_ip: String,
  net_qr: Option<std::path::PathBuf>,
  net_qr_url: String,
  net_last: Instant,
  /// AEC 校准结论（只取这一份，不拉整份调试快照）
  calib_report: crate::debug::Probe<crate::engine::calib::CalibReport>,
  /// 上一个重建还在跑时收到的最新计划：**只保留最后一份**（连点模型按钮不排长队）
  pending_apply: Option<Plan>,
  /// 文件对话框在后台线程跑，结果经这里交回界面线程（选中 → 导入媒体库 → 重建）
  media_pick: Arc<std::sync::Mutex<Option<PendingMedia>>>,
  /// 正在出声的应用（单应用音频节点选源用；空 = 没刷新过）
  app_list: Vec<(u32, String)>,
  /// 应用内更新状态（1 s 轮询；与 /debug 同源）
  update_status: crate::update::Status,
  /// 全局热键线程（启停 + 音效触发）；设置/计划变化时整体重注册
  hotkeys: Option<crate::hotkey::Hotkeys>,
  /// 启动引擎成功后的提示音（`tick` 看到 job 完成再播，失败不播）
  pending_cue: Option<&'static str>,
  /// 正在等待用户按下新的热键组合（录制中时根节点拦截按键）
  recording: Option<RecordTarget>,
  /// 上一拍引擎运行状态：托盘/全局热键启停发生在界面之外，靠它触发重绘
  was_running: bool,
  /// 上次把窗口几何写进 settings 的时刻（拖动时节流，关闭时无条件写）
  last_geom_save: Instant,
  /// EQ 画布几何（每个检查器复用；prepaint 写入）
  eq_geom: Rc<Cell<EqGeom>>,
  /// 正在拖的 EQ 段
  eq_drag: Option<EqDrag>,
}

impl Workspace {
  pub fn new(
    audio: Arc<AudioManager>,
    hub: SharedHub,
    window: &mut Window,
    cx: &mut Context<Self>,
  ) -> Self {
    crate::ui::i18n::set(&crate::config::load_lang());
    let focus_handle = cx.focus_handle();
    window.focus(&focus_handle, cx);
    let latency = hub.latency_report();
    let mut plan = audio.plan();
    // 迁移（v1→v2→v3）产生的布局要落盘一次，否则每次启动都得再算一遍。
    if plan.normalize() {
      let _ = audio.persist_plan(plan.clone());
    }
    let selected = plan.nodes().next().map(|n| n.id);
    let settings = crate::config::load_settings();
    let net_ifaces = crate::net::addrs::list();
    let net_ip = if !settings.net_ip.is_empty() {
      settings.net_ip.clone()
    } else {
      net_ifaces
        .iter()
        .find(|i| i.is_default)
        .or_else(|| net_ifaces.first())
        .map(|i| i.ip.clone())
        .unwrap_or_default()
    };
    let calib_report = hub.calib_report();
    let was_running = audio.is_running();
    let mut this = Workspace {
      focus_handle,
      audio,
      hub,
      plan,
      devices: crate::devices::cached().devices,
      last_refresh: Instant::now(),
      selected,
      dock_page: DockPage::Node,
      status: "从顶部添加输入 / AI / 处理 / 内容 / 输出；拖动端口连线，点节点改参数".into(),
      job: None,
      latency,
      drag: None,
      pending: None,
      menu: None,
      undo: Vec::new(),
      redo: Vec::new(),
      sliders: std::collections::HashMap::new(),
      subs: Vec::new(),
      settings,
      net_stats: None,
      net_ifaces,
      net_ip,
      net_qr: None,
      net_qr_url: String::new(),
      net_last: Instant::now() - Duration::from_secs(2),
      calib_report,
      pending_apply: None,
      media_pick: Arc::new(std::sync::Mutex::new(None)),
      app_list: Vec::new(),
      update_status: crate::update::status(),
      hotkeys: None,
      pending_cue: None,
      recording: None,
      was_running,
      last_geom_save: Instant::now(),
      eq_geom: Rc::new(Cell::new(EqGeom::default())),
      eq_drag: None,
    };
    this.restart_hotkeys();
    // 拖动窗口时不要每个像素都写盘；400 ms 节流，关闭时由 main 再无条件写一次。
    let geom_sub = cx.observe_window_bounds(window, |this, window, _cx| {
      if this.last_geom_save.elapsed() >= Duration::from_millis(400) {
        this.last_geom_save = Instant::now();
        let _ = crate::config::save_window_geom(crate::config::geom_from_window(window));
      }
    });
    this.subs.push(geom_sub);
    cx.spawn(async move |weak, cx| {
      loop {
        cx.background_executor()
          .timer(Duration::from_millis(80))
          .await;
        if weak
          .update(cx, |this, cx| {
            // 只有真正有变化才重绘：引擎停着时界面不该每秒画 12 次。
            if this.tick(cx) {
              cx.notify();
            }
          })
          .is_err()
        {
          break;
        }
      }
    })
    .detach();
    this
  }

  fn node(&self, id: u32) -> Option<&RowSpec> {
    self.plan.node(id)
  }

  fn pos(&self, id: u32) -> Option<(usize, usize)> {
    self.plan.position(id)
  }

  fn selected_node(&self) -> Option<&RowSpec> {
    self.selected.and_then(|id| self.node(id))
  }

  /// 定时轮询：返回 true = 界面有变化，需要重绘。
  fn tick(&mut self, cx: &mut Context<Self>) -> bool {
    let mut dirty = false;
    // 引擎可能在界面之外被托盘 / 全局热键启停：状态变了就重绘。
    let running = self.audio.is_running();
    if running != self.was_running {
      self.was_running = running;
      dirty = true;
    }
    // 调试接口投递的界面命令（用 API 做交互，不模拟鼠标）。
    for cmd in self.hub.take_ui_commands() {
      self.apply_command(cmd, cx);
      dirty = true;
    }
    if self.last_refresh.elapsed() >= Duration::from_secs(2) {
      self.last_refresh = Instant::now();
      self.devices = crate::devices::cached().devices;
      self.latency = self.hub.latency_report();
      dirty = true;
    }
    // 网络卡片常驻在右侧：1 s 刷一次统计；二维码只在地址变化时重新生成。
    if self.net_last.elapsed() >= Duration::from_secs(1) {
      self.net_last = Instant::now();
      self.net_stats = Some(crate::net::hub().stats());
      self.net_ifaces = crate::net::addrs::list();
      self.calib_report = self.hub.calib_report();
      // 手机端也可能改环深度（双向同步）：界面缓存跟着 hub 走。
      self.settings.net_ring_ms = crate::net::hub().ring_ms();
      self.update_status = crate::update::status();
      let running = self.net_stats.as_ref().map(|s| s.running).unwrap_or(false);
      if running {
        let url = self.net_url();
        if url != self.net_qr_url {
          self.net_qr_url = url.clone();
          self.net_qr = match qr::png_for(&url) {
            Ok(p) => Some(p),
            Err(e) => {
              crate::debug::events::log("warn", "ui", format!("二维码不可用：{e}"));
              None
            }
          };
        }
      } else if self.net_qr.take().is_some() {
        self.net_qr_url.clear();
      }
      dirty = true;
    }
    if let Some(job) = self.job.as_ref() {
      let msg = job.lock().unwrap().take();
      if let Some(Ok(msg)) = msg {
        if !msg.is_empty() {
          self.status = msg;
        }
        if let Some(kind) = self.pending_cue.take()
          && self.settings.cue_on
        {
          crate::cues::play(&self.settings.cue_start, kind);
        }
        self.job = None;
        dirty = true;
      } else if let Some(Err(e)) = msg {
        self.status = e.clone();
        self.hub.push_ui("error", format!("混音台：{e}"));
        self.pending_cue = None;
        self.job = None;
        dirty = true;
      }
    }
    // 文件选择器在别的线程里跑；结果到了就导入媒体库并重建会话。
    let pick = self
      .media_pick
      .lock()
      .unwrap_or_else(|e| e.into_inner())
      .take();
    if let Some(pm) = pick {
      self.finish_media_pick(pm, cx);
      dirty = true;
    }
    // 上一个重建跑完后，应用排队里的**最后一份**计划（连点模型/结构参数不排长队）。
    if self.job.is_none()
      && let Some(plan) = self.pending_apply.take()
    {
      self.spawn_apply(plan);
      dirty = true;
    }
    // 引擎运行时逐节点电平在动，必须重绘；停着就只等上面两件事。
    dirty || self.audio.is_running()
  }

  fn spawn_job(&mut self, what: &str, f: impl FnOnce() -> Result<String, String> + Send + 'static) {
    self.status = format!("{what}…");
    let slot = Arc::new(std::sync::Mutex::new(None));
    let slot2 = slot.clone();
    std::thread::spawn(move || {
      *slot2.lock().unwrap() = Some(f());
    });
    self.job = Some(slot);
  }

  fn snapshot(&mut self) {
    self.undo.push(self.plan.clone());
    if self.undo.len() > 50 {
      self.undo.remove(0);
    }
    self.redo.clear();
  }

  /// 松开鼠标：节点拖动作收尾（落盘），拉线则连接/取消。
  fn end_drag(&mut self, cx: &mut Context<Self>) {
    self.pending = None;
    if self.eq_drag.take().is_some() {
      let _ = self.audio.persist_plan(self.plan.clone());
      cx.notify();
    }
    let Some(d) = self.drag.take() else {
      return;
    };
    match d {
      Drag::Node { id, .. } => {
        if let Some(n) = self.plan.node_mut(id) {
          n.x = (n.x / GRID).round() * GRID;
          n.y = (n.y / GRID).round() * GRID;
        }
        self.persist_layout();
        cx.notify();
      }
      Drag::Wire {
        node,
        port,
        input,
        target,
        ..
      } => {
        if let Some((tn, _tinput, tp)) = target {
          if input {
            // 从输入端子往上方拉：目标是输出端子
            self.connect(tn, tp, node, port, cx);
          } else {
            // 从输出端子往下方拉：目标是输入端子
            self.connect(node, port, tn, tp, cx);
          }
        } else {
          self.status = "已取消连线".into();
          cx.notify();
        }
      }
    }
  }

  /// 找离线头最近的**对向**端子（拉输出找输入，拉输入找输出），28 px 内吸附。
  fn snap_target(
    &self,
    source_node: u32,
    source_input: bool,
    cur: (f32, f32),
  ) -> Option<(u32, bool, u8)> {
    let want_input = !source_input;
    let mut best: Option<(u32, bool, u8)> = None;
    let mut best_d = 28.0f32;
    for n in self.plan.nodes() {
      if n.id == source_node {
        continue;
      }
      let count = if want_input {
        input_ports(&n.ptype, n.channels)
      } else {
        crate::plan::output_ports(&n.ptype, n.channels)
      };
      if count == 0 {
        continue;
      }
      let (w, h) = Self::card_size(n);
      for p in 0..count {
        let (x, y) = Self::port_xy(n, want_input, p, w, h);
        // 吸附到端子的外沿（线本来画到方块外面）
        let y = if want_input {
          y - PORT_SIZE / 2.0
        } else {
          y + PORT_SIZE / 2.0
        };
        let d = ((x - cur.0).powi(2) + (y - cur.1).powi(2)).sqrt();
        if d < best_d {
          best_d = d;
          best = Some((n.id, want_input, p as u8));
        }
      }
    }
    best
  }

  fn apply_command(&mut self, cmd: UiCommand, cx: &mut Context<Self>) {
    match cmd {
      UiCommand::ToggleEngine => self.toggle_engine(cx),
      UiCommand::OpenSettings => {
        self.dock_page = DockPage::Settings;
        self.selected = None;
      }
      UiCommand::Quit => cx.quit(),
      UiCommand::Move { id, x, y } => {
        if let Some(n) = self.plan.node_mut(id) {
          n.x = x;
          n.y = y;
        }
        self.persist_layout();
      }
      UiCommand::Connect {
        from,
        from_port,
        to,
        to_port,
      } => self.connect(from, from_port, to, to_port, cx),
      UiCommand::Disconnect {
        from,
        from_port,
        to,
        to_port,
      } => self.disconnect(from, from_port, to, to_port, cx),
      UiCommand::SetChannels { id, channels } => {
        let cur = self.plan.node(id).map(|n| n.channels.max(1)).unwrap_or(1);
        self.set_channels(id, channels as i32 - cur as i32, cx);
      }
      UiCommand::SetEnabled { id, enabled } => {
        self.snapshot();
        if let Some(n) = self.plan.node_mut(id) {
          n.enabled = enabled;
        }
        self.apply_structure(cx);
      }
      UiCommand::Select { id } => {
        self.selected = Some(id);
        self.dock_page = DockPage::Node;
      }
      UiCommand::Arrange => self.arrange(),
    }
    cx.notify();
  }
}

// ── 绘制 ──────────────────────────────────────────────────────────────

impl Render for Workspace {
  fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
    let dock = self.dock(window, cx);
    div()
      .id("workspace")
      .relative()
      .flex()
      .flex_col()
      .size_full()
      .bg(rgb(t::BG))
      .text_color(rgb(t::TEXT))
      // 键盘命令挂在根上，按焦点向上冒泡都能到（快捷键见 `key_bindings`）。
      .on_action(cx.listener(Self::on_toggle_engine))
      .on_action(cx.listener(Self::on_refresh))
      .on_action(cx.listener(Self::on_undo))
      .on_action(cx.listener(Self::on_redo))
      .on_action(cx.listener(Self::on_delete))
      .on_key_down(cx.listener(Self::on_record_key))
      .on_mouse_move(cx.listener(Self::on_mouse_move))
      // 拖节点由 GPUI 的 drag 会话接管；位移处理挂在**根**上：
      // 指针离开卡片、经过线/端口/别的卡时仍然收得到事件。
      .on_drag_move::<DragPayload>(cx.listener(
        |this, e: &gpui_kit::DragMoveEvent<DragPayload>, _, cx| {
          let payload = *e.drag(cx);
          let pos = e.event.position;
          match payload {
            DragPayload::Move { id, .. } => {
              let (start_x, start_y, pointer) = match this.drag {
                Some(Drag::Node {
                  id: did,
                  start_x,
                  start_y,
                  pointer,
                }) if did == id => (start_x, start_y, pointer),
                _ => {
                  // 优先用 mouse_down 记下的真实指针；没有就从当前计划 + 本次事件起算。
                  let (sx, sy, pointer) = match this.pending.take() {
                    Some(Pending::Node {
                      id: pid,
                      pointer,
                      start_x,
                      start_y,
                    }) if pid == id => (start_x, start_y, pointer),
                    _ => {
                      let (sx, sy) = this.plan.node(id).map(|n| (n.x, n.y)).unwrap_or((0.0, 0.0));
                      (sx, sy, pos)
                    }
                  };
                  this.drag = Some(Drag::Node {
                    id,
                    start_x: sx,
                    start_y: sy,
                    pointer,
                  });
                  (sx, sy, pointer)
                }
              };
              let nx = start_x + (pos.x - pointer.x).as_f32();
              let ny = start_y + (pos.y - pointer.y).as_f32();
              if let Some(n) = this.plan.node_mut(id) {
                n.x = nx;
                n.y = ny;
              }
              cx.notify();
            }
            DragPayload::Port {
              node, port, input, ..
            } => {
              let (start_x, start_y, pointer) = match this.drag {
                Some(Drag::Wire {
                  node: n2,
                  port: p2,
                  input: i2,
                  start_x,
                  start_y,
                  pointer,
                  ..
                }) if n2 == node && p2 == port && i2 == input => (start_x, start_y, pointer),
                _ => {
                  let (sx, sy, pointer) = match this.pending.take() {
                    Some(Pending::Port {
                      node: pn,
                      port: pp,
                      input: pi,
                      pointer,
                      start_x,
                      start_y,
                    }) if pn == node && pp == port && pi == input => (start_x, start_y, pointer),
                    _ => {
                      let Some(n) = this.plan.node(node) else {
                        return;
                      };
                      let (w, h) = Self::card_size(n);
                      let (sx, sy) = Self::port_xy(n, input, port as usize, w, h);
                      (sx, sy, pos)
                    }
                  };
                  this.drag = Some(Drag::Wire {
                    node,
                    port,
                    input,
                    start_x: sx,
                    start_y: sy,
                    pointer,
                    cur: (sx, sy),
                    target: None,
                  });
                  (sx, sy, pointer)
                }
              };
              let mut cur = (
                start_x + (pos.x - pointer.x).as_f32(),
                start_y + (pos.y - pointer.y).as_f32(),
              );
              let target = this.snap_target(node, input, cur);
              if let Some((tn, tinput, tp)) = target
                && let Some(n) = this.plan.node(tn)
              {
                let (w, h) = Self::card_size(n);
                let (tx, ty) = Self::port_xy(n, tinput, tp as usize, w, h);
                cur = (
                  tx,
                  ty + if tinput {
                    -PORT_SIZE / 2.0
                  } else {
                    PORT_SIZE / 2.0
                  },
                );
              }
              this.drag = Some(Drag::Wire {
                node,
                port,
                input,
                start_x,
                start_y,
                pointer,
                cur,
                target,
              });
              cx.notify();
            }
          }
        },
      ))
      // 松开鼠标：结束拖动 / 完成连线。
      .on_mouse_up(
        gpui_kit::MouseButton::Left,
        cx.listener(|this, _, _, cx| this.end_drag(cx)),
      )
      .child(self.top_bar(cx))
      .child(
        div()
          .flex()
          .flex_1()
          .min_h_0()
          .child(
            div()
              .id("canvas-scroll")
              .flex_1()
              .min_w_0()
              .h_full()
              .overflow_scroll()
              .child(self.canvas(window, cx)),
          )
          .child(dock),
      )
      .when_some(self.menu, |el, menu| el.child(self.menu_panel(menu, cx)))
      .child(self.status_bar())
  }
}

impl Workspace {
  fn top_bar(&self, cx: &mut Context<Self>) -> impl IntoElement {
    let running = self.audio.is_running();
    let dot = if running { t::OK } else { t::DANGER };
    div()
      .flex()
      .items_center()
      .gap_2()
      .h(px(t::TOP_H))
      .px(px(t::SPACE_3))
      .bg(rgb(t::PANEL))
      .border_b_1()
      .border_color(rgb(t::BORDER))
      .child(
        div().flex().items_center().gap_1().child(
          div()
            .text_size(px(t::TEXT_XL))
            .text_color(rgb(t::ACCENT))
            .child("PureVox"),
        ),
      )
      .child(
        div()
          .flex()
          .items_center()
          .gap_2()
          .px(px(t::SPACE_1))
          .child(div().w(px(9.)).h(px(9.)).rounded_full().bg(rgb(dot)))
          .child(
            div()
              .text_size(px(t::TEXT_BASE))
              .text_color(rgb(dot))
              .child(if running {
                i18n::tr("引擎运行中")
              } else {
                i18n::tr("引擎已停止")
              }),
          ),
      )
      .child(div().w(px(1.)).h(px(22.)).bg(rgb(t::BORDER)))
      .child(
        Button::new("engine")
          .small()
          .icon(if running {
            IconName::Square
          } else {
            IconName::Play
          })
          .label(if running {
            i18n::tr("停止引擎")
          } else {
            i18n::tr("启动引擎")
          })
          .when(running, |b| b.danger())
          .when(!running, |b| b.success())
          .on_click(cx.listener(|this, _, _, cx| this.toggle_engine(cx))),
      )
      .child(div().w(px(8.)))
      .child(
        quiet_button(cx, "menu-source", "＋ 输入").on_click(cx.listener(|this, _, _, cx| {
          this.menu = Some(Menu::Source);
          cx.notify();
        })),
      )
      .child(
        quiet_button(cx, "menu-ai", "＋ AI").on_click(cx.listener(|this, _, _, cx| {
          this.menu = Some(Menu::Ai);
          cx.notify();
        })),
      )
      .child(
        quiet_button(cx, "menu-process", "＋ 处理").on_click(cx.listener(|this, _, _, cx| {
          this.menu = Some(Menu::Process);
          cx.notify();
        })),
      )
      .child(
        quiet_button(cx, "menu-content", "＋ 内容").on_click(cx.listener(|this, _, _, cx| {
          this.menu = Some(Menu::Content);
          cx.notify();
        })),
      )
      .child(
        quiet_button(cx, "menu-output", "＋ 输出").on_click(cx.listener(|this, _, _, cx| {
          this.menu = Some(Menu::Output);
          cx.notify();
        })),
      )
      .child(div().w(px(8.)))
      .child(
        quiet_button(cx, "arrange", "整理").on_click(cx.listener(|this, _, _, cx| {
          this.arrange();
          cx.notify();
        })),
      )
      .child(
        quiet_button(cx, "undo", "撤销").on_click(cx.listener(|this, _, _window, cx| {
          this.undo(cx);
        })),
      )
      .child(
        quiet_button(cx, "redo", "重做").on_click(cx.listener(|this, _, _window, cx| {
          this.redo(cx);
        })),
      )
      .child(
        quiet_button(cx, "refresh", "刷新设备").on_click(cx.listener(|this, _, _, cx| {
          this.status = crate::devices::spawn_refresh(this.hub.clone());
          cx.notify();
        })),
      )
      .child(div().flex_1())
      .child(
        div()
          .text_size(px(t::TEXT_SM))
          .text_color(rgb(t::MUTED))
          .child(format!(
            "{} 节点 · {} 条线",
            self.plan.nodes().count(),
            self.plan.nodes().map(|n| n.inputs.len()).sum::<usize>()
          )),
      )
      .child(
        quiet_button(cx, "settings", "设置").on_click(cx.listener(|this, _, _, cx| {
          // 全局设置是独立目的地：清掉节点选中，右侧只显示设置页。
          this.dock_page = DockPage::Settings;
          this.selected = None;
          cx.notify();
        })),
      )
      .child(
        Button::new("debug")
          .small()
          .label(i18n::tr("调试"))
          .on_click(|_, _, _| {
            let url = format!("http://127.0.0.1:{}/debug/ui", crate::debug::http_port());
            let _ = crate::openurl::open(&url);
          }),
      )
  }

  fn menu_panel(&self, menu: Menu, cx: &mut Context<Self>) -> AnyElement {
    let items: Vec<(&str, RowKind, &str)> = match menu {
      Menu::Source => vec![
        ("麦克风", RowKind::Input, "audio_input"),
        ("系统声音（回环）", RowKind::Input, "loopback"),
        ("应用音频（Windows）", RowKind::Input, "app_capture"),
        ("手机麦克风", RowKind::Input, "remote_mic"),
        ("测试音", RowKind::Input, "tone"),
      ],
      // AI 三巨头单独一栏：它们都是模型节点，但语义不是「普通处理」（AEC 还是两输入）。
      Menu::Ai => vec![
        ("AI 降噪", RowKind::Process, "denoise"),
        ("目标说话人 TSE", RowKind::Process, "tse"),
        ("回声消除 AEC（两输入）", RowKind::Process, "echo_cancel"),
      ],
      Menu::Process => vec![
        ("增益", RowKind::Process, "gain"),
        ("噪声门", RowKind::Process, "noise_gate"),
        ("压缩器", RowKind::Process, "compressor"),
        ("限制器", RowKind::Process, "limiter"),
        ("扩展器", RowKind::Process, "expander"),
        ("向上压缩器", RowKind::Process, "upward_compressor"),
        ("31 段 EQ", RowKind::Process, "eq31"),
        ("AGC", RowKind::Process, "agc"),
        ("极性反转", RowKind::Process, "invert"),
        ("可视化", RowKind::Process, "viz"),
      ],
      // 内容源：音效 / 音乐 / 文件。都是「自己产生声音」的源节点（0 个输入端口）。
      Menu::Content => vec![
        ("音效板", RowKind::Process, "soundpad"),
        ("音乐播放器（音频文件）", RowKind::Process, "player"),
      ],
      Menu::Output => vec![
        ("输出设备", RowKind::Output, "audio_output"),
        ("手机 / 客户端", RowKind::Output, "remote_speaker"),
      ],
    };
    let left = match menu {
      Menu::Source => 361.0,
      Menu::Ai => 441.0,
      Menu::Process => 521.0,
      Menu::Content => 601.0,
      Menu::Output => 681.0,
    };
    let mut list = div()
      .absolute()
      .top(px(t::TOP_H))
      .left(px(left))
      .flex()
      .flex_col()
      .gap_1()
      .p(px(t::SPACE_2))
      .rounded_lg()
      .bg(rgb(t::PANEL_HI))
      .border_1()
      .border_color(rgb(t::BORDER));
    for (label, kind, ptype) in items {
      let p = ptype;
      list = list.child(
        quiet_button(cx, format!("menu-item:{ptype}"), label).on_click(cx.listener(
          move |this, _, _window, cx| {
            this.add_node(kind, p, cx);
          },
        )),
      );
    }
    list.into_any_element()
  }
}

impl Workspace {
  fn status_bar(&self) -> impl IntoElement {
    let running = self.audio.is_running();
    div()
      .flex()
      .items_center()
      .gap_3()
      .h(px(t::STATUS_H))
      .px(px(t::SPACE_3))
      .bg(rgb(t::PANEL))
      .border_t_1()
      .border_color(rgb(t::BORDER))
      .text_size(px(t::TEXT_SM))
      .text_color(rgb(if running { t::TEXT } else { t::MUTED }))
      .child(if self.job.is_some() {
        "应用图中…".to_string()
      } else {
        self.status.clone()
      })
      .child(div().flex_1())
      .child(format!(
        "{} 节点 · {} 条线 · {}",
        self.plan.nodes().count(),
        self.plan.nodes().map(|n| n.inputs.len()).sum::<usize>(),
        if running {
          i18n::tr("引擎运行中")
        } else {
          i18n::tr("引擎已停止")
        }
      ))
  }
}

#[cfg(test)]
mod tests {
  use super::{NodeModule, node_module};

  #[test]
  fn network_module_only_for_network_nodes() {
    assert_eq!(node_module("remote_mic"), NodeModule::Network);
    assert_eq!(node_module("remote_speaker"), NodeModule::Network);
    for ptype in [
      "audio_input",
      "loopback",
      "tone",
      "denoise",
      "gain",
      "audio_output",
    ] {
      assert_ne!(node_module(ptype), NodeModule::Network, "{ptype}");
    }
  }

  #[test]
  fn calibration_only_for_aec_node() {
    assert_eq!(node_module("echo_cancel"), NodeModule::Aec);
    for ptype in ["remote_mic", "remote_speaker", "denoise", "tse"] {
      assert_ne!(node_module(ptype), NodeModule::Aec, "{ptype}");
    }
  }
}
