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

use std::cell::{Cell, RefCell};
use std::rc::Rc;
use std::sync::Arc;
use std::time::{Duration, Instant};

use gpui_kit::component::button::{Button, ButtonVariants as _};
use gpui_kit::component::slider::SliderState;
use gpui_kit::component::{IconName, Sizable as _};
// `on_double_click` 只在 Linux 分支用（Windows 双击最大化交给系统）。
#[cfg(not(windows))]
use gpui_kit::component::InteractiveElementExt as _;
use gpui_kit::{
  AnyElement, Context, Entity, FocusHandle, KeyBinding, Subscription, Window, div, prelude::*, px,
  rgb,
};

use crate::audio::AudioManager;
use crate::debug::{SharedHub, UiCommand};
use crate::engine::hot::EqSettings;
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

/// 端子只占**一排**：号码直接写进突起的端点方块，不再另开编号条。
const PORT_H: f32 = 14.0;
/// 普通通道端点的宽度（一位 / 两位数字都在里面）。
const PORT_W: f32 = 16.0;
/// AEC 的「近端 / 远端」端点：两个字比通道号宽。
const PORT_W_WIDE: f32 = 21.0;
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
  /// 唯一的「添加节点」菜单：一张面板里按组列出所有节点类型。
  Add,
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
  Player { node: u32 },
}

/// EQ 画布在窗口里的逻辑像素矩形（prepaint 时写入，鼠标事件读）。
#[derive(Clone, Copy, Default)]
struct EqGeom {
  x: f32,
  y: f32,
  w: f32,
  h: f32,
}

/// 正在拖动的 EQ 段：节点 + 段号（横向拖动不换段）+ 按下时的纵坐标。
#[derive(Clone, Copy)]
struct EqDrag {
  node: u32,
  band: usize,
  /// 按下位置：垂直移动超过阈值才开始改值，单击只选中、不误改增益。
  start_y: f32,
}

/// 31 段 EQ 的插槽数（S1–S8，和图形 EQ 的惯例一致）。
pub(super) const EQ_SLOTS: usize = 8;

/// EQ 插槽：每槽保存一份完整设置；`active` 是当前编辑的槽。
///
/// 存在计划节点的 `params.slots` 里（JSON），和 `params.settings`（当前设置）一起持久化。
/// 插槽只属于界面，不参与音频路径。
#[derive(Clone, Debug, serde::Serialize, serde::Deserialize)]
pub(super) struct EqSlots {
  #[serde(default)]
  pub active: usize,
  #[serde(default)]
  pub slots: Vec<EqSettings>,
}

impl EqSlots {
  /// 第一次使用：S1 = 当前设置，其余空槽为平直。
  pub(super) fn with_current(current: &EqSettings) -> Self {
    let mut slots = vec![EqSettings::default(); EQ_SLOTS];
    slots[0] = current.clone();
    Self { active: 0, slots }
  }

  /// 补齐 / 截断到 8 个槽，并夹住 active。
  pub(super) fn normalize(&mut self) {
    self.slots.resize(EQ_SLOTS, EqSettings::default());
    self.slots.truncate(EQ_SLOTS);
    self.active = self.active.min(EQ_SLOTS - 1);
  }
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

thread_local! {
  /// 主视图：Linux 关闭窗口只是收进托盘，窗口被移除后仍要持有它，
  /// 这样 tick 还能继续处理「显示窗口 / 退出」命令。
  static MAIN_VIEW: RefCell<Option<Entity<Workspace>>> = const { RefCell::new(None) };
}

/// 记下主视图（第一次开窗口时调用）。
pub(crate) fn remember_view(view: Entity<Workspace>) {
  MAIN_VIEW.with(|v| *v.borrow_mut() = Some(view));
}

/// 当前主视图（主线程；窗口关掉后仍然可用）。
pub(crate) fn current_view() -> Option<Entity<Workspace>> {
  MAIN_VIEW.with(|v| v.borrow().clone())
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
  /// 节点卡片是否显示 VU 电平行（工具栏开关，落盘到 settings.show_vu）
  show_vu: bool,
  /// 网络服务快照（每 1 s 刷一次；卡片常驻在右侧）
  net_stats: Option<crate::net::hub::NetStats>,
  /// 网络卡片里的端口输入框（懒创建；输入即更新二维码，暂停后自动应用）
  net_port_input: Option<Entity<gpui_kit::component::input::InputState>>,
  /// 输入框里正在显示的端口（二维码 / 复制立即跟随；实际监听在暂停后重启）
  net_port_draft: Option<u16>,
  /// 端口输入防抖代号：只有最后一次输入会真正重启服务
  net_port_gen: u64,
  net_ifaces: Vec<crate::net::addrs::NetIface>,
  net_ip: String,
  net_qr: Option<std::path::PathBuf>,
  net_qr_url: String,
  net_last: Instant,
  /// 播放器续播位置回写节流（播放中每 2 s 把实际位置写进 `resume_sec`）
  resume_last: Instant,
  /// AEC 校准结论（只取这一份，不拉整份调试快照）
  calib_report: crate::debug::Probe<crate::engine::calib::CalibReport>,
  /// 已回填过的校准报告 `created_ms`（避免每秒重复回填/重建）
  last_calib: u64,
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
  /// TSE 参考录音已经处理过的最新代号：完成后只重建一次，不让每拍都重载
  tse_ref_seen: u64,
  /// 上一拍引擎运行状态：托盘/全局热键启停发生在界面之外，靠它触发重绘
  was_running: bool,
  /// 上次把窗口几何写进 settings 的时刻（拖动时节流，关闭时无条件写）
  last_geom_save: Instant,
  /// EQ 画布几何（每个检查器复用；prepaint 写入）
  eq_geom: Rc<Cell<EqGeom>>,
  /// 正在拖的 EQ 段
  eq_drag: Option<EqDrag>,
  /// EQ 检查器里当前选中的段（节点 + 段号）：点柱子选中，可精确输入 dB。
  eq_band: Option<(u32, usize)>,
  /// 鼠标停在 EQ 哪一段（画布内显示频率 + 当前 dB 气泡）。
  eq_hover: Option<(u32, usize)>,
  /// 参数精确输入框（按 `num:{节点}:{参数}` / `eq:{节点}` / `gain:{节点}:{参数}` 缓存）。
  inputs: std::collections::HashMap<String, Entity<gpui_kit::component::input::InputState>>,
  /// 正在拖的参数滑杆 key：只在第一次 Change 记一次撤销快照，松手清空。
  slider_drag: Option<String>,
  /// 托盘要求显示窗口：下一次 render 把窗口显示 / 拉到前台
  /// （Windows 的窗口是 SW_HIDE，需 ShowWindow；Wayland 是激活）。
  show_window: bool,
  /// 标题栏关闭：置位后由 tick 移除窗口（不能在窗口事件处理里直接 remove）。
  hide_window: bool,
  /// 非热参数滑杆停手后的待应用值：`(何时, 节点, 参数, 文本)`；200 ms 内没有新值就重建一次。
  pending_param: Option<(Instant, u32, String, String)>,
  /// 虚拟驱动创建 / 移除完成后，刷新一次状态与设备列表（按钮不用再点刷新）。
  pending_vm_refresh: bool,
  /// 可视化频谱的显示平滑（按 (列, 行)）；Rust 侧不改实测值
  viz_smooth: std::collections::HashMap<(usize, usize), Vec<f32>>,
  /// 已经平滑到哪一帧：卡片和检查器可能在同一拍各渲染一次，不能平滑两次。
  viz_seen: std::collections::HashMap<(usize, usize), u64>,
}

impl Drop for Workspace {
  fn drop(&mut self) {
    graph::set_recording_keys(false);
  }
}

impl Workspace {
  /// 给重新打开窗口时恢复键盘焦点用。
  pub(crate) fn focus_handle(&self) -> FocusHandle {
    self.focus_handle.clone()
  }

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
    let last_calib = match &calib_report {
      crate::debug::Probe::Ok { value } => value.created_ms,
      _ => 0,
    };
    let was_running = audio.is_running();
    let show_vu = settings.show_vu;
    let mut this = Workspace {
      focus_handle,
      audio,
      hub,
      plan,
      devices: crate::devices::cached().devices,
      last_refresh: Instant::now(),
      selected,
      dock_page: DockPage::Node,
      status: i18n::tr("点「添加节点」加节点；拖动端口连线，点节点改参数").into(),
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
      show_vu,
      net_stats: None,
      net_port_input: None,
      net_port_draft: None,
      net_port_gen: 0,
      net_ifaces,
      net_ip,
      net_qr: None,
      net_qr_url: String::new(),
      net_last: Instant::now() - Duration::from_secs(2),
      resume_last: Instant::now() - Duration::from_secs(2),
      calib_report,
      last_calib,
      pending_apply: None,
      media_pick: Arc::new(std::sync::Mutex::new(None)),
      app_list: Vec::new(),
      update_status: crate::update::status(),
      hotkeys: None,
      pending_cue: None,
      recording: None,
      tse_ref_seen: 0,
      was_running,
      last_geom_save: Instant::now(),
      eq_geom: Rc::new(Cell::new(EqGeom::default())),
      eq_drag: None,
      eq_band: None,
      eq_hover: None,
      inputs: std::collections::HashMap::new(),
      slider_drag: None,
      show_window: false,
      hide_window: false,
      pending_param: None,
      pending_vm_refresh: false,
      viz_smooth: std::collections::HashMap::new(),
      viz_seen: std::collections::HashMap::new(),
    };
    this.restart_hotkeys();
    // 热键录制：在 GPUI 动作分发之前截走按键（普通 key_down 会被动作抢先，见 `graph.rs`）。
    let capture = cx.intercept_keystrokes(|event, _window, cx| {
      if graph::capture_recording_key(&event.keystroke) {
        cx.stop_propagation();
      }
    });
    this.subs.push(capture);
    // 拖动窗口时不要每个像素都写盘；400 ms 节流，关闭时由 main 再无条件写一次。
    let geom_sub = cx.observe_window_bounds(window, |this, window, cx| {
      if this.last_geom_save.elapsed() >= Duration::from_millis(400) {
        this.last_geom_save = Instant::now();
        let _ = crate::config::save_window_geom(crate::config::geom_from_window(window));
      }
      // 最大化 / 还原后标题栏的按钮图标要跟着换。
      cx.notify();
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

  /// 某一行的显示频谱（已做 EMA 平滑）。同一帧只平滑一次：卡片先渲染、检查器
  /// 后渲染时不会把 α 吃掉两次。Rust 侧的实测值不改，这里只压显示抖动。
  pub(super) fn viz_bands(&mut self, col: usize, row: usize) -> Option<Vec<f32>> {
    let snap = crate::engine::viz::snapshot();
    let frame = snap.frames.iter().find(|f| f.col == col && f.row == row)?;
    let key = (col, row);
    if self.viz_seen.get(&key) == Some(&frame.seq) {
      return self
        .viz_smooth
        .get(&key)
        .cloned()
        .or_else(|| Some(frame.bands_db.clone()));
    }
    let prev = self.viz_smooth.get(&key);
    let bands: Vec<f32> = frame
      .bands_db
      .iter()
      .enumerate()
      .map(|(i, &v)| match prev.and_then(|p| p.get(i)) {
        Some(&old) => old + 0.45 * (v - old),
        None => v,
      })
      .collect();
    self.viz_smooth.insert(key, bands.clone());
    self.viz_seen.insert(key, frame.seq);
    Some(bands)
  }

  fn selected_node(&self) -> Option<&RowSpec> {
    self.selected.and_then(|id| self.node(id))
  }

  /// 定时轮询：返回 true = 界面有变化，需要重绘。
  fn tick(&mut self, cx: &mut Context<Self>) -> bool {
    crate::debug::input_probe::note_tick();
    crate::debug::input_probe::note_windows(cx.windows().len());
    let mut dirty = false;
    // 标题栏关闭由 tick 执行：点击事件里直接 remove_window 会被窗口更新挡住。
    if self.hide_window {
      self.hide_window = false;
      crate::hide_main_window(cx);
      dirty = true;
    }
    // 热键录制：按键由拦截器存下，这里在动作分发之外处理。
    if self.recording.is_some()
      && let Some(ks) = graph::take_recorded_key()
    {
      self.apply_recorded_key(&ks, cx);
      dirty = true;
    }
    // 引擎可能在界面之外被托盘 / 全局热键启停：状态变了就重绘。
    let running = self.audio.is_running();
    if running != self.was_running {
      self.was_running = running;
      // 引擎停 / 起时频谱换代：不要拿着上一代的平滑值继续画。
      self.viz_smooth.clear();
      self.viz_seen.clear();
      // 托盘图标：运行中 = 亮，已停止 = 暗（与 Tauri 版的 sync_tray 同一行为）。
      crate::tray::set_running(running);
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
    // 播放器断点续播：运行中每 2 s 把**实际播放位置**写回 `resume_sec`（不重建、不进撤销）。
    // 这样「继续播放」不依赖用户手动拖进度条：暂停 / 关机 / 热键停止后都能接着放。
    if running && self.resume_last.elapsed() >= Duration::from_secs(2) {
      self.resume_last = Instant::now();
      let mut updates: Vec<(u32, String)> = Vec::new();
      for c in &self.plan.columns {
        for r in &c.rows {
          if r.ptype != "player" || !r.enabled {
            continue;
          }
          if let Some((ci, ri)) = self.pos(r.id)
            && let Some(slot) = crate::engine::media::hub().slot(ci, ri)
          {
            let st = slot.status();
            if st.dur > 0.0 {
              let text = format!("{:.2}", st.pos.clamp(0.0, st.dur));
              if r.params.get("resume_sec") != Some(&text) {
                updates.push((r.id, text));
              }
            }
          }
        }
      }
      if !updates.is_empty() {
        for (id, text) in updates {
          if let Some(n) = self.plan.node_mut(id) {
            n.params.insert("resume_sec".into(), text);
          }
        }
        crate::config::save_plan_debounced(self.plan.clone());
        dirty = true;
      }
    }
    // 网络卡片常驻在右侧：1 s 刷一次统计；二维码只在地址变化时重新生成。
    if self.net_last.elapsed() >= Duration::from_secs(1) {
      self.net_last = Instant::now();
      self.net_stats = Some(crate::net::hub().stats());
      self.net_ifaces = crate::net::addrs::list();
      self.calib_report = self.hub.calib_report();
      // 校准成功：按 Tauri 版的行为回填到对应 AEC 节点（延时 + 两路增益），再重建会话。
      let apply = match &self.calib_report {
        crate::debug::Probe::Ok { value } if value.ok && value.created_ms != self.last_calib => {
          Some((
            value.col,
            value.row,
            value.created_ms,
            value.delay_ms,
            value.mic_gain_db,
            value.far_gain_db,
          ))
        }
        _ => None,
      };
      if let Some((col, row, created, delay, mic_gain, far_gain)) = apply {
        let node = self
          .plan
          .columns
          .get_mut(col)
          .and_then(|c| c.rows.get_mut(row));
        if let Some(node) = node
          && node.ptype == "echo_cancel"
        {
          if let Some(ms) = delay {
            node
              .params
              .insert("far_delay_ms".into(), format!("{ms:.0}"));
          }
          if let Some(g) = mic_gain {
            node.params.insert("mic_gain_db".into(), format!("{g:.1}"));
          }
          if let Some(g) = far_gain {
            node.params.insert("far_gain_db".into(), format!("{g:.1}"));
          }
          self.last_calib = created;
          self.snapshot();
          self.status = match delay {
            Some(ms) => format!("校准已回填：延时 {ms:.0} ms"),
            None => "校准已回填".into(),
          };
          self.apply_structure(cx);
        } else {
          // 目标节点不在了：记下这份报告，避免每秒重试。
          self.last_calib = created;
        }
      }
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
    // 虚拟驱动创建 / 移除完成：自动刷新状态与设备列表（不用再手点「刷新」）。
    if self.job.is_none() && self.pending_vm_refresh {
      self.pending_vm_refresh = false;
      self
        .hub
        .set_virtual_mic(crate::audio::virtual_mic::status_probe());
      self.status = crate::devices::spawn_refresh(self.hub.clone());
      dirty = true;
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
    // TSE 参考写完：重建会话，让运行中的 TSE Stage 重新加载新参考。
    // 同一代只处理一次；录音失败也在这里落一次界面提示。
    let rec = self.audio.recorder_status();
    if rec.token != 0 && rec.token != self.tse_ref_seen {
      match rec.phase {
        crate::recorder::RecorderPhase::Done => {
          self.tse_ref_seen = rec.token;
          self.status = "TSE 参考已更新，正在重新加载".into();
          self.apply_structure(cx);
          dirty = true;
        }
        crate::recorder::RecorderPhase::Failed => {
          self.tse_ref_seen = rec.token;
          self.status = "录制 TSE 参考失败（详见调试面板）".into();
          dirty = true;
        }
        _ => {}
      }
    }
    // 上一个重建跑完后，应用排队里的**最后一份**计划（连点模型/结构参数不排长队）。
    if self.job.is_none()
      && let Some(plan) = self.pending_apply.take()
    {
      self.spawn_apply(plan);
      dirty = true;
    }
    // 非热参数滑杆停手 200 ms 后应用一次（键盘调值没有 Release，也靠它兜底）。
    if let Some((at, id, key, value)) = self.pending_param.clone()
      && at.elapsed() >= Duration::from_millis(200)
    {
      self.pending_param = None;
      self.set_param(id, &key, value, cx);
      dirty = true;
    }
    // 引擎运行时逐节点电平在动，必须重绘；关掉 VU 且没有频谱/录音进度时，
    // 不再按 12.5 Hz 空转重绘（音频引擎本身不受这个开关影响）。
    let recorder_busy = matches!(
      rec.phase,
      crate::recorder::RecorderPhase::Recording
        | crate::recorder::RecorderPhase::Finishing
        | crate::recorder::RecorderPhase::Encoding
    );
    let live = self.audio.is_running()
      && (self.show_vu || recorder_busy || self.plan.nodes().any(|n| n.ptype == "viz"));
    dirty || live
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
      let (w, h) = Self::card_size_for(n, self.show_vu);
      for p in 0..count {
        let (x, y) = Self::port_xy(n, want_input, p, w, h);
        // 吸附到端子的外沿（线本来画到方块外面）
        let y = if want_input {
          y - PORT_H / 2.0
        } else {
          y + PORT_H / 2.0
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
      UiCommand::SetEngine { on } => {
        if on != self.audio.is_running() {
          self.toggle_engine(cx);
        }
      }
      UiCommand::OpenSettings => {
        self.dock_page = DockPage::Settings;
        self.selected = None;
      }
      UiCommand::Quit => cx.quit(),
      UiCommand::ShowWindow => {
        if cx.windows().is_empty() {
          // 窗口已经被移除（收进托盘）：defer 到本轮更新结束再重开，避免重入。
          cx.defer(crate::reopen_main_window);
        } else {
          // Windows 的窗口只是 SW_HIDE，直接 ShowWindow，不等下一帧；Wayland 仍靠 render 激活。
          #[cfg(windows)]
          crate::tray::show_main();
          self.show_window = true;
        }
      }
      UiCommand::ToggleWindow => {
        // Windows 的窗口只是 SW_HIDE，不能靠 windows() 是否为空判断显隐。
        #[cfg(windows)]
        crate::tray::toggle_main();
        #[cfg(not(windows))]
        {
          if cx.windows().is_empty() {
            cx.defer(crate::reopen_main_window);
          } else {
            self.hide_window = true;
          }
        }
      }
      UiCommand::HideWindow => {
        crate::hide_main_window(cx);
      }
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
      UiCommand::AddNode { ptype } => {
        let kind = Self::kind_for(&ptype);
        self.add_node(kind, &ptype, cx);
      }
      UiCommand::CalibrateAec { id } => self.start_calibrate(id, cx),
    }
    cx.notify();
  }
}

// ── 绘制 ──────────────────────────────────────────────────────────────

/// 自绘标题栏（Windows / Linux 共用外观与行为）：拖动、双击最大化、关闭。
///
/// 平台差别只在系统集成方式：
/// - Windows 用 `window_control_area` 把拖动 / 最大化交给系统（保留 Win11 贴靠布局），
///   关闭走系统 `WM_CLOSE`，仍由 `on_window_should_close` 收进托盘；
/// - Linux 用 `start_window_move()`（Wayland / X11 只能这样），按钮走点击回调。
///
/// 关闭收进托盘，与托盘菜单同一条路径；不设最小化按钮（也可从托盘收起）。
fn title_bar(cx: &mut Context<Workspace>, window: &Window) -> AnyElement {
  // Linux 走服务端装饰时窗口管理器已经画了标题栏，再画一套就是两组按钮。
  #[cfg(target_os = "linux")]
  if !matches!(
    window.window_decorations(),
    gpui_kit::Decorations::Client { .. }
  ) {
    return div().into_any_element();
  }
  // 其他平台仍用系统标题栏。
  #[cfg(not(any(windows, target_os = "linux")))]
  {
    let _ = (cx, window);
    return div().into_any_element();
  }
  #[cfg(any(windows, target_os = "linux"))]
  {
    let max_icon = if window.is_maximized() {
      IconName::WindowRestore
    } else {
      IconName::WindowMaximize
    };

    let drag = div()
      .id("title-bar-drag")
      .flex()
      .items_center()
      .flex_1()
      .h_full()
      .pl(px(t::SPACE_3))
      .child(
        div()
          .text_size(px(t::TEXT_SM))
          .text_color(rgb(t::MUTED))
          .child(format!("PureVox {}", crate::version::VERSION)),
      );

    #[cfg(windows)]
    let drag = drag.window_control_area(gpui_kit::WindowControlArea::Drag);

    #[cfg(not(windows))]
    let drag = {
      // 拖动要等真正移动才启动；在按钮上按下时 stop_propagation，不会误拖窗口。
      let armed = Rc::new(Cell::new(false));
      let arm_down = armed.clone();
      let arm_up = armed.clone();
      let arm_move = armed.clone();
      drag
        .on_mouse_down(gpui_kit::MouseButton::Left, move |_, _, _| {
          arm_down.set(true);
        })
        .on_mouse_up(gpui_kit::MouseButton::Left, move |_, _, _| {
          arm_up.set(false);
        })
        .on_double_click(|_, window, _| window.zoom_window())
        .on_mouse_move(
          cx.listener(move |_this, e: &gpui_kit::MouseMoveEvent, window, _cx| {
            if arm_move.get() && e.pressed_button == Some(gpui_kit::MouseButton::Left) {
              arm_move.set(false);
              window.start_window_move();
            }
          }),
        )
        .on_mouse_down(gpui_kit::MouseButton::Right, |e, window, _| {
          window.show_window_menu(e.position)
        })
    };

    div()
      .id("title-bar")
      .flex()
      .items_center()
      .justify_between()
      .h(px(t::TITLE_H))
      .bg(rgb(t::PANEL))
      .border_b_1()
      .border_color(rgb(t::BORDER))
      .child(drag)
      .child(
        div()
          .flex()
          .items_center()
          .h_full()
          .child(title_button(cx, TitleAction::Max, max_icon))
          .child(title_button(cx, TitleAction::Close, IconName::WindowClose)),
      )
      .into_any_element()
  }
}

#[derive(Clone, Copy)]
enum TitleAction {
  Max,
  Close,
}

/// 标题栏按钮：和窗口同高，悬停变色；点击不触发拖动 / 双击。
///
/// Windows 交给系统（`window_control_area`），才有 Win11 贴靠布局与正常的
/// `WM_CLOSE` 路径。Linux 走点击回调。
fn title_button(cx: &mut Context<Workspace>, action: TitleAction, icon: IconName) -> AnyElement {
  let (id, danger) = match action {
    TitleAction::Max => ("tb-max", false),
    TitleAction::Close => ("tb-close", true),
  };
  let button = div()
    .id(id)
    .flex()
    .items_center()
    .justify_center()
    .w(px(t::TITLE_H))
    .h_full()
    .flex_shrink_0()
    .cursor_pointer()
    .hover(move |style| {
      style.bg(if danger {
        rgb(t::DANGER)
      } else {
        rgb(t::CONTROL_HI)
      })
    })
    .child(
      gpui_kit::component::Icon::new(icon)
        .small()
        .text_color(rgb(t::TEXT)),
    );

  #[cfg(windows)]
  {
    // Windows 不需要点击回调（交给系统），只有 Linux 分支用 `cx`。
    let _ = cx;
    button
      .window_control_area(match action {
        TitleAction::Max => gpui_kit::WindowControlArea::Max,
        TitleAction::Close => gpui_kit::WindowControlArea::Close,
      })
      .into_any_element()
  }

  #[cfg(not(windows))]
  button
    .on_mouse_down(gpui_kit::MouseButton::Left, |_, _, cx| {
      cx.stop_propagation()
    })
    .on_click(cx.listener(move |this, _, window, cx| match action {
      TitleAction::Max => window.zoom_window(),
      TitleAction::Close => {
        this.hide_window = true;
        cx.notify();
      }
    }))
    .into_any_element()
}

impl Render for Workspace {
  fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
    if self.show_window {
      self.show_window = false;
      crate::show_main_window(window);
    }
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
      .on_mouse_move(cx.listener(Self::on_mouse_move))
      .capture_any_mouse_down(
        cx.listener(|_this, e: &gpui_kit::MouseDownEvent, window, _cx| {
          crate::debug::input_probe::note(
            true,
            e.position.x.as_f32(),
            e.position.y.as_f32(),
            window.scale_factor(),
          );
        }),
      )
      .on_key_down(cx.listener(|this, e: &gpui_kit::KeyDownEvent, _, cx| {
        if this.menu.is_some() && e.keystroke.key.eq_ignore_ascii_case("escape") {
          this.menu = None;
          cx.stop_propagation();
          cx.notify();
        }
      }))
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
                      let (w, h) = Self::card_size_for(n, this.show_vu);
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
                let (w, h) = Self::card_size_for(n, this.show_vu);
                let (tx, ty) = Self::port_xy(n, tinput, tp as usize, w, h);
                cur = (tx, ty + if tinput { -PORT_H / 2.0 } else { PORT_H / 2.0 });
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
      .child(title_bar(cx, window))
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
              .on_mouse_up(
                gpui_kit::MouseButton::Left,
                cx.listener(|this, _, _, cx| {
                  if this.menu.is_some() {
                    this.menu = None;
                    cx.notify();
                  }
                }),
              )
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
      .child(div().w(px(t::SPACE_1)))
      .child(
        add_button(cx, "menu-add", "添加节点", self.menu.is_some()).on_click(cx.listener(
          |this, _, _, cx| {
            this.menu = if this.menu.is_some() {
              None
            } else {
              Some(Menu::Add)
            };
            cx.notify();
          },
        )),
      )
      .child(div().w(px(1.)).h(px(22.)).bg(rgb(t::BORDER)))
      .child(
        toolbar_icon(cx, "arrange", IconName::LayoutDashboard, "整理节点").on_click(cx.listener(
          |this, _, _, cx| {
            this.arrange();
            cx.notify();
          },
        )),
      )
      .child(
        toolbar_icon(cx, "undo", IconName::Undo2, "撤销").on_click(cx.listener(
          |this, _, _window, cx| {
            this.undo(cx);
          },
        )),
      )
      .child(
        toolbar_icon(cx, "redo", IconName::Redo2, "重做").on_click(cx.listener(
          |this, _, _window, cx| {
            this.redo(cx);
          },
        )),
      )
      .child(
        toolbar_icon(cx, "refresh", IconName::RefreshCw, "刷新设备").on_click(cx.listener(
          |this, _, _, cx| {
            this.status = crate::devices::spawn_refresh(this.hub.clone());
            cx.notify();
          },
        )),
      )
      // VU 显示开关：眼睛带斜线 = 隐藏；不是胶囊开关，保持顶栏图标命令的统一形态。
      .child(
        toolbar_icon(
          cx,
          "vu",
          if self.show_vu {
            IconName::Eye
          } else {
            IconName::EyeOff
          },
          if self.show_vu {
            "隐藏电平"
          } else {
            "显示电平"
          },
        )
        .on_click(cx.listener(|this, _, _, cx| {
          let next = !this.show_vu;
          this.show_vu = next;
          this.settings.show_vu = next;
          this.status = match crate::config::save_settings(&this.settings) {
            Ok(()) => i18n::tr(if next {
              "已显示电平"
            } else {
              "已隐藏电平"
            })
            .to_string(),
            Err(e) => format!("保存设置失败：{e}"),
          };
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
        toolbar_icon(cx, "settings", IconName::Settings2, "设置").on_click(cx.listener(
          |this, _, _, cx| {
            // 全局设置是独立目的地：清掉节点选中，右侧只显示设置页。
            this.dock_page = DockPage::Settings;
            this.selected = None;
            cx.notify();
          },
        )),
      )
      .child(
        toolbar_icon(cx, "debug", IconName::SquareTerminal, "调试面板").on_click(|_, _, _| {
          let url = format!("http://127.0.0.1:{}/debug/ui", crate::debug::http_port());
          let _ = crate::openurl::open(&url);
        }),
      )
  }

  /// 「添加节点」面板：按组横排，一次看全所有类型。
  ///
  /// 顶栏只有一个入口（`add_button`），不再排五个长得一样的分类按钮；
  /// 组内条目是 ghost 行，只有文字和悬停底色，读起来像菜单而不是一排按钮。
  fn menu_panel(&self, _menu: Menu, cx: &mut Context<Self>) -> AnyElement {
    type Item = (&'static str, RowKind, &'static str);
    let groups: [(&str, &[Item]); 4] = [
      (
        "输入",
        &[
          ("麦克风", RowKind::Input, "audio_input"),
          ("系统声音（回环）", RowKind::Input, "loopback"),
          ("应用音频（Windows）", RowKind::Input, "app_capture"),
          ("手机麦克风", RowKind::Input, "remote_mic"),
          ("测试音", RowKind::Input, "tone"),
        ],
      ),
      (
        "AI",
        &[
          ("AI 降噪", RowKind::Process, "denoise"),
          ("目标说话人 TSE", RowKind::Process, "tse"),
          ("回声消除 AEC", RowKind::Process, "echo_cancel"),
        ],
      ),
      (
        "处理",
        &[
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
      ),
      (
        "内容",
        &[
          ("音效板", RowKind::Process, "soundpad"),
          ("音乐播放器", RowKind::Process, "player"),
        ],
      ),
    ];
    let mut panel = div()
      .absolute()
      .top(px(t::TOP_H + t::SPACE_1))
      .left(px(t::SPACE_3))
      .flex()
      .flex_wrap()
      .items_start()
      .gap_5()
      .w(px(880.))
      .p(px(t::SPACE_4))
      .rounded_lg()
      .bg(rgb(t::PANEL_HI))
      .border_1()
      .border_color(rgb(t::BORDER))
      // 菜单是浮层：整块挡住下面的画板/检查器，否则点击会穿到背景上。
      .occlude();
    for (title, items) in groups {
      let mut col = div().flex().flex_col().gap_1().min_w(px(140.));
      col = col.child(
        div()
          .pb(px(t::SPACE_1))
          .text_size(px(t::TEXT_XS))
          .text_color(rgb(t::MUTED))
          .child(i18n::tr(title).to_string()),
      );
      for (label, kind, ptype) in items {
        let p = ptype;
        let label = i18n::tr(label).to_string();
        col = col.child(
          Button::new(format!("menu-item:{ptype}"))
            .xsmall()
            .ghost()
            .w_full()
            .accessibility_label(label.clone())
            .child(div().flex_1().min_w_0().text_left().child(label))
            .on_click(cx.listener(move |this, _, _window, cx| {
              this.menu = None;
              this.add_node(*kind, p, cx);
            })),
        );
      }
      panel = panel.child(col);
    }
    // 输出：每台设备一行；点一下直接建一张已绑定设备的输出卡片，并自动接在选中节点后。
    // 同设备已经在画板上时会在点击后选中已有卡片（`add_node` 的去重），标签也先标出来。
    let mut out_col = div().flex().flex_col().gap_1().min_w(px(160.));
    out_col = out_col.child(
      div()
        .pb(px(t::SPACE_1))
        .text_size(px(t::TEXT_XS))
        .text_color(rgb(t::MUTED))
        .child(i18n::tr("输出").to_string()),
    );
    let mut outputs: Vec<(String, String, bool)> = self
      .devices
      .iter()
      .filter(|d| d.direction == "output")
      .map(|d| (d.id.clone(), d.name.clone(), d.is_default))
      .collect();
    outputs.sort_by_key(|(_, _, is_default)| !*is_default);
    if outputs.is_empty() {
      out_col = out_col.child(
        Button::new("menu-item:audio_output")
          .xsmall()
          .ghost()
          .w_full()
          .accessibility_label(i18n::tr("输出设备").to_string())
          .child(
            div()
              .flex_1()
              .min_w_0()
              .truncate()
              .text_left()
              .child(i18n::tr("输出设备").to_string()),
          )
          .on_click(cx.listener(move |this, _, _window, cx| {
            this.menu = None;
            this.add_node(RowKind::Output, "audio_output", cx);
          })),
      );
    } else {
      for (id, name, is_default) in outputs {
        let used = self
          .plan
          .nodes()
          .any(|n| n.ptype == "audio_output" && n.device.as_deref() == Some(id.as_str()));
        let mut label = if is_default {
          format!("{} · {name}", i18n::tr("默认"))
        } else {
          name.clone()
        };
        if used {
          label.push_str(&format!(" · {}", i18n::tr("已添加")));
        }
        let item_id = format!("menu-out:{id}");
        let dev = (id, name);
        out_col = out_col.child(
          Button::new(item_id)
            .xsmall()
            .ghost()
            .w_full()
            .accessibility_label(label.clone())
            .child(div().flex_1().min_w_0().truncate().text_left().child(label))
            .on_click(cx.listener(move |this, _, _window, cx| {
              this.menu = None;
              this.add_node_with_device(RowKind::Output, "audio_output", Some(dev.clone()), cx);
            })),
        );
      }
    }
    let remote = i18n::tr("手机 / 客户端").to_string();
    out_col = out_col.child(
      Button::new("menu-item:remote_speaker")
        .xsmall()
        .ghost()
        .w_full()
        .accessibility_label(remote.clone())
        .child(
          div()
            .flex_1()
            .min_w_0()
            .truncate()
            .text_left()
            .child(remote),
        )
        .on_click(cx.listener(move |this, _, _window, cx| {
          this.menu = None;
          this.add_node(RowKind::Output, "remote_speaker", cx);
        })),
    );
    panel = panel.child(out_col);
    panel.into_any_element()
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
        i18n::tr("应用图中…").to_string()
      } else {
        self.status.clone()
      })
      .child(div().flex_1())
      .child(format!(
        "{} {} · {} {} · {}",
        self.plan.nodes().count(),
        i18n::tr("节点"),
        self.plan.nodes().map(|n| n.inputs.len()).sum::<usize>(),
        i18n::tr("条线"),
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
