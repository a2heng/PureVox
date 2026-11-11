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

mod theme;

use std::sync::Arc;
use std::time::{Duration, Instant};

use gpui_kit::PathBuilder;
use gpui_kit::component::button::{Button, ButtonCustomVariant, ButtonVariants as _};
use gpui_kit::component::slider::{Slider, SliderEvent, SliderState, SliderValue};
use gpui_kit::component::{IconName, Selectable as _, Sizable as _};
use gpui_kit::{
  AnyElement, App, Context, Div, ElementId, Entity, FocusHandle, KeyBinding, SharedString,
  Subscription, Window, canvas, div, point, prelude::*, px, rgb, rgba,
};

use crate::audio::AudioManager;
use crate::debug::{SharedHub, UiCommand};
use crate::plan::{Plan, PortInput, RowKind, RowSpec, input_ports, max_channels, node_size};
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

fn quiet_button(cx: &App, id: impl Into<ElementId>, label: impl Into<SharedString>) -> Button {
  Button::new(id)
    .xsmall()
    .custom(
      ButtonCustomVariant::new(cx)
        .color(rgb(t::CONTROL).into())
        .foreground(rgb(t::TEXT).into())
        .hover(rgb(t::CONTROL_HI).into())
        .active(rgb(t::CONTROL_HI).into()),
    )
    .label(label)
}

fn quiet_button_md(cx: &App, id: impl Into<ElementId>, label: impl Into<SharedString>) -> Button {
  Button::new(id)
    .small()
    .custom(
      ButtonCustomVariant::new(cx)
        .color(rgb(t::CONTROL).into())
        .foreground(rgb(t::TEXT).into())
        .hover(rgb(t::CONTROL_HI).into())
        .active(rgb(t::CONTROL_HI).into()),
    )
    .label(label)
}

const HEADER_H: f32 = 30.0;
const PORT_ROW_H: f32 = 22.0;
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
  Process,
  Output,
}

pub struct Workspace {
  focus_handle: FocusHandle,
  audio: Arc<AudioManager>,
  hub: SharedHub,
  plan: Plan,
  devices: Vec<crate::devices::DeviceInfo>,
  last_refresh: Instant,
  selected: Option<u32>,
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
}

impl Workspace {
  pub fn new(
    audio: Arc<AudioManager>,
    hub: SharedHub,
    window: &mut Window,
    cx: &mut Context<Self>,
  ) -> Self {
    let focus_handle = cx.focus_handle();
    window.focus(&focus_handle, cx);
    let latency = hub.latency_report();
    let mut plan = audio.plan();
    // 迁移（v1→v2→v3）产生的布局要落盘一次，否则每次启动都得再算一遍。
    if plan.normalize() {
      let _ = audio.persist_plan(plan.clone());
    }
    let selected = plan.nodes().next().map(|n| n.id);
    let this = Workspace {
      focus_handle,
      audio,
      hub,
      plan,
      devices: crate::devices::cached().devices,
      last_refresh: Instant::now(),
      selected,
      status: "从顶部添加输入 / 处理 / 输出；拖动端口连线，点节点改参数".into(),
      job: None,
      latency,
      drag: None,
      pending: None,
      menu: None,
      undo: Vec::new(),
      redo: Vec::new(),
      sliders: std::collections::HashMap::new(),
      subs: Vec::new(),
    };
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
    if let Some(job) = self.job.as_ref() {
      let msg = job.lock().unwrap().take();
      if let Some(Ok(msg)) = msg {
        if !msg.is_empty() {
          self.status = msg;
        }
        self.job = None;
        dirty = true;
      } else if let Some(Err(e)) = msg {
        self.status = e.clone();
        self.hub.push_ui("error", format!("混音台：{e}"));
        self.job = None;
        dirty = true;
      }
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
      }
      UiCommand::Arrange => self.arrange(),
    }
    cx.notify();
  }

  fn apply_structure(&mut self, cx: &mut Context<Self>) {
    self.plan.normalize();
    let plan = self.plan.clone();
    let mgr = self.audio.clone();
    self.spawn_job("应用图", move || {
      mgr.apply_plan(plan).map(|problems| {
        if problems.is_empty() {
          "图已生效".into()
        } else {
          format!("已生效；{}", problems.join("；"))
        }
      })
    });
    cx.notify();
  }

  fn persist_layout(&self) {
    let _ = self.audio.persist_plan(self.plan.clone());
  }

  fn undo(&mut self, cx: &mut Context<Self>) {
    if let Some(prev) = self.undo.pop() {
      self.redo.push(self.plan.clone());
      self.plan = prev;
      self.selected = self.selected.filter(|id| self.plan.node(*id).is_some());
      self.apply_structure(cx);
    }
  }

  fn redo(&mut self, cx: &mut Context<Self>) {
    if let Some(next) = self.redo.pop() {
      self.undo.push(self.plan.clone());
      self.plan = next;
      self.selected = self.selected.filter(|id| self.plan.node(*id).is_some());
      self.apply_structure(cx);
    }
  }

  fn toggle_engine(&mut self, cx: &mut Context<Self>) {
    if self.audio.is_running() {
      self.audio.stop();
      self.status = "引擎已停止（设备已释放）".into();
    } else {
      let mgr = self.audio.clone();
      self.spawn_job("启动引擎", move || {
        mgr.start().map(|problems| {
          if problems.is_empty() {
            "引擎已启动".into()
          } else {
            format!("已启动；{}", problems.join("；"))
          }
        })
      });
    }
    cx.notify();
  }

  fn node_kind_label(kind: RowKind, ptype: &str) -> &'static str {
    match (kind, ptype) {
      (RowKind::Process, "denoise") => "AI 降噪",
      (RowKind::Process, "gain") => "增益",
      (RowKind::Process, "agc") => "自动增益",
      (RowKind::Process, "limiter") => "限幅器",
      (RowKind::Process, "eq31") => "31 段 EQ",
      (RowKind::Process, "noise_gate") => "噪声门",
      (RowKind::Process, "compressor") => "压缩器",
      (RowKind::Process, "tse") => "目标说话人 TSE",
      (RowKind::Process, "echo_cancel") => "回声消除 AEC",
      (RowKind::Process, _) => "处理",
      (RowKind::Input, "remote_mic") => "手机麦克风",
      (RowKind::Input, "loopback") => "系统声音",
      (RowKind::Input, "tone") => "测试音",
      (RowKind::Input, _) => "麦克风",
      (RowKind::Output, "remote_speaker") => "手机 / 客户端",
      (RowKind::Output, _) => "输出",
    }
  }

  fn node_color(kind: RowKind, ptype: &str) -> u32 {
    match (kind, ptype) {
      (RowKind::Input, "remote_mic") => t::NET,
      (RowKind::Input, "loopback") => t::LOOPBACK,
      (RowKind::Input, _) => t::MIC,
      (RowKind::Process, "echo_cancel") => t::AEC,
      (RowKind::Process, _) => t::ACCENT,
      (RowKind::Output, _) => t::OK,
    }
  }

  fn node_title(&self, n: &RowSpec) -> String {
    if let Some(name) = n.device_name.as_ref().filter(|x| !x.is_empty()) {
      return name.clone();
    }
    if let Some(d) = n.device.as_ref().filter(|x| !x.is_empty()) {
      return crate::devices::display_name(d, true).unwrap_or_else(|_| d.clone());
    }
    Self::node_kind_label(n.kind, &n.ptype).to_string()
  }

  fn card_size_parts(kind: RowKind, ptype: &str, channels: u32) -> (f32, f32) {
    let (base_w, base) = node_size(kind, ptype, channels);
    let ins = input_ports(ptype, channels);
    let outs = crate::plan::output_ports(ptype, channels);
    // 端子按固定槽位从左侧排；16 路时卡片才变宽，平常一律 240。
    let need = TERM_PAD * 2.0 + ins.max(outs) as f32 * TERM_SLOT;
    let w = base_w.max(need);
    let add = (term_rows(ins) + term_rows(outs)) as f32 * TERM_H;
    let y = if ptype == "echo_cancel" { 26.0 } else { 0.0 };
    (w, base + add + y)
  }

  fn card_size(n: &RowSpec) -> (f32, f32) {
    Self::card_size_parts(n.kind, &n.ptype, n.channels)
  }

  /// 端子的绝对坐标（画线、画端口、命中都用这一份，绝不各算一套）。
  fn port_xy(n: &RowSpec, input: bool, p: usize, _w: f32, h: f32) -> (f32, f32) {
    let count = if input {
      input_ports(&n.ptype, n.channels)
    } else {
      crate::plan::output_ports(&n.ptype, n.channels)
    };
    if count == 0 {
      return (n.x, n.y);
    }
    // 固定槽位：第 1 路起点永远在卡片左侧 TERM_PAD，之后每路 +TERM_SLOT。
    let x = n.x + TERM_PAD + (p as f32 + 0.5) * TERM_SLOT;
    let y = if input { n.y } else { n.y + h };
    (x, y)
  }

  fn free_spot(&self, w: f32, h: f32) -> (f32, f32) {
    let mut x = 64.0;
    let mut y = 72.0;
    for _ in 0..400 {
      let hit = self.plan.nodes().any(|n| {
        let (nw, nh) = Self::card_size(n);
        x < n.x + nw + 24.0 && x + w + 24.0 > n.x && y < n.y + nh + 24.0 && y + h + 24.0 > n.y
      });
      if !hit {
        return (x, y);
      }
      x += 288.0;
      if x > 2600.0 {
        x = 64.0;
        y += 132.0;
      }
    }
    (64.0, 72.0)
  }

  fn add_node(&mut self, kind: RowKind, ptype: &str, cx: &mut Context<Self>) {
    self.snapshot();
    let id = self.plan.next_id();
    let (w, h) = Self::card_size_parts(kind, ptype, 1);
    let (x, y) = self.free_spot(w, h);
    let mut params = std::collections::BTreeMap::new();
    let mut device = None;
    let mut device_name = None;
    let mut input_gain_db = "0.0".to_string();
    match ptype {
      "audio_input" => {}
      "loopback" => {
        device = Some("loopback".into());
        device_name = Some("系统声音（默认输出）".into());
        // 系统混音通常偏响，默认压低 6 dB；和所有节点共用同一个输入微调。
        input_gain_db = "-6.0".into();
      }
      "remote_mic" => device_name = Some("手机（网络麦克风）".into()),
      "tone" => {}
      "denoise" => {
        if let Some((f, _)) = crate::infer::denoise_models().first() {
          params.insert("model".into(), (*f).to_string());
        }
      }
      "eq31" => {
        params.insert(
          "settings".into(),
          crate::engine::hot::EqSettings::default().to_json(),
        );
      }
      "tse" => {
        params.insert("model".into(), crate::infer::tse::MODEL_TSE.into());
      }
      "echo_cancel" => {
        params.insert("model".into(), crate::infer::aec::MODEL_AEC.into());
        params.insert("mic_gain_db".into(), "0.0".into());
        params.insert("far_gain_db".into(), "0.0".into());
        params.insert("far_delay_ms".into(), "0.0".into());
      }
      "audio_output" => {
        if let Ok(id) =
          crate::devices::default_output_id().ok_or_else(|| "没有输出设备".to_string())
        {
          device_name = crate::devices::display_name(&id, false).ok();
          device = Some(id);
        }
      }
      "remote_speaker" => device_name = Some("手机 / 客户端".into()),
      _ => {}
    }
    let mut node = RowSpec {
      kind,
      ptype: ptype.into(),
      device,
      device_name,
      enabled: true,
      params,
      channels: 1,
      input_gain_db,
      id,
      x,
      y,
      inputs: Vec::new(),
    };
    if ptype == "audio_output" && node.device.is_none() {
      self.status = "没有可用的输出设备".into();
      self.undo.pop();
      cx.notify();
      return;
    }
    if ptype == "audio_output"
      && let Some(dev) = node.device.as_deref()
      && let Some(existing) = self
        .plan
        .nodes()
        .find(|n| n.ptype == "audio_output" && n.device.as_deref() == Some(dev))
        .map(|n| n.id)
    {
      self.selected = Some(existing);
      self.menu = None;
      self.status = "这个输出设备已经在画板上".into();
      self.undo.pop();
      cx.notify();
      return;
    }
    // 新节点默认接在选中节点的输出上（如果类型合适），像 Element 的 auto-connect。
    if let Some(sel) = self.selected.and_then(|id| self.node(id)).cloned()
      && input_ports(&node.ptype, node.channels) > 0
      && crate::plan::output_ports(&sel.ptype, sel.channels) > 0
    {
      node.inputs.push(PortInput {
        from: sel.id,
        from_port: 0,
        to_port: 0,
      });
    }
    if self.plan.columns.is_empty() {
      self.plan.columns.push(crate::plan::ColumnSpec {
        rows: Vec::new(),
        enabled: true,
      });
    }
    self.plan.columns[0].rows.push(node);
    self.selected = Some(id);
    self.menu = None;
    self.apply_structure(cx);
  }

  fn set_param(&mut self, id: u32, key: &str, value: String, cx: &mut Context<Self>) {
    let Some(node) = self.plan.node_mut(id) else {
      return;
    };
    let ptype = node.ptype.clone();
    node.params.insert(key.to_string(), value.clone());
    let hot = crate::engine::registry::is_hot(&ptype, key) || key == "input_gain_db";
    if hot && self.audio.is_running() {
      if let Some((ci, ri)) = self.pos(id) {
        let _ = self.audio.set_row_param(ci, ri, key, &value);
        let _ = self.audio.persist_plan(self.plan.clone());
        self.status = format!("{} = {value}", key);
      }
      cx.notify();
    } else {
      self.status = format!("{} = {value}（重建生效）", key);
      self.apply_structure(cx);
    }
  }

  /// 该节点允许的最大通道数：节点类型上限与设备原生通道数取小。
  fn node_max_channels(&self, n: &RowSpec) -> u32 {
    let mut max = max_channels(&n.ptype);
    if matches!(n.kind, RowKind::Input | RowKind::Output) {
      let dev = n.device.as_deref().and_then(|id| {
        self.devices.iter().find(|d| {
          if id == "loopback" {
            d.is_default && d.direction == "output"
          } else {
            d.id == id
          }
        })
      });
      if let Some(ch) = dev.and_then(|d| match &d.native {
        crate::debug::Probe::Ok { value } => Some(value.channels),
        _ => None,
      }) {
        max = max.min(ch.max(1));
      }
    }
    max
  }

  fn set_channels(&mut self, id: u32, delta: i32, cx: &mut Context<Self>) {
    let Some(n) = self.plan.node(id) else {
      return;
    };
    let max = self.node_max_channels(n);
    let cur = n.channels.max(1);
    let next = (cur as i32 + delta).clamp(1, max as i32) as u32;
    if next == cur {
      return;
    }
    self.snapshot();
    // ��少通道时丢掉越界的入线
    let mut dropped = 0;
    if let Some(n) = self.plan.node_mut(id) {
      n.channels = next;
      let ports = input_ports(&n.ptype, next);
      let before = n.inputs.len();
      n.inputs.retain(|e| (e.to_port as usize) < ports);
      dropped = before - n.inputs.len();
    }
    // 上游输出通道也可能越界
    let ids: Vec<u32> = self.plan.nodes().map(|n| n.id).collect();
    for nid in ids {
      let Some(src) = self.plan.node(nid) else {
        continue;
      };
      let outs = crate::plan::output_ports(&src.ptype, src.channels);
      // 输出节点自己没有输出端口（outs = 0）：它的入线不能被这一轮清掉，
      // 否则「把通道从 10 降到 2」会把输出节点上 1/2 的线一起删掉。
      if outs == 0 {
        continue;
      }
      if let Some(n) = self.plan.node_mut(nid) {
        n.inputs.retain(|e| (e.from_port as usize) < outs);
      }
    }
    self.status = if dropped > 0 {
      format!("通道已改为 {next}；丢弃了 {dropped} 条越界入线")
    } else {
      format!("通道已改为 {next}")
    };
    self.apply_structure(cx);
  }

  #[allow(clippy::too_many_arguments)]
  fn connect(&mut self, from: u32, from_port: u8, to: u32, to_port: u8, cx: &mut Context<Self>) {
    self.snapshot();
    match self.plan.connect(from, from_port, to, to_port) {
      Ok(()) => {
        self.status = "已连接".into();
        self.apply_structure(cx);
      }
      Err(e) => {
        self.status = e;
        self.undo.pop();
        cx.notify();
      }
    }
  }

  fn disconnect(&mut self, from: u32, from_port: u8, to: u32, to_port: u8, cx: &mut Context<Self>) {
    self.snapshot();
    self.plan.disconnect(from, from_port, to, to_port);
    self.status = "已断开（节点保留）".into();
    self.apply_structure(cx);
  }

  fn delete_node(&mut self, id: u32, cx: &mut Context<Self>) {
    self.snapshot();
    self.plan.remove_node(id);
    self.selected = None;
    self.status = "已删除节点".into();
    self.apply_structure(cx);
  }

  fn arrange(&mut self) {
    self.snapshot();
    // 只有一份排版实现（plan::layout_by_depth）：按深度纵向排，同一深度横向分支。
    self.plan.layout_by_depth();
    self.persist_layout();
    self.status = "已按列整理".into();
  }

  fn add_input_row(&mut self, col: usize, row: usize, cx: &mut Context<Self>) {
    let _ = (col, row);
    self.add_node(RowKind::Input, "audio_input", cx);
  }

  // ── 动作 ────────────────────────────────────────────────────────────

  fn on_toggle_engine(&mut self, _: &ToggleEngine, _: &mut Window, cx: &mut Context<Self>) {
    self.toggle_engine(cx);
  }
  fn on_refresh(&mut self, _: &RefreshDevices, _: &mut Window, cx: &mut Context<Self>) {
    self.status = crate::devices::spawn_refresh(self.hub.clone());
    cx.notify();
  }
  fn on_undo(&mut self, _: &Undo, _window: &mut Window, cx: &mut Context<Self>) {
    self.undo(cx);
  }
  fn on_redo(&mut self, _: &Redo, _window: &mut Window, cx: &mut Context<Self>) {
    self.redo(cx);
  }
  fn on_delete(&mut self, _: &DeleteNode, _window: &mut Window, cx: &mut Context<Self>) {
    if let Some(id) = self.selected {
      self.delete_node(id, cx);
    }
  }
}

// ── 绘制 ──────────────────────────────────────────────────────────────

impl Render for Workspace {
  fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
    let inspector = self.inspector(window, cx);
    div()
      .id("workspace")
      .relative()
      .flex()
      .flex_col()
      .size_full()
      .bg(rgb(t::BG))
      .text_color(rgb(t::TEXT))
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
          .child(inspector),
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
      .px(px(12.))
      .bg(rgb(t::PANEL))
      .border_b_1()
      .border_color(rgb(t::BORDER))
      .child(
        div()
          .flex()
          .items_center()
          .gap_1()
          .child(
            div()
              .text_size(px(15.))
              .text_color(rgb(t::ACCENT))
              .child("PureVox"),
          )
          .child(
            div()
              .text_size(px(11.))
              .text_color(rgb(t::MUTED))
              .child("MIX"),
          ),
      )
      .child(
        div()
          .flex()
          .items_center()
          .gap_2()
          .px(px(4.))
          .child(div().w(px(9.)).h(px(9.)).rounded_full().bg(rgb(dot)))
          .child(
            div()
              .text_size(px(12.))
              .text_color(rgb(dot))
              .child(if running {
                "引擎运行中"
              } else {
                "引擎已停止"
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
            "停止引擎"
          } else {
            "启动引擎"
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
        quiet_button(cx, "menu-process", "＋ 处理").on_click(cx.listener(|this, _, _, cx| {
          this.menu = Some(Menu::Process);
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
          .text_size(px(11.))
          .text_color(rgb(t::MUTED))
          .child(format!(
            "{} 节点 · {} 条线",
            self.plan.nodes().count(),
            self.plan.nodes().map(|n| n.inputs.len()).sum::<usize>()
          )),
      )
      .child(
        Button::new("debug")
          .small()
          .label("调试")
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
        ("手机麦克风", RowKind::Input, "remote_mic"),
        ("测试音", RowKind::Input, "tone"),
        ("AEC（两输入）", RowKind::Process, "echo_cancel"),
      ],
      Menu::Process => vec![
        ("AI 降噪", RowKind::Process, "denoise"),
        ("增益", RowKind::Process, "gain"),
        ("AGC", RowKind::Process, "agc"),
        ("限幅", RowKind::Process, "limiter"),
        ("31 段 EQ", RowKind::Process, "eq31"),
        ("噪声门", RowKind::Process, "noise_gate"),
        ("压缩", RowKind::Process, "compressor"),
        ("TSE", RowKind::Process, "tse"),
      ],
      Menu::Output => vec![
        ("输出设备", RowKind::Output, "audio_output"),
        ("手机 / 客户端", RowKind::Output, "remote_speaker"),
      ],
    };
    let mut list = div()
      .absolute()
      .top(px(t::TOP_H))
      .left(px(230.))
      .flex()
      .flex_col()
      .gap_1()
      .p(px(8.))
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

  fn canvas(&mut self, _window: &mut Window, cx: &mut Context<Self>) -> AnyElement {
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
          .text_size(px(11.))
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

  fn node_card(&self, n: RowSpec, cx: &mut Context<Self>) -> AnyElement {
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
    let label = Self::node_kind_label(n.kind, &n.ptype).to_string();
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
              .map(|(_, l)| (*l).to_string())
              .unwrap_or_else(|| "降噪".into())
          })
          .unwrap_or_else(|| "降噪".into()),
        "echo_cancel" => "近端 + 远端".into(),
        _ => "处理".into(),
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
          .px(px(8.))
          .border_b_1()
          .border_color(rgb(t::BORDER))
          .child(div().w(px(6.)).h(px(6.)).rounded_full().bg(rgb(color)))
          .child(div().text_size(px(10.)).text_color(rgb(color)).child(label))
          .child(div().flex_1())
          .child(
            div()
              .text_size(px(9.))
              .text_color(rgb(if on { t::MUTED } else { t::DANGER }))
              .child(if on { "启用" } else { "静音" }),
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
          .px(px(8.))
          .py(px(4.))
          .child(
            div()
              .text_size(px(12.))
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
        .px(px(8.))
        .py(px(4.))
        .rounded_md()
        .bg(rgb(t::ACCENT))
        .text_size(px(10.))
        .text_color(rgb(0xffffff))
        .child(self.label.clone())
    }
  }
}

impl Workspace {
  // ── 检查器 ──────────────────────────────────────────────────────────

  fn inspector(&mut self, window: &mut Window, cx: &mut Context<Self>) -> AnyElement {
    let body: AnyElement = match self.selected.and_then(|id| self.plan.node(id).cloned()) {
      Some(n) => self.inspect_node(n, window, cx),
      None => div()
        .flex()
        .flex_col()
        .gap_2()
        .child(div().text_size(px(14.)).child("没有选中节点"))
        .child(hint("点画板上的节点；拖动右侧端口连线。"))
        .into_any_element(),
    };
    div()
      .id("inspector")
      .flex()
      .flex_col()
      .gap_3()
      .w(px(320.))
      .h_full()
      .p(px(12.))
      .bg(rgb(t::PANEL))
      .border_l_1()
      .border_color(rgb(t::BORDER))
      .overflow_y_scroll()
      .child(body)
      .into_any_element()
  }

  fn inspect_node(
    &mut self,
    n: RowSpec,
    window: &mut Window,
    cx: &mut Context<Self>,
  ) -> AnyElement {
    let id = n.id;
    let color = Self::node_color(n.kind, &n.ptype);
    let label = Self::node_kind_label(n.kind, &n.ptype).to_string();
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
              .map(|(_, l)| (*l).to_string())
              .unwrap_or_else(|| "降噪".into())
          })
          .unwrap_or_else(|| "降噪".into()),
        "echo_cancel" => "近端 + 远端".into(),
        _ => "处理".into(),
      }
    } else {
      self.node_title(&n)
    };
    let enabled = n.enabled;
    let (ci, ri) = self.pos(id).unwrap_or((0, 0));
    let peak = self.hub.row_peak(ci, ri);
    let latency = self.latency_short(ci);

    // ── 操作（上）────────────────────────────────────────────────────
    let mut list = div()
      .flex()
      .flex_col()
      .gap_3()
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
                  .text_size(px(14.))
                  .child(title),
              ),
          )
          .child(
            quiet_button(cx, format!("del:{id}"), "删除").on_click(cx.listener(
              move |this, _, _window, cx| {
                this.delete_node(id, cx);
              },
            )),
          ),
      )
      .child(section("操作"))
      .child(
        quiet_button(
          cx,
          format!("en:{id}"),
          if enabled {
            "启用中 · 点击静音"
          } else {
            "已静音 · 点击启用"
          },
        )
        .on_click(cx.listener(move |this, _, _window, cx| {
          this.snapshot();
          if let Some(n) = this.plan.node_mut(id) {
            n.enabled = !n.enabled;
          }
          this.apply_structure(cx);
        })),
      );

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
              quiet_button(cx, format!("dev:{id}:{}", d.id), d.name)
                .when(on, |b| b.selected(true))
                .on_click(cx.listener(move |this, _, _window, cx| {
                  this.snapshot();
                  if let Some(n) = this.plan.node_mut(id) {
                    n.device = Some(did.clone());
                    n.device_name = Some(dname.clone());
                  }
                  this.apply_structure(cx);
                })),
            );
          }
        }
        "loopback" => {
          list = list.child(section("系统声音"));
          let on = n.device.as_deref().map(|d| d == "loopback").unwrap_or(true);
          list = list.child(
            quiet_button(cx, format!("loop:{id}"), "系统默认输出")
              .when(on, |b| b.selected(true))
              .on_click(cx.listener(move |this, _, _window, cx| {
                this.snapshot();
                if let Some(n) = this.plan.node_mut(id) {
                  n.device = Some("loopback".into());
                  n.device_name = Some("系统声音（默认输出）".into());
                }
                this.apply_structure(cx);
              })),
          );
          for t in crate::devices::loopback_targets() {
            let on = n.device.as_deref() == Some(t.id.as_str());
            let did = t.id.clone();
            let dname = t.name.clone();
            list = list.child(
              quiet_button(cx, format!("loop:{id}:{}", t.id), t.name)
                .when(on, |b| b.selected(true))
                .on_click(cx.listener(move |this, _, _window, cx| {
                  this.snapshot();
                  if let Some(n) = this.plan.node_mut(id) {
                    n.device = Some(did.clone());
                    n.device_name = Some(dname.clone());
                  }
                  this.apply_structure(cx);
                })),
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
          quiet_button(cx, format!("odev:{id}:{}", d.id), d.name)
            .when(on, |b| b.selected(true))
            .on_click(cx.listener(move |this, _, _window, cx| {
              this.snapshot();
              if let Some(n) = this.plan.node_mut(id) {
                n.device = Some(did.clone());
                n.device_name = Some(dname.clone());
              }
              this.apply_structure(cx);
            })),
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
              .text_size(px(12.))
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
              .text_size(px(10.))
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
    } else if n.ptype == "echo_cancel" {
      list = list.child(section("AEC")).child(self.params_editor(id, cx));
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
      list = list
        .child(section("参考语音"))
        .child(
          quiet_button(cx, format!("rec:{id}"), "录制 10 秒参考").on_click(cx.listener(
            move |this, _, _, cx| {
              let mgr = this.audio.clone();
              this.snapshot();
              this.spawn_job("录制 TSE 参考", move || {
                mgr.record_reference().map(|p| format!("参考已写入 {p}"))
              });
              cx.notify();
            },
          )),
        )
        .child(hint(if exists {
          format!("已有参考：{}", path.display())
        } else {
          "还没录（TSE 直通）".to_string()
        }));
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
              .text_size(px(11.))
              .truncate()
              .child(format!("{src} · {fp} → {tp}")),
          )
          .child(
            quiet_button(cx, format!("disc:{id}:{from}:{fp}:{tp}"), "断开").on_click(cx.listener(
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

  #[allow(clippy::too_many_arguments)]
  fn db_slider(
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
          .text_size(px(11.))
          .text_color(rgb(t::TEXT))
          .child(format!("{value:+.1} dB")),
      )
      .into_any_element()
  }

  fn params_editor(&self, id: u32, cx: &mut Context<Self>) -> Div {
    let Some(n) = self.plan.node(id) else {
      return div();
    };
    let ptype = n.ptype.clone();
    let spec = crate::engine::registry::get_spec(&ptype);
    let mut list = div().flex().flex_col().gap_2();
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
        .text_size(px(11.))
        .text_color(rgb(t::MUTED))
        .child(p.label.to_string());
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
            .child(div().w(px(76.)).text_size(px(11.)).child(format!("{v:.2}")))
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
          quiet_button(
            cx,
            format!("pb:{id}:{}", p.key),
            if on { "开" } else { "关" },
          )
          .on_click(cx.listener(move |this, _, _window, cx| {
            this.snapshot();
            this.set_param(id, &key, if on { "0" } else { "1" }.into(), cx);
          }))
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
              quiet_button(cx, format!("pc:{id}:{}:{value}", p.key), *label)
                .when(cur == *value, |b| b.selected(true))
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
                .map(|(f, l)| ((*f).to_string(), (*l).to_string()))
                .collect(),
              "tse" => crate::infer::tse::tse_models()
                .iter()
                .map(|(f, l)| ((*f).to_string(), (*l).to_string()))
                .collect(),
              "echo_cancel" => crate::infer::aec::aec_models()
                .iter()
                .map(|(f, l)| ((*f).to_string(), (*l).to_string()))
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
                quiet_button(cx, format!("pm:{id}:{}:{}", p.key, value), label)
                  .when(on, |b| b.selected(true))
                  .on_click(cx.listener(move |this, _, _window, cx| {
                    this.snapshot();
                    this.set_param(id, &k, v.clone(), cx);
                  })),
              );
            }
            row.into_any_element()
          } else {
            div()
              .text_size(px(11.))
              .text_color(rgb(t::TEXT))
              .child(cur)
              .into_any_element()
          }
        }
      };
      list = list.child(div().flex().flex_col().gap_1().child(label).child(control));
    }
    list
  }

  fn latency_short(&self, col: usize) -> String {
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

  fn status_bar(&self) -> impl IntoElement {
    let running = self.audio.is_running();
    div()
      .flex()
      .items_center()
      .gap_3()
      .h(px(t::STATUS_H))
      .px(px(12.))
      .bg(rgb(t::PANEL))
      .border_t_1()
      .border_color(rgb(t::BORDER))
      .text_size(px(11.))
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
          "引擎运行中"
        } else {
          "引擎已停止"
        }
      ))
  }
}

fn hint(text: impl Into<SharedString>) -> Div {
  div()
    .text_size(px(11.))
    .text_color(rgb(t::MUTED))
    .child(text.into())
}

fn field(label: &str, value: String) -> Div {
  div()
    .flex()
    .items_center()
    .justify_between()
    .gap_2()
    .text_size(px(11.))
    .child(div().text_color(rgb(t::MUTED)).child(label.to_string()))
    .child(
      div()
        .flex_1()
        .text_right()
        .truncate()
        .text_color(rgb(t::TEXT))
        .child(value),
    )
}

fn section(text: &str) -> Div {
  div()
    .mt(px(4.))
    .text_size(px(11.))
    .text_color(rgb(t::ACCENT))
    .child(text.to_string())
}
