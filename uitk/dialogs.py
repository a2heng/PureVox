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

"""uitk 对话框：关于（文档标签页）/ EQ 编辑器 / 简易确认。

关于页大文本是真正的 markdown 文件（about/*.md），经 about_content.load_doc
按页加载；本模块保持零第三方 GUI 依赖（仅标准库 Tkinter）。
"""

import tkinter as tk
import math

from . import theme
from .metrics import make_sizes


class DarkDialog(tk.Toplevel):
    """深色无边框弹窗：自绘标题栏（可拖动）+ 方形关闭钮，无系统标题栏。"""

    def __init__(self, parent, title, w, h, sizes=None, fonts=None):
        super().__init__(parent, bg=theme.WINDOW)
        self.sizes = sizes or make_sizes(100)
        self.fonts = fonts or {}
        self.title(title)
        self.withdraw()               # 先藏窗避免白闪
        self.overrideredirect(True)   # 去系统标题栏，与主窗风格一致
        # 尺寸随缩放挡位放大
        s = self.sizes["scale"]
        w, h = int(w * s), int(h * s)
        self.geometry(f"{w}x{h}")
        self._dlgw, self._dlgh = w, h
        bar = tk.Frame(self, bg=theme.TITLE_BG, height=self.sizes["titlebar_h"])
        bar.pack(fill=tk.X)
        bar.pack_propagate(False)
        # 三边同色细边（与主窗一致，消除罐头瓶观感）
        bd_l = tk.Frame(self, bg=theme.TITLE_BG, width=2)
        bd_r = tk.Frame(self, bg=theme.TITLE_BG, width=2)
        bd_b = tk.Frame(self, bg=theme.TITLE_BG, height=2)
        bd_l.pack(side=tk.LEFT, fill=tk.Y)
        bd_r.pack(side=tk.RIGHT, fill=tk.Y)
        bd_b.pack(side=tk.BOTTOM, fill=tk.X)
        lbl = tk.Label(bar, text=title, bg=theme.TITLE_BG, fg=theme.TITLE_FG,
                       font=self.fonts.get("bold"))
        lbl.pack(side=tk.LEFT, padx=self.sizes["pad_md"])
        # 方形关闭钮（与主窗同款：外壳锁正方形，× 居中）
        tb = self.sizes["titlebar_h"]
        wrap = tk.Frame(bar, bg=theme.TITLE_BG, width=tb, height=tb)
        wrap.pack(side=tk.RIGHT)
        wrap.pack_propagate(False)
        x = tk.Label(wrap, text="×", bg=theme.TITLE_BG, fg=theme.TITLE_FG,
                     font=self.fonts.get("bold"), cursor="hand2")
        x.place(relx=0.5, rely=0.5, anchor="center")
        x.bind("<Button-1>", lambda e: self.destroy())
        x.bind("<Enter>", lambda e: (x.configure(bg=theme.STOP_BG, fg="#ffffff"),
                                     wrap.configure(bg=theme.STOP_BG)))
        x.bind("<Leave>", lambda e: (x.configure(bg=theme.TITLE_BG, fg=theme.TITLE_FG),
                                     wrap.configure(bg=theme.TITLE_BG)))
        # 标题栏整体/文字可拖动
        for wd in (bar, lbl):
            wd.bind("<ButtonPress-1>", self._drag_begin)
            wd.bind("<B1-Motion>", self._drag_move)
        self._tdx = self._tdy = 0
        self.body = tk.Frame(self, bg=theme.WINDOW)
        self.body.pack(fill=tk.BOTH, expand=True)
        self.transient(parent)
        # 弹出位置：主窗口居中（拿不到主窗几何时回退屏幕居中）
        self.update_idletasks()
        try:
            px, py = parent.winfo_rootx(), parent.winfo_rooty()
            pw, ph = parent.winfo_width(), parent.winfo_height()
            pos_x = px + max(0, (pw - self._dlgw) // 2)
            pos_y = py + max(0, (ph - self._dlgh) // 3)
        except Exception:
            pos_x = pos_y = 60
        self.geometry(f"+{pos_x}+{pos_y}")
        # 主窗可能处于 -topmost（托盘呼出），无边框子窗必须同样置顶才会
        # 显示在主窗之上，否则会被压在后面点不到。
        self.attributes("-topmost", True)
        self.deiconify()
        self.lift()
        self.focus_force()
        self.bind("<Escape>", lambda e: self.destroy())

    def _drag_begin(self, e):
        self._tdx, self._tdy = e.x, e.y

    def _drag_move(self, e):
        try:
            x = self.winfo_x() + e.x - self._tdx
            y = self.winfo_y() + e.y - self._tdy
            self.geometry(f"+{x}+{y}")
        except Exception:
            pass


def show_message(parent, title, message, sizes=None, fonts=None):
    """深色提示框（确定按钮）。

    必须用自定义 override-redirect 弹窗，不能用原生 messagebox：主窗是
    override-redirect，Mutter 会把受 WM 管理（含原生 messagebox）的窗口排在
    它下面，导致提示框被主窗盖住看不见；override 弹窗才能浮在其上。
    """
    from .widgets import FlatButton
    sizes = sizes or make_sizes(100)
    fonts = fonts or {}
    scale = sizes.get("scale", 1)
    text = str(message)
    w = 440
    # 依换行 + 估算折行行数定高
    lines = text.count("\n") + 1 + len(text) // 44
    h = min(360, 110 + lines * 22)
    dlg = DarkDialog(parent, title, w, h, sizes=sizes, fonts=fonts)
    tk.Label(dlg.body, text=text, bg=theme.WINDOW, fg=theme.TEXT,
             font=fonts.get("body"), justify="left", anchor="w",
             wraplength=max(280, int(w * scale) - 40)).pack(
        fill=tk.BOTH, expand=True, padx=16, pady=(14, 4))
    bar = tk.Frame(dlg.body, bg=theme.WINDOW)
    bar.pack(fill=tk.X, pady=(0, 12))
    FlatButton(bar, "确定", sizes=sizes, command=dlg.destroy).pack(
        side=tk.RIGHT, padx=16)
    dlg.bind("<Return>", lambda e: dlg.destroy())
    return dlg


def _md_to_text_widget(parent, md_text, fonts):
    """极简 markdown → Text 控件：#/##/### 标题、- 列表、**粗体**去星号、
    [文本](URL) 可点击链接。"""
    import re
    import webbrowser

    txt = tk.Text(parent, bg=theme.BASE, fg=theme.TEXT, bd=0,
                  wrap="word", padx=12, pady=10, cursor="arrow",
                  font=fonts.get("body"))
    bar_bg = theme.DARK
    txt.configure(selectbackground=bar_bg)
    txt.tag_configure("h1", font=fonts.get("title"),
                      foreground=theme.ACCENT, spacing1=8, spacing3=4)
    txt.tag_configure("h2", font=fonts.get("bold"),
                      foreground=theme.TEXT, spacing1=10, spacing3=4)
    txt.tag_configure("li", lmargin1=14, lmargin2=14,
                      spacing3=2)
    txt.tag_configure("dim", foreground=theme.TEXT_DIM)
    txt.tag_configure("link", foreground=theme.ACCENT, underline=True)

    def _open_link(url):
        try:
            webbrowser.open(url)
        except Exception:
            pass

    link_re = re.compile(r'\[([^\]]+)\]\((https?://[^)\s]+)\)')
    link_seq = [0]

    def _insert_with_links(widget, text, base_tags):
        """按 [文本](URL) 切分插入；URL 部分挂可点击 tag。"""
        pos = 0
        for m in link_re.finditer(text):
            if m.start() > pos:
                widget.insert("end", text[pos:m.start()], base_tags)
            name = "lnk%d" % link_seq[0]
            link_seq[0] += 1
            widget.tag_configure(name, foreground=theme.ACCENT, underline=True)
            widget.tag_bind(name, "<Button-1>",
                            lambda e, u=m.group(2): _open_link(u))
            widget.insert("end", m.group(1), tuple(base_tags) + (name,))
            pos = m.end()
        if pos < len(text):
            widget.insert("end", text[pos:], base_tags)

    for raw in md_text.splitlines():
        line = raw.replace("**", "")
        if line.startswith("### "):
            txt.insert("end", line[4:] + "\n", "h2")
        elif line.startswith("## "):
            txt.insert("end", line[3:] + "\n", "h2")
        elif line.startswith("# "):
            txt.insert("end", line[2:] + "\n", "h1")
        elif line.startswith("- "):
            _insert_with_links(txt, line[2:], ("li",))
            txt.insert("end", "\n")
        elif line.startswith("> "):
            _insert_with_links(txt, line[2:], ("li", "dim"))
            txt.insert("end", "\n")
        elif line.strip().startswith("<"):
            continue    # 跳过 HTML 片段行
        elif line.strip():
            _insert_with_links(txt, line, ())
            txt.insert("end", "\n")
        else:
            txt.insert("end", "\n")
    txt.configure(state="disabled")
    return txt


def _scrollable_text(parent, md_text, fonts):
    """原生 Text + Scrollbar：宽度随窗口缩放（word wrap），无 canvas hack。"""
    frame = tk.Frame(parent, bg=theme.BASE)
    txt = _md_to_text_widget(frame, md_text, fonts)
    bar = tk.Scrollbar(frame, command=txt.yview,
                       troughcolor=theme.DARK, bg=theme.BUTTON,
                       activebackground=theme.ACCENT,
                       width=sizes_bar_width())
    txt.configure(yscrollcommand=bar.set)
    bar.pack(side=tk.RIGHT, fill=tk.Y)
    txt.pack(side=tk.LEFT, fill=tk.BOTH, expand=True)
    return frame


def sizes_bar_width():
    return 10


def show_about_dialog(parent, sizes=None, fonts=None):
    """关于：整页标签 —— 关于 / Windows 使用 / Linux 使用 / 更新日志 / 许可证。"""
    import about_content as about
    app_name = about.APP_NAME
    build = about.BUILD_DATE or "开发版"
    intro = about._INTRO_TEXT.replace("{BUILD_DATE}", str(build))
    dlg = DarkDialog(parent, "关于 %s" % app_name, 680, 620,
                     sizes=sizes, fonts=fonts)
    dlg.minsize(480, 380)
    # 允许拉伸：body/canvas/Text 全部 fill+expand，文本框跟随窗口
    dlg.body.pack_configure(fill=tk.BOTH, expand=True)
    tabs = tk.Frame(dlg.body, bg=theme.WINDOW)
    tabs.pack(fill=tk.X)
    holder = tk.Frame(dlg.body, bg=theme.BASE)
    holder.pack(fill=tk.BOTH, expand=True)
    pages = [
        ("关于", intro),
        ("Windows 使用", about.load_doc("windows")),
        ("Linux 使用", about.load_doc("linux")),
        ("更新日志", about.load_doc("changelog")),
        ("许可证", about._LICENSE_TEXT),
    ]
    cur = [None]

    def show(idx):
        if cur[0] == idx:
            return
        cur[0] = idx
        for w in holder.winfo_children():
            w.destroy()
        f = _scrollable_text(holder, pages[idx][1], fonts or {})
        f.pack(fill=tk.BOTH, expand=True)
        for i, b in enumerate(tab_btns):
            b.configure(bg=theme.DARK if i == idx else theme.WINDOW,
                        fg=theme.ACCENT if i == idx else theme.TEXT_DIM)

    tab_btns = []
    for i, (name, _t) in enumerate(pages):
        b = tk.Label(tabs, text=name, bg=theme.WINDOW, fg=theme.TEXT_DIM,
                     font=(fonts or {}).get("bold"), padx=10,
                     pady=sizes["pad_sm"] if sizes else 4, cursor="hand2")
        b.pack(side=tk.LEFT)
        b.bind("<Button-1>", lambda e, i=i: show(i))
        tab_btns.append(b)
    show(0)


# ── EQ 编辑器：真实频点 Canvas（引擎单一来源，10/31/61 段共用）+ 高切/低切 ──
from pvengine.components.eq import EQ_FREQS as _EQ_FREQS, EQ_Q as _EQ_Q
from pvengine.components.eq import response_at as _eq_response_at

HP_DEFAULT_HZ = 80.0     # 低切（高通）默认截止
LP_DEFAULT_HZ = 16000.0  # 高切（低通）默认截止

# 预设：{频点: dB} 稀疏定义（键为标准频点；展开时按各规格栅格匹配，
# 栅格里没有的频点自动跳过——同一套预设适配 10/31/61 三种段数）
_PRESETS_SPARSE = {
    "平直": {},
    "低音增强": {63.0: 4.0, 125.0: 3.0, 250.0: 1.5},
    "人声增强": {125.0: -1.5, 250.0: -1.0, 1000.0: 2.0, 2500.0: 2.5, 4000.0: 1.5},
    "高音增强": {8000.0: 2.0, 12500.0: 3.0, 16000.0: 3.0},
}


def _expand_preset(sparse, freqs=None):
    if freqs is None:
        freqs = _EQ_FREQS
    out = []
    for f in freqs:
        g = 0.0
        for bf, bg in sparse.items():
            if abs(math.log10(bf) - math.log10(f)) < 1e-9:
                g = bg
                break
        out.append(g)
    return out


class EQCurveCanvas(tk.Canvas):
    """真实频点响应曲线（栅格可配：10/31/61 段），拖拽/滚轮直接调
    对应频段；高切/低切虚线标记。"""

    Y_LIMIT = 15

    def __init__(self, parent, gains, filters=None, on_change=None,
                 sizes=None, fonts=None, freqs=None, q=0.0):
        self.sizes = sizes or make_sizes(100)
        self.fonts = fonts or {}
        self.on_change = on_change
        self._freqs = tuple(freqs) if freqs is not None else _EQ_FREQS
        self._q = float(q) if q > 0.0 else _EQ_Q
        self._gains = list(gains)
        if len(self._gains) != len(self._freqs):
            self._gains = [0.0] * len(self._freqs)
        self._drag_idx = None
        s = self.sizes["scale"]
        # 线宽/手柄/字号随挡位缩放
        self._lw = max(2, int(round(2 * s)))
        self._hs = max(2, int(round(3 * s)))          # 手柄半边长
        self._axis_font = ("TkDefaultFont", max(9, int(round(9 * s))))
        self._tick_font = ("TkDefaultFont", max(7, int(round(7 * s))))
        super().__init__(parent, bg="#FFFFFF", highlightthickness=0,
                         bd=0, cursor="hand2")
        self._hp = [False, HP_DEFAULT_HZ]
        self._lp = [False, LP_DEFAULT_HZ]
        if filters:
            self.set_filters(*filters)
        self.bind("<Configure>", lambda e: self.redraw())
        self.bind("<ButtonPress-1>", self._press)
        self.bind("<B1-Motion>", self._motion)
        self.bind("<ButtonRelease-1>", lambda e: setattr(self, "_drag_idx", None))
        self.bind("<MouseWheel>", self._wheel)
        self.bind("<Button-4>", self._wheel)
        self.bind("<Button-5>", self._wheel)

    def set_filters(self, hp_on, hp_hz, lp_on, lp_hz):
        self._hp = [bool(hp_on), float(hp_hz)]
        self._lp = [bool(lp_on), float(lp_hz)]
        self.redraw()

    def get_filters(self):
        return (bool(self._hp[0]), float(self._hp[1]),
                bool(self._lp[0]), float(self._lp[1]))

    # ── 几何 ──
    def _geom(self):
        w = max(self.winfo_width(), 120)
        h = max(self.winfo_height(), 80)
        L, R, T, B = 26, 12, 8, 18
        return w, h, L, R, T, B

    def _x_of_band(self, i, w, L, gw):
        lo, hi = math.log10(self._freqs[0]), math.log10(self._freqs[-1])
        return L + (math.log10(self._freqs[i]) - lo) / (hi - lo) * gw

    def _x_of_freq(self, f, w, L, gw):
        lo, hi = math.log10(self._freqs[0]), math.log10(self._freqs[-1])
        u = min(max(math.log10(f), lo), hi)
        return L + (u - lo) / (hi - lo) * gw

    def _y_of_gain(self, g, T, gh):
        # ±15dB 满幅
        return T + gh / 2 - (g / float(self.Y_LIMIT)) * (gh / 2)

    def _gain_at_y(self, y, T, gh):
        g = (1 - (y - T) / gh) * 2 * self.Y_LIMIT - self.Y_LIMIT
        return max(-self.Y_LIMIT, min(self.Y_LIMIT, round(g)))

    def _band_at_x(self, x, w, L, gw):
        lo, hi = math.log10(self._freqs[0]), math.log10(self._freqs[-1])
        u = min(max(x - L, 0.0), gw)
        target = lo + (u / gw) * (hi - lo)
        best, bd = 0, 1e9
        for i, f in enumerate(self._freqs):
            d = abs(math.log10(f) - target)
            if d < bd:
                best, bd = i, d
        return best

    # ── 绘制 ──
    def redraw(self):
        w, h, L, R, T, B = self._geom()
        gw, gh = w - L - R, h - T - B
        n = len(self._freqs)
        self.delete("all")
        # 网格
        for db in (-15, -10, -5, 0, 5, 10, 15):
            y = self._y_of_gain(db, T, gh)
            solid = db == 0
            self.create_line(L, y, L + gw, y,
                             fill=theme.MID if solid else "#EEE3CB")
            self.create_text(L - 4, y, text=f"{db:+d}" if db else "0",
                             anchor="e", fill=theme.TEXT_FAINT,
                             font=self._tick_font)
        label_step = 5
        for i in range(n):
            x = self._x_of_band(i, w, L, gw)
            if i % label_step == 0:
                f = self._freqs[i]
                lbl = f"{round(f / 1000)}k" if f >= 10000 \
                    else (f"{f / 1000:g}k" if f >= 1000 else f"{int(f)}")
                self.create_text(x, T + gh + 2, text=lbl, anchor="n",
                                 fill=theme.TEXT_FAINT,
                                 font=self._tick_font)
        # 高切/低切截止虚线
        for on, hz in ((self._hp[0], self._hp[1]), (self._lp[0], self._lp[1])):
            if not on or not (self._freqs[0] <= hz <= self._freqs[-1]):
                continue
            x = self._x_of_freq(hz, w, L, gw)
            self.create_line(x, T, x, T + gh,
                             fill=theme.MID, dash=(4, 3))
        # 响应曲线：引擎 response_at() 单一来源（含高/低切，按本规格栅格与 Q）；
        # 限幅在 ±Y_LIMIT 内——越界会画出绘图区（压过轴标/边框）
        pts = []
        for k in range(160):
            u = k / 159.0
            freq = (self._freqs[0]) * ((self._freqs[-1] / self._freqs[0]) ** u)
            hp = self._hp[1] if self._hp[0] else 0.0
            lp = self._lp[1] if self._lp[0] else 0.0
            dbv = max(-float(self.Y_LIMIT),
                      min(float(self.Y_LIMIT),
                          _eq_response_at(freq, self._gains, hp_hz=hp, lp_hz=lp,
                                          freqs=self._freqs, q=self._q)))
            pts.append((L + u * gw, self._y_of_gain(dbv, T, gh)))
        flat = [c for p in pts for c in p]
        if len(flat) >= 4:
            self.create_line(*flat, fill=theme.ACCENT, width=self._lw)
        # 频段手柄：正方形像素点（按增益着色深浅）
        s = self._hs
        for i in range(n):
            x = self._x_of_band(i, w, L, gw)
            y = self._y_of_gain(self._gains[i], T, gh)
            active = i == getattr(self, "_drag_idx", None)
            fill = theme.START_BG if active else (
                theme.ACCENT if abs(self._gains[i]) > 1e-9 else theme.TRACK)
            self.create_rectangle(x - s, y - s, x + s, y + s,
                                  fill=fill,
                                  outline=theme.MID, width=1)

    # ── 交互 ──
    def _press(self, e):
        w, h, L, R, T, B = self._geom()
        self._drag_idx = self._band_at_x(e.x, w, L, w - L - R)
        self._apply_y(e.y)

    def _motion(self, e):
        if self._drag_idx is None:
            return
        self._apply_y(e.y)

    def _apply_y(self, y):
        _, _, _, _, T, Bm = self._geom()
        gh = self.winfo_height() - T - Bm
        self._gains[self._drag_idx] = self._gain_at_y(y, T, gh)
        self.redraw()
        if self.on_change:
            try:
                self.on_change(list(self._gains))
            except Exception:
                pass

    def _wheel(self, e):
        import sys as _s
        d = int(-e.delta / 120) if _s.platform.startswith("win") else (
            -1 if getattr(e, "num", 0) == 4 else
            1 if getattr(e, "num", 0) == 5 else 0)
        if not d:
            return
        w, h, L, R, T, B = self._geom()
        i = self._band_at_x(e.x, w, L, w - L - R)
        self._gains[i] = max(-self.Y_LIMIT, min(self.Y_LIMIT, self._gains[i] + d))
        self.redraw()
        if self.on_change:
            try:
                self.on_change(list(self._gains))
            except Exception:
                pass

    # ── 外部接口 ──
    def get_gains(self):
        return list(self._gains)

    def set_gains(self, gains):
        self._gains = list(gains)
        if len(self._gains) != len(self._freqs):
            self._gains = [0.0] * len(self._freqs)
        self.redraw()


def open_eq_editor(parent, freqs, q, get_gains, set_gains, sizes=None,
                   fonts=None, get_filters=None, set_filters=None):
    """均衡器编辑器：真实频点直接拖拽（栅格随插件规格 10/31/61 段）；
    高切/低切复选框 + 截止频率。"""
    dlg = DarkDialog(parent, "均衡器", 560, 430, sizes=sizes, fonts=fonts)
    cur = list(get_gains())
    if len(cur) != len(freqs):
        cur = [0.0] * len(freqs)
    filters0 = tuple(get_filters()) if get_filters else \
        (False, HP_DEFAULT_HZ, False, LP_DEFAULT_HZ)

    curve = EQCurveCanvas(dlg.body, cur, filters=filters0,
                          on_change=lambda v: set_gains(list(v)),
                          sizes=sizes, fonts=fonts, freqs=freqs, q=q)
    curve.pack(fill=tk.BOTH, expand=True, padx=8, pady=(4, 0))

    # ── 高切/低切控制行 ──
    row = tk.Frame(dlg.body, bg=theme.WINDOW)
    row.pack(fill=tk.X, padx=10, pady=(0, 2))
    hp_var = tk.BooleanVar(value=bool(filters0[0]))
    lp_var = tk.BooleanVar(value=bool(filters0[2]))
    hp_hz_var = tk.DoubleVar(value=float(filters0[1]))
    lp_hz_var = tk.DoubleVar(value=float(filters0[3]))

    def push_filters(*_a):
        vals = (bool(hp_var.get()), float(hp_hz_var.get()),
                bool(lp_var.get()), float(lp_hz_var.get()))
        curve.set_filters(*vals)
        if set_filters:
            try:
                set_filters(*vals)
            except Exception:
                pass

    def _cut_block(var_on, var_hz, lo, hi, label):
        box = tk.Frame(row, bg=theme.WINDOW)
        box.pack(side=tk.LEFT, padx=(0, 14))
        tk.Checkbutton(box, text=label, variable=var_on, command=push_filters,
                       bg=theme.WINDOW, fg=theme.TEXT,
                       activebackground=theme.WINDOW, highlightthickness=0,
                       font=(fonts or {}).get("small")).pack(side=tk.LEFT)
        tk.Scale(box, variable=var_hz, from_=lo, to=hi, resolution=10,
                 orient=tk.HORIZONTAL, length=150, showvalue=True,
                 command=push_filters, bg=theme.WINDOW, fg=theme.TEXT_DIM,
                 troughcolor=theme.TRACK, highlightthickness=0,
                 bd=0, font=(fonts or {}).get("small")).pack(side=tk.LEFT)

    _cut_block(hp_var, hp_hz_var, 20, 1000, "低切")
    _cut_block(lp_var, lp_hz_var, 1000, 20000, "高切")

    # ── 预设行（按本规格栅格展开；栅格没有的频点自动跳过）──
    prow = tk.Frame(dlg.body, bg=theme.WINDOW)
    prow.pack(fill=tk.X, padx=10, pady=(0, 8))
    for name, sparse in _PRESETS_SPARSE.items():
        vals = _expand_preset(sparse, freqs)
        b = tk.Label(prow, text=name, bg=theme.BUTTON, fg=theme.TEXT,
                     font=(fonts or {}).get("small"), padx=8, pady=2,
                     cursor="hand2")
        b.pack(side=tk.LEFT, padx=2)
        b.bind("<Button-1>",
               lambda e, vs=vals: (curve.set_gains(vs), set_gains(vs)))
        b.bind("<Enter>", lambda e, w=b: w.configure(bg=theme.DARK))
        b.bind("<Leave>", lambda e, w=b: w.configure(bg=theme.BUTTON))


# ── 快捷键与提示音 ──

def open_hotkey_dialog(parent, get_toggle, set_toggle,
                       get_cue, set_cue, get_cue_enabled,
                       set_cue_enabled, sizes=None, fonts=None):
    """全局快捷键 + 启停提示音设置（音效板快捷键在音效行内直接录制）。

    - 热键字段点一下即进入录制，按下组合键完成；Delete 清除；空串 = 不监听。
    - 提示音：独立复选框控制总开关；启动/停止各自下拉选预设，选中即试听
      （写配置后立即生效）。
    所有写操作经回调交回主窗（持久化 + 重注册全局热键）。
    """
    from .widgets import FlatButton, HotkeyField, DarkCheck, DarkCombo
    from .cues import play as play_cue
    from pvengine.cues import PRESETS

    S = sizes or make_sizes(100)
    F = fonts or {}
    dlg = DarkDialog(parent, "快捷键与提示音", 420, 300, sizes=S, fonts=F)
    body = dlg.body

    def section(text):
        tk.Label(body, text=text, bg=theme.WINDOW, fg=theme.ACCENT,
                 font=F.get("bold"), anchor="w").pack(
            fill=tk.X, padx=14, pady=(10, 2))

    def hint(text):
        tk.Label(body, text=text, bg=theme.WINDOW, fg=theme.TEXT_FAINT,
                 font=F.get("small"), anchor="w", justify="left").pack(
            fill=tk.X, padx=14)

    # ── 启停 ──
    section("启动 / 停止音频处理")
    row = tk.Frame(body, bg=theme.WINDOW)
    row.pack(fill=tk.X, padx=14, pady=(2, 0))
    HotkeyField(row, spec=get_toggle(), command=set_toggle,
                sizes=S, fonts=F).pack(side=tk.LEFT)
    hint("点击方框后按下组合键；Delete 清除（清空 = 不监听）")

    # ── 提示音 ──
    section("启停提示音")
    cue_values = [(label, pid) for pid, label in PRESETS]
    from .widgets import DarkCombo
    on_var = tk.BooleanVar(value=bool(get_cue_enabled()))

    def toggle_cues():
        set_cue_enabled(bool(on_var.get()))
    check = DarkCheck(body, "启用启停提示音", on_var, command=toggle_cues,
                      sizes=S, fonts=F)
    check.pack(anchor="w", padx=14, pady=(2, 4))

    def cue_row(kind, label):
        r = tk.Frame(body, bg=theme.WINDOW)
        r.pack(fill=tk.X, padx=14, pady=2)
        tk.Label(r, text=label, bg=theme.WINDOW, fg=theme.TEXT,
                 font=F.get("body"), width=6, anchor="w").pack(side=tk.LEFT)
        var = tk.StringVar(value=get_cue(kind))

        def on_change(*_a):
            set_cue(kind, var.get())
            play_cue(var.get(), kind)
        DarkCombo(r, cue_values, var, on_change=on_change,
                  sizes=S, fonts=F).pack(side=tk.LEFT, padx=(3, 6))

        def preview():
            play_cue(var.get(), kind)
        FlatButton(r, "试听", command=preview, font=F.get("body"),
                   sizes=S).pack(side=tk.LEFT)
    cue_row("start", "启动音")
    cue_row("stop", "停止音")
    hint("复选框＝总开关；「试听」始终可听。提示音从系统默认输出设备播放，"
         "与音频处理的输出设备无关。")

    return dlg


# ── TSE 参考录音 ──

def open_tse_dialog(parent, engine, config, sizes=None, fonts=None):
    """TSE 参考音频：显示当前参考状态；运行中可录 10s 参考并即时生效。

    依赖引擎已启动（recording_hook 由处理线程喂采样）。
    """
    import os
    import time as _t
    from audio_processor import (get_tse_recorder, RECORD_DURATION,
                                 _samples_to_wav_bytes, load_tse_reference,
                                 CFG_REF_WAV_PATH)
    from user_paths import WAV_PATH

    dlg = DarkDialog(parent, "目标说话人 TSE · 参考音频", 380, 200,
                     sizes=sizes, fonts=fonts)
    info = tk.Label(dlg.body, text="", bg=theme.WINDOW, fg=theme.TEXT_DIM,
                    font=(fonts or {}).get("body"), justify="left",
                    anchor="w")
    info.pack(fill=tk.X, padx=14, pady=(10, 4))
    status_lbl = tk.Label(dlg.body, text="", bg=theme.WINDOW,
                          fg=theme.ACCENT, font=(fonts or {}).get("bold"))
    status_lbl.pack(fill=tk.X, padx=14)

    wav = config.get(CFG_REF_WAV_PATH, "") if config else ""
    if wav and os.path.exists(wav):
        kb = os.path.getsize(wav) / 1024
        mt = _t.strftime("%Y-%m-%d %H:%M", _t.localtime(os.path.getmtime(wav)))
        info.configure(text=f"已有参考：{os.path.basename(wav)}\n"
                            f"{kb:.0f} KB · {mt}")
    else:
        info.configure(text="尚无参考音频——TSE 插件将直通。\n"
                            "启动音频处理后点「开始录音」，对麦克风说 10 秒话。")

    recording = [False]

    def do_record():
        th = engine.thread
        if th is None or not engine.running:
            show_message(parent, "PureVox", "请先启动音频处理，再录制参考。",
                         sizes=sizes, fonts=fonts)
            return
        rec = get_tse_recorder()
        rec.start()   # 打开 _active 门（feed/wait_and_get 均由此 gate，缺失即"未捕获到音频"）
        th.set_recording_hook(lambda s: rec.feed(list(s)))
        th.set_recording_enabled(True)
        recording[0] = True
        deadline = [RECORD_DURATION]

        def tick():
            if not recording[0]:
                return
            if deadline[0] > 0:
                status_lbl.configure(
                    text=f"录音中… {deadline[0]:.0f}s（请持续说话）")
                deadline[0] -= 1
                dlg.after(1000, tick)
                return
            finish()

        def finish():
            recording[0] = False
            try:
                th.set_recording_enabled(False)
            except Exception:
                pass
            raw = get_tse_recorder().wait_and_get()
            if not raw:
                status_lbl.configure(text="录音失败（未捕获到音频）")
                return
            try:
                with open(WAV_PATH, "wb") as f:
                    f.write(_samples_to_wav_bytes(raw))
            except Exception as e:
                status_lbl.configure(text=f"保存失败: {e}")
                return
            if config:
                config.set(CFG_REF_WAV_PATH, WAV_PATH)
                config.save_config()
            proc = engine.processor
            ok = load_tse_reference(proc, WAV_PATH) if proc else False
            status_lbl.configure(
                text="完成！参考已生效。" if ok
                else "已保存，但加载失败——重启音频处理后生效。")

        tick()

    btn_row = tk.Frame(dlg.body, bg=theme.WINDOW)
    btn_row.pack(fill=tk.X, padx=14, pady=8)
    rec_btn = tk.Label(btn_row, text="● 开始录音 (10s)", bg=theme.STOP_BG,
                       fg=theme.ACCENT_TEXT, font=(fonts or {}).get("bold"),
                       padx=12, pady=sizes["pad_sm"] if sizes else 4,
                       cursor="hand2")
    rec_btn.pack(side=tk.LEFT)
    rec_btn.bind("<Button-1>", lambda e: None if recording[0] else do_record())

