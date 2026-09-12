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

"""uitk 主题令牌（纯 tk，无 Qt、无系统依赖）——星露谷像素浅色主题。

与 lite_mic/ui.py 同一套配色；颜色集中在此，组件禁止写死十六进制。
不读系统 accent（跨设备不可靠且引入额外依赖）。
"""

# ── 基础面（lite 同源；层级：深棕标题栏 > 羊皮纸窗底 > 面板行 > 纯白输入区）──
WINDOW      = "#FFF8E1"   # 羊皮纸窗底
PANEL       = "#FFECB3"   # 行/卡片面板（插件行统一底色，避免白色割裂）
BASE        = "#FFFFFF"   # 输入控件（Entry/下拉弹层/文本区）
BUTTON      = "#FFB74D"   # 南瓜橙主按钮
DARK        = "#FFE0B2"   # 悬停底
MID         = "#8D6E63"   # 木纹边框/分隔线
TRACK       = "#E6C79A"   # 滑杆槽（介于面板与边框之间）
VIZ_BG      = "#FFFDF3"   # 可视化画布底色（统一暖白，随主题令牌，勿写死）
SPEC_BG     = "#4E342E"   # 频谱底（深暖棕，衬托三色柱；比窗底略反差）
SPEC_SLOT   = "#6D4C41"   # 频谱空槽（略亮于底色，静音段也不空洞）

# ── 标题栏（深棕锚点，lite 同源）──
TITLE_BG    = "#6D4C41"
TITLE_FG    = "#FFF8E1"

# ── 前景 ──
TEXT        = "#5D4037"
TEXT_DIM    = "#8D6E63"
TEXT_FAINT  = "#BCAAA4"

# ── 状态色 ──
START_BG    = "#81C784"
START_HOVER = "#66BB6A"
STOP_BG     = "#E57373"
STOP_HOVER  = "#EF5350"

ACCENT      = "#FFB74D"   # 强调色 = 南瓜橙
ACCENT_TEXT = "#5D4037"   # accent 底上的前景


def hover(bg: str) -> str:
    """返回某底色的悬停变体：状态色用各自 hover，中性色提亮一档。"""
    return {
        START_BG: START_HOVER,
        STOP_BG: STOP_HOVER,
        BUTTON: DARK,
    }.get(bg, DARK)
