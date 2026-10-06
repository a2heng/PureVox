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

"""EQ 组件——单一人声图示均衡器 + 高切/低切（标准做法）。

- 唯一栅格 EQ_FREQS：13 段人声频点（80 Hz ~ 8 kHz），中段约 1/2 倍频程、
  低段稍疏、高段稍密；8 kHz 以上不设可调点（人声能量止于此），
  展示面仍画全 20 Hz ~ 20 kHz 以便低切/高切可视化；
- 每段一个 RBJ peaking biquad（audio-eq-cookbook 系式）；
- Q 为逐段匹配：Q[i] = √(2^N)/(2^N − 1)，N[i] 取该段与相邻段的
  对数间隔（边缘段取单边），相邻段提升在段间自然叠加成平台；
- 高切/低切为可选 5 阶巴特沃斯（30 dB/oct：1 个一阶节 + 2 个二阶节），
  信号流顺序：低切 → 峰值段级联 → 高切；
- 全零增益且无切滤时整体旁路；运行时跳过零增益段（恒等滤波器，跳过不改输出）；
- IIR 递推走 scipy.signal.lfilter（C 实现），zi 状态跨帧连续。

response_at() 是全链频响的唯一权威实现：引擎与 UI 曲线共用同一份系数与 Q。
"""

import math

import numpy as np
from scipy.signal import lfilter

from pvengine.context import FrameContext, SAMPLE_RATE
from pvengine.stages.base import Stage


def _matched_q(n_octaves: float) -> float:
    """图示 EQ 标准取法：-3dB 带宽 = n_octaves 倍频程的匹配 Q。"""
    b = 2.0 ** n_octaves
    return (b ** 0.5) / (b - 1.0)


# ── 唯一栅格：13 段人声频点（80 Hz ~ 8 kHz，全部点位在此区间内；
# 展示面固定画 20 Hz ~ 20 kHz 全范围，极高/极低频只为低切/高切可视化）──
EQ_FREQS = (
    80.0, 150.0, 250.0, 400.0, 600.0, 850.0, 1200.0,
    1700.0, 2400.0, 3400.0, 4800.0, 6500.0, 8000.0,
)
# 展示横轴（固定 20 Hz ~ 20 kHz，值不变；位置映射见 Canvas _X_WARP，
# 前段（80 Hz 以前无点位）位置压缩、后段放宽）
EQ_VIEW_LO = 20.0
EQ_VIEW_HI = 20000.0
# 单段可调范围（±dB）：UI 拖拽/滚轮钳制于此，展示纵轴 ±30 dB
# （多出的空间给高/低切下潜到 -30 dB 用）
EQ_GAIN_LIMIT = 10.0


def _band_qs(freqs, width_mul: float = 2.0) -> tuple:
    """逐段匹配 Q：N[i] 取该段与相邻段的对数间隔（边缘段取单边），
    再展宽 width_mul 倍——相邻段大幅交叠，曲线平滑无纹波
    （代价是相邻同向提升叠加更高，属音乐性 EQ 取舍）。"""
    fs = tuple(float(f) for f in freqs)
    n = len(fs)
    out = []
    for i in range(n):
        if n == 1:
            octv = 1.0
        elif i == 0:
            octv = math.log2(fs[1] / fs[0])
        elif i == n - 1:
            octv = math.log2(fs[-1] / fs[-2])
        else:
            octv = math.log2(fs[i + 1] / fs[i - 1]) / 2.0
        out.append(_matched_q(max(octv, 1e-3) * width_mul))
    return tuple(out)


EQ_QS = _band_qs(EQ_FREQS)
# 兼容标量：各段 Q 均值（仅供外部估算用，引擎/UI 一律用 EQ_QS）
EQ_Q = sum(EQ_QS) / len(EQ_QS)


def _norm_qs(q, freqs) -> tuple:
    """归一化 Q 参数：None/≤0 → 默认匹配；标量 → 广播；序列 → 逐段。"""
    n = len(freqs)
    if q is None:
        return EQ_QS if tuple(freqs) == EQ_FREQS else _band_qs(freqs)
    if isinstance(q, (int, float)):
        if float(q) <= 0.0:
            return EQ_QS if tuple(freqs) == EQ_FREQS else _band_qs(freqs)
        return (float(q),) * n
    qs = tuple(float(x) for x in q)
    if len(qs) != n:
        raise ValueError("q length %d != freqs length %d" % (len(qs), n))
    return qs


# 插件名 → 规范 id：只有单一人声 "eq"；旧 eq10/eq31/eq61
# 按强配置原则视为未知类型直接丢弃（不做迁移），见 session_plan 注释。


def _peaking_eq(freq: float, gain_db: float, q: float, fs: float):
    """RBJ peaking EQ 双二阶系数，返回 (b0,b1,b2,a1,a2)（a0 归一化）。"""
    a = 10.0 ** (gain_db / 40.0)
    w0 = 2.0 * np.pi * freq / fs
    cos_w0 = np.cos(w0)
    sin_w0 = np.sin(w0)
    alpha = sin_w0 / (2.0 * q)
    a0 = 1.0 + alpha / a
    return ((1.0 + alpha * a) / a0,
            (-2.0 * cos_w0) / a0,
            (1.0 - alpha * a) / a0,
            (-2.0 * cos_w0) / a0,
            (1.0 - alpha / a) / a0)


def _butter_cut(freq: float, fs: float, highpass: bool):
    """RBJ 二阶巴特沃斯切滤（Q=1/√2，12 dB/oct），返回 (b0,b1,b2,a1,a2)。

    仅供参考保留的单节实现；运行时切滤走下面的 5 阶级联。"""
    q = 1.0 / math.sqrt(2.0)
    w0 = 2.0 * math.pi * min(max(freq, 1e-3), 0.499 * fs) / fs
    cos_w0 = math.cos(w0)
    sin_w0 = math.sin(w0)
    alpha = sin_w0 / (2.0 * q)
    if highpass:
        b0 = (1.0 + cos_w0) / 2.0
        b1 = -(1.0 + cos_w0)
    else:
        b0 = (1.0 - cos_w0) / 2.0
        b1 = 1.0 - cos_w0
    b2 = b0
    a0 = 1.0 + alpha
    return (b0 / a0, b1 / a0, b2 / a0,
            (-2.0 * cos_w0) / a0, (1.0 - alpha) / a0)


def _cut_sections(freq: float, fs: float, highpass: bool):
    """5 阶巴特沃斯切滤（30 dB/oct）的级联节：[(b, a), …]。

    经 scipy.signal.butter + tf2sos 精确设计（含预畸变校正，
    拐角就在设定频率处）；b/a 为完整多项式系数（a[0]=1）。
    手写 RBJ Q 级联在此会被双线性翘曲拉偏拐角，故不用。"""
    from scipy.signal import butter, tf2sos
    wn = min(max(float(freq), 1e-3), 0.499 * fs) / (fs / 2.0)
    sos = tf2sos(*butter(5, wn,
                         btype="highpass" if highpass else "lowpass"))
    return [((b0, b1, b2), (1.0, a1, a2))
            for b0, b1, b2, _a0, a1, a2 in sos]


def response_at(freq: float, gains, fs: float = SAMPLE_RATE,
                hp_hz: float = 0.0, lp_hz: float = 0.0,
                freqs=None, q=None) -> float:
    """全链在 freq 处的总响应（dB）：低切 × 峰值段级联 × 高切。
    hp_hz/lp_hz 为 0 表示未启用。freqs/q 缺省用人声栅格与逐段匹配 Q；
    q 可为 None/标量（广播）/逐段序列。UI 曲线绘制与本函数共用。"""
    if freqs is None:
        freqs = EQ_FREQS
    qs = _norm_qs(q, freqs)
    total_db = 0.0
    w = 2.0 * math.pi * freq / fs
    _c = [math.cos(k * w) for k in range(3)]
    _s = [math.sin(k * w) for k in range(3)]

    def _mag_sec(b, a):
        nr = sum(bk * _c[k] for k, bk in enumerate(b))
        ni = -sum(bk * _s[k] for k, bk in enumerate(b))
        dr = sum(ak * _c[k] for k, ak in enumerate(a))
        di = -sum(ak * _s[k] for k, ak in enumerate(a))
        return 20.0 * math.log10(math.hypot(nr, ni) / math.hypot(dr, di))

    def _mag(b0, b1, b2, a1, a2):
        return _mag_sec((b0, b1, b2), (1.0, a1, a2))

    if hp_hz > 0.0:
        for b, a in _cut_sections(hp_hz, fs, highpass=True):
            total_db += _mag_sec(b, a)
    for i, g in enumerate(gains):
        if i >= len(freqs) or abs(float(g)) < 1e-9:
            continue
        total_db += _mag(*_peaking_eq(freqs[i], float(g), qs[i], fs))
    if lp_hz > 0.0:
        for b, a in _cut_sections(lp_hz, fs, highpass=False):
            total_db += _mag_sec(b, a)
    return total_db


class EqStage(Stage):
    name = "eq"

    def __init__(self, freqs=None, q=None,
                 sample_rate: float = SAMPLE_RATE):
        super().__init__()
        self.fs = sample_rate
        self._freqs = tuple(freqs) if freqs is not None else EQ_FREQS
        self._qs = _norm_qs(q, self._freqs)
        n = len(self._freqs)
        self.active = False
        self._active_idx: tuple[int, ...] = ()
        self._coeffs = [_peaking_eq(f, 0.0, self._qs[i], sample_rate)
                        for i, f in enumerate(self._freqs)]
        self._zi = [np.zeros(2) for _ in range(n)]
        # 高切/低切（5 阶巴特沃斯，30 dB/oct，级联节）
        self._hp_on = False
        self._lp_on = False
        self._hp_hz = 80.0
        self._lp_hz = 8000.0
        self._hp_secs = _cut_sections(self._hp_hz, sample_rate, highpass=True)
        self._lp_secs = _cut_sections(self._lp_hz, sample_rate, highpass=False)
        self._zi_hp = [np.zeros(len(b) - 1) for b, _a in self._hp_secs]
        self._zi_lp = [np.zeros(len(b) - 1) for b, _a in self._lp_secs]

    def _set_cut(self, which: str, enabled: bool, hz: float) -> None:
        """切滤开关/频率设置共用：频率变更或重新启用时清零该路状态。"""
        hz = float(min(max(float(hz), 1.0), 0.499 * self.fs))
        on_key, hz_key = (("_hp_on", "_hp_hz") if which == "hp"
                          else ("_lp_on", "_lp_hz"))
        restate = (enabled and not getattr(self, on_key)) or \
            (enabled and getattr(self, on_key) and hz != getattr(self, hz_key))
        setattr(self, on_key, bool(enabled))
        setattr(self, hz_key, hz)
        secs = _cut_sections(hz, self.fs, highpass=(which == "hp"))
        if which == "hp":
            self._hp_secs = secs
        else:
            self._lp_secs = secs
        if restate:
            if which == "hp":
                self._zi_hp = [np.zeros(len(b) - 1) for b, _a in secs]
            else:
                self._zi_lp = [np.zeros(len(b) - 1) for b, _a in secs]

    def set_highpass(self, enabled: bool, hz: float) -> None:
        """低切（高通）：enabled=False 即旁路。频率变更时清零滤波器状态。"""
        self._set_cut("hp", enabled, hz)

    def set_lowpass(self, enabled: bool, hz: float) -> None:
        """高切（低通）：enabled=False 即旁路。频率变更时清零滤波器状态。"""
        self._set_cut("lp", enabled, hz)

    def set_gains(self, gains) -> None:
        """设置全部段增益（dB，长度不足补 0）；全零且无切滤即旁路。
        新激活段清零滤波器状态。"""
        n = len(self._freqs)
        active = []
        prev_active = self._active_idx
        for i in range(n):
            g = float(gains[i]) if i < len(gains) else 0.0
            self._coeffs[i] = _peaking_eq(self._freqs[i], g, self._qs[i], self.fs)
            if g != 0.0:
                active.append(i)
        self._active_idx = tuple(active)
        for i in self._active_idx:
            if i not in prev_active:
                self._zi[i][:] = 0.0
        self.active = bool(active)

    def mirror(self, other: "EqStage") -> None:
        """复制另一实例的系数/开关（不复制滤波器状态）——预览路径专用。

        频谱预览等旁路消费绝不能共享主链实例：两路不同信号轮流推进
        同一份 zi 会在每个帧边界产生不连续（可闻杂音）。镜像只取
        系数与开关，zi 保持自己独立连续。
        """
        self._coeffs = list(other._coeffs)
        self._active_idx = other._active_idx
        self.active = other.active
        self._hp_on = other._hp_on
        self._hp_hz = other._hp_hz
        self._hp_secs = list(other._hp_secs)
        self._lp_on = other._lp_on
        self._lp_hz = other._lp_hz
        self._lp_secs = list(other._lp_secs)

    def process(self, frame, ctx: FrameContext):
        if not (self.active or self._hp_on or self._lp_on):
            return frame
        y = frame.astype(np.float64)
        if self._hp_on:
            for (b, a), zi in zip(self._hp_secs, self._zi_hp):
                y, zf = lfilter(b, a, y, zi=zi)
                zi[:] = zf
        for i in self._active_idx:
            b0, b1, b2, a1, a2 = self._coeffs[i]
            y, self._zi[i] = lfilter([b0, b1, b2], [1.0, a1, a2], y, zi=self._zi[i])
        if self._lp_on:
            for (b, a), zi in zip(self._lp_secs, self._zi_lp):
                y, zf = lfilter(b, a, y, zi=zi)
                zi[:] = zf
        return y.astype(np.float32)

    def reset(self):
        self._zi = [np.zeros(2) for _ in range(len(self._freqs))]
        self._zi_hp = [np.zeros(len(b) - 1) for b, _a in self._hp_secs]
        self._zi_lp = [np.zeros(len(b) - 1) for b, _a in self._lp_secs]
