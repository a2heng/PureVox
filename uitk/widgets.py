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

"""uitk 基础组件：FlatButton / DarkCombo / DarkCheck / ScrollFrame。

全部纯 tk（无 ttk），颜色一律取自 uitk.theme，尺寸持共享 sizes 表，
换挡时由宿主调用 apply_sizes()。
"""

import tkinter as tk

from . import theme
from .metrics import make_sizes


class FlatButton(tk.Label):
    """自绘扁平按钮（Label 实现，可完全控色）。"""

    def __init__(self, parent, text, command=None, bg=theme.BUTTON,
                 fg=theme.TEXT, font=None, sizes=None, pad=None, **kw):
        self.sizes = sizes if sizes is not None else make_sizes(100)
        super().__init__(parent, text=text, bg=bg, fg=fg,
                         font=font, padx=self.sizes["pad_lg"] if pad is None else pad,
                         pady=max(0, (self.sizes["ctl_h"] - (font.metrics("linespace") if font else 16)) // 2),
                         **kw)
        self._bg = bg
        self._cmd = command
        self.bind("<Button-1>", self._click)
        # 悬停色按【当前】底色计算（运行态会绿↔红切换，不能用构造时快照）
        self.bind("<Enter>", lambda e: self.configure(bg=theme.hover(self._bg)))
        self.bind("<Leave>", lambda e: self.configure(bg=self._bg))

    def set_bg(self, bg):
        """运行态换底色（同步更新悬停基准）。"""
        self._bg = bg
        self.configure(bg=bg)

    def _click(self, _e):
        if self._cmd:
            try:
                self._cmd()
            except Exception:
                pass

    def set_bg(self, bg):
        self._bg = bg
        self.configure(bg=bg)

    def apply_sizes(self):
        try:
            ls = self.cget("font").metrics("linespace")
        except Exception:
            ls = 16
        self.configure(padx=self.sizes["pad_lg"],
                       pady=max(0, (self.sizes["ctl_h"] - ls) // 2))


class DarkCheck(tk.Frame):
    """深色复选框：Canvas 绘制严格正方形 + 像素风直角对勾。"""

    def __init__(self, parent, text, variable, command=None,
                 sizes=None, fonts=None):
        self.sizes = sizes if sizes is not None else make_sizes(100)
        self.fonts = fonts if fonts is not None else {}
        super().__init__(parent, bg=parent.cget("bg") if isinstance(parent, tk.Widget)
                         else theme.WINDOW)
        self.variable = variable
        self.command = command
        sz = self.sizes["check_box"]
        self.canvas = tk.Canvas(self, width=sz, height=sz,
                                bg=self["bg"], highlightthickness=0, bd=0,
                                cursor="hand2")
        self.canvas.pack(side=tk.LEFT)
        self.label = tk.Label(self, text=text, bg=self["bg"],
                              fg=theme.TEXT, font=self.fonts.get("body"))
        self.label.pack(side=tk.LEFT, padx=self.sizes["pad_md"])
        self._sync()
        variable.trace_add("write", lambda *a: self._sync())
        for w in (self, self.canvas, self.label):
            w.bind("<Button-1>", lambda e: self.toggle())

    def _sync(self):
        on = bool(self.variable.get())
        sz = int(self.canvas["width"])
        g = self.sizes["pad_sm"]   # 内边距
        c = self.canvas
        c.delete("all")
        # 严格正方形外框（未勾选=羊皮纸底，避免白色割裂）
        c.create_rectangle(0, 0, sz - 1, sz - 1,
                           fill=theme.ACCENT if on else theme.WINDOW,
                           outline=theme.MID, width=1)
        if on:
            # 像素风直角对勾（两段粗线，无抗锯齿斜线）
            w = max(2, self.sizes["pad_sm"])
            pts = [(sz*0.22, sz*0.52), (sz*0.42, sz*0.72), (sz*0.80, sz*0.28)]
            for (x0, y0), (x1, y1) in zip(pts, pts[1:]):
                c.create_line(x0, y0, x1, y1, fill=theme.ACCENT_TEXT, width=w)

    def toggle(self):
        self.variable.set(not bool(self.variable.get()))
        self._sync()
        if self.command:
            try:
                self.command()
            except Exception:
                pass

    def apply_sizes(self):
        sz = self.sizes["check_box"]
        self.canvas.configure(width=sz, height=sz)
        self.label.configure(padx=self.sizes["pad_md"],
                             font=self.fonts.get("body"))
        self._sync()


class HSlider(tk.Canvas):
    """自绘水平滑杆：粗槽顶满全宽 + accent 把手（数值显示交给外部标签）。"""

    def __init__(self, parent, lo, hi, value, step, command=None,
                 sizes=None, width_px=None):
        self.sizes = sizes if sizes is not None else make_sizes(100)
        S = self.sizes
        w = width_px or S["win_w"] // 2
        h = max(S["ctl_h"], 24)
        super().__init__(parent, width=w, height=h, bg=parent.cget("bg"),
                         highlightthickness=0, bd=0, cursor="hand2")
        self.lo, self.hi, self.step = float(lo), float(hi), float(step)
        self.value = float(value)
        self.command = command
        self._hw = max(6, S["ctl_h"] // 3)   # 把手半宽（行程夹紧用）
        self.bind("<Button-1>", self._on_drag)
        self.bind("<B1-Motion>", self._on_drag)
        self.bind("<Configure>", lambda e: self._draw())
        self._draw()

    def _width(self):
        """实际渲染宽（未映射前回退请求宽）。

        画布被父布局压窄时 winfo_width < 请求宽——必须按实际宽绘制，
        否则把手/填充画到裁剪区外（正值区"跑到不见了"）。
        """
        w = self.winfo_width()
        return w if w > 8 else int(self["width"])

    def _val_to_x(self, v):
        w = self._width()
        span = w - 2 * self._hw
        return self._hw + int((v - self.lo) / (self.hi - self.lo) * span)

    def _draw(self):
        S = self.sizes
        w = self._width()
        h = max(self.winfo_height(), int(self["height"]))
        self.delete("all")
        cy = h // 2
        th = max(8, S["ctl_h"] // 3)      # 加粗槽厚
        # 槽顶满全宽（0 → w-1，留 1px 防描边被裁）
        self.create_rectangle(0, cy - th // 2, w - 1, cy + th // 2,
                              fill=theme.TRACK, width=0)
        x = self._val_to_x(self.value)
        # 有符号量程（跨 0，如 dB 增益）：填充从 0 位到把手——
        # 增/减两侧各自向把手延伸，默认值不在"看起来已拉满"的位置
        if self.lo < 0.0 < self.hi:
            z = self._val_to_x(0.0)
            self.create_rectangle(min(x, z), cy - th // 2,
                                  max(x, z), cy + th // 2,
                                  fill=theme.ACCENT, width=0)
        else:
            self.create_rectangle(0, cy - th // 2, min(x, w - 1),
                                  cy + th // 2,
                                  fill=theme.ACCENT, width=0)
        # 把手（行程夹在两端内并留 1px，防止右端描边被画布裁掉）
        hh = S["ctl_h"] - 4
        x = max(self._hw + 1, min(w - self._hw - 1, x))
        self.create_rectangle(x - self._hw, cy - hh // 2,
                              x + self._hw, cy + hh // 2,
                              fill=theme.ACCENT, outline=theme.TEXT_DIM,
                              width=1)

    def _set_from_x(self, ex):
        w = self._width()
        frac = (ex - self._hw) / max(1, w - 2 * self._hw)
        frac = max(0.0, min(1.0, frac))
        v = self.lo + frac * (self.hi - self.lo)
        if self.step:
            v = round(v / self.step) * self.step
            v = max(self.lo, min(self.hi, v))
        if v != self.value:
            self.value = v
            self._draw()
            if self.command:
                try:
                    self.command()
                except Exception:
                    pass

    def set_value(self, v, silent=False):
        """编程式设值（进度回显用）；silent=True 不触发 command。"""
        self.value = min(max(float(v), self.lo), self.hi)
        self._draw()
        if not silent and self.command:
            self.command()

    def _on_drag(self, e):
        self._set_from_x(e.x)


def _combo_pairs(values):
    """规范化下拉项为 [(显示, 值)]：str → (str, str)；二元组原样保留。

    Linux PipeWire 设备需要「显示标签 ≠ 真实 node.name」（value 存 node.name），
    等价于旧 PySide 版的 userData；Windows 下显示=值，行为不变。
    """
    pairs = []
    for v in list(values):
        if isinstance(v, (tuple, list)) and len(v) == 2:
            disp, val = v
        else:
            disp = val = v
        disp = "" if disp is None else str(disp)
        val = "" if val is None else str(val)
        if not disp.strip():
            continue
        pairs.append((disp, val))
    return pairs


class DarkCombo(tk.Frame):
    """深色下拉（弹层与外框严格同宽，长项像素级省略）——参考 lite BlackCombo。

    values 元素可为 str（显示=值）或 (显示, 值) 二元组；var 始终存「值」，
    控件按值反查显示文本。设备下拉据此把 node.name 存进配置、标签只用于显示。
    """

    def __init__(self, parent, values, var, on_change=None,
                 sizes=None, fonts=None):
        self.sizes = sizes if sizes is not None else make_sizes(100)
        self.fonts = fonts if fonts is not None else {}
        # 外壳与宿主同色（不产生第二圈色），边框只由 inner 的 1px 描边承担
        host_bg = parent.cget("bg") if isinstance(parent, tk.Widget) \
            else theme.WINDOW
        super().__init__(parent, bg=host_bg, bd=0, padx=0, pady=0)
        self.var = var
        self._pairs = _combo_pairs(values)
        self.values = [d for d, _v in self._pairs]
        self._disp_by_val = {}
        for d, v in self._pairs:
            self._disp_by_val.setdefault(v, d)
        self.on_change = on_change
        self._popup = None
        inner = tk.Frame(self, bg=theme.WINDOW,
                         highlightbackground=theme.MID,
                         highlightthickness=1)
        self.inner = inner
        inner.pack(fill=tk.BOTH, expand=True)
        self._display = tk.StringVar()
        var.trace_add("write", lambda *a: self._sync_display())
        self._sync_display()
        self.lbl = tk.Label(inner, textvariable=self._display,
                            bg=theme.WINDOW, fg=theme.TEXT, anchor="w",
                            padx=self.sizes["pad_md"],
                            font=self.fonts.get("body"))
        self.lbl.pack(side=tk.LEFT, fill=tk.BOTH, expand=True)
        self.arrow = tk.Label(inner, text="▾", bg=theme.BUTTON,
                              fg=theme.TEXT_DIM,
                              font=self.fonts.get("bold"), width=2)
        self.arrow.pack(side=tk.RIGHT, fill=tk.Y)
        inner.bind("<Configure>", lambda e: self._sync_display())
        inner.pack_propagate(False)
        inner.configure(height=self.sizes["combo_h"])
        for w in (self, inner, self.lbl, self.arrow):
            w.bind("<Button-1>", lambda e: self._toggle())
        if self._pairs and var.get() not in self._disp_by_val:
            var.set(self._pairs[0][1])

    def _elide(self, v, avail):
        try:
            f = self.fonts.get("body")
            if avail > 20 and f.measure(v) > avail:
                while v and f.measure(v + "…") > avail:
                    v = v[:-1]
                v += "…"
        except Exception:
            pass
        return v

    def _sync_display(self, *a):
        v = self.var.get() or ""
        v = self._disp_by_val.get(v, v)
        try:
            avail = (self.inner.winfo_width()
                     - self.arrow.winfo_reqwidth()
                     - 2 * (self.sizes["pad_md"] + 2))
            v = self._elide(v, avail)
        except Exception:
            pass
        self._display.set(v)

    def set_values(self, values):
        self._pairs = _combo_pairs(values)
        self.values = [d for d, _v in self._pairs]
        self._disp_by_val = {}
        for d, v in self._pairs:
            self._disp_by_val.setdefault(v, d)
        if self._pairs and self.var.get() not in self._disp_by_val:
            self.var.set(self._pairs[0][1])
        self._sync_display()
        # 弹层开着时原地重建——异步枚举回来后列表即时变新
        if self._popup is not None and self._popup.winfo_exists():
            self._close()
            self._open()

    def apply_sizes(self):
        self.inner.configure(height=self.sizes["combo_h"])
        self.lbl.configure(padx=self.sizes["pad_md"],
                           font=self.fonts.get("body"))

    def _toggle(self):
        if self._popup and self._popup.winfo_exists():
            self._close()
        else:
            self._open()

    def _open(self):
        import sys as _sys
        if not self.values or (self._popup and self._popup.winfo_exists()):
            return
        x = self.winfo_rootx()
        y = self.winfo_rooty() + self.winfo_height()
        S = self.sizes
        pw = max(self.winfo_width(), 60)
        # 行内文字可用宽 = 弹层宽 − 滚动条 − 行内边距
        avail_item = pw - S["scrollbar_w"] - 2 * S["pad_md"] - 8
        row_h = S["combo_h"]
        self._popup = tk.Toplevel(self)
        self._popup.overrideredirect(True)
        self._popup.configure(bg=theme.MID, bd=1)
        self._popup.attributes("-topmost", True)
        outer = tk.Frame(self._popup, bg=theme.MID, bd=0)
        outer.pack(fill=tk.BOTH, expand=True)
        # 滚轮一格一设备，行高/滚动步长全部来自尺寸表
        self.canvas = canvas = tk.Canvas(
            outer, bg=theme.WINDOW, bd=0, highlightthickness=0,
            yscrollincrement=row_h + 2)
        bar = tk.Frame(outer, bg=theme.BUTTON, width=S["scrollbar_w"],
                       bd=1, relief=tk.FLAT,
                       highlightbackground=theme.MID, highlightthickness=1)
        bar.pack(side=tk.RIGHT, fill=tk.Y, padx=(1, 0))
        thumb = tk.Frame(bar, bg=theme.MID, bd=0)
        thumb.place(relx=0, rely=0, relwidth=1, height=S["thumb_min"])
        canvas.configure(yscrollcommand=lambda *a: _update_thumb(*a))

        def _update_thumb(first, last):
            """标准进度：thumb 高= h*可见/总数，y= h*first，夹在边界内。"""
            try:
                h = bar.winfo_height() or outer.winfo_height() or S["win_h"]
                th = max(S["thumb_min"],
                         int(h * (float(last) - float(first))))
                y0 = int(h * float(first))
                th = min(th, h)
                y0 = max(0, min(y0, h - th))
                thumb.place_configure(height=th, y=y0)
            except Exception:
                pass

        canvas.pack(side=tk.LEFT, fill=tk.BOTH, expand=True)
        inner = tk.Frame(canvas, bg=theme.WINDOW)
        win_id = canvas.create_window((0, 0), window=inner, anchor="nw")

        def _sync_w(event=None):
            try:
                canvas.itemconfig(win_id, width=canvas.winfo_width())
            except Exception:
                pass
        canvas.bind("<Configure>", _sync_w)
        for idx, disp in enumerate(self.values):
            is_sel = disp == self._disp_by_val.get(self.var.get(),
                                                   self.var.get())
            bgc = theme.PANEL if is_sel else theme.WINDOW
            # 外壳锁定行高（pack_propagate 关闭），与 lite BlackCombo 同构
            item = tk.Frame(inner, bg=bgc, bd=0, height=row_h)
            item.pack(fill=tk.X, padx=1, pady=1)
            item.pack_propagate(False)
            l1 = tk.Label(item, text=self._elide(disp, avail_item),
                          bg=bgc,
                          fg=(theme.ACCENT if is_sel else theme.TEXT),
                          anchor="w", padx=S["pad_md"],
                          font=self.fonts.get("body"))
            l1.pack(fill=tk.BOTH, expand=True)
            for w in (item, l1):
                w.bind("<Button-1>", lambda e, i=idx: self._pick(i))
                w.bind("<Enter>",
                       lambda e, f=item, lb=l1, sel=is_sel: (
                           f.configure(bg=theme.DARK if not sel else theme.PANEL),
                           lb.configure(bg=f.cget("bg"))))
                w.bind("<Leave>",
                       lambda e, f=item, lb=l1, sel=is_sel: (
                           f.configure(bg=theme.WINDOW if not sel else theme.PANEL),
                           lb.configure(bg=f.cget("bg"))))
        inner.update_idletasks()
        h = min(len(self.values), S["popup_rows"]) * (row_h + 2)
        canvas.configure(height=h)
        # 外层 bd=1，补 2px 边框
        self._popup.geometry(f"{pw}x{h + 2}+{x}+{y}")
        canvas.configure(scrollregion=canvas.bbox("all"))
        _update_thumb("0", "1")
        # 初始滚动到选中项（尾部贴底避免空行）
        try:
            cur_disp = self._disp_by_val.get(self.var.get(), self.var.get())
            idx = self.values.index(cur_disp)
            n = len(self.values)
            vis = S["popup_rows"]
            top = max(0, min(idx, n - vis)) / max(1, n) if n > vis else 0
            canvas.yview_moveto(top)
        except Exception:
            pass

        def _wheel(e):
            if _sys.platform.startswith("win"):
                delta = int(-1 * (e.delta / 120))
            elif getattr(e, "num", 0) == 4:
                delta = -1
            elif getattr(e, "num", 0) == 5:
                delta = 1
            else:
                delta = 0
            canvas.yview_scroll(delta, "units")
            _update_thumb(*canvas.yview())
            return "break"
        for w in (canvas, inner, outer, self._popup, bar, thumb):
            w.bind("<MouseWheel>", _wheel)
            w.bind("<Button-4>", _wheel)
            w.bind("<Button-5>", _wheel)

        def _bar_click(e):
            """点/拖滚动条：thumb 跟随光标，按比例跳到对应条目。"""
            try:
                bh = bar.winfo_height()
                th = thumb.winfo_height()
                y0 = max(0, min(e.y - th // 2, bh - th))
                frac = y0 / max(1, bh - th)
                n = len(self.values)
                idx = int(frac * (n - 1) + 0.5)
                canvas.yview_moveto(idx / max(1, n))
                _update_thumb(*canvas.yview())
            except Exception:
                pass
        bar.bind("<Button-1>", _bar_click)
        thumb.bind("<B1-Motion>", lambda e: _bar_click(e))

        # 点击别处收起（含再次点击下拉本体）
        self._root_bind = self.winfo_toplevel().bind(
            "<Button-1>", self._on_root, add="+")
        # 最小化/隐藏主窗时收起
        self._unmap_bind = self.winfo_toplevel().bind(
            "<Unmap>", lambda e: self._close(), add="+")
        self._popup.bind("<Escape>", lambda e: self._close())
        self._popup.focus_set()

    def _on_root(self, e):
        if not self._popup or not self._popup.winfo_exists():
            return
        try:
            px, py = self._popup.winfo_rootx(), self._popup.winfo_rooty()
            pw, ph = self._popup.winfo_width(), self._popup.winfo_height()
            if px <= e.x_root <= px + pw and py <= e.y_root <= py + ph:
                return
            sx, sy = self.winfo_rootx(), self.winfo_rooty()
            sw, sh = self.winfo_width(), self.winfo_height()
            if sx <= e.x_root <= sx + sw and sy <= e.y_root <= sy + sh:
                return
        except Exception:
            pass
        self._close()

    def _pick(self, idx):
        if 0 <= idx < len(self._pairs):
            self.var.set(self._pairs[idx][1])
            if self.on_change:
                try:
                    self.on_change()
                except Exception:
                    pass
        self._close()

    def _close(self):
        try:
            if hasattr(self, "_root_bind"):
                self.winfo_toplevel().unbind("<Button-1>", self._root_bind)
            if hasattr(self, "_unmap_bind"):
                self.winfo_toplevel().unbind("<Unmap>", self._unmap_bind)
        except Exception:
            pass
        if self._popup and self._popup.winfo_exists():
            try:
                self._popup.destroy()
            except Exception:
                pass
        self._popup = None


class SquareButton(tk.Frame):
    """正方形方框按钮（图标字形专用）：固定 px 边长 + 居中字形。

    FlatButton 由文字度量决定尺寸（窄字形× + ctl_h 行距 → 又窄又高，不成方形），
    本控件锁死边长（默认 ctl_h），方框背景 + 悬停变亮，用于删除「×」等图标钮。
    """

    def __init__(self, parent, glyph, command=None, bg=theme.BUTTON,
                 fg=theme.TEXT, size=None, font=None, sizes=None):
        self.sizes = sizes if sizes is not None else make_sizes(100)
        self._bg = bg
        side = int(size or self.sizes["ctl_h"])
        # 外框底色必须就是按钮底色：若用宿主底色，静止时只见居中小字形，
        # 悬停重绘才显出整块方框（「一开始很小、划过才正常」的根因）
        super().__init__(parent, bg=bg, width=side, height=side)
        self.pack_propagate(False)
        self.grid_propagate(False)
        self.command = command
        self.lbl = tk.Label(self, text=glyph, bg=bg, fg=fg, font=font,
                            cursor="hand2")
        self.lbl.place(relx=0.5, rely=0.5, anchor="center")
        for w in (self, self.lbl):
            w.bind("<Button-1>", self._click)
            w.bind("<Enter>", lambda e: self._paint(theme.hover(self._bg)))
            w.bind("<Leave>", lambda e: self._paint(self._bg))

    def _paint(self, bg):
        self.configure(bg=bg)
        self.lbl.configure(bg=bg)

    def _click(self, _e):
        if self.command:
            try:
                self.command()
            except Exception:
                pass


class HotkeyField(tk.Frame):
    """可录制全局热键的字段：点值标签进入录制，按下组合键即捕获。

    - 空串 = 不监听（显示「未设置」）；
    - 录制中 Esc 取消、Delete/Backspace 清除；
    - 纯字母/数字必须带至少一个修饰键（F1–F24 可单键）；
    - `command(spec)` 在捕获成功后回调（spec 已规范化为 "" 或 "Ctrl+Alt+1"）。
    """

    def __init__(self, parent, spec="", command=None, sizes=None, fonts=None,
                 width=14, show_clear=True):
        from .hotkeys import normalize_spec, keysym_to_spec, MODIFIER_KEYSYMS
        self._keysyms = MODIFIER_KEYSYMS
        self._to_spec = keysym_to_spec
        self._normalize = normalize_spec
        self.sizes = sizes if sizes is not None else make_sizes(100)
        self.fonts = fonts if fonts is not None else {}
        bg = parent.cget("bg") if isinstance(parent, tk.Widget) else theme.WINDOW
        super().__init__(parent, bg=bg)
        self.command = command
        self._spec = normalize_spec(spec)
        self._capturing = False
        self._mods = set()
        self._bind_ids = []
        # 1px 木色描边：白底在浅色斑马纹/面板上会「融进背景」看不出是个输入框
        self.value_label = tk.Label(self, text="", bg=theme.BASE,
                                    fg=theme.TEXT_DIM, cursor="hand2",
                                    takefocus=1, width=width, anchor="center",
                                    font=self.fonts.get("body"),
                                    highlightbackground=theme.MID,
                                    highlightcolor=theme.MID,
                                    highlightthickness=1,
                                    padx=self.sizes["pad_md"],
                                    pady=max(1, self.sizes["pad_sm"] // 2))
        self.value_label.pack(side=tk.LEFT)
        self.value_label.bind("<Button-1>", self._begin)
        self.value_label.bind("<FocusOut>", lambda e: self._cancel())
        # 清除钮可按需省略（有前置开关控制启停时不必再清空录制内容）
        self.clear_label = None
        if show_clear:
            self.clear_label = tk.Label(self, text="✕", bg=bg,
                                        fg=theme.TEXT_FAINT, cursor="hand2",
                                        takefocus=0,
                                        font=self.fonts.get("body"),
                                        padx=self.sizes["pad_sm"])
            self.clear_label.pack(side=tk.LEFT)
            self.clear_label.bind("<Button-1>", lambda e: self._set(""))
        self._render()

    def get(self) -> str:
        return self._spec

    def set(self, spec):
        self._set(spec, notify=False)

    def _set(self, spec, notify=True):
        self._cancel()
        self._spec = self._normalize(spec)
        self._render()
        if notify and self.command:
            try:
                self.command(self._spec)
            except Exception:
                pass

    def _render(self, hint=None):
        if self._capturing:
            self.value_label.configure(text=hint or "按下组合键…",
                                       fg=theme.ACCENT, bg=theme.DARK)
        else:
            self.value_label.configure(
                text=self._spec or "未设置",
                fg=theme.TEXT if self._spec else theme.TEXT_FAINT,
                bg=theme.BASE)

    def _begin(self, _e=None):
        if self._capturing:
            return
        self._capturing = True
        self._mods = set()
        self._render("按下组合键…")
        self.value_label.focus_set()
        self._bind_ids = [
            self.value_label.bind("<KeyPress>", self._on_press, add="+"),
            self.value_label.bind("<KeyRelease>", self._on_release, add="+"),
        ]

    def _cancel(self):
        if not self._capturing:
            return
        self._capturing = False
        self._mods = set()
        for seq, fid in zip(("<KeyPress>", "<KeyRelease>"), self._bind_ids):
            if fid:
                try:
                    self.value_label.unbind(seq, fid)
                except Exception:
                    pass
        self._bind_ids = []
        self._render()

    def _on_press(self, e):
        ks = e.keysym
        if ks in ("Escape",):
            self._cancel()
            return "break"
        if ks in ("Delete", "BackSpace"):
            self._set("")
            return "break"
        mod = self._keysyms.get(ks.lower())
        if mod:
            self._mods.add(mod)
            order = [m for m in ("Ctrl", "Alt", "Shift", "Win")
                     if m in self._mods]
            self._render("+".join(order) + "+…")
            return "break"
        spec = self._to_spec(ks, self._mods)
        if not spec:
            self._render("需带 Ctrl/Alt/Shift（或 F1–F24）")
            return "break"
        self._set(spec)
        return "break"

    def _on_release(self, e):
        mod = self._keysyms.get(str(e.keysym).lower())
        if mod:
            self._mods.discard(mod)
        return None


class ScrollFrame(tk.Frame):
    """深色滚动容器：Canvas + 自绘右滚动条。"""

    def __init__(self, parent, sizes=None, fonts=None):
        self.sizes = sizes if sizes is not None else make_sizes(100)
        super().__init__(parent, bg=theme.WINDOW)
        self.canvas = tk.Canvas(self, bg=theme.WINDOW, bd=0,
                                highlightthickness=0)
        self.bar = tk.Frame(self, bg=theme.BUTTON,
                            width=self.sizes["scrollbar_w"])
        self.thumb = tk.Frame(self.bar, bg=theme.MID, bd=0)
        self.body = tk.Frame(self.canvas, bg=theme.WINDOW)

        self.bar.pack(side=tk.RIGHT, fill=tk.Y)
        self.canvas.pack(side=tk.LEFT, fill=tk.BOTH, expand=True)
        self._win = self.canvas.create_window(
            (0, 0), window=self.body, anchor="nw")
        self.body.bind("<Configure>", self._on_body_configure)
        self.canvas.bind("<Configure>", self._on_canvas_configure)
        self.canvas.configure(yscrollcommand=self._update_thumb)
        self.thumb.place(relx=0, rely=0, relwidth=1,
                         height=self.sizes["thumb_min"])

    def _on_body_configure(self, _e):
        self.canvas.configure(scrollregion=self.canvas.bbox("all"))
        # 内容缩到不满一屏时回到顶部，杜绝滚出空白
        try:
            if (self.body.winfo_reqheight()
                    <= self.canvas.winfo_height()):
                self.canvas.yview_moveto(0)
        except Exception:
            pass

    def _on_canvas_configure(self, e):
        self.canvas.itemconfig(self._win, width=e.width)

    def _update_thumb(self, first, last):
        try:
            h = self.bar.winfo_height()
            th = max(self.sizes["thumb_min"],
                     int(h * (float(last) - float(first))))
            y0 = max(0, min(int(h * float(first)), h - th))
            self.thumb.place_configure(height=th, y=y0)
            need = float(last) - float(first) < 0.999
            self.bar.pack_forget()
            if need:
                self.bar.pack(side=tk.RIGHT, fill=tk.Y)
        except Exception:
            pass

    def apply_sizes(self):
        self.bar.configure(width=self.sizes["scrollbar_w"])
