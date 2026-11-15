#!/usr/bin/env python3
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
"""图标生成：**所有平台的图标产物都由这一份脚本生成**（不参与 CI）。

P 是四个直角盒子的矢量定义，不是字体 —— 早期版本走 legacy 的 ark-pixel 像素字体
渲染（tools/gen_lite_tray_icon.py 的托盘黄 P），中途试过移植那段渲染逻辑，**最后放弃
字体**，三个理由：

1. 字体只在**只读快照**里（AGENTS §2.3 / §2.5），现役树依赖它不合适；
2. PIL / GDI+ / FreeType 的**字形度量各不相同**（同一参数实测差 8%），同一份代码在不同
   平台会画出不同的 P —— 对「跨平台同款图标」是硬伤；
3. P 本身是直角几何图形，矢量描述比「先渲染成字形再缩放」更准。

这一份实现取代了曾经的 gen_icons.ps1（GDI+）与 gen_icons.sh（ImageMagick）两份：
两个图像库的渐变抖动、抗锯齿、降采样滤波器都不同，实测同尺寸像素差 50~90% ——
两台机器生成的图标长得不一样，「唯一实现」就是一句空话。现在同一条命令在 Windows /
Linux / macOS 上跑出**逐字节相同**的产物。

用法：
    python tools/automation/gen_icons.py                 # 全量
    python tools/automation/gen_icons.py -Compare        # 只比对形状，不写文件
    python tools/automation/gen_icons.py -Skip Android,Ios
    python tools/automation/gen_icons.py -Out /tmp/icons # 换个输出目录试跑

依赖：Pillow（pip install pillow）。仅本机一次性使用，CI 不需要（产物入库）。
"""

from __future__ import annotations

import argparse
import io
import json
import struct
import sys
from pathlib import Path

# 中文输出必须先切 UTF-8（Windows 控制台 / CI runner 的代码页都不是 UTF-8），见 _utf8.py。
sys.path.insert(0, str(Path(__file__).resolve().parent))
import _utf8  # noqa: E402

_utf8.force()

try:
    from PIL import Image, ImageDraw
except ImportError:  # 给出可执行的补救，而不是 ImportError 栈
    sys.exit("缺 Pillow。先装：pip install pillow（只本机用，CI 不需要）")

REPO = Path(__file__).resolve().parents[2]
ICONS = REPO / "" / "icons"
# Android 自适应图标**不进 icons/**：那里曾经放 cargo tauri icon 的产物，
# 现已删除（见 AGENTS §2.5 现役引用清单）。Android 的图标归 res 管，脚本直接写过去。
RES = REPO / "android" / "app" / "src" / "main" / "res"

# --------------------------------------------------------------------------
# P 的矢量定义 —— 512 画布坐标，逐像素量自 audio_icon_base.png
#
# 本体是四个直角盒子；描边 = 每个盒子向外扩 STROKE（方形外扩，不是圆角化）。
# 右上角那个 1px 错位（上横止于 336、右竖起于 338）是像素风刻意的，别「顺手对齐」。
# --------------------------------------------------------------------------
P_BOXES = (
    (120.0, 54.0, 336.0, 107.0),   # 上横  217×54
    (120.0, 54.0, 173.0, 488.0),   # 左竖   54×435
    (338.0, 109.0, 391.0, 270.0),  # 右竖   54×162
    (120.0, 272.0, 336.0, 325.0),  # 中横  217×54
)
STROKE = 30.0
P_BOX_ALL = (90.0, 24.0, 421.0, 518.0)   # 含描边的整体包围盒（332×495）

FILL = (0x64, 0xB4, 0xFF)    # P 本体 浅蓝
EDGE = (0x0F, 0x23, 0x50)    # P 描边 深蓝

# 自适应图标的 monochrome 层（Android 13+ 主题图标）：**只有 alpha 有意义**，
# 系统会拿它当蒙版整体着色。所以画成一整块单色剪影（描边与本体同色），
# 颜色随便选白色即可，别「按设计稿上色」——那会被系统丢掉。
MONO = (0xFF, 0xFF, 0xFF)

BG_TOP = (0x10, 0x2F, 0x5D)
BG_BOTTOM = (0x1E, 0x6F, 0xD0)
CORNER_R = 110.0 / 512.0      # 圆角半径 / 边长

# 托盘「已停止」态：底色换暗灰，**P 也换成暗蓝** —— 只压底色的话 P 仍是亮蓝，在暗灰底上
# 比运行时还跳，一眼看不出停了。整套一起压暗才有「熄灭」的观感。
TRAY_STOPPED_TOP = (0x14, 0x14, 0x17)
TRAY_STOPPED_BOTTOM = (0x24, 0x24, 0x29)
TRAY_STOPPED_FILL = (0x2F, 0x5C, 0x8F)   # P 本体 暗蓝
TRAY_STOPPED_EDGE = (0x12, 0x16, 0x1F)   # P 描边 近黑

P_IN_LEGACY = 0.797    # 传统图标里 P 高 / 边长（沿用现有观感）
P_IN_ADAPTIVE = 0.57   # 自适应前景里 P 高 / 108dp 画布（留足安全区）
P_IN_TRAY = 0.72       # 托盘：64px 下 P 要占七成才看得清，0.62 明显偏小
SUPERSAMPLE = 4        # 一律 4× 画完再降采样，圆边才是多像素灰阶而不是一圈台阶

# Pillow 10 起 Resampling 从 Image 挪到 Image.enums，兼容两种写法
_LANCZOS = getattr(getattr(Image, "Resampling", Image), "LANCZOS")

SHAPES = ("none", "squircle", "round", "full")


# --------------------------------------------------------------------------
# 绘制
# --------------------------------------------------------------------------
def _vgradient(size: int, top: tuple, bottom: tuple) -> Image.Image:
    """竖向线性渐变（1×size 画一遍再横向拉伸：逐像素算 1024² 次太慢）。"""
    strip = Image.new("RGBA", (1, size))
    px = strip.load()
    span = max(size - 1, 1)
    for y in range(size):
        t = y / span
        px[0, y] = tuple(int(round(a + (b - a) * t)) for a, b in zip(top, bottom)) + (255,)
    return strip.resize((size, size), _LANCZOS)


def draw_player(img: Image.Image, scale: float, off_x: float, off_y: float,
                dark: bool = False, mono: bool = False) -> None:
    """按给定缩放与偏移把 P 画到 ``img`` 上。

    注意 Pillow 的 ``rectangle`` 端点**含**、GDI+ 的 ``FillRectangle`` 是「起点 + 宽高」，
    所以右/下边界要减 1，否则每块胖 1 像素、含描边包围盒会从 332×495 变成 334×497。

    ``mono=True`` 时描边与本体同色（[MONO]）—— 只留剪影给系统着色，见 [MONO] 的说明。
    """
    d = ImageDraw.Draw(img)
    if mono:
        fill = edge = MONO
    else:
        fill = TRAY_STOPPED_FILL if dark else FILL
        edge = TRAY_STOPPED_EDGE if dark else EDGE

    # 先画描边：每个盒子向外扩 STROKE（方形），相邻盒子的外扩自然合并
    for x0, y0, x1, y1 in P_BOXES:
        d.rectangle(
            [
                off_x + (x0 - STROKE) * scale,
                off_y + (y0 - STROKE) * scale,
                off_x + (x1 + STROKE + 1) * scale - 1,
                off_y + (y1 + STROKE + 1) * scale - 1,
            ],
            fill=edge + (255,),
        )
    # 再压本体：覆盖内部，剩下的就是描边
    for x0, y0, x1, y1 in P_BOXES:
        d.rectangle(
            [
                off_x + x0 * scale,
                off_y + y0 * scale,
                off_x + (x1 + 1) * scale - 1,
                off_y + (y1 + 1) * scale - 1,
            ],
            fill=fill + (255,),
        )


def draw_player_centered(img: Image.Image, big: int, p_ratio: float,
                         dark: bool = False, mono: bool = False) -> None:
    """P 按自然包围盒等比缩放（高 = big × p_ratio）后居中。"""
    pw = P_BOX_ALL[2] - P_BOX_ALL[0] + 1
    ph = P_BOX_ALL[3] - P_BOX_ALL[1] + 1
    scale = (big * p_ratio) / ph
    off_x = (big - pw * scale) / 2 - P_BOX_ALL[0] * scale
    off_y = (big - ph * scale) / 2 - P_BOX_ALL[1] * scale
    draw_player(img, scale, off_x, off_y, dark, mono)


def render(size: int, shape: str, p_ratio: float, dark: bool = False,
           mono: bool = False) -> Image.Image:
    """画一个 size×size 的图标。

    shape: full（满幅无圆角，iOS/macOS）/ squircle（圆角矩形）/ round（圆）/
           none（自适应前景层，只有 P，背景交给 ic_launcher_bg）
    """
    big = size * SUPERSAMPLE
    img = Image.new("RGBA", (big, big), (0, 0, 0, 0))
    if shape == "none":
        draw_player_centered(img, big, p_ratio, dark, mono)
        return img.resize((size, size), _LANCZOS)

    top = TRAY_STOPPED_TOP if dark else BG_TOP
    bottom = TRAY_STOPPED_BOTTOM if dark else BG_BOTTOM
    mask = Image.new("L", (big, big), 0)
    md = ImageDraw.Draw(mask)
    if shape == "round":
        md.ellipse([0, 0, big - 1, big - 1], fill=255)
    elif shape == "full":
        md.rectangle([0, 0, big - 1, big - 1], fill=255)
    elif shape == "squircle":
        md.rounded_rectangle([0, 0, big - 1, big - 1], radius=CORNER_R * big, fill=255)
    else:
        raise ValueError(f"未知 shape：{shape}（可选 {SHAPES}）")
    # 用 mask 上色：渐变本身铺满，形状交给 mask —— 比先生成带 alpha 的渐变再合成省事
    img.paste(_vgradient(big, top, bottom), (0, 0), mask)

    draw_player_centered(img, big, p_ratio, dark, mono)
    return img.resize((size, size), _LANCZOS)


def render_player_only(canvas: int = 512) -> Image.Image:
    """源图 audio_icon_base.png：只有 P，**按自然尺寸**摆在 512 画布正中。

    注意不是按 ``P_IN_LEGACY`` 缩放 —— 源图是「按原始盒子坐标摆正中」，拿它当带底色
    图标里的 P 去比会得到无意义的 IoU（同一份脚本里两种尺度，别混）。
    """
    img = Image.new("RGBA", (canvas, canvas), (0, 0, 0, 0))
    pw = P_BOX_ALL[2] - P_BOX_ALL[0] + 1
    ph = P_BOX_ALL[3] - P_BOX_ALL[1] + 1
    draw_player(img, 1.0, (canvas - pw) / 2 - P_BOX_ALL[0],
                (canvas - ph) / 2 - P_BOX_ALL[1])
    return img


def png_bytes(img: Image.Image) -> bytes:
    buf = io.BytesIO()
    img.save(buf, format="PNG", optimize=True)
    return buf.getvalue()


def save_png(img: Image.Image, path: Path) -> None:
    path.parent.mkdir(parents=True, exist_ok=True)
    img.save(path, format="PNG", optimize=True)


# --------------------------------------------------------------------------
# 容器：.ico / .icns
# --------------------------------------------------------------------------
def write_ico(path: Path, entries: list[tuple[int, bytes]]) -> None:
    """Windows .ico：ICONDIR + ICONDIRENTRY[] + 各尺寸 PNG（条目宽高是**单字节**，256 编码成 0）。"""
    n = len(entries)
    offset = 6 + 16 * n
    directory = b""
    payload = b""
    for size, png in entries:
        w = 0 if size >= 256 else size
        directory += struct.pack("<BBBBHHII", w, w, 0, 0, 1, 32, len(png), offset)
        offset += len(png)
        payload += png
    blob = struct.pack("<HHH", 0, 1, n) + directory + payload
    # 自检：目录里登记的末条目结束位置必须正好等于文件长度。不等就是上面某处算错了 ——
    # 坏掉的 .ico 不报错、照样生成，只是 Windows 资源编译器会报 "RC2176: old DIB"。
    if len(blob) != offset:
        raise AssertionError(f"ICO 长度不符：目录声称 {offset}，实际 {len(blob)}（{path}）")
    path.write_bytes(blob)


def write_icns(path: Path, blocks: list[tuple[str, bytes]]) -> None:
    """macOS .icns：'icns' + 总长 + 若干「4 字节类型 + 4 字节块长(含头) + PNG」。

    长度是**大端** 32 位 —— 写成小端 Finder 直接不认这个文件。
    """
    total = 8 + sum(8 + len(png) for _, png in blocks)
    out = bytearray(b"icns" + struct.pack(">I", total))
    for btype, png in blocks:
        out += btype.encode("ascii") + struct.pack(">I", 8 + len(png)) + png
    if len(out) != total:
        raise AssertionError(f"ICNS 长度不符：头部声称 {total}，实际 {len(out)}（{path}）")
    path.write_bytes(bytes(out))


# --------------------------------------------------------------------------
# 形状比对（-Compare）
# --------------------------------------------------------------------------
def shape_iou(a: Image.Image, b: Image.Image) -> float:
    """两个图标里「非透明像素」的 IoU —— 换个渲染器时确认 P 的形状没跑偏。"""
    if a.size != b.size:
        a = a.resize(b.size, _LANCZOS)
    # 取 alpha 通道的原始字节（Pillow 12 起 getdata() 已弃用，tobytes() 才是稳的）
    ba = a.convert("RGBA").split()[3].tobytes()
    bb = b.convert("RGBA").split()[3].tobytes()
    both = 0
    either = 0
    for va, vb in zip(ba, bb):
        on_a, on_b = va > 127, vb > 127
        if on_a or on_b:
            either += 1
            if on_a and on_b:
                both += 1
    return both / either if either else 0.0


# --------------------------------------------------------------------------
# 产物清单
# --------------------------------------------------------------------------
BUNDLE_SIZES = (32, 64, 128, 256, 512)
ICO_SIZES = (16, 24, 32, 48, 64, 128, 256)
STORE_SIZES = (30, 44, 71, 89, 107, 142, 150, 284, 310)
ICNS_TYPES = (
    ("icp4", 16), ("icp5", 32), ("icp6", 64), ("ic07", 128),
    ("ic08", 256), ("ic09", 512), ("ic10", 1024),
)
# Android 五档密度：(传统图标边长, 自适应前景画布边长=108dp)
ANDROID_DENSITIES = (
    ("mdpi", 48, 108), ("hdpi", 72, 162), ("xhdpi", 96, 216),
    ("xxhdpi", 144, 324), ("xxxhdpi", 192, 432),
)
# Xcode 的 AppIcon 集。同一物理尺寸在不同 idiom 下是**同一个文件**（20x20@2x 的 iPhone 与
# iPad 都是 40px），只有 iPad 独有的槽位才另起 `~ipad` 文件——旧图标里那几个
# `@2x-1.png` 是 cargo tauri icon 自己的 iPad 命名，既不是 Xcode 约定，内容又和各自
# 的 `@2x` 完全相同（同一尺寸而已），属重复文件。
IOS_SLOTS = (
    "AppIcon-20x20@1x.png|20|universal|1x|20x20",
    "AppIcon-20x20@2x.png|40|universal|2x|20x20",
    "AppIcon-20x20@3x.png|60|iphone|3x|20x20",
    "AppIcon-20x20@1x~ipad.png|20|ipad|1x|20x20",
    "AppIcon-20x20@2x~ipad.png|40|ipad|2x|20x20",
    "AppIcon-29x29@1x.png|29|universal|1x|29x29",
    "AppIcon-29x29@2x.png|58|universal|2x|29x29",
    "AppIcon-29x29@3x.png|87|iphone|3x|29x29",
    "AppIcon-29x29@1x~ipad.png|29|ipad|1x|29x29",
    "AppIcon-29x29@2x~ipad.png|58|ipad|2x|29x29",
    "AppIcon-40x40@2x.png|80|universal|2x|40x40",
    "AppIcon-40x40@3x.png|120|iphone|3x|40x40",
    "AppIcon-40x40@1x~ipad.png|40|ipad|1x|40x40",
    "AppIcon-40x40@2x~ipad.png|80|ipad|2x|40x40",
    "AppIcon-60x60@2x.png|120|iphone|2x|60x60",
    "AppIcon-60x60@3x.png|180|iphone|3x|60x60",
    "AppIcon-76x76@1x~ipad.png|76|ipad|1x|76x76",
    "AppIcon-76x76@2x~ipad.png|152|ipad|2x|76x76",
    "AppIcon-83.5x83.5@2x~ipad.png|167|ipad|2x|83.5x83.5",
    "AppIcon-1024x1024.png|1024|ios-marketing|1x|1024x1024",
)

GROUPS = ("Source", "Bundle", "Store", "Macos", "Ios", "Tray", "Android")

# Android 自适应图标的**声明文件**（mipmap-anydpi-v26/）。由脚本一并写出，
# 免得「图标=脚本产物、XML=手改」两头漂移（曾经 XML 的注释还写着早已不存在的
# gen_icons.sh|.ps1）。背景是桌面版同款深蓝渐变（`ic_launcher_bg`，手写、不在这里生成），
# 前景与 monochrome 都是本脚本的 PNG 产物。monochrome 供 Android 13+ 主题图标取用。
ADAPTIVE_XML = """<?xml version="1.0" encoding="utf-8"?>
<!-- 由 tools/automation/gen_icons.py 生成，请勿手改（改图标改脚本再全量重跑）。

     PureVox — AI 麦克风降噪工具
     Copyright (C) 2024-2026 a2heng <752848283@qq.com>

     PureVox is licensed under the GNU General Public License v3.0 or
     later (GPL-3.0-or-later).  See LICENSE for details.

     The built-in AI models are NOT covered by the GPL; they are the
     property of a2heng and may only be used with PureVox under
     authorization.  See MODEL-LICENSE.md for details.

     SPDX-License-Identifier: GPL-3.0-or-later

     自适应图标（API 26+）：背景是桌面版同款深蓝竖向渐变（满幅无圆角，系统自己裁形状）；
     前景 P 落在中央 72dp 安全区内；monochrome 是单色剪影，供 Android 13+ 主题图标着色。
-->
<adaptive-icon xmlns:android="http://schemas.android.com/apk/res/android">
  <background android:drawable="@drawable/ic_launcher_bg"/>
  <foreground android:drawable="@mipmap/ic_launcher_foreground"/>
  <monochrome android:drawable="@mipmap/ic_launcher_monochrome"/>
</adaptive-icon>
"""


def write_adaptive_xml(anydpi_dir: Path) -> None:
    """写 `mipmap-anydpi-v26/` 下的自适应图标声明：方形与圆形都指向同一套层。"""
    anydpi_dir.mkdir(parents=True, exist_ok=True)
    for fn in ("ic_launcher.xml", "ic_launcher_round.xml"):
        (anydpi_dir / fn).write_text(ADAPTIVE_XML, encoding="utf-8", newline="\n")


def main() -> int:
    ap = argparse.ArgumentParser(description="生成 PureVox 全部平台的图标产物")
    ap.add_argument("-Skip", default="", help="跳过分组，逗号分隔：" + "/".join(GROUPS))
    ap.add_argument("-Compare", action="store_true", help="只把生成的 P 与现有源图做形状比对，不写文件")
    ap.add_argument("-Out", default="", help="输出根目录（默认 icons）")
    args = ap.parse_args()

    skip = {s.strip() for s in args.Skip.split(",") if s.strip()}
    unknown = skip - set(GROUPS)
    if unknown:
        sys.exit(f"未知分组：{sorted(unknown)}（可选 {'/'.join(GROUPS)}）")

    # -Out 必须**完全隔离**：桌面图标与 Android 图标一起搬过去，一个字节都不碰仓库。
    # 之前 Android 那段写死仓库路径，于是「试跑到临时目录」反而把仓库里的 mipmap 覆盖了
    # —— 生成脚本半隔离比不隔离更危险。
    if args.Out:
        base = Path(args.Out)
        icons = base / "icons"
        res = base / "res"
        print(f"输出到 {base}（试跑模式：不碰仓库）")
    else:
        icons = ICONS
        res = RES

    print(f"P 矢量定义：{len(P_BOXES)} 个盒子，描边 {STROKE:.0f}px，"
          f"包围盒 {P_BOX_ALL[2]-P_BOX_ALL[0]+1:.0f}x{P_BOX_ALL[3]-P_BOX_ALL[1]+1:.0f}")

    if args.Compare:
        base_path = icons / "audio_icon_base.png"
        if not base_path.exists():
            sys.exit(f"找不到 {base_path}，先生成一次（去掉 -Compare）")
        ref = Image.open(base_path).convert("RGBA")
        got = render_player_only(ref.size[0])
        print(f"形状 IoU = {shape_iou(ref, got) * 100:.2f}%"
              "（旧的字体扩散带抗锯齿、新的矢量是实边，差 1px 边界属正常）")
        return 0

    def app(size: int) -> Image.Image:
        return render(size, "squircle", P_IN_LEGACY)

    icons.mkdir(parents=True, exist_ok=True)

    if "Source" not in skip:
        save_png(render_player_only(), icons / "audio_icon_base.png")
        print("[源图  ] audio_icon_base.png")

    if "Bundle" not in skip:
        for s in BUNDLE_SIZES:
            save_png(app(s), icons / f"{s}x{s}.png")
        save_png(app(256), icons / "128x128@2x.png")   # 256 的副本（Tauri 的 @2x 槽位）
        save_png(app(512), icons / "icon.png")
        write_ico(icons / "icon.ico",
                  [(s, png_bytes(app(s))) for s in ICO_SIZES])
        # Linux 托盘：32×32 原始 RGBA（`tray_linux.rs` 用 include_bytes!，运行时不解码 PNG）
        (icons / "tray.rgba").write_bytes(app(32).tobytes())
        print(f"[通用  ] {'/'.join(str(s) for s in BUNDLE_SIZES)} + 128x128@2x + icon.png + icon.ico + tray.rgba")

    if "Store" not in skip:
        for s in STORE_SIZES:
            save_png(app(s), icons / f"Square{s}x{s}Logo.png")
        save_png(app(50), icons / "StoreLogo.png")
        print(f"[Store  ] Square{min(STORE_SIZES)}..{max(STORE_SIZES)} + StoreLogo")

    if "Macos" not in skip:
        write_icns(
            icons / "icon.icns",
            [(t, png_bytes(render(s, "full", P_IN_LEGACY))) for t, s in ICNS_TYPES],
        )
        print(f"[macOS  ] icon.icns（{ICNS_TYPES[0][0]}..{ICNS_TYPES[-1][0]}）")

    if "Ios" not in skip:
        # Xcode 的 AppIcon 集：**必须是 appiconset 目录 + Contents.json**，裸 PNG Xcode
        # 根本不认。先清干净，避免上一轮的旧文件残留。
        ios_dir = icons / "ios" / "AppIcon.appiconset"
        if ios_dir.exists():
            for old in sorted(ios_dir.rglob("*"), reverse=True):
                old.unlink() if old.is_file() else old.rmdir()
        ios_dir.mkdir(parents=True, exist_ok=True)
        entries = []
        for row in IOS_SLOTS:
            fn, sz, idiom, scale, slot = row.split("|")
            save_png(render(int(sz), "full", P_IN_LEGACY), ios_dir / fn)
            entries.append({"filename": fn, "idiom": idiom, "scale": scale, "size": slot})
        # newline="\n"：Path.write_text 默认跟 os.linesep，Windows 上会写 CRLF，于是每次跑脚本
        # git 都报 "CRLF will be replaced by LF"、diff 里凭空多出全文改动。仓库统一 LF。
        (ios_dir / "Contents.json").write_text(
            json.dumps({"images": entries, "info": {"author": "purevox", "version": 1}},
                       ensure_ascii=False, indent=2) + "\n",
            encoding="utf-8", newline="\n",
        )
        print(f"[iOS    ] AppIcon.appiconset（{len(entries)} 个槽位 + Contents.json）")

    if "Tray" not in skip:
        save_png(render(64, "squircle", P_IN_TRAY, dark=False), icons / "tray_running.png")
        save_png(render(64, "squircle", P_IN_TRAY, dark=True), icons / "tray_stopped.png")
        print("[托盘   ] tray_running / tray_stopped（64px，蓝 / 暗灰）")

    if "Android" not in skip:
        for name, legacy, fg in ANDROID_DENSITIES:
            d = res / f"mipmap-{name}"
            save_png(render(legacy, "squircle", P_IN_LEGACY), d / "ic_launcher.png")
            save_png(render(legacy, "round", P_IN_LEGACY), d / "ic_launcher_round.png")
            save_png(render(fg, "none", P_IN_ADAPTIVE), d / "ic_launcher_foreground.png")
            save_png(render(fg, "none", P_IN_ADAPTIVE, mono=True), d / "ic_launcher_monochrome.png")
        write_adaptive_xml(res / "mipmap-anydpi-v26")
        print(f"[Android] mipmap {len(ANDROID_DENSITIES)} 档 × 4"
              f"（传统 squircle/round + 自适应前景/monochrome，均 {SUPERSAMPLE}× 超采样）"
              " + mipmap-anydpi-v26/{ic_launcher,ic_launcher_round}.xml")

    print("完成。")
    return 0


if __name__ == "__main__":
    sys.exit(main())