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

"""uitk 尺寸系统：分辨率挡位 + 像素尺寸表（参考 lite_mic/ui.py）。

所有组件尺寸/间距/字号的唯一来源；换挡 = sizes.update(make_sizes(z))
后各组件 apply_sizes()。负数字号 = 像素，tk scaling 固定 1。
"""

import os
import sys

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
    """一个挡位一套 px 尺寸表。"""
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
        "ctl_h": r(26),
        "combo_h": r(28),
        "titlebar_h": r(34),
        "row_h": r(40),          # 节点行常态高
        "check_box": r(16),
        "scrollbar_w": r(10),
        "thumb_min": r(14),
        # 弹出列表
        "popup_rows": 6,
        # 间距
        "pad_sm": r(4),
        "pad_md": r(6),
        "pad_lg": r(10),
        # 窗口基准（已含倍率）
        "win_w": r(400),
        "win_h": r(700),
    }


def enable_hidpi():
    """声明 PerMonitor DPI Aware，缩放完全由 tk scaling 自管。"""
    if not sys.platform.startswith("win"):
        return
    try:
        import ctypes
        try:
            ctypes.windll.shcore.SetProcessDpiAwareness(2)
        except Exception:
            try:
                ctypes.windll.user32.SetProcessDpiAwarenessContext(
                    ctypes.c_void_p(-4))
            except Exception:
                try:
                    ctypes.windll.user32.SetProcessDPIAware()
                except Exception:
                    pass
    except Exception:
        pass


def fix_tk_scaling(root):
    """像素化前提：tk scaling 固定 1，DPI 缩放交给尺寸表。"""
    try:
        root.tk.call("tk", "scaling", 1.0)
    except Exception:
        pass


FONT_FAMILY_CANDIDATES = ["Ark Pixel 12px Monospaced zh_cn",
                          "Ark Pixel 12px Mono zh_cn",
                          "Ark Pixel 12px Mono",
                          "Microsoft YaHei UI", "Microsoft YaHei",
                          "PingFang SC", "Noto Sans CJK SC", "Segoe UI"]

# 像素字体（仓库唯一副本 assets/fonts/；lite_mic/lite_net 与打包脚本共用）
_FONT_FILE = "ark-pixel-12px-monospaced-zh_cn.ttf"


def find_pixel_font_ttf() -> str:
    """定位内置像素字体：PyInstaller 资源目录 → 应用根/仓库根 → 上级目录。

    源码态仓库根 = uitk/ 上一级；打包态（deb/rpm/AppImage）= /opt/purevox
    等应用根，字体随 assets/fonts/ 携带。找不到返回空串。
    """
    here = os.path.dirname(os.path.abspath(__file__))
    roots = []
    meipass = getattr(sys, "_MEIPASS", None)
    if meipass:
        roots.append(meipass)
    roots.append(os.path.dirname(here))
    roots.append(os.path.dirname(os.path.dirname(here)))
    for r in roots:
        p = os.path.join(r, "assets", "fonts", _FONT_FILE)
        if os.path.isfile(p):
            return p
    return ""


def install_fonts_fontconfig(font_dir: str) -> None:
    """Linux/macOS：把目录内 ttf/otf 安装到用户字体目录并刷新 fontconfig。

    freedesktop 标准用户字体机制（$XDG_DATA_HOME/fonts/purevox），
    不需要 root、不污染系统字体；内容相同则跳过拷贝，fc-cache 失败静默。
    """
    import hashlib
    import shutil
    import subprocess
    data_home = os.environ.get("XDG_DATA_HOME") or os.path.expanduser("~/.local/share")
    dst_root = os.path.join(data_home, "fonts", "purevox")
    try:
        os.makedirs(dst_root, exist_ok=True)
        changed = False
        for fn in os.listdir(font_dir):
            if not fn.lower().endswith((".ttf", ".otf")):
                continue
            s = os.path.join(font_dir, fn)
            d = os.path.join(dst_root, fn)

            def _md5(p):
                h = hashlib.md5()
                with open(p, "rb") as f:
                    h.update(f.read())
                return h.hexdigest()

            if os.path.isfile(d) and _md5(s) == _md5(d):
                continue
            shutil.copyfile(s, d)
            changed = True
        if changed:
            subprocess.run(["fc-cache", "-f", dst_root], check=False,
                           stdout=subprocess.DEVNULL, stderr=subprocess.DEVNULL)
    except Exception:
        pass


def load_pixel_font():
    """注册内置 Ark Pixel 字体（跨平台；失败静默回退系统字体）。

    Windows: GDI AddFontResourceExW(FR_PRIVATE) 仅本进程可见；
    Linux/macOS: 经 fontconfig 用户字体目录注册——此前仅 Windows 生效，
    Linux 包内又未携带字体文件，无 CJK 字体的系统会中文豆腐/缺字形。
    """
    src = find_pixel_font_ttf()
    if not src:
        return
    try:
        if sys.platform.startswith("win"):
            import ctypes
            # 0x10 = FR_PRIVATE：仅本进程可见，不污染系统
            ctypes.windll.gdi32.AddFontResourceExW(src, 0x10, 0)
        else:
            install_fonts_fontconfig(os.path.dirname(src))
    except Exception:
        pass


def pick_font_family(root):
    load_pixel_font()
    try:
        avail = set(root.tk.call("font", "families"))
    except Exception:
        avail = set()
    for name in FONT_FAMILY_CANDIDATES:
        if name in avail:
            return name
    return "TkDefaultFont"
