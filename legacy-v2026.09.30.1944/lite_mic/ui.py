# PureVox Lite Denoise Only — Tk UI 黑底白字 纯tk无ttk
# Copyright (C) 2024-2026 a2heng <752848283@qq.com>
# SPDX-License-Identifier: GPL-3.0-or-later

import tkinter as tk
import tkinter.font as tkfont
from tkinter import messagebox
import subprocess
import sys
import os

# 星露谷物语像素风配色
BG = "#FFF8E1"          # 羊皮纸底
PANEL_BG = "#FFECB3"    # 面板
FG = "#5D4037"          # 深棕文字
BTN_BG = "#FFB74D"      # 南瓜橙按钮
BTN_BG2 = "#81C784"     # 田野绿按钮
ENTRY_BG = "#FFFFFF"    # 输入白底
BORDER = "#8D6E63"      # 木纹边框
HOVER_BG = "#FFE0B2"    # 悬停浅橙
SELECT_BG = "#FFCC80"   # 选中橙
TITLE_BG = "#6D4C41"    # 标题深棕
TITLE_FG = "#FFF8E1"

# 像素风格中英文免费开源字体（Ark Pixel 12px Mono，自带，不依赖系统）
PIXEL_FONTS = ["Ark Pixel 12px Mono zh_cn", "Ark Pixel 12px Mono", "Microsoft YaHei"]
PIXEL_FONT = "Microsoft YaHei"

# ---------------------------------------------------------------------------
# 分辨率自动挡位：屏幕等效高度跨过门槛升一档，所有组件共用同一倍率。
# 基准档 100% 对应 1080P；宽度按 16:9 折算成等效高度参与判断（兼容异形屏）。
# 手动百分比（托盘菜单）可覆盖，选「自动」恢复按分辨率定挡。
# ---------------------------------------------------------------------------
RES_GEARS = [
    (0,    85),   # ≤ HD 768
    (801,  95),   # ≤ 900
    (951,  100),  # ≤ 1080（基准档）
    (1151, 110),  # ≤ 1200
    (1351, 125),  # ≤ 2K 1440
    (1651, 145),  # ≤ 1650
    (2001, 175),  # 4K 2160
]

def detect_zoom_for_screen(w, h):
    eff = max(int(h), int(w * 9 / 16))
    z = RES_GEARS[0][1]
    for th, pct in RES_GEARS:
        if eff >= th:
            z = pct
    return z

def clamp_zoom(percent):
    try:
        p = int(percent)
    except Exception:
        p = 100
    return max(RES_GEARS[0][1], min(RES_GEARS[-1][1], p))

def make_sizes(zoom):
    """一个挡位一套 px：所有组件尺寸/间距/字号的唯一来源（Tk 负数字号 = 像素）。"""
    s = zoom / 100.0

    def r(v):
        return max(1, int(round(v * s)))

    return {
        "scale": s,
        # 字号（px）
        "font_body": r(13),
        "font_small": r(11),
        "font_title": r(15),
        # 控件目标高（px）
        "ctl_h": r(26),          # 按钮/输入框统一高
        "entry_w": r(64),        # 增益输入框宽（外壳锁定，防 propagate 关闭后塌缩）
        "combo_h": r(28),        # 下拉框内行高
        "titlebar_h": r(34),
        "check_box": r(16),
        "scrollbar_w": r(10),
        "thumb_min": r(14),
        # 弹出列表
        "popup_rows": 6,
        "popup_min_w": r(260),
        "popup_max_w": r(560),
        # 间距
        "pad_sm": r(4),
        "pad_md": r(6),
        "pad_lg": r(10),
        # 窗口基准（已含倍率）
        "win_w": r(430),
        "win_h": r(250),
    }

def _enable_hidpi():
    # 声明 PerMonitor DPI Aware 避免系统位图拉伸发糊；缩放完全由 tk scaling 自管
    try:
        if sys.platform.startswith("win"):
            import ctypes
            # 必须在创建窗口前声明，否则已糊；此处尽早声明，失败回退
            try:
                # 2 = Per Monitor DPI Aware (Win 8.1+)，最清晰
                ctypes.windll.shcore.SetProcessDpiAwareness(2)
            except Exception:
                try:
                    # Win10 1703+ PerMonitorV2
                    ctypes.windll.user32.SetProcessDpiAwarenessContext(ctypes.c_void_p(-4))
                except Exception:
                    try:
                        ctypes.windll.user32.SetProcessDPIAware()
                    except Exception:
                        pass
    except Exception:
        pass

def _load_pixel_font():
    try:
        # 字体唯一副本在仓库 assets/fonts/；定位用主线同一份多根探测
        # （打包态 _MEIPASS/assets/fonts ← add-data assets/fonts 落点）
        from uitk.metrics import find_pixel_font_ttf
        ttf = find_pixel_font_ttf()
        if not ttf:
            return
        if sys.platform.startswith("win"):
            import ctypes
            ctypes.windll.gdi32.AddFontResourceExW(ttf, 0x10, 0)
            # 异步广播（SendNotifyMessage）：同步 SendMessage 会被某个
            # 不泵消息的顶层窗口永久挂住，冻结态启动即卡死在字体注册
            ctypes.windll.user32.SendNotifyMessageW(0xFFFF, 0x001D, 0, 0)
        else:
            # Linux/macOS：fontconfig 用户字体目录注册（无需 root）
            from uitk.metrics import install_fonts_fontconfig
            install_fonts_fontconfig(os.path.dirname(ttf))
    except Exception:
        pass

def _pick_pixel_font(root):
    global PIXEL_FONT
    _load_pixel_font()
    try:
        avail = set(root.tk.call("font", "families"))
    except Exception:
        avail = set()
    for name in PIXEL_FONTS:
        if name in avail:
            PIXEL_FONT = name
            return name
    PIXEL_FONT = "Microsoft YaHei"
    return PIXEL_FONT

class BlackCombo(tk.Frame):
    """星露谷像素风下拉，单行，过滤空行避免计算错位"""
    def __init__(self, parent, values, var, on_change, props_map=None, sizes=None, fonts=None):
        super().__init__(parent, bg=BG)
        self.var = var
        # 过滤空字符串/空白，避免输入空行导致显示空白与索引错位
        self.values = [v for v in list(values) if v and str(v).strip()]
        self.props_map = props_map or {}
        self.on_change = on_change
        self._popup = None
        # 共享尺寸表与命名字体（LiteUI 统一原地更新，本组件只读）
        self.sizes = sizes if sizes is not None else make_sizes(100)
        self.fonts = fonts if fonts is not None else {}
        self.configure(bg=BORDER, bd=0, padx=2, pady=2)
        inner = tk.Frame(self, bg=ENTRY_BG)
        self.inner = inner
        inner.pack(fill=tk.BOTH, expand=True)
        self._display = tk.StringVar()
        self.var.trace_add("write", lambda *a: self._sync_display())
        self._sync_display()
        # 显示区按像素自适应，已放宽省略
        self.btn = tk.Label(inner, textvariable=self._display, bg=ENTRY_BG, fg=FG, anchor="w", padx=self.sizes["pad_md"], font=self.fonts.get("body"))
        self.btn.pack(side=tk.LEFT, fill=tk.BOTH, expand=True)
        self.arrow = tk.Label(inner, text="▾", bg=BTN_BG, fg=TITLE_BG, font=self.fonts.get("bold"), width=2, anchor="center", padx=0, pady=self.sizes["pad_sm"], bd=1, relief=tk.RAISED)
        self.arrow.pack(side=tk.RIGHT, fill=tk.Y)
        inner.bind("<Configure>", lambda e: self._sync_display())
        inner.pack_propagate(False)
        inner.configure(height=self.sizes["combo_h"])
        for w in (self, inner, self.btn, self.arrow):
            w.bind("<Button-1>", lambda e: self._toggle())
        if var.get() not in self.values and self.values:
            var.set(self.values[0])

    def _elide(self, v, avail):
        # 按像素宽度省略：超宽即逐字截断加 …，文字再长也只截断自己
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
        # 可用宽 = 内框宽 − 箭头宽 − 边距，保证右侧箭头永不被挤出框外
        try:
            avail = (self.inner.winfo_width()
                     - self.arrow.winfo_reqwidth()
                     - 2 * (self.sizes["pad_md"] + 2))
            v = self._elide(v, avail)
        except Exception:
            pass
        self._display.set(v)

    def _toggle(self):
        if self._popup and self._popup.winfo_exists():
            self._close()
        else:
            self._open()

    def set_values(self, values, props_map=None):
        self.values = [v for v in list(values) if v and str(v).strip()]
        if props_map is not None:
            self.props_map = props_map
        # 变量为空或不在列表时回退首项，避免空行选中
        if (not self.var.get() or self.var.get().strip() == "" or self.var.get() not in self.values) and self.values:
            self.var.set(self.values[0])

    def apply_sizes(self):
        # 尺寸表已由 LiteUI 原地更新，这里把新 px 推到本组件
        try:
            self.inner.configure(height=self.sizes["combo_h"])
            self.btn.configure(padx=self.sizes["pad_md"], font=self.fonts.get("body"))
            self.arrow.configure(pady=self.sizes["pad_sm"], font=self.fonts.get("bold"))
        except Exception:
            pass

    def _open(self):
        if not self.values or (self._popup and self._popup.winfo_exists()):
            return
        x = self.winfo_rootx()
        y = self.winfo_rooty() + self.winfo_height()
        S = self.sizes
        # 弹层与外框严格同宽：不做自动扩宽，长项由 _elide 按像素截断
        pw = max(self.winfo_width(), 60)
        # 行内文字可用宽 = 弹层宽 − 滚动条 − 行内边距
        avail_item = pw - S["scrollbar_w"] - 2 * S["pad_md"] - 8
        self._popup = tk.Toplevel(self)
        self._popup.overrideredirect(True)
        self._popup.configure(bg=BORDER)
        self._popup.attributes("-topmost", True)
        outer = tk.Frame(self._popup, bg=BORDER, bd=1)
        outer.pack(fill=tk.BOTH, expand=True)
        # 滚轮一格一设备，行高/滚动步长全部来自尺寸表
        row_h = S["combo_h"]
        canvas = tk.Canvas(outer, bg=ENTRY_BG, bd=0, highlightthickness=0, yscrollincrement=row_h + 2)
        bar = tk.Frame(outer, bg=PANEL_BG, width=S["scrollbar_w"], bd=1, relief=tk.FLAT, highlightbackground=BORDER, highlightthickness=1)
        bar.pack(side=tk.RIGHT, fill=tk.Y, padx=(1, 0))
        thumb = tk.Frame(bar, bg=BORDER, bd=0)
        thumb.place(relx=0, rely=0, relwidth=1, height=S["thumb_min"])
        canvas.configure(yscrollcommand=lambda *a: _update_thumb(*a))
        # 标准进度：thumb 高= h*可见/总数，y= h*first
        def _update_thumb(first, last):
            try:
                h = bar.winfo_height() or outer.winfo_height() or S["win_h"]
                th = max(S["thumb_min"], int(h * (float(last) - float(first))))
                y0 = int(h * float(first))
                # 保持在边界内
                th = min(th, h)
                y0 = max(0, min(y0, h - th))
                thumb.place_configure(height=th, y=y0)
            except Exception:
                pass
        canvas.pack(side=tk.LEFT, fill=tk.BOTH, expand=True)
        inner = tk.Frame(canvas, bg=ENTRY_BG)
        win_id = canvas.create_window((0, 0), window=inner, anchor="nw")
        def _sync_w(event=None):
            try:
                canvas.itemconfig(win_id, width=canvas.winfo_width())
            except Exception:
                pass
        canvas.bind("<Configure>", lambda e: _sync_w())
        # 单行显示，已去掉参数第二行，仅展示设备名
        for idx, disp in enumerate(self.values):
            is_sel = (disp == self.var.get())
            bgc = SELECT_BG if is_sel else ENTRY_BG
            item = tk.Frame(inner, bg=bgc, bd=0, height=row_h)
            item.pack(fill=tk.X, padx=1, pady=1)
            item.pack_propagate(False)
            l1 = tk.Label(item, text=self._elide(disp, avail_item), bg=bgc, fg=FG, anchor="w", font=self.fonts.get("body"))
            l1.pack(fill=tk.BOTH, expand=True, padx=S["pad_md"], pady=S["pad_sm"])
            for w in (item, l1):
                w.bind("<Button-1>", lambda e, i=idx: self._pick(i))
                w.bind("<Enter>", lambda e, f=item: self._hover(f, True))
                w.bind("<Leave>", lambda e, f=item, sel=is_sel: self._hover(f, False, sel))
        inner.update_idletasks()
        h = min(len(self.values), S["popup_rows"]) * (row_h + 2)
        canvas.configure(height=h)
        # 外层 bd=1，需补 2px 边框
        self._popup.geometry(f"{pw}x{h+2}+{x}+{y}")
        canvas.configure(scrollregion=canvas.bbox("all"))
        _update_thumb("0", "1")
        # 初始滚动到选中项，确保可见且不留空底
        try:
            idx = self.values.index(self.var.get())
            n = len(self.values)
            vis = S["popup_rows"]
            # 选中项尽量居中，尾部时贴底避免空行
            top = max(0, min(idx, n - vis)) / max(1, n) if n > vis else 0
            canvas.yview_moveto(top)
        except Exception:
            pass
        # 滚轮一格一设备
        def _wheel(e):
            if sys.platform.startswith("win"):
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
            try:
                h = bar.winfo_height()
                th = thumb.winfo_height()
                y0 = max(0, min(e.y - th//2, h - th))
                frac = y0 / max(1, h - th)
                # 转换为选中索引
                n = len(self.values)
                idx = int(frac * (n - 1) + 0.5)
                canvas.yview_moveto(idx / max(1, n))
                _update_thumb(*canvas.yview())
            except Exception:
                pass
        bar.bind("<Button-1>", _bar_click)
        thumb.bind("<B1-Motion>", lambda e: _bar_click(e))
        self._root_bind = self.winfo_toplevel().bind("<Button-1>", self._on_root, add="+")
        self._popup.bind("<Escape>", lambda e: self._close())
        self._popup.bind("<FocusOut>", lambda e: self.after(80, self._close))
        self._popup.focus_set()

    def _hover(self, frame, enter, sel=False):
        try:
            bgc = HOVER_BG if enter else (SELECT_BG if sel else ENTRY_BG)
            frame.configure(bg=bgc)
            for c in frame.winfo_children():
                c.configure(bg=bgc)
        except Exception:
            pass

    def _on_root(self, e):
        if not self._popup or not self._popup.winfo_exists():
            return
        try:
            px, py = self._popup.winfo_rootx(), self._popup.winfo_rooty()
            pw, ph = self._popup.winfo_width(), self._popup.winfo_height()
            if px <= e.x_root <= px+pw and py <= e.y_root <= py+ph:
                return
            sx, sy = self.winfo_rootx(), self.winfo_rooty()
            sw, sh = self.winfo_width(), self.winfo_height()
            if sx <= e.x_root <= sx+sw and sy <= e.y_root <= sy+sh:
                return
        except Exception:
            pass
        self._close()

    def _pick(self, idx):
        if 0 <= idx < len(self.values):
            self.var.set(self.values[idx])
            if self.on_change:
                self.on_change()
        self._close()

    def _close(self):
        try:
            if hasattr(self, "_root_bind"):
                self.winfo_toplevel().unbind("<Button-1>", self._root_bind)
        except Exception:
            pass
        if self._popup and self._popup.winfo_exists():
            try:
                self._popup.destroy()
            except Exception:
                pass
        self._popup = None


class PixelCheck(tk.Frame):
    """星露谷像素风复选框，纯 tk，匹配黑底白字/木纹边框风格，替代系统 Checkbutton"""
    def __init__(self, parent, text, variable, command=None, sizes=None, fonts=None):
        super().__init__(parent, bg=BG)
        self.variable = variable
        self.command = command
        # 共享尺寸表与命名字体（LiteUI 统一原地更新，本组件只读）
        self.sizes = sizes if sizes is not None else make_sizes(100)
        self.fonts = fonts if fonts is not None else {}
        # 方框：ENTRY_BG 未选中，SELECT_BG 选中，BORDER 边框
        sz = self.sizes["check_box"]
        self.box = tk.Frame(self, bg=ENTRY_BG, width=sz, height=sz, bd=0, highlightbackground=BORDER, highlightthickness=1, relief=tk.FLAT)
        self.box.pack(side=tk.LEFT)
        self.box.pack_propagate(False)
        self.mark = tk.Label(self.box, text="", bg=ENTRY_BG, fg=FG, font=self.fonts.get("bold"), anchor="center")
        self.mark.pack(expand=True, fill=tk.BOTH)
        self.label = tk.Label(self, text=text, bg=BG, fg=FG, font=self.fonts.get("body"))
        self.label.pack(side=tk.LEFT, padx=self.sizes["pad_md"])
        self._sync()
        try:
            variable.trace_add("write", lambda *a: self._sync())
        except Exception:
            pass
        for w in (self, self.box, self.mark, self.label):
            w.bind("<Button-1>", lambda e: self.toggle())
            w.bind("<Enter>", lambda e: self._on_hover(True))
            w.bind("<Leave>", lambda e: self._on_hover(False))
        self.box.bind("<Button-1>", lambda e: self.toggle())
        self.mark.bind("<Button-1>", lambda e: self.toggle())

    def _sync(self):
        on = bool(self.variable.get())
        bg = SELECT_BG if on else ENTRY_BG
        try:
            self.box.configure(bg=bg)
            self.mark.configure(bg=bg, text="✔" if on else "")
        except Exception:
            pass

    def toggle(self):
        try:
            self.variable.set(not bool(self.variable.get()))
        except Exception:
            pass
        self._sync()
        if self.command:
            try:
                self.command()
            except Exception:
                pass

    def _on_hover(self, enter):
        if bool(self.variable.get()):
            return
        bg = HOVER_BG if enter else ENTRY_BG
        try:
            self.box.configure(bg=bg)
            self.mark.configure(bg=bg)
        except Exception:
            pass

    def apply_sizes(self):
        # 尺寸表已由 LiteUI 原地更新，这里把新 px 推到本组件
        try:
            sz = self.sizes["check_box"]
            self.box.configure(width=sz, height=sz)
            self.mark.configure(font=self.fonts.get("bold"))
            self.label.configure(font=self.fonts.get("body"), padx=self.sizes["pad_md"])
        except Exception:
            pass


class LiteUI:
    def __init__(self, cfg, ins, outs, on_gain, on_device, on_autostart, on_close=None, on_minimize=None):
        self.cfg = cfg
        self.on_gain = on_gain
        self.on_device = on_device
        self.on_autostart = on_autostart
        self._on_close = on_close
        self._on_minimize = on_minimize

        # 创建 Tk 前声明 PerMonitor Aware，避免系统位图拉伸发糊
        _enable_hidpi()
        self.root = tk.Tk()
        # 全组件像素化（负数字号 + px 尺寸表），tk scaling 固定 1 不参与缩放
        try:
            self.root.tk.call("tk", "scaling", 1.0)
        except Exception:
            pass
        self.pixel_font = _pick_pixel_font(self.root)
        # 挡位：默认按分辨率自动定挡；用户手动选过百分比则用保存值
        if bool(cfg.get("auto_zoom", True)):
            self._zoom = detect_zoom_for_screen(self.root.winfo_screenwidth(), self.root.winfo_screenheight())
        else:
            self._zoom = clamp_zoom(cfg.get("zoom", 100))
        cfg["zoom"] = self._zoom
        # 共享尺寸表 + 命名字体：全部组件尺寸的唯一来源，换挡只改这里
        self.sizes = make_sizes(self._zoom)
        S = self.sizes
        self.fonts = {
            "body": tkfont.Font(family=self.pixel_font, size=-S["font_body"]),
            "bold": tkfont.Font(family=self.pixel_font, size=-S["font_body"], weight="bold"),
            "title": tkfont.Font(family=self.pixel_font, size=-S["font_title"], weight="bold"),
            "small": tkfont.Font(family=self.pixel_font, size=-S["font_small"]),
        }
        try:
            for fname in ("TkDefaultFont", "TkTextFont", "TkFixedFont", "TkMenuFont", "TkHeadingFont"):
                try:
                    tkfont.nametofont(fname).configure(family=self.pixel_font, size=-S["font_body"])
                except Exception:
                    pass
        except Exception:
            pass
        self.root.title("PureVox Lite")
        self.root.configure(bg=BG)
        # 窗口图标 = 仓库资产 assets/icons/lite_tray.png（Tk 8.6+ 原生 PNG）
        try:
            _meipass = getattr(sys, "_MEIPASS", None)
            _cands = []
            if _meipass:
                _cands.append(os.path.join(_meipass, "assets", "icons", "lite_tray.png"))
            _cands.append(os.path.normpath(os.path.join(
                os.path.dirname(__file__), "..", "assets", "icons", "lite_tray.png")))
            _png = next((c for c in _cands if os.path.isfile(c)), None)
            if _png:
                _photo = tk.PhotoImage(file=_png)
                self.root.iconphoto(False, _photo)
                self._icon_photo = _photo
        except Exception:
            pass
        self.root.overrideredirect(True)
        # 使无边框窗口仍显示在任务栏
        try:
            import ctypes
            hwnd = ctypes.windll.user32.GetParent(self.root.winfo_id())
            if not hwnd:
                hwnd = self.root.winfo_id()
            exstyle = ctypes.windll.user32.GetWindowLongW(hwnd, -20)
            exstyle |= 0x00040000  # WS_EX_APPWINDOW
            ctypes.windll.user32.SetWindowLongW(hwnd, -20, exstyle)
            ctypes.windll.user32.SetWindowPos(hwnd, 0, 0, 0, 0, 0, 0x0007)  # SWP_NOMOVE|SWP_NOSIZE|SWP_NOZORDER|SWP_FRAMECHANGED
        except Exception:
            pass
        self.root.resizable(False, False)

        # 自绘标题栏（可拖动）
        # 星露谷木纹标题栏，高度随缩放
        bar = tk.Frame(self.root, bg=TITLE_BG, height=S["titlebar_h"], bd=0)
        self._title_bar = bar
        bar.pack(fill=tk.X, side=tk.TOP)
        bar.bind("<Button-1>", self._drag_start)
        bar.bind("<B1-Motion>", self._drag_move)
        self._drag_x = 0
        self._drag_y = 0
        # 像素标题（可拖动）
        title_lbl = tk.Label(bar, text="◆ PureVox Lite", bg=TITLE_BG, fg=TITLE_FG, font=self.fonts["title"])
        title_lbl.pack(side=tk.LEFT, padx=S["pad_lg"], pady=S["pad_sm"])
        # 仅保留叉：外壳锁定正方形（ctl_h x ctl_h），按钮填满，保证严格方形
        close_wrap = tk.Frame(bar, bg=TITLE_BG, width=S["ctl_h"], height=S["ctl_h"], bd=0)
        close_wrap.pack(side=tk.RIGHT, padx=S["pad_sm"], pady=S["pad_sm"])
        close_wrap.pack_propagate(False)
        self.btn_close_wrap = close_wrap
        close_btn = tk.Button(close_wrap, text="✕", bg="#E57373", fg="white", bd=1, relief=tk.RAISED, highlightbackground=BORDER, font=self.fonts["bold"], padx=0, pady=0, command=self._do_close, activebackground="#EF5350")
        self.btn_close = close_btn
        close_btn.pack(fill=tk.BOTH, expand=True)
        # 标题栏整体及文字均可拖动
        for w in (bar, title_lbl):
            w.bind("<Button-1>", self._drag_start)
            w.bind("<B1-Motion>", self._drag_move)

        body = tk.Frame(self.root, bg=BG)
        body.pack(fill=tk.BOTH, expand=True, padx=S["pad_lg"], pady=S["pad_md"])

        self.in_var = tk.StringVar(value=cfg.get("input_device", ""))
        self.out_var = tk.StringVar(value=cfg.get("output_device", ""))

        # ins/outs 可能是 (disp,idx) 或 (disp,idx,props)
        def _unpack(lst):
            names, mp, props = [], {}, {}
            for item in lst:
                if len(item) == 3:
                    n, i, p = item
                    names.append(n); mp[n] = i; props[n] = p
                else:
                    n, i = item
                    names.append(n); mp[n] = i; props[n] = ""
            return names, mp, props
        self.in_names, self.in_map, self.in_props = _unpack(ins)
        self.out_names, self.out_map, self.out_props = _unpack(outs)

        row1 = tk.Frame(body, bg=BG)
        row1.pack(fill=tk.X, pady=S["pad_sm"])
        tk.Label(row1, text="输入", bg=BG, fg=FG, width=6, anchor="w", font=self.fonts["body"]).pack(side=tk.LEFT)
        self.cb_in = BlackCombo(row1, self.in_names, self.in_var, self._device_changed, props_map=self.in_props, sizes=self.sizes, fonts=self.fonts)
        self.cb_in.pack(side=tk.LEFT, padx=S["pad_md"], fill=tk.X, expand=True)

        row2 = tk.Frame(body, bg=BG)
        row2.pack(fill=tk.X, pady=S["pad_sm"])
        tk.Label(row2, text="输出", bg=BG, fg=FG, width=6, anchor="w", font=self.fonts["body"]).pack(side=tk.LEFT)
        self.cb_out = BlackCombo(row2, self.out_names, self.out_var, self._device_changed, props_map=self.out_props, sizes=self.sizes, fonts=self.fonts)
        self.cb_out.pack(side=tk.LEFT, padx=S["pad_md"], fill=tk.X, expand=True)

        row3 = tk.Frame(body, bg=BG)
        row3.pack(fill=tk.X, pady=S["pad_md"])
        tk.Label(row3, text="前增益", bg=BG, fg=FG, width=6, anchor="w", font=self.fonts["body"]).pack(side=tk.LEFT)
        # 内边距 pady 由 _apply_pads 按统一控件高计算，保证四行控件等高
        self.btn_pre_dec = tk.Button(row3, text="−", bg=BTN_BG, fg=FG, bd=0, relief=tk.FLAT, width=3, font=self.fonts["bold"], activebackground=HOVER_BG, activeforeground=FG, command=lambda: self._step("pre", -1))
        self.btn_pre_dec.pack(side=tk.LEFT, padx=S["pad_sm"])
        self.pre_var = tk.StringVar(value=str(int(cfg.get("pre_gain_db", 0))))
        # Entry 外壳：宽高全部锁定（propagate 关闭后 Frame 不再跟随子控件，必须显式给尺寸）
        self.ent_pre_wrap = tk.Frame(row3, bg=BORDER, bd=1, relief=tk.FLAT, width=self.sizes["entry_w"], height=self.sizes["ctl_h"])
        self.ent_pre_wrap.pack(side=tk.LEFT)
        self.ent_pre_wrap.pack_propagate(False)
        self.ent_pre = tk.Entry(self.ent_pre_wrap, textvariable=self.pre_var, bg=ENTRY_BG, fg=FG, insertbackground=FG, width=6, justify="center", relief=tk.FLAT, bd=0, highlightthickness=0, font=self.fonts["body"])
        self.ent_pre.pack(fill=tk.BOTH, expand=True)
        self.ent_pre.bind("<Return>", lambda e: self._gain_enter("pre"))
        self.ent_pre.bind("<FocusOut>", lambda e: self._gain_enter("pre"))
        self.btn_pre_inc = tk.Button(row3, text="+", bg=BTN_BG, fg=FG, bd=0, relief=tk.FLAT, width=3, font=self.fonts["bold"], activebackground=HOVER_BG, activeforeground=FG, command=lambda: self._step("pre", 1))
        self.btn_pre_inc.pack(side=tk.LEFT, padx=S["pad_sm"])
        tk.Label(row3, text="dB", bg=BG, fg=FG, font=self.fonts["body"]).pack(side=tk.LEFT)

        row4 = tk.Frame(body, bg=BG)
        row4.pack(fill=tk.X, pady=S["pad_sm"])
        tk.Label(row4, text="后增益", bg=BG, fg=FG, width=6, anchor="w", font=self.fonts["body"]).pack(side=tk.LEFT)
        self.btn_post_dec = tk.Button(row4, text="−", bg=BTN_BG, fg=FG, bd=0, relief=tk.FLAT, width=3, font=self.fonts["bold"], activebackground=HOVER_BG, activeforeground=FG, command=lambda: self._step("post", -1))
        self.btn_post_dec.pack(side=tk.LEFT, padx=S["pad_sm"])
        self.post_var = tk.StringVar(value=str(int(cfg.get("post_gain_db", 0))))
        self.ent_post_wrap = tk.Frame(row4, bg=BORDER, bd=1, relief=tk.FLAT, width=self.sizes["entry_w"], height=self.sizes["ctl_h"])
        self.ent_post_wrap.pack(side=tk.LEFT)
        self.ent_post_wrap.pack_propagate(False)
        self.ent_post = tk.Entry(self.ent_post_wrap, textvariable=self.post_var, bg=ENTRY_BG, fg=FG, insertbackground=FG, width=6, justify="center", relief=tk.FLAT, bd=0, highlightthickness=0, font=self.fonts["body"])
        self.ent_post.pack(fill=tk.BOTH, expand=True)
        self.ent_post.bind("<Return>", lambda e: self._gain_enter("post"))
        self.ent_post.bind("<FocusOut>", lambda e: self._gain_enter("post"))
        self.btn_post_inc = tk.Button(row4, text="+", bg=BTN_BG, fg=FG, bd=0, relief=tk.FLAT, width=3, font=self.fonts["bold"], activebackground=HOVER_BG, activeforeground=FG, command=lambda: self._step("post", 1))
        self.btn_post_inc.pack(side=tk.LEFT, padx=S["pad_sm"])
        tk.Label(row4, text="dB", bg=BG, fg=FG, font=self.fonts["body"]).pack(side=tk.LEFT)

        row5 = tk.Frame(body, bg=BG)
        row5.pack(fill=tk.X, pady=S["pad_lg"])
        self.autostart_var = tk.BooleanVar(value=bool(cfg.get("autostart", False)))
        self.cb_autostart = PixelCheck(row5, text="开机自启", variable=self.autostart_var, command=self._autostart_toggle, sizes=self.sizes, fonts=self.fonts)
        self.cb_autostart.pack(side=tk.LEFT)
        self.btn_sound = tk.Button(row5, text="声音控制面板", bg=BTN_BG, fg=FG, bd=0, relief=tk.FLAT, font=self.fonts["body"], padx=S["pad_lg"], activebackground=HOVER_BG, activeforeground=FG, command=self._open_sound)
        self.btn_sound.pack(side=tk.RIGHT, padx=S["pad_sm"])
        self.btn_vb = tk.Button(row5, text="VB 面板", bg=BTN_BG, fg=FG, bd=0, relief=tk.FLAT, font=self.fonts["body"], padx=S["pad_lg"], activebackground=HOVER_BG, activeforeground=FG, command=self._open_vb)
        self.btn_vb.pack(side=tk.RIGHT)

        self.status = tk.Label(body, text="运行中 · 48kHz 单声道 · 降噪常驻", bg=BG, fg="#AAAAAA", font=self.fonts["small"])
        self.status.pack(side=tk.BOTTOM, pady=S["pad_sm"])

        # 初始统一控件高（按钮 pady / 输入框外壳全部来自尺寸表）
        self._apply_pads()
        # 初始定窗：必须在全部控件创建完成后测量，否则按空窗口截断内容
        w, h = self._fit_size()
        self._zoom_w, self._zoom_h = w, h
        self.root.geometry(f"{w}x{h}+600+400")

    def _ctl_pad_v(self):
        # 统一控件高：目标 ctl_h 与字体行高之差 = 按钮 pady / 输入框 ipady，保证同行等高
        try:
            ls = self.fonts["body"].metrics("linespace")
        except Exception:
            ls = self.sizes["font_body"] + 4
        return max(0, (self.sizes["ctl_h"] - ls) // 2)

    def _apply_sizes(self):
        S = self.sizes
        # 命名字体改 px 后，所有引用该字体的控件由 Tk 自动重排
        font_key = (("body", "font_body"), ("bold", "font_body"), ("title", "font_title"), ("small", "font_small"))
        for name, key in font_key:
            try:
                self.fonts[name].configure(size=-S[key])
            except Exception:
                pass
        try:
            self._title_bar.configure(height=S["titlebar_h"])
        except Exception:
            pass
        for cb in (self.cb_in, self.cb_out, self.cb_autostart):
            try:
                cb.apply_sizes()
            except Exception:
                pass
        self._apply_pads()

    def _apply_pads(self):
        pv = self._ctl_pad_v()
        S = self.sizes
        for b in (self.btn_pre_dec, self.btn_pre_inc, self.btn_post_dec, self.btn_post_inc,
                  self.btn_sound, self.btn_vb):
            try:
                b.configure(pady=pv)
            except Exception:
                pass
        # 关闭按钮：pady 恒 0（由方形外壳撑满），换挡只重设外壳
        try:
            self.btn_close_wrap.configure(width=S["ctl_h"], height=S["ctl_h"])
        except Exception:
            pass
        # 完整事件循环后取按钮实际需求高（Tk 按钮有随字体缩放的固有内高，
        # 无法纯公式预测），Entry 外壳直接对齐该值，实现同行严格等高
        try:
            self.root.update()
            ref = self.btn_pre_inc.winfo_reqheight()
        except Exception:
            ref = S["ctl_h"]
        for wrap in (self.ent_pre_wrap, self.ent_post_wrap):
            try:
                wrap.configure(width=S["entry_w"], height=max(S["ctl_h"], ref))
            except Exception:
                pass

    def _fit_size(self):
        # 窗口尺寸 = max(挡位基准 px, 内容需求)，保证任何挡位下内容不被裁剪
        self.root.update_idletasks()
        S = self.sizes
        try:
            w = max(S["win_w"], self.root.winfo_reqwidth())
            h = max(S["win_h"], self.root.winfo_reqheight())
        except Exception:
            w, h = S["win_w"], S["win_h"]
        return w, h

    def _apply_zoom(self, percent):
        percent = clamp_zoom(percent)
        self._zoom = percent
        self.cfg["zoom"] = percent
        # 原地更新尺寸表：所有子组件持同一 dict，读到的即是新挡位 px
        self.sizes.update(make_sizes(percent))
        self._apply_sizes()
        # 完整事件循环：让新字体的需求尺寸传播到几何系统，
        # 只用 update_idletasks 会取到上一挡的陈旧 req 值
        try:
            self.root.update()
        except Exception:
            pass
        w, h = self._fit_size()
        self._zoom_w, self._zoom_h = w, h
        # 保留窗口位置，只改变大小
        try:
            cur = self.root.geometry()
            parts = cur.split("+", 1)
            pos = "+" + parts[1] if len(parts) > 1 else ""
            self.root.geometry(f"{w}x{h}{pos}")
        except Exception:
            self.root.geometry(f"{w}x{h}")

    def _save_cfg(self):
        try:
            from config import save as _save
            _save(self.cfg)
        except Exception:
            pass

    def set_zoom(self, percent):
        # 手动选挡：退出自动模式并记住百分比
        try:
            self.cfg["auto_zoom"] = False
            self._apply_zoom(percent)
            self._save_cfg()
        except Exception:
            pass

    def set_auto_zoom(self):
        # 回到自动挡：按当前屏幕分辨率定挡
        try:
            self.cfg["auto_zoom"] = True
            z = detect_zoom_for_screen(self.root.winfo_screenwidth(), self.root.winfo_screenheight())
            self._apply_zoom(z)
            self._save_cfg()
        except Exception:
            pass

    def _drag_start(self, e):
        self._drag_x = e.x
        self._drag_y = e.y

    def _drag_move(self, e):
        try:
            x = self.root.winfo_x() + e.x - self._drag_x
            y = self.root.winfo_y() + e.y - self._drag_y
            self.root.geometry(f"+{x}+{y}")
        except Exception:
            pass

    def _do_close(self):
        if self._on_close:
            self._on_close()
        else:
            self.root.destroy()

    def _do_minimize(self):
        if self._on_minimize:
            self._on_minimize()
        else:
            self.root.iconify()

    def _step(self, which, delta):
        var = self.pre_var if which == "pre" else self.post_var
        try:
            v = int(float(var.get()))
        except Exception:
            v = 0
        v = max(-20, min(30, v + delta))
        var.set(str(v))
        self._gain_enter(which)

    def _gain_enter(self, which):
        try:
            v = int(float(self.pre_var.get() if which == "pre" else self.post_var.get()))
        except Exception:
            messagebox.showerror("错误", "增益必须是整数 (-20~30)")
            return
        if v < -20 or v > 30:
            messagebox.showerror("错误", "增益范围 -20~30 dB")
            return
        # 回写整数规范
        if which == "pre":
            self.pre_var.set(str(v))
        else:
            self.post_var.set(str(v))
        self.on_gain(which, v)

    def _device_changed(self):
        in_name = self.in_var.get()
        out_name = self.out_var.get()
        self.on_device(in_name, out_name)

    def _autostart_toggle(self):
        self.on_autostart(bool(self.autostart_var.get()))

    def _open_sound(self):
        try:
            if sys.platform.startswith("win"):
                subprocess.Popen(["control", "mmsys.cpl"], shell=True)
            else:
                subprocess.Popen(["pavucontrol"])
        except Exception as e:
            messagebox.showerror("错误", str(e))

    def _open_vb(self):
        try:
            candidates = [
                r"C:\Program Files\VB\CABLE\VBCABLE_ControlPanel.exe",
                r"C:\Program Files (x86)\VB\CABLE\VBCABLE_ControlPanel.exe",
            ]
            for p in candidates:
                if os.path.exists(p):
                    import ctypes
                    # VB-CABLE 控制面板需要管理员权限（改系统驱动级配置），UAC 提权打开
                    ctypes.windll.shell32.ShellExecuteW(None, "runas", p, None, None, 1)
                    return
            self._open_sound()
            self.root.after(500, lambda: messagebox.showinfo("VB-CABLE", "未找到 VBCABLE_ControlPanel.exe，已打开声音控制面板。\n请在播放/录制页查看 CABLE Input/Output。"))
        except Exception as e:
            messagebox.showerror("错误", str(e))

    def run(self):
        self.root.mainloop()

