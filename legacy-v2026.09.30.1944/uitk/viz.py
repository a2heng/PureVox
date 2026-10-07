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

"""uitk 可视化组件——对齐 v2026.08.14 legacy PySide 版的显示质量。

VUBar   连续三区电平条（绿 -60..-20 / 黄 -20..-9 / 红 -9..0），
        峰值保持 10s 后以 20dB/s 回落，刻度线 + 标签。
Spectrum 64 段 Mel 实时输入/输出频谱重叠对比（pvengine.compute_spectrum，
        dB 域 -90..-20；输出=绿基准，输入>输出=灰(噪声残留)，
        输入<输出=浅(增强)；EMA α=0.3 平滑；960 窗(=2×hop) / 480 步进累积；
        每段一列连续槽位，静音段也铺底槽，不断裂）。
LevelRing 圆形运行指示灯。
"""

import math
import time
import tkinter as tk

from . import theme

# ── VU（与 legacy VUBar 同参数）──
VU_DB_MIN, VU_DB_MAX = -60.0, 0.0
VU_DB_RNG = VU_DB_MAX - VU_DB_MIN
VU_PEAK_HOLD = 10.0
VU_PEAK_FALL = 20.0     # dB/s
VU_DB_FALL = 90.0       # 条身慢放 dB/s（快攻慢放，杜绝闪烁感）
G1_R = (-20.0 - VU_DB_MIN) / VU_DB_RNG    # 绿区上界比例
G2_R = (-9.0 - VU_DB_MIN) / VU_DB_RNG     # 黄区上界比例

UNLIT_GREEN, UNLIT_YELLOW, UNLIT_RED = "#C8E6C9", "#FFF59D", "#FFCDD2"

# ── Spectrum（与 legacy SpectrumWidget 同参数）──
try:
    from pvengine import SPECTRUM_NUM_BANDS as NUM_BANDS
    from pvengine import SPECTRUM_FFT as FFT_SIZE
except Exception:
    NUM_BANDS = 64
    FFT_SIZE = 960              # 2×hop @48kHz（FFT 无损窗长）
# 频谱段数 = Mel 滤波器组段数（20Hz–16kHz）；底槽用主题令牌，静音段也铺满
SPEC_BANDS = NUM_BANDS
SPEC_SLOT = theme.SPEC_SLOT      # 每段底槽（连续无缝，避免断开空洞）
MIN_SAMPLES = FFT_SIZE // 2     # 累积步进 = 480 = hop
DB_MIN, DB_MAX = -90.0, -20.0
DB_RANGE = DB_MAX - DB_MIN
SPEC_EMA = 0.3
# 三色柱（类 VU）：按高度分三区，每段最多 3 个矩形（省计算，非逐像素）
SPEC_Z1_DB, SPEC_Z2_DB = -45.0, -30.0
SPEC_Z1 = "#4CAF50"             # 低段 绿
SPEC_Z2 = "#FFB300"             # 中段 黄（深黄，贴近绿的明度）
SPEC_Z3 = "#E53935"             # 高段 红（深红，同上）
SPEC_NOISE = "#B0BEC5"          # 输入高于输出：蓝灰 = 噪声残留


VU_LIT_GREEN, VU_LIT_YELLOW, VU_LIT_RED = "#4CAF50", "#FFB300", "#E53935"


def db_from_peak(peak: float) -> float:
    return 20.0 * math.log10(max(peak, 1e-10))


class VUCanvas(tk.Canvas):
    """分段格子电平条（马赛克 LED）：单条高度、无外置刻度，
    峰值 dB 内嵌在条右端；分区着色 绿(-60..-20)/黄(-20..-9)/红(-9..0)。"""

    SEG_W, SEG_GAP = 6, 2

    def __init__(self, parent, sizes=None, height=22):
        self.sizes = sizes or make_sizes(100)
        s = self.sizes["scale"]
        self.seg_w = max(4, int(round(self.SEG_W * s)))
        self.seg_gap = max(1, int(round(self.SEG_GAP * s)))
        super().__init__(parent, bg=theme.PANEL,
                         highlightthickness=0, bd=0, height=height)
        self._db = VU_DB_MIN
        self._peak = VU_DB_MIN
        self._peak_time = 0.0
        self._t = time.monotonic()
        self._last_painted_db = VU_DB_MIN - 10.0
        self.bind("<Configure>", lambda e: self.redraw(force=True))

    def update_level_db(self, db):
        now = time.monotonic()
        dt = now - self._t
        self._t = now
        # 快攻慢放：上升瞬间到位，下降按 dB/s 滑落——电平是"动"不是"闪"
        if db > self._db:
            self._db = db
        else:
            self._db = max(db, self._db - VU_DB_FALL * dt)
        if self._db > self._peak:
            self._peak = db
            self._peak_time = now
        elif now - self._peak_time > VU_PEAK_HOLD:
            self._peak = max(VU_DB_MIN, self._peak - VU_PEAK_FALL * dt)
        if abs(self._db - self._last_painted_db) >= 0.5:
            self._last_painted_db = self._db
            self.redraw()

    def update_level(self, peak, now=0.0):
        self.update_level_db(db_from_peak(peak))

    @staticmethod
    def _seg_color(r):
        """按段位置比例给点亮色。"""
        if r < G1_R:
            return "#4CAF50"
        if r < G2_R:
            return "#FFB300"
        return "#E53935"

    def redraw(self, force=False):
        w = max(self.winfo_width(), 40)
        h = max(self.winfo_height(), 14)
        self.delete("all")
        pad = 2
        T, B = pad + 1, h - pad - 1
        seg_step = self.seg_w + self.seg_gap
        bar_w = w - 2 * pad
        n_seg = max(8, bar_w // seg_step)
        # 分段格子（马赛克 LED）：每格独立，格间露底色
        zone_tints = ("#DCEDC8", "#FFF9C4", "#FFCDD2")
        lit_r = max(0.0, min(1.0, (self._db - VU_DB_MIN) / VU_DB_RNG))
        peak_r = max(0.0, min(1.0, (self._peak - VU_DB_MIN) / VU_DB_RNG))
        lit_n = int(round(lit_r * n_seg))
        peak_i = int(round(peak_r * n_seg)) - 1
        for i in range(n_seg):
            x0 = pad + i * seg_step
            r = (i + 1) / n_seg
            tint = zone_tints[0] if r < G1_R else (
                zone_tints[1] if r < G2_R else zone_tints[2])
            if i < lit_n:
                fill = self._seg_color(r)
            elif i == peak_i and self._peak > VU_DB_MIN + 0.5:
                fill = theme.MID          # 峰值格：木纹色区分
            else:
                fill = tint
            self.create_rectangle(x0, T, x0 + self.seg_w, B,
                                  fill=fill, width=0)


class SpectrumCanvas(tk.Canvas):
    """64 段 Mel 频谱重叠对比（20Hz–16kHz，legacy SpectrumWidget 的 tk 移植）。"""

    def __init__(self, parent, sizes=None, height=150):
        self.sizes = sizes or make_sizes(100)
        self._lbl_font = ("TkDefaultFont", max(7, int(round(7 * self.sizes["scale"]))))
        super().__init__(parent, bg=theme.SPEC_BG,
                         highlightthickness=0, bd=0, height=height)
        self._input_bands = [DB_MIN] * SPEC_BANDS
        self._output_bands = [DB_MIN] * SPEC_BANDS
        self._smoothed_in = [DB_MIN] * SPEC_BANDS
        self._smoothed_out = [DB_MIN] * SPEC_BANDS
        self._in_accum = []
        self._out_accum = []
        self.bind("<Configure>", lambda e: self.redraw())

    # ── 数据入口（与 legacy update_spectrum 同签名）──
    def update_spectrum(self, input_samples, output_samples):
        updated = False
        for accum_attr, bands_attr, samples in (
                ("_in_accum", "_input_bands", input_samples),
                ("_out_accum", "_output_bands", output_samples)):
            if not samples:
                continue
            accum = getattr(self, accum_attr)
            accum.extend(samples)
            if len(accum) > FFT_SIZE * 2:
                del accum[:-FFT_SIZE]
            if len(accum) >= FFT_SIZE:
                setattr(self, bands_attr,
                        self._compute_bands(accum[-FFT_SIZE:]))
                del accum[:-MIN_SAMPLES]
                updated = True
        if updated:
            for i in range(SPEC_BANDS):
                self._smoothed_in[i] += SPEC_EMA * \
                    (self._input_bands[i] - self._smoothed_in[i])
                self._smoothed_out[i] += SPEC_EMA * \
                    (self._output_bands[i] - self._smoothed_out[i])
            self.redraw()

    @staticmethod
    def _compute_bands(samples):
        # 只取前 80 段：Mel 轴 20Hz 起的前 80 bin（覆盖到约 7kHz）
        try:
            from pvengine import compute_spectrum
            return list(compute_spectrum(samples))[:SPEC_BANDS]
        except Exception:
            return [DB_MIN] * SPEC_BANDS

    # ── 绘制 ──
    def redraw(self):
        w = max(self.winfo_width(), 120)
        h = max(self.winfo_height(), 40)
        self.delete("all")
        T, Bm = 0, 0                    # 顶/底不留边：避免上缘露出一条底色带
        gw, gh = w, h - T - Bm          # 铺满全宽全高：纯频谱无网格
        if gw < 20 or gh < 10:
            return

        # 每段一整列：先铺满底槽（静音段也在，连续不断裂），再叠三色柱。
        # 三色柱按高度分绿/黄/红三区，每段最多 3 个矩形（省计算）。
        step = gw / SPEC_BANDS
        gap = 1 if step >= 4 else 0
        col_w = max(1.0, step - gap)

        def y_of(db):
            db = max(DB_MIN, min(DB_MAX, db))
            return T + gh - (db - DB_MIN) / DB_RANGE * gh

        zones = ((DB_MIN, SPEC_Z1_DB, SPEC_Z1),
                 (SPEC_Z1_DB, SPEC_Z2_DB, SPEC_Z2),
                 (SPEC_Z2_DB, DB_MAX, SPEC_Z3))
        for i in range(SPEC_BANDS):
            bx = i * step
            in_db = max(DB_MIN, self._smoothed_in[i])
            out_db = max(DB_MIN, self._smoothed_out[i])
            # 底槽：整列满高（含静音段），保证槽位连续
            self.create_rectangle(bx, T, bx + col_w, T + gh,
                                  fill=SPEC_SLOT, width=0)
            if out_db > DB_MIN:
                for lo_db, hi_db, color in zones:
                    seg_lo = max(DB_MIN, lo_db)
                    seg_hi = min(out_db, hi_db)
                    if seg_hi > seg_lo:
                        self.create_rectangle(bx, y_of(seg_hi),
                                              bx + col_w, y_of(seg_lo),
                                              fill=color, width=0)
            if in_db > out_db:
                # 输入高于输出：上方蓝灰段 = 噪声残留
                self.create_rectangle(bx, y_of(in_db), bx + col_w,
                                      y_of(out_db), fill=SPEC_NOISE, width=0)

class LevelRing(tk.Canvas):
    """圆形运行指示灯：运行绿圈 / 停止灰圈。"""

    def __init__(self, parent, size=14):
        super().__init__(parent, bg=theme.WINDOW, width=size, height=size,
                         highlightthickness=0, bd=0)
        self._on = False
        self._size = size
        self._draw()

    def _draw(self):
        s = self._size
        self.delete("all")
        color = theme.START_BG if self._on else theme.BUTTON
        outline = theme.MID if not self._on else theme.START_HOVER
        self.create_oval(2, 2, s - 2, s - 2, fill=color, outline=outline,
                         width=1)
