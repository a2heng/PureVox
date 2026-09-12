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

"""Mel 频谱合成测试（纯 DSP，无硬件）：python tests/test_spectrum.py

验证：64 段 / 20Hz–16kHz 契约、静音 -90、单音峰值落段、输出域 [-90,-20]。
"""

import math
import os
import sys

sys.path.insert(0, os.path.dirname(os.path.dirname(os.path.abspath(__file__))))

import numpy as np

from pvengine import compute_spectrum, SPECTRUM_NUM_BANDS

SR = 48000
FFT = 960


def _tone(hz, amp=0.5):
    t = np.arange(FFT) / SR
    return (amp * np.sin(2 * math.pi * hz * t)).astype(np.float32)


def test_band_count_and_range():
    assert SPECTRUM_NUM_BANDS == 64, SPECTRUM_NUM_BANDS
    v = compute_spectrum(_tone(1000.0))
    assert len(v) == 64
    assert all(-90.0 - 1e-6 <= z <= -20.0 + 1e-6 for z in v), (min(v), max(v))
    print("  64 段 + 输出域 [-90,-20]  OK")


def test_silence_floor():
    v = compute_spectrum(np.zeros(FFT))
    assert all(abs(z + 90.0) < 1e-6 for z in v)
    print("  静音恒为 -90  OK")


def test_tone_peaks_in_band():
    # 1kHz 落在中低段；12kHz 应落在最后 1/4 段（覆盖到 16kHz）
    lo = compute_spectrum(_tone(1000.0))
    hi = compute_spectrum(_tone(12000.0))
    lo_i = int(np.argmax(lo))
    hi_i = int(np.argmax(hi))
    assert 5 <= lo_i <= 40, lo_i
    assert hi_i >= 48, hi_i
    print(f"  单音落段: 1kHz→第{lo_i}段  12kHz→第{hi_i}段  OK")


if __name__ == "__main__":
    print("Mel 频谱测试:")
    test_band_count_and_range()
    test_silence_floor()
    test_tone_peaks_in_band()
    print("全部通过")
