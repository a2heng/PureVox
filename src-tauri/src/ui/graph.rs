// PureVox — AI 麦克风降噪工具
// Copyright (C) 2024-2026 a2heng <752848283@qq.com>
//
// PureVox is licensed under the GNU General Public License v3.0 or
// later (GPL-3.0-or-later).  See LICENSE for details.
//
// SPDX-License-Identifier: GPL-3.0-or-later

//! 图的编辑命令：加删节点、连线、通道/参数、撤销重做、应用计划。
//! 只改 `Plan` 并交给引擎；绘制在 `canvas.rs`，检查器在 `inspector.rs`。

use std::sync::Arc;

use gpui_kit::{Context, KeyDownEvent, Window};

use super::theme as t;
use super::{
  DeleteNode, PendingMedia, RecordTarget, Redo, RefreshDevices, TERM_H, TERM_PAD, TERM_SLOT,
  ToggleEngine, Undo, Workspace, i18n, term_rows,
};
use crate::plan::{Plan, PortInput, RowKind, RowSpec, input_ports, max_channels, node_size};

impl Workspace {
  /// 录制热键：录制中时拦截根节点收到的下一个按键，写成规范串。
  ///
  /// 只修工具链里已有的两条路径：设置页的启停热键、音效板每个音效的触发键。
  /// 纯修饰键不算；Esc 取消；非法组合（字母不带修饰键等）留在录制态让用户重按。
  pub(super) fn on_record_key(
    &mut self,
    e: &KeyDownEvent,
    _window: &mut Window,
    cx: &mut Context<Self>,
  ) {
    let Some(target) = self.recording else {
      return;
    };
    cx.stop_propagation();
    let key = e.keystroke.key.to_lowercase();
    if matches!(
      key.as_str(),
      "control" | "ctrl" | "alt" | "shift" | "super" | "meta" | "cmd" | "win" | "windows"
    ) {
      return;
    }
    if matches!(key.as_str(), "escape" | "esc") {
      self.recording = None;
      self.restart_hotkeys();
      self.status = "已取消录制热键".into();
      cx.notify();
      return;
    }
    let m = &e.keystroke.modifiers;
    let mut parts: Vec<&str> = Vec::new();
    if m.control {
      parts.push("Ctrl");
    }
    if m.alt {
      parts.push("Alt");
    }
    if m.shift {
      parts.push("Shift");
    }
    if m.platform {
      parts.push("Win");
    }
    parts.push(&key);
    let spec = parts.join("+");
    let norm = crate::hotkey::normalize_spec(&spec);
    if norm.is_empty() {
      self.status = format!("热键无效：{spec}（字母/数字要带修饰键，F1–F24 可单键；Esc 取消）");
      cx.notify();
      return;
    }
    match target {
      RecordTarget::EngineHotkey => {
        self.settings.hotkey = norm.clone();
        self.status = match crate::config::save_settings(&self.settings) {
          Ok(()) => {
            self.restart_hotkeys();
            format!("全局热键：{norm}")
          }
          Err(e) => format!("保存热键失败：{e}"),
        };
      }
      RecordTarget::Pad { node, index } => {
        let mut changed = false;
        if let Some(n) = self.plan.node_mut(node) {
          let mut pads: Vec<serde_json::Value> = n
            .params
            .get("pads")
            .and_then(|s| serde_json::from_str(s).ok())
            .unwrap_or_default();
          if let Some(pad) = pads.get_mut(index)
            && let Some(obj) = pad.as_object_mut()
          {
            obj.insert("hotkey".into(), serde_json::Value::String(norm.clone()));
            changed = true;
          }
          if changed {
            n.params
              .insert("pads".into(), serde_json::Value::Array(pads).to_string());
          }
        }
        if changed {
          self.snapshot();
          self.status = format!("音效热键：{norm}");
          self.apply_structure(cx);
        } else {
          self.status = "没能写入这个音效的热键".into();
        }
      }
    }
    self.recording = None;
    cx.notify();
  }

  /// 重注册全局热键：启停键 + 当前计划里所有音效热键。
  ///
  /// 计划结构变化（加删/换音效、改热键）和设置开关都会调用这里；旧线程先停再起，
  /// 避免同一键位被注册两次（`RegisterHotKey` 第二次必然失败）。
  pub(super) fn restart_hotkeys(&mut self) {
    self.hotkeys = None;
    if !self.settings.hotkey_on {
      return;
    }
    let mut bindings = crate::plan::pad_hotkeys(&self.plan, self.hub.clone());
    let spec = crate::hotkey::normalize_spec(&self.settings.hotkey);
    if !spec.is_empty() {
      let audio = self.audio.clone();
      let hub = self.hub.clone();
      let cue_on = self.settings.cue_on;
      let cue_start = self.settings.cue_start.clone();
      let cue_stop = self.settings.cue_stop.clone();
      bindings.push((
        spec,
        Arc::new(move || {
          let audio = audio.clone();
          let hub = hub.clone();
          let cue_start = cue_start.clone();
          let cue_stop = cue_stop.clone();
          // 热键线程不能阻塞在设备重建上，否则后续按键会排队。
          std::thread::spawn(move || {
            if audio.is_running() {
              audio.stop();
              if cue_on {
                crate::cues::play(&cue_stop, "stop");
              }
              hub.push_ui("info", "全局热键：停止引擎".into());
            } else if let Err(e) = audio.start() {
              hub.push_ui("error", format!("全局热键启动引擎失败：{e}"));
            } else {
              if cue_on {
                crate::cues::play(&cue_start, "start");
              }
              hub.push_ui("info", "全局热键：启动引擎".into());
            }
          });
        }),
      ));
    }
    if !bindings.is_empty() {
      self.hotkeys = Some(crate::hotkey::Hotkeys::start(bindings, self.hub.clone()));
    }
  }

  pub(super) fn apply_structure(&mut self, cx: &mut Context<Self>) {
    self.plan.normalize();
    self.restart_hotkeys();
    let plan = self.plan.clone();
    if self.job.is_some() {
      // 重建是秒级的（模型加载 / 设备重开）。连点模型按钮时不要每个都起一条线程，
      // 只留最后一份计划，当前这次跑完再应用 —— 否则界面会「点了半天，最后停在中间那版」。
      self.pending_apply = Some(plan);
      self.status = "排队应用图（上一个重建还在跑）…".into();
      cx.notify();
      return;
    }
    self.spawn_apply(plan);
    cx.notify();
  }

  pub(super) fn spawn_apply(&mut self, plan: Plan) {
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
  }

  /// 文件对话框返回：把解码后的库内路径写进节点参数，再重建会话。
  ///
  /// `player` 用 `path`（单曲）；`soundpad` 把新音效追加到 `pads` JSON（保留已有）。
  pub(super) fn finish_media_pick(&mut self, pm: PendingMedia, cx: &mut Context<Self>) {
    let imp = match pm.result {
      Ok(v) => v,
      Err(e) => {
        self.status = format!("导入音频失败：{e}");
        self.hub.push_ui("error", format!("导入音频失败：{e}"));
        cx.notify();
        return;
      }
    };
    let path = imp.path.display().to_string();
    let name = imp.name.clone();
    if let Some(n) = self.plan.node_mut(pm.node) {
      match pm.kind {
        crate::media::MediaKind::Music => {
          n.params.insert("path".into(), path.clone());
        }
        crate::media::MediaKind::Soundpad => {
          let mut pads: Vec<serde_json::Value> = n
            .params
            .get("pads")
            .and_then(|s| serde_json::from_str(s).ok())
            .unwrap_or_default();
          pads.push(serde_json::json!({
            "name": name,
            "path": path,
            "volume_db": 0.0,
            "hotkey": "",
          }));
          n.params
            .insert("pads".into(), serde_json::Value::Array(pads).to_string());
        }
      }
    }
    self.status = if imp.reused {
      format!("已在媒体库中：{}", imp.path.display())
    } else {
      format!("已导入：{} → {}", imp.name, imp.path.display())
    };
    self.apply_structure(cx);
    cx.notify();
  }

  pub(super) fn persist_layout(&self) {
    let _ = self.audio.persist_plan(self.plan.clone());
  }

  pub(super) fn undo(&mut self, cx: &mut Context<Self>) {
    if let Some(prev) = self.undo.pop() {
      self.redo.push(self.plan.clone());
      self.plan = prev;
      self.selected = self.selected.filter(|id| self.plan.node(*id).is_some());
      self.apply_structure(cx);
    }
  }

  pub(super) fn redo(&mut self, cx: &mut Context<Self>) {
    if let Some(next) = self.redo.pop() {
      self.undo.push(self.plan.clone());
      self.plan = next;
      self.selected = self.selected.filter(|id| self.plan.node(*id).is_some());
      self.apply_structure(cx);
    }
  }

  pub(super) fn toggle_engine(&mut self, cx: &mut Context<Self>) {
    if self.audio.is_running() {
      self.audio.stop();
      self.status = "引擎已停止（设备已释放）".into();
      if self.settings.cue_on {
        crate::cues::play(&self.settings.cue_stop, "stop");
      }
    } else {
      let mgr = self.audio.clone();
      // 启动是后台任务：提示音等 job 成功回来再播，失败不能响。
      self.pending_cue = Some("start");
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

  pub(super) fn node_kind_label(kind: RowKind, ptype: &str) -> &'static str {
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
      (RowKind::Input, "app_capture") => "应用音频",
      (RowKind::Input, "tone") => "测试音",
      (RowKind::Input, _) => "麦克风",
      (RowKind::Output, "remote_speaker") => "手机 / 客户端",
      (RowKind::Output, _) => "输出",
    }
  }

  pub(super) fn node_color(kind: RowKind, ptype: &str) -> u32 {
    match (kind, ptype) {
      (RowKind::Input, "remote_mic") => t::NET,
      (RowKind::Input, "loopback") => t::LOOPBACK,
      (RowKind::Input, "app_capture") => t::LOOPBACK,
      (RowKind::Input, _) => t::MIC,
      (RowKind::Process, "echo_cancel") => t::AEC,
      (RowKind::Process, _) => t::ACCENT,
      (RowKind::Output, _) => t::OK,
    }
  }

  pub(super) fn node_title(&self, n: &RowSpec) -> String {
    if let Some(name) = n.device_name.as_ref().filter(|x| !x.is_empty()) {
      return name.clone();
    }
    if let Some(d) = n.device.as_ref().filter(|x| !x.is_empty()) {
      return crate::devices::display_name(d, true).unwrap_or_else(|_| d.clone());
    }
    i18n::tr(Self::node_kind_label(n.kind, &n.ptype)).to_string()
  }

  pub(super) fn card_size_parts(kind: RowKind, ptype: &str, channels: u32) -> (f32, f32) {
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

  pub(super) fn card_size(n: &RowSpec) -> (f32, f32) {
    Self::card_size_parts(n.kind, &n.ptype, n.channels)
  }

  /// 端子的绝对坐标（画线、画端口、命中都用这一份，绝不各算一套）。
  pub(super) fn port_xy(n: &RowSpec, input: bool, p: usize, _w: f32, h: f32) -> (f32, f32) {
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

  pub(super) fn free_spot(&self, w: f32, h: f32) -> (f32, f32) {
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

  pub(super) fn add_node(&mut self, kind: RowKind, ptype: &str, cx: &mut Context<Self>) {
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
      "app_capture" => device_name = Some("选择应用…".into()),
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

  pub(super) fn set_param(&mut self, id: u32, key: &str, value: String, cx: &mut Context<Self>) {
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
  pub(super) fn node_max_channels(&self, n: &RowSpec) -> u32 {
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

  pub(super) fn set_channels(&mut self, id: u32, delta: i32, cx: &mut Context<Self>) {
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
  pub(super) fn connect(
    &mut self,
    from: u32,
    from_port: u8,
    to: u32,
    to_port: u8,
    cx: &mut Context<Self>,
  ) {
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

  pub(super) fn disconnect(
    &mut self,
    from: u32,
    from_port: u8,
    to: u32,
    to_port: u8,
    cx: &mut Context<Self>,
  ) {
    self.snapshot();
    self.plan.disconnect(from, from_port, to, to_port);
    self.status = "已断开（节点保留）".into();
    self.apply_structure(cx);
  }

  pub(super) fn delete_node(&mut self, id: u32, cx: &mut Context<Self>) {
    self.snapshot();
    self.plan.remove_node(id);
    self.selected = None;
    self.status = "已删除节点".into();
    self.apply_structure(cx);
  }

  pub(super) fn arrange(&mut self) {
    self.snapshot();
    // 只有一份排版实现（plan::layout_by_depth）：按深度纵向排，同一深度横向分支。
    self.plan.layout_by_depth();
    self.persist_layout();
    self.status = "已按列整理".into();
  }

  pub(super) fn on_toggle_engine(
    &mut self,
    _: &ToggleEngine,
    _: &mut Window,
    cx: &mut Context<Self>,
  ) {
    self.toggle_engine(cx);
  }

  pub(super) fn on_refresh(&mut self, _: &RefreshDevices, _: &mut Window, cx: &mut Context<Self>) {
    self.status = crate::devices::spawn_refresh(self.hub.clone());
    cx.notify();
  }

  pub(super) fn on_undo(&mut self, _: &Undo, _window: &mut Window, cx: &mut Context<Self>) {
    self.undo(cx);
  }

  pub(super) fn on_redo(&mut self, _: &Redo, _window: &mut Window, cx: &mut Context<Self>) {
    self.redo(cx);
  }

  pub(super) fn on_delete(&mut self, _: &DeleteNode, _window: &mut Window, cx: &mut Context<Self>) {
    if let Some(id) = self.selected {
      self.delete_node(id, cx);
    }
  }
}
