# PureVox — AI 麦克风降噪工具
# Copyright (C) 2024-2026 a2heng <752848283@qq.com>
#
# PureVox is licensed under the GNU General Public License v3.0 or
# later (GPL-3.0-or-later).  See LICENSE for details.
#
# This program is free software: you can redistribute it and/or modify
# it under the terms of the GNU General Public License as published by
# the Free Software Foundation, either version 3 of the License, or
# (at your option) any later version.
#
# The built-in AI models are NOT covered by the GPL; they are the
# property of a2heng and may only be used with PureVox under
# authorization.  See MODEL-LICENSE.md for details.
#
# SPDX-License-Identifier: GPL-3.0-or-later

"""uitk 主窗口：顶部工具条 + 单列节点面板，接真实 plugin_chain。

数据流（DESIGN.md §7）：config.plugin_chain（type/enabled/params）
↔ NodeRow 双向绑定；任何增删/排序/开关/参数变化即持久化。
节点类型清单唯一来源 = pvengine.plugins.all_specs()，UI 禁止自建。
"""

import math
import os
import sys
import threading
import time
import webbrowser as _webbrowser_mod


def webbrowser_open(url):
    try:
        _webbrowser_mod.open(url)
    except Exception:
        pass

import tkinter as tk
import tkinter.font as tkfont

from logger import Logger
from . import theme
from .metrics import make_sizes, detect_zoom_for_screen, \
    fix_tk_scaling, pick_font_family
from .widgets import FlatButton, DarkCheck, DarkCombo, ScrollFrame
from .engine import EngineController, enum_io_devices
from .viz import VUCanvas, SpectrumCanvas

KIND_LABELS = {"input": "输入", "output": "输出", "fx": "处理", "viz": "可视化"}
# 需要设备下拉的节点类型：input/output 选 device。
# echo_cancel 的 mic 继承 audio_input 同一套机制（("device", "inputs")，
# 同解析同回填）；far 参考源是第二下拉（_build_far_combo），不另起炉灶。
DEV_KEY = {"audio_input": ("device", "inputs"),
           "audio_output": ("device", "outputs"),
           "remote_mic": None,
           "loopback": ("device", "speakers"),
           "echo_cancel": ("device", "inputs")}


class ParamSlider(tk.Frame):
    """行内参数滑杆：自绘 HSlider + 右侧大号数值（紧贴 × 前）。"""

    def __init__(self, parent, label, lo, hi, default, step,
                 sizes, fonts, on_commit):
        super().__init__(parent, bg=parent.cget("bg"))
        self.sizes = sizes
        self.fonts = fonts
        # 纯单位标签（如 dB）放数字后面；描述性标签才放左侧
        self._unit = label if len(label) <= 3 else ""
        if label and not self._unit:
            tk.Label(self, text=label, bg=self["bg"], fg=theme.TEXT_DIM,
                     font=fonts.get("small")).pack(side=tk.LEFT,
                                                   padx=(0, sizes["pad_sm"]))
        # 数值+单位在右（× 前），大号加粗醒目
        self.val_lbl = tk.Label(self, text=f"{default:g} {self._unit}".strip(),
                                bg=self["bg"], fg=theme.TEXT,
                                font=fonts.get("bold"), anchor="e")
        self.val_lbl.pack(side=tk.RIGHT, padx=(self.sizes["pad_sm"], 0))
        from .widgets import HSlider
        ref = {}
        s = HSlider(self, lo, hi, default, step, sizes=sizes,
                    command=lambda: (
                        self.var.set(ref["s"].value),
                        self.val_lbl.configure(
                            text=f"{ref['s'].value:g} {self._unit}".strip()),
                        on_commit()))
        ref["s"] = s
        s.pack(side=tk.LEFT, fill=tk.X, expand=True)
        self.var = tk.DoubleVar(value=float(default))
        s.bind("<ButtonRelease-1>", lambda e: on_commit())
        s.bind("<ButtonRelease-1>", lambda e: on_commit())


class NodeRow(tk.Frame):
    """节点行：手柄 + 名称 + 启用勾选 + 删除 + inline 参数区。

    手柄「‖」与删除「×」用像素字体渲染；手柄支持拖拽排序。
    """

    GRIP_GLYPH = "‖"
    CLOSE_GLYPH = "×"

    def __init__(self, parent, cfg, spec, sizes, fonts,
                 on_remove=None, on_toggle=None, on_drag_preview=None,
                 on_drag_commit=None, on_param=None):
        self.sizes = sizes
        self.fonts = fonts
        self._on_drag_preview = on_drag_preview
        self._on_drag_commit = on_drag_commit
        self.cfg = cfg            # {type, enabled, params} 引用
        self.spec = spec
        super().__init__(parent.body, bg=theme.PANEL, bd=0)
        head = tk.Frame(self, bg=theme.PANEL)
        # 标题行内部零留白：左右顶到卡片边缘，× 钮正好落在卡片右上角
        head.pack(fill=tk.X)
        self.head = head
        # 布局（左→右）：手柄 · 开关 · 名称 ······ 用户操作区（下拉/滑杆）· 删除 ×
        # 类型名不再占横向空间；中间全部让给用户操作控件
        self.grip = tk.Label(head, text=self.GRIP_GLYPH,
                             bg=theme.PANEL, fg=theme.MID,
                             font=fonts.get("bold"), cursor="fleur")
        self.grip.pack(side=tk.LEFT, padx=(0, self.sizes["pad_sm"]))
        self.grip.bind("<ButtonPress-1>", self._drag_begin)
        self.grip.bind("<B1-Motion>", self._drag_motion)
        self.grip.bind("<ButtonRelease-1>", self._drag_release)
        self.on_var = tk.BooleanVar(value=bool(cfg.get("enabled", True)))
        self.check = DarkCheck(head, "", self.on_var, command=self._toggled,
                               sizes=sizes, fonts=fonts)
        self.check.pack(side=tk.LEFT, padx=(0, self.sizes["pad_sm"]))
        self.title_lbl = tk.Label(head, text=f"{spec.label}", bg=theme.PANEL,
                                  fg=theme.TEXT, anchor="w",
                                  font=fonts.get("body"))
        self.title_lbl.pack(side=tk.LEFT, padx=(0, self.sizes["pad_sm"]))
        # × 正方形红色方框按钮（边长锁死 ctl_h，各节点右缘成列对齐）
        from .widgets import SquareButton
        rm = SquareButton(head, self.CLOSE_GLYPH, command=on_remove,
                          bg=theme.STOP_BG, fg=theme.ACCENT_TEXT,
                          font=fonts.get("bold"), sizes=sizes)
        rm.pack(side=tk.RIGHT)   # × 永远最后（最右）
        self.rm_lbl = rm
        # 中间操作区：设备下拉 / 单参数滑杆都放这里，吃掉全部剩余宽度
        self.mid = tk.Frame(head, bg=theme.PANEL)
        self.mid.pack(side=tk.LEFT, fill=tk.BOTH, expand=True,
                      padx=(0, self.sizes["pad_sm"]))
        self.dev_combo = None
        self._build_device_combo()
        self.far_combo = None
        self._far_items = {}
        self._build_far_combo()
        # 多参数/编辑入口/viz 走下方参数区（始终显示）
        self.body_frame = tk.Frame(self, bg=theme.PANEL)
        self._build_ec_body()
        self._build_inline(on_param)
        self._build_denoise_hint()
        self.ensure_body()

    def ensure_body(self):
        """参数区有「可见（已 pack）」内容才显示，避免隐藏卡片留下空占位。

        横向内边距可按行覆盖（`_body_padx`）：音效板要求整行顶满节点左右边缘。
        """
        has_visible = any(c.winfo_manager()
                          for c in self.body_frame.winfo_children())
        if has_visible:
            padx = getattr(self, "_body_padx", None)
            pady = getattr(self, "_body_pady", None)
            self.body_frame.pack(fill=tk.X,
                                 padx=self.sizes["pad_lg"] if padx is None
                                 else padx,
                                 pady=(2, 4) if pady is None else pady)
        else:
            self.body_frame.pack_forget()

    def _dev_spec(self):
        return DEV_KEY.get(self.spec.name)

    def _build_device_combo(self):
        """设备下拉置于行头最右端（值存 params.device/far_device）。
        echo_cancel 行除外：mic 与 far 各占下方一行（见 _build_ec_body）。"""
        if self.spec.name == "echo_cancel":
            return
        dspec = self._dev_spec()
        if not dspec:
            return
        key, _dir = dspec
        holder: dict = {}
        cur = str((self.cfg.get("params") or {}).get(key, "") or "")
        var = tk.StringVar(value=cur)
        self.dev_var = var
        self.dev_combo = DarkCombo(
            self.mid, [(cur, cur)] if cur else [("（默认）", "")], var,
            on_change=lambda: self._on_dev_changed(holder, key),
            sizes=self.sizes, fonts=self.fonts)
        # 下拉铺满标题与「×」之间的可用宽度：长设备名不被截短，
        # 也不在标题后留下悬空空隙（独占操作区，无同行滑杆）
        self.dev_combo.pack(side=tk.LEFT, fill=tk.BOTH, expand=True)
        holder["row"] = self

    def _on_dev_changed(self, holder, key):
        val = self.dev_var.get()
        if val in ("（默认）",):
            val = ""
        self.cfg.setdefault("params", {})[key] = val
        sync = getattr(self, "_linux_vm_apply", None)
        if sync:
            sync(val)
        cb = getattr(self, "_on_param_cb", None)
        if cb:
            cb()

    def _build_denoise_hint(self):
        """AI 降噪节点（中号/小号）下方的使用建议（小字，多行）。"""
        if self.spec.name not in ("denoiser_m", "denoiser_s"):
            return
        hint = tk.Label(
            self.body_frame,
            text="模型输入音量在 VU 电平表黄色区效果最佳；\n"
                 "输入音量过小会被当做噪音过滤。",
            bg=theme.PANEL, fg=theme.TEXT_DIM,
            font=self.fonts.get("small"), anchor="w", justify="left")
        hint.pack(fill=tk.X, padx=self.sizes["pad_lg"], pady=self.sizes["pad_sm"])
        hint.bind("<Configure>",
                  lambda e: hint.configure(wraplength=max(120, e.width - 4)))

    # ── echo_cancel far 参考源第二下拉（扬声器/麦克风分组二选一）──

    def _far_label(self, kind, name):
        return ("扬声器 | " if kind == "speaker" else "麦克风 | ") + name

    def _build_far_combo(self):
        """旧行头版 far 下拉已废弃：echo_cancel 行的 far 在下方参数区
        独占第三行（见 _build_ec_body）。此函数保留空实现。"""
        return

    _LBL_W = 9   # 参数区左侧标签统一列宽，保证下行/滑杆/电平左对齐

    def _ec_line(self, label):
        """下方参数区一行：固定宽左标签 + 右侧控件（撑满剩余宽度）。"""
        line = tk.Frame(self.body_frame, bg=theme.PANEL)
        line.pack(fill=tk.X, padx=self.sizes["pad_sm"], pady=2)
        tk.Label(line, text=label, bg=theme.PANEL, fg=theme.TEXT_DIM,
                 font=self.fonts.get("small"), width=self._LBL_W,
                 anchor="w").pack(side=tk.LEFT, padx=(0, self.sizes["pad_sm"]))
        return line

    def _build_ec_body(self):
        """echo_cancel 行参数区（自上而下）：扬声器(far) → 麦克风(mic) →
        Far 延迟滑块(0~1000/10) → 三路电平 Mic/Far/Out。

        第一行（行头：标题 + 麦克风 dB 滑杆）走通用 inline 滑杆机制。
        左侧标签统一列宽对齐（_LBL_W）。
        """
        if self.spec.name != "echo_cancel":
            return
        # ── 扬声器（远端参考，保留 far=麦克风选项）──
        fvar = tk.StringVar(value="")
        self.far_var = fvar
        self._far_items = {}
        self.far_combo = DarkCombo(
            self._ec_line("扬声器"), [""], fvar,
            on_change=self._on_far_changed,
            sizes=self.sizes, fonts=self.fonts)
        self.far_combo.pack(side=tk.LEFT, fill=tk.X, expand=True)
        # ── 麦克风（本行消回声的输入麦，与 audio_input 同一套 device 机制）──
        var = tk.StringVar(value=str((self.cfg.get("params") or {}).get(
            "device", "") or ""))
        self.dev_var = var
        self.dev_combo = DarkCombo(
            self._ec_line("麦克风"),
            [(var.get(), var.get())] if var.get() else [],
            var, on_change=lambda: self._on_dev_changed({}, "device"),
            sizes=self.sizes, fonts=self.fonts)
        self.dev_combo.pack(side=tk.LEFT, fill=tk.X, expand=True)
        # ── Far 延迟：滑块 + 值(ms) + 校准 ──
        from .widgets import HSlider, FlatButton
        delay_line = self._ec_line("Far 延迟")
        self._aec_delay_var = tk.DoubleVar(value=0.0)
        auto_btn = FlatButton(delay_line, "校准", sizes=self.sizes,
                              command=self._on_aec_auto_calibrate)
        auto_btn.pack(side=tk.RIGHT, padx=(0, 4))
        self._aec_auto_btn = auto_btn
        delay_lbl = tk.Label(delay_line, text="0ms", bg=theme.PANEL,
                             fg=theme.TEXT, font=self.fonts.get("small"),
                             width=7, anchor="e")
        delay_lbl.pack(side=tk.RIGHT, padx=(4, 0))
        self._aec_delay_lbl = delay_lbl
        delay_slider = HSlider(
            delay_line, 0, 1000, 0.0, 10,
            command=self._on_aec_delay_changed,
            sizes=self.sizes)
        delay_slider.pack(side=tk.LEFT, fill=tk.X, expand=True)
        self._aec_delay_slider = delay_slider
        # 回显已持久化的 far_delay_ms（重启后仍显示并沿用）
        saved_ms = float((self.cfg.get("params") or {}).get(
            "far_delay_ms", 0.0) or 0.0)
        if saved_ms:
            delay_slider.set_value(min(1000.0, max(0.0, saved_ms)))
            delay_lbl.config(text=f"{saved_ms:.0f}ms")
        # ── 三路电平 Mic/Far/Out（放在延迟滑块下方）──
        vu_frame = tk.Frame(self.body_frame, bg=theme.PANEL)
        vu_frame.pack(fill=tk.X, padx=self.sizes["pad_sm"], pady=2)
        self._aec_vu_widgets = {}
        for key, label in [("mic", "Mic"), ("far", "Far"), ("out", "Out")]:
            row = tk.Frame(vu_frame, bg=theme.PANEL)
            row.pack(fill=tk.X, pady=1)
            tk.Label(row, text=label, bg=theme.PANEL, fg=theme.TEXT_DIM,
                     font=self.fonts.get("small"), width=self._LBL_W,
                     anchor="w").pack(side=tk.LEFT,
                                      padx=(0, self.sizes["pad_sm"]))
            vu = VUCanvas(row, sizes=self.sizes, height=16)
            vu.pack(side=tk.LEFT, fill=tk.X, expand=True)
            self._aec_vu_widgets[key] = vu

    def _on_far_changed(self):
        kind, name = self._far_items.get(self.far_var.get(), ("", ""))
        self.cfg.setdefault("params", {})["far_kind"] = kind
        self.cfg.setdefault("params", {})["far_device"] = name
        cb = getattr(self, "_on_param_cb", None)
        if cb:
            cb()

    def _on_aec_delay_changed(self):
        ms = self._aec_delay_slider.value
        self._aec_delay_lbl.config(text=f"{ms:.0f}ms")
        self.cfg.setdefault("params", {})["far_delay_ms"] = ms
        cb = getattr(self, "_on_aec_delay_cb", None)
        if cb:
            cb(ms)

    def _on_aec_auto_calibrate(self):
        cb = getattr(self, "_on_aec_auto_cb", None)
        if cb:
            cb()

    def _refresh_far_combo(self, devices):
        """按输出/输入分组成 far 候选；恢复已存选择，缺省首个扬声器。

        far_device 存真实设备名（Linux=node.name），标签只用于显示；
        keymap 把已存 (kind, 设备名) 反查回下拉标签。
        """
        if self.spec.name != "echo_cancel" or self.far_combo is None:
            return
        items = {}
        keymap = {}
        for t, d in devices.get("speakers", []):
            lb = self._far_label("speaker", t)
            items[lb] = ("speaker", d)
            keymap[("speaker", d)] = lb
        for t, d in devices.get("inputs", []):
            lb = self._far_label("mic", t)
            items[lb] = ("mic", d)
            keymap[("mic", d)] = lb
        self._far_items = items
        labels = list(items) or []
        self.far_combo.set_values(labels)
        params = self.cfg.setdefault("params", {})
        saved = (str(params.get("far_kind", "") or ""),
                 str(params.get("far_device", "") or ""))
        cur = keymap.get(saved, "")
        if not cur:
            # 缺省首个扬声器并回写（行配置显式化，不留空歧义）
            for lb, (k, n) in items.items():
                if k == "speaker":
                    cur = lb
                    params["far_kind"] = k
                    params["far_device"] = n
                    break
            else:
                cur = labels[0] if labels else ""
        self.far_var.set(cur)
        if cur in items:
            k, n = items[cur]
            params["far_kind"] = k
            params["far_device"] = n

    def set_devices(self, devices):
        """刷新设备下拉（保持当前选择）。

        传入项为 (显示, 值) 对：值写进 params[key]（Linux=node.name），
        标签仅用于下拉显示（DarkCombo 按值反查显示）。
        """
        dspec = self._dev_spec()
        if not dspec or not hasattr(self, "dev_combo"):
            return
        key, direction = dspec
        params = self.cfg.setdefault("params", {})
        defaults = devices.get("defaults", {})
        pairs = list(devices.get(direction, [])) or [("（默认）", "")]
        self.dev_combo.set_values(pairs)
        vals = [v for _d, v in pairs]
        cur = str(params.get(key, "") or "")
        if cur in vals:
            self.dev_var.set(cur)
        elif vals:
            # 未选/失效：优先平台默认（Linux 输出默认 purevox_out），否则第一个
            default = defaults.get(direction, "")
            pick = default if default in vals else vals[0]
            self.dev_var.set(pick)
            params[key] = pick
            cb = getattr(self, "_on_param_cb", None)
            if cb:
                cb()
        # 设备变化后同步输出行的虚拟声卡卡显隐（Linux）
        sync = getattr(self, "_linux_vm_apply", None)
        if sync:
            sync(str(params.get(key, "") or ""))
        # echo_cancel 行同步刷新 far 第二下拉（mic 走上面同一套机制）
        self._refresh_far_combo(devices)

    def _build_inline(self, on_param):
        saved = self.cfg.get("params") or {}
        params = self.spec.params or {}
        # 单参数节点：滑杆直接放进行中间操作区（与下拉同一行）
        # 多参数/expand（eq/tse）/viz/agc：走下方参数区
        # （AGC 标题栏恒显「标题 + 增益/峰值数据」，滑杆放下方避免挤掉数据）
        inline_ok = (len(params) == 1
                     and self.spec.tier != "expand"
                     and self.spec.kind != "viz"
                     and self.spec.name != "agc")
        for key, pdef in params.items():
            label, lo, hi, default, step = pdef
            cur = saved.get(key, default)
            ref = {}
            ps = ParamSlider(
                self.mid if inline_ok else self.body_frame,
                label, lo, hi,
                default if cur is None else cur, step,
                self.sizes, self.fonts,
                on_commit=lambda k=key: (
                    on_param and on_param(
                        self, k, round(ref["s"].var.get(), 4))))
            ref["s"] = ps
            ps._key = key
            ps.pack(fill=tk.X, padx=self.sizes["pad_sm"],
                    pady=0 if inline_ok else 2)
        # AGC 节点：标题行 = 「标题 + 增益/峰值数据」（数据紧贴标题）；
        # 最大增益滑杆在下方参数区（inline_ok 已排除 agc）
        if self.spec.name == "agc":
            self._agc_gain_lbl = tk.Label(
                self.mid, text="", width=28,
                bg=theme.PANEL, fg=theme.ACCENT,
                font=self.fonts.get("bold"), anchor="w")
            self._agc_gain_lbl.pack(side=tk.LEFT, padx=(self.sizes["pad_sm"], 0))
            self._agc_instance_id = None
            self._agc_last_val = None
            self._agc_last_change = 0.0

    def _toggled(self):
        self.cfg["enabled"] = bool(self.on_var.get())
        if self._hot_toggle_cb is not None and self._hot_toggle_ok():
            # fx 行热更启停（DESIGN §7）：不重启音频流
            self._hot_toggle_cb(self, bool(self.on_var.get()))
        elif self._on_toggle_cb:
            self._on_toggle_cb()
        # 开关节点同步视觉弱化
        self._apply_enabled_look()

    def _hot_toggle_ok(self):
        # fx 行热更；输入/输出/viz 行（含 echo_cancel）走重启
        # （AEC 行采集生命周期绑定建流，与输入行一致）
        return self.spec.kind == "fx"

    _on_toggle_cb = None
    _hot_toggle_cb = None

    def set_toggle_cb(self, cb):
        self._on_toggle_cb = cb

    def set_hot_toggle_cb(self, cb):
        self._hot_toggle_cb = cb

    def _apply_enabled_look(self):
        fg = theme.TEXT if bool(self.on_var.get()) else theme.MID
        try:
            self.title_lbl.configure(fg=fg)
        except Exception:
            pass

    # ── 拖拽排序：拖动实时换位（直观），松手才持久化/热重建 ──
    def _drag_begin(self, e):
        self._drag_target = None

    def _drag_motion(self, e):
        if self._on_drag_preview is None:
            return
        target = None
        for w in self.master.winfo_children():
            if isinstance(w, NodeRow) and w is not self:
                top = w.winfo_rooty()
                if top <= e.y_root < top + w.winfo_height():
                    target = w
                    break
        if target is not None and target is not self._drag_target:
            self._drag_target = target
            self._on_drag_preview(self, target)

    def _drag_release(self, _e):
        self._drag_target = None
        if self._on_drag_commit is not None:
            self._on_drag_commit()


class MainWindowTk:
    """主窗口：单一工具条 + 节点滚动面板（plugin_chain 持久化）。"""

    def __init__(self, zoom=None, config=None):
        from .metrics import enable_hidpi
        enable_hidpi()
        self.config = config
        self.root = tk.Tk()
        fix_tk_scaling(self.root)
        family = pick_font_family(self.root)
        self.zoom = zoom or detect_zoom_for_screen(
            self.root.winfo_screenwidth(), self.root.winfo_screenheight())
        self.sizes = make_sizes(self.zoom)
        S = self.sizes
        self.fonts = {
            "body": tkfont.Font(family=family, size=-S["font_body"]),
            "bold": tkfont.Font(family=family, size=-S["font_body"],
                                weight="bold"),
            "title": tkfont.Font(family=family, size=-S["font_title"],
                                 weight="bold"),
            "small": tkfont.Font(family=family, size=-S["font_small"]),
        }
        try:
            from _build_version import BUILD_DATE   # 打包脚本生成；源码态缺失
            _ver = str(BUILD_DATE).strip()
        except Exception:
            _ver = ""
        self.root.title("PureVox" + ((" " + _ver) if _ver else "（开发版）"))
        self.root.configure(bg=theme.WINDOW)
        self.root.geometry(f"{S['win_w']}x{S['win_h']}")

        # ── 自绘顶栏（去系统标题栏，保证颜色一致；整体可拖动）──
        self.root.withdraw()   # 先藏窗，全部上色后再显示——避免白闪
        self.root.overrideredirect(True)
        bar_title = tk.Frame(self.root, bg=theme.TITLE_BG,
                             height=S["titlebar_h"])
        bar_title.pack(fill=tk.X)
        bar_title.pack_propagate(False)
        # 三边同色细边：消除「深顶浅底罐头瓶/钉子」观感，形成整框包裹
        bd_l = tk.Frame(self.root, bg=theme.TITLE_BG, width=2)
        bd_r = tk.Frame(self.root, bg=theme.TITLE_BG, width=2)
        bd_b = tk.Frame(self.root, bg=theme.TITLE_BG, height=2)
        bd_l.pack(side=tk.LEFT, fill=tk.Y)
        bd_r.pack(side=tk.RIGHT, fill=tk.Y)
        bd_b.pack(side=tk.BOTTOM, fill=tk.X)
        title_lbl = tk.Label(bar_title,
                             text="PureVox" + ((" " + _ver) if _ver else ""),
                             bg=theme.TITLE_BG,
                             fg=theme.TITLE_FG, font=self.fonts["bold"])
        title_lbl.pack(side=tk.LEFT, padx=S["pad_md"])
        # 关闭钮：外壳锁定正方形（titlebar 内切），按钮填满
        close_wrap = tk.Frame(bar_title, bg=theme.TITLE_BG,
                              width=S["titlebar_h"], height=S["titlebar_h"])
        close_wrap.pack(side=tk.RIGHT)
        close_wrap.pack_propagate(False)
        btn_x = tk.Label(close_wrap, text="×", bg=theme.TITLE_BG,
                         fg=theme.TITLE_FG, font=self.fonts["bold"],
                         cursor="hand2")
        btn_x.place(relx=0.5, rely=0.5, anchor="center")
        btn_x.bind("<Button-1>", lambda e: self._close_request())
        btn_x.bind("<Enter>", lambda e: (btn_x.configure(
            bg=theme.STOP_BG, fg="#ffffff"), close_wrap.configure(bg=theme.STOP_BG)))
        btn_x.bind("<Leave>", lambda e: (btn_x.configure(
            bg=theme.TITLE_BG, fg=theme.TITLE_FG),
            close_wrap.configure(bg=theme.TITLE_BG)))
        for w in (bar_title, title_lbl):
            w.bind("<ButtonPress-1>", self._title_drag_begin)
            w.bind("<B1-Motion>", self._title_drag_move)
        # 无边框窗口保留任务栏图标（WS_EX_APPWINDOW）
        try:
            import ctypes
            hwnd = ctypes.windll.user32.GetParent(self.root.winfo_id()) \
                or self.root.winfo_id()
            exstyle = ctypes.windll.user32.GetWindowLongW(hwnd, -20)
            ctypes.windll.user32.SetWindowLongW(
                hwnd, -20, exstyle | 0x00040000)
            ctypes.windll.user32.SetWindowPos(
                hwnd, 0, 0, 0, 0, 0, 0x0007)
        except Exception:
            pass

        # ── 工具条：启动 → 退出 → 添加 → 设置 ──
        bar = tk.Frame(self.root, bg=theme.WINDOW)
        bar.pack(fill=tk.X, padx=S["pad_md"], pady=S["pad_md"])
        self.btn_start = FlatButton(bar, "启动音频处理",
                                    command=self._on_start,
                                    bg=theme.START_BG, fg=theme.ACCENT_TEXT,
                                    font=self.fonts["body"], sizes=self.sizes)
        self.btn_start.pack(side=tk.LEFT, fill=tk.X, expand=True)

        self.btn_quit = FlatButton(bar, "退出", command=self.quit_app,
                                   bg=theme.STOP_BG,
                                   font=self.fonts["body"],
                                   sizes=self.sizes, pad=S["pad_md"])
        self.btn_quit.pack(side=tk.LEFT, padx=(S["pad_sm"], 0))

        self.btn_add = FlatButton(bar, "添加 ▾", command=self._add_menu,
                                  font=self.fonts["body"], sizes=self.sizes,
                                  pad=S["pad_md"])
        self.btn_add.pack(side=tk.LEFT, padx=(S["pad_sm"], 0))

        self.btn_gear = FlatButton(bar, "设置 ▾", command=self._gear_menu,
                                   font=self.fonts["body"], sizes=self.sizes,
                                   pad=S["pad_md"])
        self.btn_gear.pack(side=tk.LEFT, padx=(S["pad_sm"], 0))

        # ── 输入采样率状态行：输入/far 端实际采样率 → 48k 自适应说明 ──
        self.lbl_sr = tk.Label(self.root, text="输入：未启动",
                               bg=theme.WINDOW, fg=theme.TEXT_FAINT,
                               font=self.fonts["small"], anchor="w",
                               justify="left")
        self.lbl_sr.pack(fill=tk.X, padx=S["pad_md"], pady=(0, S["pad_sm"]))
        self.lbl_sr.bind("<Configure>",
                         lambda e: self.lbl_sr.configure(
                             wraplength=max(120, e.width - 4)))

        # ── 节点面板（滚动）──
        self.panel = ScrollFrame(self.root, sizes=self.sizes, fonts=self.fonts)
        self.panel.pack(fill=tk.BOTH, expand=True,
                        padx=S["pad_md"], pady=(0, S["pad_md"]))
        self.rows: list[NodeRow] = []

        self.root.bind_all("<MouseWheel>", self._wheel)
        self.root.bind_all("<Button-4>", self._wheel)
        self.root.bind_all("<Button-5>", self._wheel)
        self.engine = EngineController(Logger(), config=self.config)
        self._viz_widgets: list = []
        self.root.after(33, self._viz_tick)
        tkvar = tk.BooleanVar
        self._autorun_var = tkvar(value=bool(self._cfg_get("auto_start", False)))
        boot = False
        try:
            from pvplatform.system import is_autostart
            boot = is_autostart()
        except Exception:
            pass
        self._boot_var = tkvar(value=bool(boot))
        self._running_ui = False
        self._setup_tray()
        self._setup_hotkey()
        self._refresh_hotkeys()
        # 全部控件上色完成后一次性显示——消除启动白闪；屏幕居中
        self.root.update_idletasks()
        sw = self.root.winfo_screenwidth()
        sh = self.root.winfo_screenheight()
        ww, hh = S["win_w"], S["win_h"]
        self.root.geometry(f"+{(sw - ww) // 2}+{(sh - hh) // 2}")
        # 启动时自动运行：不弹主窗，直接进托盘（对齐 legacy）
        if bool(self._cfg_get("auto_start", False)):
            self._shown = False
        else:
            self.root.deiconify()

    # ── 托盘（动作经队列转投主线程）──
    def _setup_tray(self):
        import os
        from collections import deque
        from .tray import create_tray
        self._tray_actions = deque()
        res = getattr(sys, "_MEIPASS", None) or os.path.dirname(
            os.path.dirname(os.path.abspath(__file__)))
        ic = os.path.join(res, "assets", "icons")
        self.tray = create_tray(
            os.path.join(ic, "tray_running.ico"),
            os.path.join(ic, "tray_stopped.ico"),
            on_toggle=lambda: self._tray_actions.append("toggle"),
            on_quit=lambda: self._tray_actions.append("quit"),
            on_start_stop=lambda: self._tray_actions.append("start_stop"))
        if self.tray:
            self.tray.set_state(False)
            self._poll_tray()

    def _poll_tray(self):
        while getattr(self, "_tray_actions", None):
            try:
                act = self._tray_actions.popleft()
            except IndexError:
                break
            if act == "toggle":
                self.toggle_window()
            elif act == "start_stop":
                self._on_start()
            elif act == "quit":
                self.quit_app()
                return
        self.root.after(120, self._poll_tray)

    def toggle_window(self):
        """显隐切换：overrideredirect 窗口的 state() 不可靠，用自维护标志。

        呼出即置顶（保持 topmost 直至收起）；隐藏只发生在点托盘图标或关窗
        （X）。"""
        shown = getattr(self, "_shown", True)
        if shown:
            self._hide_window()
        else:
            self.root.deiconify()
            self.root.lift()
            self.root.focus_force()
            self.root.attributes("-topmost", True)
            self._shown = True

    def _hide_window(self):
        self.root.withdraw()
        try:
            self.root.attributes("-topmost", False)
        except Exception:
            pass
        self._shown = False

    def _close_request(self):
        """关窗策略跟随真实托盘状态：图标确实存在才隐藏，否则直接退出。"""
        tray = getattr(self, "tray", None)
        if tray and getattr(tray, "alive", False):
            self._hide_window()
        else:
            self.quit_app()

    def quit_app(self):
        """完整退出：保存播放进度 → 停引擎 → 删托盘图标 → 关窗口。"""
        self._save_music_positions()
        self._persist()
        try:
            if getattr(self, "_hk_after", None) is not None:
                self.root.after_cancel(self._hk_after)
                self._hk_after = None
        except Exception:
            pass
        try:
            host = getattr(self, "_hotkeys", None)
            if host is not None:
                host.stop()
        except Exception:
            pass
        try:
            self.engine.stop()
        except Exception:
            pass
        try:
            if getattr(self, "tray", None):
                self.tray.remove()
        except Exception:
            pass
        try:
            self.root.destroy()
        except Exception:
            pass

    def _title_drag_begin(self, e):
        self._tdx, self._tdy = e.x, e.y

    def _title_drag_move(self, e):
        try:
            x = self.root.winfo_x() + e.x - self._tdx
            y = self.root.winfo_y() + e.y - self._tdy
            self.root.geometry(f"+{x}+{y}")
        except Exception:
            pass

    # ── 链 ↔ 配置 ──
    def load_chain(self, chain_cfg):
        from pvengine.plugins import get_spec
        self.clear_rows()
        for item in chain_cfg:
            t = str(item.get("type", ""))
            spec = get_spec(t)
            if spec is None:
                continue
            self._make_row(dict(item), spec)
        self._refresh_hotkeys()

    def to_config(self):
        return [dict(r.cfg) for r in self.rows]

    def _persist(self):
        if not self.config:
            return
        try:
            self.config.set("plugin_chain", self.to_config())
            self.config.save_config()
        except Exception:
            pass

    def _save_music_positions(self):
        """把音乐播放器当前进度固化进行参数（引擎重启/退出前调用，
        重启后经 resume_sec 原地续播，不再从头开始）。"""
        try:
            for r in self.rows:
                if getattr(r, "spec", None) is None or \
                        r.spec.name != "music_player":
                    continue
                idx = self.rows.index(r)
                st = self.engine.music_status(idx)
                if st.get("dur"):
                    r.cfg.setdefault("params", {})["resume_sec"] = round(
                        float(st.get("pos", 0.0)), 1)
        except Exception:
            pass

    def _refresh_sr_label(self):
        """刷新输入/输出采样率状态行：各端实际采样率 → 48k 自适应说明。

        未运行显示「输入：未启动」；网络输入模式显示推流来源。
        """
        if not self.engine.running:
            self.lbl_sr.configure(text="输入：未启动")
            return
        parts = []
        info = self.engine.input_info()
        if info.get("active"):
            sr, ch = int(info.get("dev_sr") or 48000), \
                int(info.get("dev_ch") or 1)
            if info.get("adaptive"):
                parts.append(f"输入：{ch}ch {sr}Hz → 48kHz（自适应重采样）")
            else:
                parts.append("输入：48kHz（直通）")
        else:
            try:
                net = self.engine.network_status()
            except Exception:
                net = None
            parts.append("输入：网络推流" if net else "输入：无设备输入")
        for o in self.engine.output_info():
            osr, och = int(o.get("dev_sr") or 48000), \
                int(o.get("ch") or 2)
            if o.get("adaptive"):
                parts.append(f"输出：{och}ch 48kHz → {osr}Hz（自适应重采样）")
            else:
                parts.append("输出：48kHz（直通）")
        for a in self.engine.aec_info():
            kind = "扬声器" if a.get("far_kind") == "speaker" else "麦克风"
            fsr = int(a.get("far_sr") or 0)
            if fsr and fsr != 48000:
                parts.append(f"AEC far（{kind}）：{fsr}Hz → 48kHz（自适应）")
            else:
                parts.append(f"AEC far（{kind}）：48kHz（直通）")
        self.lbl_sr.configure(text="\n".join(parts))
        try:
            self.engine.log.msg("[状态] " + " ｜ ".join(parts))
        except Exception:
            pass

    def _dialog(self, kind, title, message):
        """提示框：走自定义深色弹窗。主窗是 override-redirect，原生
        messagebox（受 WM 管理）会被排到主窗下面而看不见，故不用原生。"""
        from .dialogs import show_message
        show_message(self.root, title, message,
                     sizes=self.sizes, fonts=self.fonts)

    def _ask_open_file(self, title, filetypes):
        """原生文件选择框。主窗是 override-redirect 且常驻置顶，原生对话框
        会被压在主窗后面（表现为「点了没反应」）——先取消置顶、显式指定父窗，
        关闭后再恢复置顶。返回选中路径或 ""。"""
        from tkinter import filedialog
        was_top = False
        try:
            was_top = bool(self.root.attributes("-topmost"))
            self.root.attributes("-topmost", False)
        except Exception:
            pass
        try:
            return filedialog.askopenfilename(
                title=title, filetypes=filetypes, parent=self.root)
        finally:
            try:
                self.root.attributes("-topmost", was_top)
                self.root.lift()
            except Exception:
                pass

    def _apply_chain_change(self):
        """结构变更（增删/排序/开关）→ 持久化；运行中则热重建音频链。"""
        was_running = self.engine.running
        self._save_music_positions()
        self._persist()
        self._refresh_hotkeys()
        if not was_running:
            return
        self.engine.stop()
        err = self.engine.start(self.to_config())
        if err:
            self._dialog("showwarning", "PureVox", f"链已更新，但重启失败：\n{err}")
            self._set_running_ui(False)
        else:
            self._set_running_ui(True)
        self._refresh_sr_label()

    def _hot_toggle(self, row, on):
        """fx 行勾选热更：持久化 + 运行中处理器原地启停（不重启音频流）。"""
        self._persist()
        self.engine.set_plugin_enabled(self.rows.index(row), on)

    # ── 工具条行为 ──
    def _on_start(self):
        if self.engine.running:
            self._save_music_positions()
            try:
                self.engine.stop()
            except Exception:
                pass
            self._set_running_ui(False)
            self._refresh_sr_label()
            self.refresh_devices()      # 停止（无论成败）都刷新设备
            return
        err = self.engine.start(self.to_config())
        self.refresh_devices()          # 启动尝试后必刷新：空设备时插上设备点启动即可见
        if err:
            self._dialog("showwarning", "PureVox", err)
            self._set_running_ui(False)
            self._refresh_sr_label()
            return
        self._set_running_ui(True)
        self._refresh_sr_label()

    def _set_running_ui(self, running):
        bg = theme.STOP_BG if running else theme.START_BG
        text = "停止音频处理" if running else "启动音频处理"
        self.btn_start.set_bg(bg)
        self.btn_start.configure(text=text)
        # 托盘图标随运行态变色（蓝=运行中，红=已停止）
        tray = getattr(self, "tray", None)
        if tray:
            try:
                tray.set_state(bool(running))
            except Exception:
                pass
        prev = getattr(self, "_running_ui", False)
        self._running_ui = bool(running)
        if bool(running) != prev:
            self._play_cue("start" if running else "stop")

    def _play_cue(self, kind):
        """启停提示音：走系统默认输出设备（与音频处理输出无关）。

        停止提示音延后一拍再播：此时引擎刚释放输出设备，立刻播可能被设备
        重配/驱动拒播吞掉（表现为「偶尔听不到」）；播放本身在 uitk.cues 的
        后台线程完成，不阻塞 UI。
        """
        if not bool(self._cfg_get("cue_enabled", True)):
            return
        key = "cue_start" if kind == "start" else "cue_stop"
        pid = self._cfg_get(key, "soft")
        if not pid:
            return
        delay = 120 if kind == "stop" else 0
        try:
            self.root.after(delay, lambda: self._play_cue_now(pid, kind))
        except Exception:
            self._play_cue_now(pid, kind)

    def _play_cue_now(self, pid, kind):
        try:
            from .cues import play
            play(pid, kind)
        except Exception:
            pass

    # ── 设置菜单 ──
    def _cfg_get(self, key, default):
        return self.config.get(key, default) if self.config else default

    def _cfg_set(self, key, value):
        if self.config:
            self.config.set(key, value)
            self.config.save_config()

    def _gear_menu(self):
        m = tk.Menu(self.root, tearoff=0, bg=theme.BUTTON, fg=theme.TEXT,
                    activebackground=theme.DARK, activeforeground=theme.TEXT,
                    bd=0, font=self.fonts["body"])
        m.add_command(label="系统声音", command=self._open_sound_panel)
        if not sys.platform.startswith("win"):
            m.add_command(label="虚拟声卡", command=self._open_virtual_mic)
        m.add_command(label="快捷键与提示音", command=self._open_hotkey_settings)
        m.add_command(label="关于", command=self._show_about)
        m.add_separator()
        m.add_checkbutton(label="启动时自动运行",
                          onvalue=True, offvalue=False,
                          variable=self._autorun_var,
                          command=lambda: self._cfg_set(
                              "auto_start", bool(self._autorun_var.get())))
        m.add_checkbutton(label="开机自启",
                          onvalue=True, offvalue=False,
                          variable=self._boot_var,
                          command=self._toggle_boot)
        m.tk_popup(self.btn_gear.winfo_rootx(),
                   self.btn_gear.winfo_rooty() + self.btn_gear.winfo_height())

    def _open_sound_panel(self):
        try:
            from pvplatform.system import open_sound_panel
            open_sound_panel(Logger())
        except Exception:
            pass

    def _open_virtual_mic(self):
        try:
            from pvplatform.system import ensure_virtual_mic, remove_virtual_mic, virtual_mic_ready
            ready = virtual_mic_ready()
            if ready:
                remove_virtual_mic(Logger())
            else:
                ensure_virtual_mic(Logger())
            self._dialog("showinfo", "虚拟声卡",
                         "已创建，请重启音频处理生效。" if not ready else "已清理。")
        except Exception as e:
            self._dialog("showwarning", "虚拟声卡", str(e))

    def _show_about(self):
        from .dialogs import show_about_dialog
        show_about_dialog(self.root, sizes=self.sizes, fonts=self.fonts)

    def _open_eq_editor(self, row):
        """EQ 曲线编辑器（按行规格选栅格）；增益/高低切存该行节点 params。"""
        from .dialogs import open_eq_editor
        from pvengine.components.eq import EQ_VARIANTS
        freqs, q = EQ_VARIANTS[row.spec.name]

        def set_gains(g):
            self._on_param(row, "gains", [float(x) for x in g])

        def set_filters(hp_on, hp_hz, lp_on, lp_hz):
            self._on_param(row, "hp_enabled", bool(hp_on))
            self._on_param(row, "hp_hz", float(hp_hz))
            self._on_param(row, "lp_enabled", bool(lp_on))
            self._on_param(row, "lp_hz", float(lp_hz))

        p = row.cfg.setdefault("params", {})
        open_eq_editor(
            self.root, freqs, q,
            lambda: list(p.get("gains") or [0.0] * len(freqs)),
            set_gains,
            get_filters=lambda: (bool(p.get("hp_enabled", False)),
                                 float(p.get("hp_hz", 80.0)),
                                 bool(p.get("lp_enabled", False)),
                                 float(p.get("lp_hz", 16000.0))),
            set_filters=set_filters,
            sizes=self.sizes, fonts=self.fonts)

    def _open_tse_dialog(self):
        from .dialogs import open_tse_dialog
        open_tse_dialog(self.root, self.engine, self.config,
                        sizes=self.sizes, fonts=self.fonts)

    # ── 全局热键：恒定 action 表（启停 / 各音效），可录制、可置空 ──
    DEFAULT_HOTKEY = "Alt+."

    def _setup_hotkey(self):
        from collections import deque
        self._hk_actions = deque()   # 热键线程 → 主线程动作队列
        try:
            from .hotkeys import GlobalHotkeys
            self._hotkeys = GlobalHotkeys(self._on_hotkey)
        except Exception:
            self._hotkeys = None
        self._hk_after = self.root.after(60, self._poll_hotkeys)

    def _soundpad_row(self):
        for r in self.rows:
            if getattr(r, "spec", None) is not None \
                    and r.spec.name == "soundpad":
                return r
        return None

    def _soundpad_pads(self):
        row = self._soundpad_row()
        if row is None:
            return []
        return list((row.cfg.get("params") or {}).get("pads") or [])

    def _discard_library_file(self, path):
        """移除音效后丢弃其库内 WAV。

        两道护栏：① 仅删 ~/.purevox/soundpad/ 内的文件（外部路径永不碰）；
        ② 仍被任意音效板节点的某个音效引用时保留（同一文件可被多次添加，
        导入按内容 hash 命名故路径相同）。"""
        p = str(path or "")
        if not p:
            return
        try:
            from user_paths import SOUNDPAD_DIR
            root = os.path.abspath(SOUNDPAD_DIR)
            target = os.path.abspath(p)
            if os.path.commonpath([root, target]) != root:
                return
        except Exception:
            return
        for r in self.rows:
            if getattr(r, "spec", None) is None or r.spec.name != "soundpad":
                continue
            for info in ((r.cfg.get("params") or {}).get("pads") or []):
                if str(info.get("path") or "") == p:
                    return
        try:
            os.remove(target)
        except OSError:
            pass

    def _refresh_hotkeys(self):
        """重注册全部全局热键：启停一个 + 各音效（空串=不监听）。"""
        host = getattr(self, "_hotkeys", None)
        if host is None:
            return
        bindings = []
        if bool(self._cfg_get("hotkey_toggle_on", True)):
            bindings.append(("toggle", self._cfg_get(
                "hotkey_toggle", self.DEFAULT_HOTKEY)))
        for i, p in enumerate(self._soundpad_pads()):
            spec = str(p.get("hotkey") or "")
            if spec and bool(p.get("hotkey_on", True)):
                bindings.append(("pad:%d" % i, spec))
        host.set_bindings(bindings)
        self._warn_hotkey_conflict(list(host.last_failed))

    def _warn_hotkey_conflict(self, specs):
        """热键被系统/他程序占用时提示一次（同一组不重复弹）。"""
        specs = [s for s in specs if s]
        if not specs or specs == getattr(self, "_hotkey_warned", None):
            return
        self._hotkey_warned = specs
        msg = ("以下快捷键已被系统或其他程序占用，未能生效：\n"
               + "、".join(specs)
               + "\n\n请在「设置 → 快捷键与提示音」中换一组。")
        try:
            self.root.after(0, lambda: self._dialog(
                "showwarning", "PureVox", msg))
        except Exception:
            pass

    def _on_hotkey(self, action):
        """热键线程回调：只入队，绝不碰 Tk。

        跨线程 root.after() 不保证被调度（实测静默丢失），统一走
        「线程安全队列 + 主线程轮询」这条既有通路（与托盘动作一致）。
        """
        try:
            self._hk_actions.append(action)
        except Exception:
            pass

    def _poll_hotkeys(self):
        while getattr(self, "_hk_actions", None):
            try:
                action = self._hk_actions.popleft()
            except IndexError:
                break
            self._dispatch_hotkey(action)
        self._hk_after = self.root.after(60, self._poll_hotkeys)

    def _dispatch_hotkey(self, action):
        if action == "toggle":
            self._on_start()
        elif str(action).startswith("pad:"):
            try:
                self.engine.soundpad_play(int(str(action)[4:]))
            except Exception:
                pass

    def _open_hotkey_settings(self):
        from .dialogs import open_hotkey_dialog
        open_hotkey_dialog(
            self.root,
            get_toggle=lambda: self._cfg_get("hotkey_toggle",
                                             self.DEFAULT_HOTKEY),
            set_toggle=self._set_toggle_hotkey,
            get_toggle_enabled=lambda: self._cfg_get("hotkey_toggle_on", True),
            set_toggle_enabled=self._set_toggle_hotkey_on,
            get_cue=lambda kind: self._cfg_get(
                "cue_start" if kind == "start" else "cue_stop", "soft"),
            set_cue=self._set_cue,
            get_cue_enabled=lambda: self._cfg_get("cue_enabled", True),
            set_cue_enabled=self._set_cue_enabled,
            sizes=self.sizes, fonts=self.fonts)

    def _set_toggle_hotkey(self, spec):
        self._cfg_set("hotkey_toggle", spec or "")
        self._refresh_hotkeys()

    def _set_toggle_hotkey_on(self, on):
        self._cfg_set("hotkey_toggle_on", bool(on))
        self._refresh_hotkeys()

    def _set_cue(self, kind, pid):
        self._cfg_set("cue_start" if kind == "start" else "cue_stop",
                      pid or "")

    def _set_cue_enabled(self, on):
        self._cfg_set("cue_enabled", bool(on))

    def _toggle_boot(self):
        val = bool(self._boot_var.get())
        self._cfg_set("registry_auto_start", val)
        try:
            from pvplatform.system import enable_autostart, disable_autostart
            (enable_autostart if val else disable_autostart)(Logger())
        except Exception:
            pass

    def _add_menu(self):
        from pvengine.plugins import get_spec, all_specs
        m = tk.Menu(self.root, tearoff=0, bg=theme.BUTTON, fg=theme.TEXT,
                    activebackground=theme.DARK, activeforeground=theme.TEXT,
                    bd=0, font=self.fonts["body"])
        # 设备组：输入/输出即全部设备（Linux 输出含 purevox_out，Windows 含
        # CABLE Input；VB 检测卡内嵌在输出行），网络输入 + AEC + 桌面输入
        dev = tk.Menu(m, tearoff=0, bg=theme.BUTTON, fg=theme.TEXT,
                      activebackground=theme.DARK,
                      activeforeground=theme.TEXT, bd=0,
                      font=self.fonts["body"])
        # 名称单一来源 = 节点 spec.label（与行标题一致，避免两处漂移）
        for nm in ("audio_input", "remote_mic", "loopback", "echo_cancel",
                   "audio_output"):
            sp = get_spec(nm)
            if sp is not None:
                dev.add_command(label=sp.label,
                                command=lambda s=sp: self.add_spec(s))
        m.add_cascade(label="设备", menu=dev)
        # 媒体输入分类：设备外音源（相互独立的插件节点）
        media = tk.Menu(m, tearoff=0, bg=theme.BUTTON, fg=theme.TEXT,
                        activebackground=theme.DARK,
                        activeforeground=theme.TEXT, bd=0,
                        font=self.fonts["body"])
        media.add_command(label="音效板",
                          command=lambda: self.add_spec(get_spec("soundpad")))
        media.add_command(label="音乐播放器",
                          command=lambda: self.add_spec(get_spec("music_player")))
        media.add_command(label="桌面声音输入",
                          command=lambda: self.add_spec(get_spec("desktop_audio")))
        m.add_cascade(label="媒体输入", menu=media)
        # 处理 / 可视化（媒体输入已独立分类，不在处理清单重复出现）
        for kind in ("fx", "viz"):
            specs = [s for s in all_specs()
                     if s.name not in ("soundpad", "music_player",
                                       "desktop_audio")]
            m.add_cascade(label=KIND_LABELS[kind],
                          menu=self._kind_menu(m, kind, specs))
        m.tk_popup(self.btn_add.winfo_rootx(),
                   self.btn_add.winfo_rooty() + self.btn_add.winfo_height())

    def _kind_menu(self, parent, kind, specs):
        m = tk.Menu(parent, tearoff=0, bg=theme.BUTTON, fg=theme.TEXT,
                    activebackground=theme.DARK, activeforeground=theme.TEXT,
                    bd=0, font=self.fonts["body"])
        for sp in specs:
            if sp.kind == kind:
                m.add_command(label=sp.label,
                              command=lambda s=sp: self.add_spec(s))
        return m

    # ── 行管理 ──
    def add_spec(self, spec):
        params = {k: pdef[3] for k, pdef in (spec.params or {}).items()}
        self._make_row({"type": spec.name, "enabled": True, "params": params},
                       spec)
        self._apply_chain_change()
        # 新建设备节点即刷新设备列表，避免下拉为空只能看到「（默认）」
        if spec.name in DEV_KEY and DEV_KEY[spec.name]:
            self.refresh_devices()

    def _make_row(self, cfg, spec):
        row = NodeRow(self.panel, cfg, spec, self.sizes, self.fonts,
                      on_remove=lambda: self.remove_row(row),
                      on_drag_preview=self._move_row_live,
                      on_drag_commit=self._apply_chain_change,
                      on_param=lambda r, k, v: self._on_param(r, k, v))
        row.set_toggle_cb(self._apply_chain_change)
        row.set_hot_toggle_cb(self._hot_toggle)
        row._on_param_cb = self._apply_chain_change
        row._apply_enabled_look()
        # AGC 行：实时增益值显示标签在 _build_inline 中创建
        # echo_cancel 行：延迟滑杆 + 自动校准回调
        if spec.name == "echo_cancel":
            row._on_aec_delay_cb = self._on_aec_delay_change
            row._on_aec_auto_cb = lambda r=row: self._on_aec_auto_calibrate(r)
        # viz 行：内嵌实时控件（无论是否勾选都挂载，未勾选时
        # _viz_tick 跳过喂数——否则未勾选启动只剩标题）
        if spec.kind == "viz":
            self._attach_viz(row, spec.name)
        # eq 行（三种规格）：标题后提供曲线编辑按钮
        if spec.name in ("eq10", "eq31", "eq61"):
            FlatButton(row.mid, "均衡器编辑",
                       command=lambda r=row: self._open_eq_editor(r),
                       font=self.fonts.get("body"), sizes=self.sizes,
                       pad=self.sizes["pad_sm"]).pack(side=tk.LEFT)
        # tse 行：标题后提供参考录音按钮
        if spec.name == "tse":
            FlatButton(row.mid, "参考录音",
                       command=self._open_tse_dialog,
                       font=self.fonts.get("body"), sizes=self.sizes,
                       pad=self.sizes["pad_sm"]).pack(side=tk.LEFT)
        # 音效板行：音效条目（播放/停止/音量/移除）+ 添加音效
        if spec.name == "soundpad":
            self._attach_soundpad(row)
        # 音乐播放器行：曲目选择 + 播放控制
        if spec.name == "music_player":
            self._attach_music_player(row)
        # 桌面声音输入行：loopback 说明（音量滑杆自动生成）
        if spec.name == "desktop_audio":
            self._attach_desktop_audio(row)
        # 网络输入行：推流地址（手机/浏览器访问）+ 说明
        if spec.name == "remote_mic":
            self._attach_remote_mic(row)
        # 输出设备行：Windows 内嵌 VB-CABLE 状态卡；Linux 输出选中虚拟麦克风
        # （purevox_out）时显示虚拟声卡创建/清理卡
        if spec.name == "audio_output":
            if sys.platform.startswith("linux"):
                self._attach_linux_virtual_mic(row)
            else:
                self._attach_vb_card(row)
        # 全部行内内容就绪后统一显示参数区（无展开收起）
        row.ensure_body()
        self._pack_row(row)
        self.rows.append(row)

    def _attach_soundpad(self, row):
        """音效板行内：每个音效一行 = 播放/停止 + 名称 + 快捷键 + 音量 + 移除。

        快捷键在该音效后面直接录制（Delete 清除，空 = 不监听）；音量也是每个
        音效一个（±10dB，文件本身已够响），拖动即时生效。
        """
        from .widgets import (HSlider, HotkeyField, FlatButton, DarkCheck,
                              SquareButton)
        S, F = self.sizes, self.fonts
        row._body_padx = 0          # 音效行左右顶满节点边缘，不留白
        holder = tk.Frame(row.body_frame, bg=theme.PANEL)
        holder.pack(fill=tk.X, padx=0, pady=(0, S["pad_sm"]))

        def pads():
            # 返回「活」列表（非副本）：增删直接生效，commit 读到的即改后的
            p = row.cfg.setdefault("params", {})
            lst = p.get("pads")
            if not isinstance(lst, list):
                lst = []
                p["pads"] = lst
            return lst

        def commit():
            self._on_param(row, "pads", pads())
            self._refresh_hotkeys()

        def db_text(db):
            d = int(round(float(db)))
            return "0" if d == 0 else ("+%d" % d if d > 0 else str(d))

        # 复选框缩进 = 标题行拖拽手柄的宽度（+同款间距），使其与标题的复选框
        # 上下对齐（标题里复选框前面是手柄，这里是留白）
        try:
            indent = row.grip.winfo_reqwidth() + S["pad_sm"]
        except Exception:
            indent = S["pad_md"] * 2

        def pad_row(idx, info):
            # 顺序：开关（最前，快捷键=整个启用的总控）· 名称（点按即播放）·
            #       快捷键 · 音量（自适应占满剩余）· × 删除（行尾成列）
            # 统一行项色（略暗于插件面板）；行间留 1px 缝隙露出面板色作分隔，
            # 不再用斑马纹（两行同色，靠缝隙区隔）
            row_bg = theme.ROW_ITEM
            r = tk.Frame(holder, bg=row_bg)
            r.pack(fill=tk.X, pady=1)
            hk_var = tk.BooleanVar(value=bool(info.get("hotkey_on", True)))
            DarkCheck(r, "", hk_var,
                      command=lambda i=idx, v=hk_var: _set_hk_on(i, v),
                      sizes=S, fonts=F).pack(side=tk.LEFT,
                                             padx=(indent, S["pad_sm"]))
            name = tk.Label(r, text=str(info.get("name") or "未命名"),
                            bg=row_bg, fg=theme.TEXT, anchor="w",
                            width=10, font=F.get("body"), cursor="hand2")
            name.pack(side=tk.LEFT, padx=(0, S["pad_sm"]))
            name.bind("<Button-1>",
                      lambda e, i=idx: self.engine.soundpad_play(i))
            # 快捷键：点方框即录制，Delete 清除（空 = 不监听）
            HotkeyField(r, spec=str(info.get("hotkey") or ""),
                        command=lambda spec, i=idx: _set_hk(i, spec),
                        sizes=S, fonts=F, width=9,
                        show_clear=False).pack(
                side=tk.LEFT, padx=(0, S["pad_sm"]))
            SquareButton(r, "×", command=lambda i=idx: _remove(i),
                         bg=theme.TITLE_BG, fg=theme.TITLE_FG,
                         font=F.get("bold"), sizes=S).pack(
                side=tk.RIGHT, padx=(S["pad_sm"], 0))
            cur = info.get("volume_db", 0.0)
            vlab = tk.Label(r, text=db_text(cur), bg=row_bg,
                            fg=theme.TEXT_DIM, width=3, anchor="e",
                            font=F.get("small"))
            vlab.pack(side=tk.RIGHT, padx=(2, S["pad_sm"]))
            ref = {}
            sl = HSlider(r, -10.0, 10.0, cur, 1.0,
                         command=lambda: _set_vol(idx, ref["sl"], vlab),
                         sizes=S, width_px=48)
            ref["sl"] = sl
            sl.pack(side=tk.LEFT, fill=tk.X, expand=True)

        def _set_hk(idx, spec):
            ps = pads()
            if 0 <= idx < len(ps):
                ps[idx]["hotkey"] = str(spec or "")
                self._persist()
                self._refresh_hotkeys()

        def _set_hk_on(idx, var):
            ps = pads()
            if 0 <= idx < len(ps):
                ps[idx]["hotkey_on"] = bool(var.get())
                self._persist()
                self._refresh_hotkeys()

        def _set_vol(idx, sl, vlab):
            ps = pads()
            if not (0 <= idx < len(ps)):
                return
            db = round(float(sl.value), 0)
            ps[idx]["volume_db"] = float(db)
            vlab.configure(text=db_text(db))
            self._on_param(row, "pads", ps)

        def _remove(idx):
            ps = pads()
            if not (0 <= idx < len(ps)):
                return
            self.engine.soundpad_stop(idx)
            gone = ps.pop(idx)
            commit()
            render()
            # 移除即丢弃库内 WAV（仅当已无其他音效引用同一文件）
            self._discard_library_file(gone.get("path"))

        def _add():
            path = self._ask_open_file(
                "添加音效",
                [("音频/容器", "*.wav *.mp3 *.flac *.ogg *.m4a *.mp4 "
                              "*.aac *.opus *.wma *.mov *.webm *.mkv"),
                 ("全部文件", "*.*")])
            if not path:
                return
            # 入库：一律归一为 48k/mono WAV 存入 ~/.purevox/soundpad/，
            # 名称取原文件名，配置只引用库内路径（不依赖外部原始文件）
            name = os.path.splitext(os.path.basename(path))[0]
            try:
                from user_paths import SOUNDPAD_DIR
                from pvengine.components.audio_decode import import_media
                real = import_media(path, SOUNDPAD_DIR, stem=name)
            except Exception as e:
                self._dialog("showwarning", "PureVox", f"该文件无法解码：\n{e}")
                return
            # 每个音效一整套：名称 / 库内路径 / 快捷键开关 / 快捷键 / 音量
            pads().append({"name": name, "path": real, "hotkey_on": True,
                           "hotkey": "", "volume_db": 0.0})
            commit()
            render()

        # 「添加音效」放在标题之后（行头 mid 区，紧贴标题）
        FlatButton(row.mid, "添加音效", command=_add,
                   font=F.get("body"), sizes=S,
                   pad=S["pad_sm"]).pack(side=tk.LEFT)

        def render():
            for w in holder.winfo_children():
                w.destroy()
            for i, info in enumerate(pads()):
                pad_row(i, info)

        render()

    def _attach_desktop_audio(self, row):
        """桌面声音输入行内说明（音量滑杆自动生成；捕获随引擎启停）。"""
        hint = tk.Label(row.body_frame,
                        text="捕获默认输出设备的系统混音（loopback），"
                             "音量滑杆实时生效；随引擎启停自动开关。",
                        bg=theme.PANEL, fg=theme.TEXT_DIM,
                        font=self.fonts.get("small"), anchor="w",
                        justify="left")
        hint.pack(fill=tk.X, padx=self.sizes["pad_lg"],
                  pady=self.sizes["pad_sm"])

    def _server_port(self) -> int:
        try:
            if self.config:
                return int(self.config.get("server_port", 59123) or 59123)
        except Exception:
            pass
        return 59123

    def _attach_remote_mic(self, row):
        """网络输入行内：网卡（IP）选择 + 推流页二维码 + 服务状态。

        推流地址由所选网卡 IP + 服务器端口推导（写回 params.url：既是启动前提，
        也是二维码内容）；状态轮询引擎的活跃客户端数。二维码渲染与网卡枚举
        复用主线共享模块（qr_tk / pvplatform.netinfo），与 Lite Net 页同源。
        """
        from pvplatform import netinfo
        S, F = self.sizes, self.fonts
        params = row.cfg.setdefault("params", {})
        port = self._server_port()
        networks = netinfo.list_lan_ips()
        ip = str(params.get("net_ip", "") or "")
        if not any(i == ip for i, _n in networks):
            ip = netinfo.best_lan_ip(networks) if networks else "127.0.0.1"
        params["net_ip"] = ip
        params["url"] = f"https://{ip}:{port}"
        self._persist()

        holder = tk.Frame(row.body_frame, bg=theme.PANEL)
        holder.pack(fill=tk.X, padx=S["pad_lg"], pady=(0, S["pad_sm"]))

        # 右侧：二维码（手机/浏览器扫码直达推流页）
        row._net_url = params["url"]
        row._net_qr_lbl = tk.Label(holder, bg=theme.BASE, bd=0)
        row._net_qr_lbl.pack(side=tk.RIGHT, padx=(S["pad_md"], 0))
        self._refresh_net_qr(row)

        # 左侧信息区：网络下拉 + 服务状态 + 说明（说明在二维码左侧，不占二维码下方）
        info = tk.Frame(holder, bg=theme.PANEL)
        info.pack(side=tk.LEFT, fill=tk.BOTH, expand=True)
        row1 = tk.Frame(info, bg=theme.PANEL)
        row1.pack(fill=tk.X)
        tk.Label(row1, text="网络", bg=theme.PANEL, fg=theme.TEXT_DIM,
                 font=F.get("small")).pack(side=tk.LEFT, padx=(0, S["pad_sm"]))
        pairs = [(f"[{name}] {i}", i) for i, name in networks] \
            or [("127.0.0.1", "127.0.0.1")]
        var = tk.StringVar(value=ip)
        DarkCombo(row1, pairs, var, on_change=lambda: self._on_net_ip(row, var),
                  sizes=S, fonts=F).pack(side=tk.LEFT, fill=tk.X, expand=True)
        row._net_status_lbl = tk.Label(
            info, text="服务未启动（启动后显示状态）", bg=theme.PANEL,
            fg=theme.TEXT_FAINT, font=F.get("small"), anchor="w", justify="left")
        row._net_status_lbl.pack(fill=tk.X, pady=(S["pad_sm"], 0))
        hint = tk.Label(
            info,
            text="手机/浏览器扫码或访问该地址推流（HTTPS，首次需信任自签证书）；"
                 "实际监听端口 = 设置中的服务器端口。",
            bg=theme.PANEL, fg=theme.TEXT_FAINT, font=F.get("small"),
            anchor="w", justify="left")
        hint.pack(fill=tk.X, pady=(S["pad_sm"], 0))
        info.bind("<Configure>",
                  lambda e: hint.configure(wraplength=max(80, e.width - 4)))

        self._start_net_status_poll()

    def _on_net_ip(self, row, var):
        """切换网卡：地址随之改写（二维码同步）+ 引擎切网（证书/mDNS）。"""
        ip = str(var.get() or "")
        params = row.cfg.setdefault("params", {})
        params["net_ip"] = ip
        params["url"] = f"https://{ip}:{self._server_port()}"
        row._net_url = params["url"]
        self._persist()
        self._refresh_net_qr(row)
        self.engine.apply_network(ip)

    def _refresh_net_qr(self, row):
        from qr_tk import make_qr_photo, qr_unavailable_reason
        lbl = getattr(row, "_net_qr_lbl", None)
        if lbl is None:
            return
        target = max(112, int(self.sizes["combo_h"] * 4))
        photo = make_qr_photo(lbl, getattr(row, "_net_url", ""), target_px=target)
        if photo is None:
            lbl.configure(image="",
                          text=f"二维码\n不可用（{qr_unavailable_reason()}）",
                          fg=theme.TEXT_FAINT, font=self.fonts.get("small"))
            return
        lbl._qr_photo = photo
        lbl.configure(image=photo, text="")

    def _start_net_status_poll(self):
        """网络输入行状态轮询（单轮定时器，全局仅一个）。"""
        if getattr(self, "_net_poll_on", False):
            return
        self._net_poll_on = True

        def _tick():
            try:
                st = self.engine.network_status()
            except Exception:
                st = None
            running = bool(getattr(self.engine, "running", False))
            for r in list(getattr(self, "rows", []) or []):
                lbl = getattr(r, "_net_status_lbl", None)
                if lbl is None:
                    continue
                if not running:
                    lbl.configure(text="服务未启动（启动后显示状态）")
                elif st is None:
                    lbl.configure(text="服务器启动中…")
                else:
                    lbl.configure(
                        text=f"端口 {st['port']} · 客户端 {st['clients']} 个")
            self.root.after(1000, _tick)

        self.root.after(1000, _tick)

    def _attach_music_player(self, row):
        """音乐播放器行内控制：选择曲目 + 播放/暂停 + 进度滑块（可拖 seek）。

        播放位置持久化：拖动/暂停/停止/退出即写 `resume_sec`，播放中每 5 秒
        兜底写入一次，重启后从该位置续播。"""
        S, F = self.sizes, self.fonts
        holder = tk.Frame(row.body_frame, bg=theme.PANEL)
        holder.pack(fill=tk.X, padx=S["pad_lg"], pady=(0, S["pad_sm"]))
        state = {"dragging": False, "dur": 0.0, "after": None,
                 "was_playing": False}
        name_lbl = tk.Label(holder, text="（未选择曲目）", bg=theme.PANEL,
                            fg=theme.TEXT, anchor="w", font=F.get("body"))
        name_lbl.pack(fill=tk.X, pady=(0, 2))
        bar = tk.Frame(holder, bg=theme.PANEL)
        bar.pack(fill=tk.X)

        def _idx():
            return self.rows.index(row)

        def _set(key, value):
            self._on_param(row, key, value)

        def _fmt(sec):
            sec = int(sec or 0)
            return f"{sec // 60:02d}:{sec % 60:02d}"

        def _save_resume(pos_sec):
            row.cfg.setdefault("params", {})["resume_sec"] = round(
                float(pos_sec), 1)
            self._persist()

        def _play():
            if str((row.cfg.get("params") or {}).get("path", "")):
                self.engine.music_play(_idx())

        def _pause():
            self.engine.music_pause(_idx())
            _save_resume(self.engine.music_status(_idx()).get("pos", 0.0))

        def _pick():
            path = self._ask_open_file(
                "选择音乐/媒体文件",
                [("音频/容器", "*.mp3 *.flac *.ogg *.wav *.m4a *.mp4 "
                              "*.aac *.opus *.wma *.mov *.webm *.mkv"),
                 ("全部文件", "*.*")])
            if not path:
                return
            # 入库：一律归一为 48k/mono WAV 存入 ~/.purevox/music/，
            # 配置只引用库内路径（不依赖外部原始文件）
            name_lbl.configure(text="导入中…")
            holder.update_idletasks()
            try:
                from user_paths import MUSIC_DIR
                from pvengine.components.audio_decode import import_media
                path = import_media(path, MUSIC_DIR)
            except Exception as e:
                refresh_name()
                self._dialog("showwarning", "PureVox", f"该文件无法解码：\n{e}")
                return
            _set("path", path)
            _set("resume_sec", 0.0)
            state["dur"] = 0.0
            refresh_name()

        from .widgets import FlatButton
        FlatButton(bar, "选择曲目", command=_pick, font=F.get("body"),
                   sizes=S, pad=S["pad_sm"]).pack(side=tk.LEFT,
                                                 padx=(0, S["pad_sm"]))
        FlatButton(bar, "播放", command=_play, font=F.get("body"),
                   sizes=S, pad=S["pad_sm"]).pack(side=tk.LEFT,
                                                 padx=(0, S["pad_sm"]))
        FlatButton(bar, "暂停", command=_pause, font=F.get("body"),
                   sizes=S, pad=S["pad_sm"]).pack(side=tk.LEFT,
                                                 padx=(0, S["pad_sm"]))
        time_lbl = tk.Label(bar, text="00:00 / 00:00", bg=theme.PANEL,
                            fg=theme.TEXT_DIM, font=F.get("small"))
        time_lbl.pack(side=tk.RIGHT)
        # 进度滑块（与进度条一体）：拖动=定位，回显=播放位置
        from .widgets import HSlider
        seek_holder = tk.Frame(holder, bg=theme.PANEL)
        seek_holder.pack(fill=tk.X)
        seek = {"slider": None}

        def _rebuild_slider(dur):
            for w in seek_holder.winfo_children():
                w.destroy()
            s = HSlider(seek_holder, 0, max(1.0, dur), 0.0, 1.0,
                        sizes=S, width_px=S["win_w"] - S["pad_lg"] * 4)
            s.pack(fill=tk.X)
            s.bind("<Button-1>", lambda e: state.update(dragging=True),
                   add="+")
            s.bind("<B1-Motion>", lambda e: state.update(dragging=True),
                   add="+")
            s.bind("<ButtonRelease-1>", lambda e: (
                state.update(dragging=False), _seek_to(s.value)), add="+")
            seek["slider"] = s

        def _seek_to(sec):
            # seek 带参：走 set_live_param 结构化钩子；进度同源持久化
            self.engine.set_live_param(_idx(), "seek_sec", float(sec))
            _save_resume(sec)

        def refresh_name():
            path = str((row.cfg.get("params") or {}).get("path", ""))
            name_lbl.configure(
                text=(os.path.basename(path) if path else "（未选择曲目）"))

        def _tick():
            try:
                if not holder.winfo_exists():
                    return
            except Exception:
                return
            st = self.engine.music_status(_idx())
            dur = float(st.get("dur") or 0.0)
            pos = float(st.get("pos") or 0.0)
            playing = bool(st.get("playing"))
            if abs(dur - state["dur"]) > 0.5:
                state["dur"] = dur
                _rebuild_slider(dur)
            s = seek["slider"]
            if s is not None and not state["dragging"] and dur > 0:
                s.set_value(pos, silent=True)
            time_lbl.configure(text=f"{_fmt(pos)} / {_fmt(dur)}")
            # 播放中每 5 秒兜底落盘一次进度（崩溃/强退也不丢太多）
            now = time.time()
            if playing and now - float(state.get("saved_at") or 0.0) >= 5.0:
                state["saved_at"] = now
                _save_resume(pos)
            # 播放→停止（复选框关/暂停/引擎停）的状态沿：触发进度持久化
            if state["was_playing"] and not playing:
                _save_resume(pos)
            state["was_playing"] = playing
            state["after"] = holder.after(400, _tick)

        def _stop_tick():
            if state["after"] is not None:
                try:
                    holder.after_cancel(state["after"])
                except Exception:
                    pass

        # 初始 resume_sec → 首次 play 自动续播（插件侧处理），UI 仅回显
        refresh_name()
        _rebuild_slider(1.0)
        state["after"] = holder.after(400, _tick)
        holder.bind("<Destroy>",
                    lambda e: _stop_tick() if e.widget is holder else None)

    def _attach_viz(self, row, name):
        row._body_padx = 0        # 画布顶满卡片左右
        row._body_pady = 0        # 且顶满卡片上下（就卡片而言的撑满）
        if name == "vu_meter":
            w = VUCanvas(row.body_frame, sizes=self.sizes, height=26)
            w.pack(fill=tk.X, pady=0)
        elif name == "spectrum":
            # 紧凑高度（数据/段数不变，只减纵向占用）
            w = SpectrumCanvas(row.body_frame, sizes=self.sizes)
            w.pack(fill=tk.BOTH, expand=True, pady=0)
        else:
            return
        # 位置抽头序号 = 本行之前【启用】的 viz 行数
        # （set_plugins 只为启用行建抽头；禁用行不占序号）
        def _ordinal(r=row):
            return sum(1 for x in self.rows
                       if x.spec.kind == "viz"
                       and x.cfg.get("enabled", True)
                       and self.rows.index(x) < self.rows.index(r))
        self._viz_widgets.append((row, name, w, _ordinal))
        row.title_lbl.unbind("<Double-Button-1>")

    def _on_aec_delay_change(self, ms):
        """延迟滑杆：运行中实时生效，并持久化（重启后沿用）。"""
        thread = self.engine.thread if self.engine.running else None
        if thread:
            for r in self.rows:
                if r.spec.name == "echo_cancel" and hasattr(r, "_aec_delay_slider") \
                        and r._aec_delay_slider is not None:
                    mic = str((r.cfg.get("params") or {}).get("device", ""))
                    if mic:
                        thread.set_aec_delay_ms(mic, ms)
        self._persist()

    def _on_aec_auto_calibrate(self, row):
        """自动校准（离线）：须先关闭音频处理，安静环境下播放脉冲测一次。

        硬件不变时测得值基本恒定，之后一直使用该 far_delay；校准失败
        （返回 None）时保留原值并提示，不把用户手调值清 0。
        """
        if self.engine.running:
            self._dialog("showinfo", "校准",
                         "请先关闭音频处理（并保持环境安静），再点击自动校准。")
            return
        mic = str((row.cfg.get("params") or {}).get("device", ""))
        far_dev = str((row.cfg.get("params") or {}).get("far_device", ""))
        far_kind = str((row.cfg.get("params") or {}).get("far_kind", "speaker"))
        if not mic or not far_dev:
            return
        btn = getattr(row, "_aec_auto_btn", None)
        if btn:
            btn.config(state=tk.DISABLED, text="校准中…")
        def _run():
            try:
                delay_ms = self.engine.calibrate_aec_delay(mic, far_dev, far_kind)
            except Exception:
                delay_ms = None
            def _update():
                if delay_ms is None:
                    self._dialog("showwarning",
                                 "校准", "延迟校准失败，已保留原值。\n"
                                 "请保持环境安静并确认麦克风能听到扬声器测试音后重试。")
                elif hasattr(row, "_aec_delay_slider") and row._aec_delay_slider:
                    row._aec_delay_slider.set_value(delay_ms)
                    row._aec_delay_lbl.config(text=f"{delay_ms:.0f}ms")
                    row.cfg.setdefault("params", {})["far_delay_ms"] = delay_ms
                    self._persist()
                if btn:
                    btn.config(state=tk.NORMAL, text="校准")
            self.root.after(0, _update)
        threading.Thread(target=_run, daemon=True).start()

    def _viz_tick(self):
        """33ms 定时喂 viz。

        线性组件语义：每个 viz 行从**自己的位置抽头**取数
        （processor._viz_taps[ordinal]，抽到的是链中该点之前的全部处理
        结果）；VU 峰值从同一份抽头样本现算。无全局第二检查点。
        """
        proc = self.engine.processor if self.engine.running else None
        now = time.time()
        for row, name, w, ordinal_fn in self._viz_widgets:
            if not row.winfo_exists() or not row.on_var.get():
                continue
            try:
                data = proc.take_viz_tap(ordinal_fn()) if proc else []
                if name == "vu_meter":
                    if data:
                        # 空抽头 = 本轮无新音频（拉模型突发到达），保持
                        # 当前电平不归零——归零只属于引擎停止
                        w.update_level(max(abs(x) for x in data), now)
                    elif not proc:
                        w.update_level(0.0, now)
                elif name == "spectrum" and data:
                    w.update_spectrum(None, data)
            except Exception:
                pass
        # ── AGC 增益值实时更新（10fps，降 CPU）──
        if now >= getattr(self, "_agc_update_next", 0.0):
            self._agc_update_next = now + 0.1
            if proc:
                try:
                    agc = proc._find("agc")
                except Exception:
                    agc = None
                for r in self.rows:
                    if r.spec.name != "agc" or not r.on_var.get():
                        continue
                    gain_lbl = getattr(r, "_agc_gain_lbl", None)
                    if not gain_lbl:
                        continue
                    if agc and hasattr(agc, 'get_debug_info'):
                        info = agc.get_debug_info()
                        pk = info.get("peak", 0)
                        pk_db = max(20.0 * math.log10(max(pk, 1e-10)), -90.0)
                        gain_db = agc.get_agc_gain_db()
                        gain_str = f"{gain_db:+.1f} dB  峰值 {pk_db:.1f} dB"
                        prev = getattr(r, "_agc_last_val", None)
                        if prev is not None and gain_str != prev:
                            r._agc_last_change = now
                        r._agc_last_val = gain_str
                        age = now - getattr(r, "_agc_last_change", 0.0)
                        fg = theme.ACCENT if age < 1.5 else theme.TEXT_DIM
                        gain_lbl.config(text=gain_str, fg=fg)
                    else:
                        gain_lbl.config(text="无 AGC 节点", fg=theme.TEXT_DIM)
        # ── AEC 行 VU 电平表更新（10fps，降 CPU）──
        aec_thread = self.engine.thread if self.engine.running else None
        if now >= getattr(self, "_aec_vu_next", 0.0):
            self._aec_vu_next = now + 0.1
            aec_vu = aec_thread.get_aec_vu() if aec_thread else {}
            for r in self.rows:
                if r.spec.name != "echo_cancel" or not r.on_var.get():
                    continue
                vu_widgets = getattr(r, "_aec_vu_widgets", None)
                if not vu_widgets:
                    continue
                mic_name = str((r.cfg.get("params") or {}).get("device", ""))
                if not aec_thread:
                    for key in ("mic", "far", "out"):
                        w = vu_widgets.get(key)
                        if w:
                            w.update_level(0.0, now)
                    continue
                vu = aec_vu.get(mic_name)
                if vu:
                    for key in ("mic", "far", "out"):
                        w = vu_widgets.get(key)
                        if w:
                            w.update_level(vu[key], now)
        self.root.after(33, self._viz_tick)

    def _attach_linux_virtual_mic(self, row):
        """Linux 输出行内虚拟声卡卡：仅当该行输出选中 `purevox_out`（虚拟麦克风）
        时显示，提供创建/清理（幂等），状态实时反映；创建/清理后刷新设备列表。"""
        from .widgets import FlatButton
        S, F = self.sizes, self.fonts
        green, red = "#3aa76d", "#d9534f"
        card = tk.Frame(row.body_frame, bg=theme.PANEL)
        head = tk.Frame(card, bg=theme.PANEL)
        head.pack(fill=tk.X, padx=8, pady=(6, 2))
        dot = tk.Canvas(head, bg=theme.PANEL, width=12, height=12,
                        highlightthickness=0)
        dot.pack(side=tk.LEFT)
        dot.create_oval(1, 1, 11, 11, fill=red, outline="")
        state_lbl = tk.Label(head, text="", bg=theme.PANEL, fg=theme.TEXT_DIM,
                             font=F.get("bold"))
        state_lbl.pack(side=tk.LEFT, padx=(6, 0))
        btn = FlatButton(head, "创建", sizes=S, command=lambda: _on_action())
        btn.pack(side=tk.RIGHT)
        tk.Label(card,
                 text="创建后，其它软件把「PureVox 虚拟麦克风」设为麦克风"
                      "即可收到降噪声音。创建/清理均幂等。",
                 bg=theme.PANEL, fg=theme.TEXT_DIM, font=F.get("body"),
                 justify="left", anchor="w",
                 wraplength=max(320, S["win_w"] - 80)).pack(
            fill=tk.X, padx=8, pady=(2, 6))

        def _ready():
            try:
                from pvplatform.system import virtual_mic_ready
                return bool(virtual_mic_ready())
            except Exception:
                return False

        def _refresh():
            ready = _ready()
            dot.delete("all")
            dot.create_oval(1, 1, 11, 11,
                            fill=(green if ready else red), outline="")
            state_lbl.configure(text=("已创建" if ready else "未创建"),
                                fg=(green if ready else red))
            btn.configure(text=("清理" if ready else "创建"))

        def _on_action():
            try:
                from pvplatform.system import (ensure_virtual_mic,
                                               remove_virtual_mic)
                from logger import Logger
                if _ready():
                    remove_virtual_mic(Logger())
                else:
                    ensure_virtual_mic(Logger())
            except Exception as e:
                self.engine.log.warn(f"[虚拟声卡] 操作失败: {e}")
            _refresh()
            self.refresh_devices()

        def _active(sel):
            if sel == "purevox_out":
                _refresh()
                card.pack(fill=tk.X, padx=S["pad_sm"], pady=(0, S["pad_sm"]))
            else:
                card.pack_forget()
            row.ensure_body()   # 切换后收起/展开参数区，不留空占位

        row._linux_vm_apply = _active
        _active(str((row.cfg.get("params") or {}).get("device", "")))

    def _attach_vb_card(self, row):
        """VB-CABLE 卡片——完整实现 legacy 弹框的内容：
        状态灯 / 双端点说明与数据流向 / 驱动卡片（打开控制面板·下载·教程）/
        启动检测开关。

        有无检测不在加载时自动跑（绝不等待）：程序启动与点击「启动/停止」
        触发设备重枚举，refresh_devices 用同一次枚举结果判定双端点并回填状态。
        """
        green, red, gray = "#3aa76d", "#d9534f", "#9e9e9e"
        download_url = ("https://download.vb-audio.com/Download_CABLE/"
                        "VBCABLE_Driver_Pack45.zip")
        tutorial_url = "https://www.bilibili.com/video/BV1i2bazGEKe/"
        wrap = max(320, self.sizes["win_w"] - 80)

        card = tk.Frame(row.body_frame, bg=theme.PANEL)
        card.pack(fill=tk.X, padx=self.sizes["pad_sm"],
                  pady=(0, self.sizes["pad_sm"]))

        # ── 状态行：指示灯 + 状态文字 ──
        head = tk.Frame(card, bg=theme.PANEL)
        head.pack(fill=tk.X, padx=8, pady=(6, 2))
        dot = tk.Canvas(head, bg=theme.PANEL, width=12, height=12,
                        highlightthickness=0)
        dot.pack(side=tk.LEFT)
        dot.create_oval(1, 1, 11, 11, fill=gray, outline="")
        state_lbl = tk.Label(head, text="待检测 —— 启动或停止音频处理时自动检测",
                             bg=theme.PANEL, fg=theme.TEXT_DIM,
                             font=self.fonts.get("bold"))
        state_lbl.pack(side=tk.LEFT, padx=(6, 0))

        # 数据流向用图示（比文字描述直观）：麦克风→PureVox→Input→Output→软件
        flow = tk.Canvas(card, bg=theme.PANEL, height=self.sizes["ctl_h"] + 12,
                         highlightthickness=0)
        flow.pack(fill=tk.X, padx=8, pady=(2, 4))

        def _draw_flow(_e=None):
            """完整链路（单向、左→右）：麦克风 → PureVox → CABLE In → Out → 软件。"""
            flow.delete("all")
            w = flow.winfo_width() or 320
            h = int(flow.winfo_height() or 34)
            f = self.fonts.get("small")
            boxes = (("麦克风", theme.TRACK), ("PureVox", theme.ACCENT),
                     ("CABLE In", theme.TRACK), ("CABLE Out", theme.TRACK),
                     ("OBS/会议/语音等软件", theme.TRACK))
            tw = [f.measure(t) if f else 40 for t, _ in boxes]
            bh = max(16, h - 8)
            cy = h // 2
            # 先按宽松间距排；放不下就收紧间距（绝不压缩文字本身）
            for pad, arrow in ((10, 14), (8, 11), (6, 9), (4, 7)):
                if sum(t + pad for t in tw) + arrow * (len(boxes) - 1) <= w:
                    break
            # 剩余宽度均摊到各框，使图示左右边缘与下方线框左右对齐
            used = sum(t + pad for t in tw) + arrow * (len(boxes) - 1)
            # 末框右边线留 1px：贴画布最右会被裁掉（看似没有右边线）
            slack = max(0, w - 1 - used)
            extra = slack // len(boxes)
            widths = [t + pad + extra for t in tw]
            widths[-1] += slack - extra * len(boxes)
            x = 0
            for i, ((text, bg), bw) in enumerate(zip(boxes, widths)):
                flow.create_rectangle(x, cy - bh // 2, x + bw, cy + bh // 2,
                                      fill=bg, outline=theme.MID)
                flow.create_text(x + bw // 2, cy, text=text, fill=theme.TEXT,
                                 font=f)
                x += bw
                if i < len(boxes) - 1:
                    x_end = x + arrow - 1
                    flow.create_line(x + 1, cy, x_end - 4, cy,
                                     fill=theme.MID, width=2)
                    # 实心三角箭头：比线帽明显，方向一眼可辨（指向右）
                    flow.create_polygon(x_end - 5, cy - 4, x_end, cy,
                                        x_end - 5, cy + 4,
                                        fill=theme.MID, outline="")
                    x += arrow
        flow.bind("<Configure>", _draw_flow)

        # ── 驱动卡片 ──
        guide = tk.Label(card,
                         text="未检测到驱动：点「驱动下载」安装后启动即可识别。",
                         bg=theme.PANEL, fg=theme.TEXT_FAINT,
                         font=self.fonts.get("body"), justify="left",
                         anchor="w", wraplength=wrap)

        # 只有「标签 + 三个按钮」这一行带外框；框宽与上方流程图一致（同 padx）
        btns = tk.Frame(card, bg=theme.PANEL,
                        highlightbackground=theme.MID, highlightthickness=1)
        btns.pack(fill=tk.X, padx=8, pady=(0, 4))
        tk.Label(btns, text=" VB-CABLE 驱动 ", bg=theme.TRACK,
                 fg=theme.TEXT_DIM, font=self.fonts.get("body")
                 ).pack(side=tk.LEFT, padx=(0, 2), pady=4)

        def _label_btn(parent, text, on_click):
            # 三个按钮等宽铺满整行（左右对齐）；不各自描边，统一由整块外框
            b = tk.Label(parent, text=text, bg=theme.PANEL,
                         fg=theme.TEXT, font=self.fonts.get("body"),
                         padx=6, pady=3, cursor="hand2")
            b.pack(side=tk.LEFT, fill=tk.X, expand=True, padx=0)
            b.bind("<Button-1>", lambda e: on_click())
            b.bind("<Enter>", lambda e: b.configure(fg=theme.ACCENT))
            b.bind("<Leave>", lambda e: b.configure(fg=theme.TEXT))
            return b

        panel_state = {"ok": False}

        def _open_panel():
            if panel_state["ok"]:
                from pvplatform.system import open_virtual_cable_panel
                open_virtual_cable_panel(Logger())

        _label_btn(btns, "控制面板", _open_panel)
        _label_btn(btns, "驱动下载", lambda: webbrowser_open(download_url))
        _label_btn(btns, "视频教程", lambda: webbrowser_open(tutorial_url))

        # ── 启动检测开关（写回配置；启动流程据此决定是否提醒）──
        cb_var = tk.BooleanVar(
            value=bool(self._cfg_get("vbcable_check_enabled", True)))

        def _toggle_check():
            self._cfg_set("vbcable_check_enabled", bool(cb_var.get()))

        DarkCheck(card, "启动时检测 VB-CABLE 驱动安装（取消勾选不再弹框）",
                  cb_var, command=_toggle_check,
                  sizes=self.sizes, fonts=self.fonts).pack(
            anchor="w", padx=8, pady=(0, 6))

        # ── 状态套用：由 refresh_devices（启动/启停触发）用同一次
        #    枚举结果调用，本卡片自身不做任何扫描/等待 ──
        def _apply(now):
            try:
                if not bool(card.winfo_exists()):
                    return
            except Exception:
                return
            panel_state["ok"] = bool(now)
            dot.delete("all")
            dot.create_oval(1, 1, 11, 11,
                            fill=(green if now else red), outline="")
            state_lbl.configure(text="已安装" if now else "未安装",
                                fg=green if now else red)
            if now:
                guide.pack_forget()
            else:
                guide.configure(text=(
                    "未检测到 VB-CABLE 驱动：请先下载官方驱动包并安装，"
                    "装好后点击「启动/停止音频处理」即可识别。"))
                guide.pack(fill=tk.X, padx=8, pady=(0, 2))

        row.vb_apply = _apply

    def refresh_devices(self):
        """后台枚举设备，回主线程刷新各设备下拉。

        唯一扫描入口（触发点=程序启动 / 点击启动·停止）。运行中不再枚举：
        引擎占着 PyAudio 时扫描会失败。VB-CABLE 有无不单独扫描——直接复用
        本次枚举结果判定双端点，经 row.vb_apply 回填。
        """
        import threading

        def _work():
            try:
                devs = enum_io_devices()
            except Exception as e:
                self.engine.log.warn(f"[设备] 枚举失败: {e}")
                return
            out_names = [t for t, _d in devs.get("outputs", [])]
            in_names = [t for t, _d in devs.get("inputs", [])]
            vb_ok = (any("CABLE Input" in t for t in out_names)
                     and any("CABLE Output" in t for t in in_names))

            def _apply():
                for r in self.rows:
                    apply_vb = getattr(r, "vb_apply", None)
                    if apply_vb is not None:
                        try:
                            apply_vb(vb_ok)
                        except Exception:
                            pass
                    r.set_devices(devs)

            try:
                # 主线程 mainloop 运行中才投递；测试等无 mainloop 场景静默跳过
                self.root.after(0, _apply)
            except Exception:
                pass

        threading.Thread(target=_work, daemon=True).start()

    def _on_param(self, row, key, value):
        row.cfg.setdefault("params", {})[key] = value
        # 实时生效：不重启链，直接推给运行中的处理器
        self.engine.set_live_param(self.rows.index(row), key, value)
        self._persist()

    def _pack_row(self, row):
        # 每边 pad_md/2 ⇒ 相邻卡片间隙 = pad_md，与左右留白同粗
        row.pack(fill=tk.X, pady=max(1, self.sizes["pad_md"] // 2))

    def _move_row_live(self, row, target):
        """拖动中实时换位：只重排显示，不持久化不重启（避免抖动）。"""
        if row is target or row not in self.rows or target not in self.rows:
            return
        self.rows.insert(self.rows.index(target),
                         self.rows.pop(self.rows.index(row)))
        for r in list(self.panel.body.winfo_children()):
            if isinstance(r, NodeRow):
                r.pack_forget()
        for r in self.rows:
            self._pack_row(r)

    def remove_row(self, row):
        row.destroy()
        if row in self.rows:
            self.rows.remove(row)
        self._apply_chain_change()

    def clear_rows(self):
        for r in list(self.rows):
            r.destroy()
        self.rows.clear()

    def _wheel(self, e):
        d = sys_wheel_delta(e)
        if not d:
            return
        # 仅当指针悬停在节点面板上才滚动
        w = self.root.winfo_containing(e.x_root, e.y_root)
        while w is not None:
            if w is self.panel.canvas or w is self.panel.body:
                # 内容不满一屏时禁止滚动（杜绝滚出下方空白）
                if (self.panel.body.winfo_reqheight()
                        > self.panel.canvas.winfo_height()):
                    # d 已按 yview_scroll 语义（负=上/前，正=下/后）
                    self.panel.canvas.yview_scroll(d, "units")
                return
            w = getattr(w, "master", None)

    def run(self):
        chain = []
        if self.config:
            try:
                chain = list(self.config.get("plugin_chain", []))
            except Exception:
                chain = []
        self.load_chain(chain)
        self.refresh_devices()
        # 启动时自动运行：稍候自动开始音频处理（窗口已在 __init__ 藏进托盘）
        if bool(self._cfg_get("auto_start", False)):
            self.root.after(1000, self._on_start)
        self.root.mainloop()


def sys_wheel_delta(e):
    """滚轮事件 → yview_scroll 单位增量（负=上/前，正=下/后）。

    统一各平台符号约定后直接交给 canvas.yview_scroll：
      Windows/macOS 走 <MouseWheel> e.delta；X11 走 Button-4(上)/Button-5(下)。
    历史上 Linux 分支返回了相反符号又被调用方取反，导致列表滚动反向。
    """
    num = getattr(e, "num", 0)
    if num == 4:
        return -1
    if num == 5:
        return 1
    d = getattr(e, "delta", 0)
    if not d:
        return 0
    if abs(d) >= 120:
        return int(-d / 120)
    return -1 if d > 0 else 1


if __name__ == "__main__":
    MainWindowTk().run()
