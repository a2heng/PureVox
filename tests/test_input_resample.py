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

"""Windows 本地输入自适应冒烟（无硬件可跑）：
python tests/test_input_resample.py

验证：
- native_hop_len：原生采样率 → 10ms hop 帧数（按时间派生）；
- downmix_mono：交织多声道 → 单声道等权平均；
- Resampler 流式 44100→48000：一秒 440Hz 正弦按 441 块喂入，
  总产出 ≈48000 样本、有限值、RMS 基本保持；
- PaBridge.input_info：未建流默认直通状态，不抛异常。
"""

import math
import os
import sys

sys.path.insert(0, os.path.dirname(os.path.dirname(os.path.abspath(__file__))))

from pvplatform.audio.pa_backend import (
    PaBridge, native_hop_len, downmix_mono)


def test_native_hop_len():
    assert native_hop_len(48000) == 480
    assert native_hop_len(44100) == 441
    assert native_hop_len(16000) == 160
    assert native_hop_len(96000) == 960
    assert native_hop_len(8000) == 80
    assert native_hop_len(11025) == 110
    assert native_hop_len(0) == 480   # 非法输入回退默认
    print("  native_hop_len  OK")


def test_downmix_mono():
    assert downmix_mono([0.5, -0.5, 0.25, 0.75], 2) == [0.0, 0.5]
    assert downmix_mono([1.0, 2.0, 3.0], 1) == [1.0, 2.0, 3.0]
    assert downmix_mono([], 2) == []
    got = downmix_mono([1.0, 1.0, 1.0, 2.0, 2.0, 2.0], 3)
    assert got == [1.0, 2.0]
    print("  downmix_mono  OK")


def test_resample_44100_to_48000():
    from pvengine.dsp.resampler import Resampler
    ratio = 48000.0 / 44100.0
    rs = Resampler()
    rs.process([0.0] * native_hop_len(44100), ratio)   # 与建流预热同做法
    out = []
    n_blocks = 100   # 1 秒：100 x 441 样本 @44100Hz
    for b in range(n_blocks):
        blk = [math.sin(2.0 * math.pi * 440.0 * (b * 441 + i) / 44100.0)
               for i in range(441)]
        out.extend(rs.process(blk, ratio))
    assert 47950 <= len(out) <= 48050, f"产出 {len(out)} 偏离 48000"
    assert all(math.isfinite(v) for v in out)
    rms = (sum(v * v for v in out) / len(out)) ** 0.5
    assert abs(rms - 2 ** -0.5) < 0.05, f"RMS {rms:.4f} 失真过大"
    print(f"  Resampler 44100→48000：{len(out)} 样本 RMS={rms:.4f}  OK")


def test_pabridge_input_info_default():
    bridge = PaBridge()
    info = bridge.input_info()
    assert info["dev_sr"] == 48000
    assert info["dev_ch"] == 1
    assert info["adaptive"] is False
    assert info["active"] is False
    print("  PaBridge.input_info 默认  OK")


def test_oneshot_resample_with_flush():
    """校准式一次性重采样：原生域录制 → 预热 → end_of_input 冲刷，长度落 48k 域。"""
    from pvengine.dsp.resampler import Resampler
    mic_sr = 44100
    ratio = 48000.0 / mic_sr
    secs = 2.0
    n = int(mic_sr * secs)
    rec = [math.sin(2.0 * math.pi * 440.0 * i / mic_sr) for i in range(n)]
    rs = Resampler()
    rs.process([0.0] * native_hop_len(mic_sr), ratio)   # 与校准预热同做法
    out = rs.process(rec, ratio, True)
    expect = int(48000 * secs)
    assert abs(len(out) - expect) <= 10, f"产出 {len(out)} 偏离 {expect}"
    assert all(math.isfinite(v) for v in out)
    print(f"  一次性重采样冲刷：{len(out)} 样本（期望~{expect}）  OK")


if __name__ == "__main__":
    print("输入自适应冒烟:")
    test_native_hop_len()
    test_downmix_mono()
    test_resample_44100_to_48000()
    test_pabridge_input_info_default()
    test_oneshot_resample_with_flush()
    print("全部通过")
