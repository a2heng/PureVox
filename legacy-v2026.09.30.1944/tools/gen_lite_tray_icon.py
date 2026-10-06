# PureVox Lite — 托盘/应用图标资产生成（一次性工具，产物入库为唯一事实）
# Copyright (C) 2024-2026 a2heng <752848283@qq.com>
# SPDX-License-Identifier: GPL-3.0-or-later
#
# 各尺寸独立渲染（1:1 像素无缩放）：亮色 P 形背景 + 放大深色 P；
# 运行时（dev / 冻结 / 打包）一律直接读 assets/icons/lite_tray.ico 与
# lite_tray.png，本脚本仅在需要改设计时手动重跑。

import os

import numpy as np
from PIL import Image, ImageDraw, ImageFont

ROOT = os.path.dirname(os.path.dirname(os.path.abspath(__file__)))
FONT = os.path.join(ROOT, "assets", "fonts", "ark-pixel-12px-monospaced-zh_cn.ttf")
OUT_DIR = os.path.join(ROOT, "assets", "icons")

MASTER = 512
FONT_SIZE = 504
P_SCALE = 1.4
DARK = "#FFD54F"
BRIGHT = "#8B6914"


def _render_frame(size):
    """亮色 P 扩散做背景，深色 P 填满画布。"""
    fs = max(8, int(size * FONT_SIZE / MASTER))
    pf = ImageFont.truetype(FONT, fs)
    # 1. 画 P，测实际像素范围，缩放填满 85% 画布
    tmp = Image.new("RGBA", (size, size), (0, 0, 0, 0))
    td = ImageDraw.Draw(tmp)
    td.text((0, 0), "P", fill=(255, 255, 255, 255), font=pf)
    cb = tmp.getbbox()
    if not cb:
        return tmp
    pw, ph = cb[2] - cb[0], cb[3] - cb[1]
    need = size * 0.85
    if pw < need or ph < need:
        s = need / max(pw, ph)
        fs = max(8, int(fs * s))
        pf = ImageFont.truetype(FONT, fs)
        tmp = Image.new("RGBA", (size, size), (0, 0, 0, 0))
        td = ImageDraw.Draw(tmp)
        td.text((0, 0), "P", fill=(255, 255, 255, 255), font=pf)
        cb = tmp.getbbox()
        pw, ph = cb[2] - cb[0], cb[3] - cb[1]
    # 2. 以实际像素居中
    ox = (size - pw) // 2 - cb[0]
    oy = (size - ph) // 2 - cb[1]
    # 3. 扩散量按比例
    spread = max(1, int(size * 0.06))
    result = Image.new("RGBA", (size, size), (0, 0, 0, 0))
    rd = ImageDraw.Draw(result)
    for dx in range(-spread, spread + 1):
        for dy in range(-spread, spread + 1):
            rd.text((ox + dx, oy + dy), "P", fill=BRIGHT, font=pf)
    # 4. 深色 P 原位
    rd.text((ox, oy), "P", fill=DARK, font=pf)
    return result


# lite_tray.png：64px 版（Tkinter iconphoto 合适尺寸）
_render_frame(64).save(os.path.join(OUT_DIR, "lite_tray.png"))

# lite_tray.ico：各尺寸独立渲染，1:1 像素无缩放
sizes = [256, 128, 96, 72, 64, 60, 48, 40, 36, 32, 30, 24, 20, 16]
frames = [_render_frame(s) for s in sizes]
frames[0].save(
    os.path.join(OUT_DIR, "lite_tray.ico"),
    sizes=[(s, s) for s in sizes],
    append_images=frames[1:])

print("written:")
for f in ("lite_tray.png", "lite_tray.ico"):
    p = os.path.join(OUT_DIR, f)
    print(" ", p, os.path.getsize(p), "bytes")
