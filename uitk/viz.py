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
Spectrum 128 段 Mel 实时输入/输出频谱重叠对比（pvengine.compute_spectrum，
        dB 域 -90..-20；输出=绿基准，输入>输出=灰(噪声残留)，
        输入<输出=浅(增强)；EMA α=0.3 平滑；960 窗(=2×hop) / 480 步进累积）。
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
    NUM_BANDS = 128
    FFT_SIZE = 960              # 2×hop @48kHz（FFT 无损窗长）
MIN_SAMPLES = FFT_SIZE // 2     # 累积步进 = 480 = hop
SPEC_BANDS = 80   # 频谱只画前 80 段（自 20Hz 起）
DB_MIN, DB_MAX = -90.0, -20.0
DB_RANGE = DB_MAX - DB_MIN
SPEC_EMA = 0.3
SPEC_BAR_OUT = "#4CAF50"
SPEC_BAR_MORE = "#1B5E20"    # 噪声残留：深绿
SPEC_BAR_LESS = "#A5D6A7"    # 增强：浅绿
SPEC_GRID = "#3a3a50"
SPEC_TEXT_C = "#666688"


VU_LIT_GREEN, VU_LIT_YELLOW, VU_LIT_RED = "#4CAF50", "#FFD54F", "#EF5350"


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
        super().__init__(parent,
                         bg=parent.cget('bg') if isinstance(parent, tk.Widget) else theme.PANEL,
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
            return "#FFD54F"
        return "#EF5350"

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


class AgcGainMeter(tk.Canvas):
    """AGC 实时增益可视化条：0dB 居中，向右绿色=放大，向左橙色=衰减，
    条上叠加当前 dB 数字。±30dB 范围（与 AgcController gain_min/gain_max 对齐）。"""

    GAIN_RANGE = 30.0  # ±dB

    def __init__(self, parent, sizes=None, height=26):
        self.sizes = sizes or make_sizes(100)
        s = self.sizes["scale"]
        self._font = ("TkDefaultFont", max(8, int(round(9 * s))), "bold")
        super().__init__(parent,
                         bg=theme.PANEL,
                         highlightthickness=0, bd=0, height=height)
        self._gain = 0.0
        self._target = 0.0
        self._active = False
        self._t = time.monotonic()
        self._last_painted = -999.0
        self.bind("<Configure>", lambda e: self.redraw(force=True))

    def set_active(self, active: bool):
        if active != self._active:
            self._active = active
            if not active:
                self._target = 0.0
            self.redraw(force=True)

    def update_gain(self, gain_db: float | None):
        """33ms 周期调用。None 表示无数据（显示「AGC 未运行」）。
        注意：AgcController.gain_db 已经过 EMA + attack/release 平滑，
        这里直接用传入值，不做二次平滑，避免响应迟钝看不见变化。"""
        if gain_db is None:
            self._active = False
            self._gain = 0.0
        else:
            self._active = True
            self._gain = max(-self.GAIN_RANGE, min(self.GAIN_RANGE, gain_db))
        if abs(self._gain - self._last_painted) >= 0.3 or not self._active:
            self._last_painted = self._gain
            self.redraw()

    def redraw(self, force=False):
        w = max(self.winfo_width(), 60)
        h = max(self.winfo_height(), 16)
        self.delete("all")
        pad = 3
        T, B = pad, h - pad
        cy = (T + B) // 2
        # 背景槽（浅灰）
        self.create_rectangle(0, 0, w, h, fill="#F5F5F5", width=0)
        # 0dB 中线
        mid_x = w // 2
        self.create_line(mid_x, T, mid_x, B, fill=theme.MID, width=1)
        if not self._active:
            self.create_text(mid_x, cy, text="AGC 未运行",
                             font=self._font, fill=theme.TEXT_FAINT)
            return
        # 增益条：从中线向两侧填充
        half_w = mid_x - pad - 2
        ratio = self._gain / self.GAIN_RANGE  # -1..1
        bar_w = 0  # 确保 ratio==0 路径下也有定义，避免 UnboundLocalError
        if ratio > 0:
            bar_w = int(round(ratio * half_w))
            x0, x1 = mid_x, mid_x + bar_w
            fill = "#4CAF50" if ratio < 0.5 else "#388E3C"
        elif ratio < 0:
            bar_w = int(round(-ratio * half_w))
            x0, x1 = mid_x - bar_w, mid_x
            fill = "#FFB74D" if ratio > -0.5 else "#F57C00"
        else:
            x0 = x1 = mid_x
            fill = "#BDBDBD"
        if bar_w > 1:
            self.create_rectangle(x0, T, x1, B, fill=fill, width=0)
        # 数字标签：先在文字下方画浅灰底色块，确保深棕字在任何条色上都清晰
        sign = "+" if self._gain >= 0 else ""
        txt = f"{sign}{self._gain:.1f} dB"
        bbox = self.create_text(mid_x, cy, text=txt, font=self._font,
                                fill=theme.TEXT)
        x1_t, y1_t, x2_t, y2_t = self.bbox(bbox)
        pad_t = 3
        # 底色块（与背景槽同色）覆盖文字区域，再重画文字
        self.create_rectangle(x1_t - pad_t, y1_t - pad_t,
                              x2_t + pad_t, y2_t + pad_t,
                              fill="#F5F5F5", outline=theme.MID, width=1)
        self.create_text(mid_x, cy, text=txt, font=self._font,
                         fill=theme.TEXT)


class SpectrumCanvas(tk.Canvas):
    """128 段 Mel 频谱重叠对比（legacy SpectrumWidget 的 tk 移植）。"""

    BAR_W, GAP = 3, 1

    def __init__(self, parent, sizes=None, height=220):
        self.sizes = sizes or make_sizes(100)
        self._lbl_font = ("TkDefaultFont", max(7, int(round(7 * self.sizes["scale"]))))
        super().__init__(parent,
                         bg=parent.cget('bg') if isinstance(parent, tk.Widget) else theme.PANEL,
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
        T, Bm = 2, 2
        gw, gh = w, h - T - Bm          # 铺满全宽、几乎全高：纯频谱无网格
        if gw < 20 or gh < 10:
            return

        # 频谱柱：固定条宽/间隔（随缩放），铺满全宽
        step = gw / SPEC_BANDS
        bar_w = max(2.0, step - max(1, int(self.GAP * self.sizes["scale"])))
        for i in range(SPEC_BANDS):
            in_db = max(DB_MIN, self._smoothed_in[i])
            out_db = max(DB_MIN, self._smoothed_out[i])
            out_h = (out_db - DB_MIN) / DB_RANGE * gh
            in_h = (in_db - DB_MIN) / DB_RANGE * gh
            if out_h < 1 and in_h < 1:
                continue
            bx = i * step
            y_out = T + gh - out_h
            y_in = T + gh - in_h
            if out_h > 1:
                self.create_rectangle(bx, y_out, bx + bar_w, T + gh,
                                      fill=SPEC_BAR_OUT, width=0)
            if in_db > out_db and in_h > 1:
                # 输入高于输出：上方灰色段 = 噪声残留
                self.create_rectangle(bx, y_in, bx + bar_w, y_out,
                                      fill=SPEC_BAR_MORE, width=0)
            elif in_db < out_db and out_h > 1:
                # 输出更强：浅色段 = 增强
                self.create_rectangle(bx, y_out, bx + bar_w, y_in,
                                      fill=SPEC_BAR_LESS, width=0)

    @staticmethod
    def _hz_to_frac(hz):
        """Hz → Mel 轴 0..1（20Hz~8kHz，与 80 段显示范围一致）。"""
        def hz_to_mel(f):
            return 2595.0 * math.log10(1.0 + f / 700.0)
        lo, hi = hz_to_mel(20.0), hz_to_mel(8000.0)
        return (hz_to_mel(max(20.0, min(8000.0, hz))) - lo) / (hi - lo)


class LevelRing(tk.Canvas):
    """圆形运行指示灯：运行绿圈 / 停止灰圈。"""

    def __init__(self, parent, size=14):
        super().__init__(parent, bg=theme.WINDOW, width=size, height=size,
                         highlightthickness=0, bd=0)
        self._on = False
        self._size = size
        self._draw()

    def set_on(self, on):
        self._on = bool(on)
        self._draw()

    def _draw(self):
        s = self._size
        self.delete("all")
        color = theme.START_BG if self._on else theme.BUTTON
        outline = theme.MID if not self._on else theme.START_HOVER
        self.create_oval(2, 2, s - 2, s - 2, fill=color, outline=outline,
                         width=1)
