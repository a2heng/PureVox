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

"""64 段 Mel 频谱直方图（UI 频谱显示数据源）。

HTK mel 刻度 20Hz–16kHz 三角滤波器组，Hann 窗 FFT，
功率谱 scale=1/nfft²，dB 输出 clamp [-90, -20]，静音带钉在 -90。
窗长恒为 2×hop（NFFT=960，FFT 无损窗长，随 hop 派生）。
"""

import numpy as np

from pvengine.context import NFFT, SAMPLE_RATE

SPECTRUM_NUM_BANDS = 64
SPECTRUM_FFT = NFFT                    # 2×hop = 960 @48kHz
_SPECTRUM_SR = float(SAMPLE_RATE)
_MEL_LOW = 20.0
_MEL_HIGH = 16000.0
_DB_FLOOR = -90.0
_DB_CEIL = -20.0


def _hz_to_mel(hz):
    return 2595.0 * np.log10(1.0 + np.asarray(hz) / 700.0)


def _mel_to_hz(mel):
    return 700.0 * (np.power(10.0, np.asarray(mel) / 2595.0) - 1.0)


def _build_filterbank():
    n_fft = SPECTRUM_FFT
    mel_min, mel_max = _hz_to_mel(_MEL_LOW), _hz_to_mel(_MEL_HIGH)
    i = np.arange(SPECTRUM_NUM_BANDS + 2)
    centers = _mel_to_hz(mel_min + (mel_max - mel_min) * i / (SPECTRUM_NUM_BANDS + 1))
    fl, fc, fr = centers[:-2], centers[1:-1], centers[2:]
    freqs = np.arange(n_fft // 2 + 1) * (_SPECTRUM_SR / n_fft)
    f = freqs[:, None]                      # (bins, 1)
    lo, ce, hi = fl[None, :], fc[None, :], fr[None, :]
    rise = ((f >= lo) & (f <= ce)) * (f - lo) / np.where(ce > lo, ce - lo, 1.0)
    fall = ((f > ce) & (f <= hi)) * (hi - f) / np.where(hi > ce, hi - ce, 1.0)
    return (np.maximum(rise, 0.0) + np.maximum(fall, 0.0)).astype(np.float64)


_FILTERBANK = None


def spectrum_warmup():
    """预建 Mel 滤波器组（compute_spectrum 首帧懒建，亦可提前调用预热）。"""
    global _FILTERBANK
    if _FILTERBANK is None:
        _FILTERBANK = _build_filterbank()


def compute_spectrum(samples):
    """输入最新一段样本（不足 960=2×hop 前置补零），返回 128 个 dB 值 list。"""
    spectrum_warmup()
    if not len(samples):
        return [ _DB_FLOOR ] * SPECTRUM_NUM_BANDS
    x = np.asarray(samples[-SPECTRUM_FFT:], dtype=np.float32)
    buf = np.zeros(SPECTRUM_FFT)
    buf[SPECTRUM_FFT - len(x):] = x
    win = 0.5 - 0.5 * np.cos(2.0 * np.pi * np.arange(SPECTRUM_FFT) / SPECTRUM_FFT)
    spec = np.fft.rfft(buf * win)
    power = (spec.real ** 2 + spec.imag ** 2) / float(SPECTRUM_FFT * SPECTRUM_FFT)
    energy = power @ _FILTERBANK
    db = 10.0 * np.log10(np.maximum(energy, 1e-300))
    out = np.where(energy > 1e-12, np.clip(db, _DB_FLOOR, _DB_CEIL), _DB_FLOOR)
    return [float(v) for v in out]
