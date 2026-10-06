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

"""AGC 合成测试（纯 DSP，无硬件）：python tests/test_agc.py

验证弹道行为与门限：
- 稳态有声信号收敛到目标峰值（-12dBFS 附近，不超上限）；
- 静音/低于门限不补增益（增益保持 1.0，不抬底噪）；
- 突发后增益降低快、静音后增益缓慢回落（不抽吸）；
- 输出 soft clip 有界、无 NaN。
"""

import math
import os
import sys

sys.path.insert(0, os.path.dirname(os.path.dirname(os.path.abspath(__file__))))

import numpy as np

from pvengine.components.agc import AgcPlugin, _soft_clip

HOP = 480


def _frames(amp, seconds, freq=1000.0):
    n = int(48000 * seconds)
    t = np.arange(n) / 48000.0
    sig = (amp * np.sin(2 * math.pi * freq * t)).astype(np.float32)
    return [sig[i:i + HOP] for i in range(0, n - HOP + 1, HOP)]


def _db(x):
    return 20.0 * math.log10(max(float(x), 1e-10))


def test_steady_converges_to_target():
    agc = AgcPlugin()
    amp = 10 ** (-30 / 20.0)
    out_peak = 0.0
    for f in _frames(amp, 1.0):
        out = agc.process(f, None)
        out_peak = float(np.max(np.abs(out)))
        assert not np.isnan(out).any()
    # 稳态输出峰值应接近 -12dBFS（目标），而非无脑顶到上限
    assert 0.18 <= out_peak <= 0.32, f"out_peak={out_peak} ({_db(out_peak):.1f}dB)"
    gdb = agc.get_agc_gain_db()
    assert 15.0 <= gdb <= 20.5, f"gain={gdb:.1f}dB"
    print(f"  稳态收敛: 输入-30dB → 输出{_db(out_peak):.1f}dB gain={gdb:+.1f}dB  OK")


def test_silence_no_boost():
    agc = AgcPlugin()
    for _ in range(50):
        out = agc.process(np.zeros(HOP, dtype=np.float32), None)
        assert float(np.max(np.abs(out))) == 0.0
    assert abs(agc.get_agc_gain_db()) < 1e-6, agc.get_agc_gain_db()
    print("  静音不补增益 (gain=0dB)  OK")


def test_below_floor_no_boost():
    # -60dBFS 低于 -55dBFS 门限：不补增益，原样（soft clip 线性段）
    agc = AgcPlugin()
    amp = 10 ** (-60 / 20.0)
    for f in _frames(amp, 1.0):
        agc.process(f, None)
    assert abs(agc.get_agc_gain_db()) < 1e-6, agc.get_agc_gain_db()
    print("  低于门限(-60dB)不补增益  OK")


def test_gain_limits_clamped():
    # 低于目标但高于门限的弱信号：增益顶到上限 +30dB
    lo = AgcPlugin()
    for f in _frames(10 ** (-50 / 20.0), 0.5):
        lo.process(f, None)
    assert 29.0 <= lo.get_agc_gain_db() <= 30.5, lo.get_agc_gain_db()
    # 极强信号：衰减触底 -30dB
    hi = AgcPlugin()
    for f in _frames(10.0, 0.5):
        hi.process(f, None)
    assert -30.5 <= hi.get_agc_gain_db() <= -29.0, hi.get_agc_gain_db()
    print("  增益限幅 ±30dB 触顶/触底  OK")


def test_max_gain_slider():
    # max_gain_db=0 → 只衰减不提升
    agc = AgcPlugin({"max_gain_db": 0.0})
    for f in _frames(10 ** (-30 / 20.0), 0.5):
        agc.process(f, None)
    assert abs(agc.get_agc_gain_db()) < 1e-6, agc.get_agc_gain_db()
    # max_gain_db=6 → 提升被夹到 +6dB（原本会 +18dB）
    agc2 = AgcPlugin({"max_gain_db": 6.0})
    for f in _frames(10 ** (-30 / 20.0), 0.5):
        agc2.process(f, None)
    assert 5.0 <= agc2.get_agc_gain_db() <= 6.5, agc2.get_agc_gain_db()
    # 运行时改滑杆（set_params）即时生效
    agc2.set_params({"max_gain_db": 12.0})
    for f in _frames(10 ** (-30 / 20.0), 0.5):
        agc2.process(f, None)
    assert 10.5 <= agc2.get_agc_gain_db() <= 12.5, agc2.get_agc_gain_db()
    print("  最大增益滑杆 0/6/12dB 生效  OK")


def test_release_returns_to_unity():
    agc = AgcPlugin()
    # 大声 0.5 → 增益压低；随后静音应缓慢回落到 unity，不卡在高增益
    for f in _frames(0.5, 0.6):
        agc.process(f, None)
    assert agc.get_agc_gain_db() < -3.0, agc.get_agc_gain_db()
    for _ in range(200):        # 2s 静音
        agc.process(np.zeros(HOP, dtype=np.float32), None)
    assert agc.get_agc_gain_db() > -0.5, agc.get_agc_gain_db()
    print("  突发后静音增益回落 unity  OK")


def test_no_pumping_on_speech_like():
    # 近似语音包络：有声/无声交替，检查增益不来回跳（相邻帧变化有界）
    agc = AgcPlugin()
    prev = None
    max_step = 0.0
    for k in range(120):
        if (k // 5) % 2 == 0:
            f = _frames(0.2, HOP / 48000.0)[0]
        else:
            f = np.zeros(HOP, dtype=np.float32)
        agc.process(f, None)
        g = agc.get_agc_gain_db()
        if prev is not None:
            max_step = max(max_step, abs(g - prev))
        prev = g
    # 非对称弹道下，单帧（10ms）增益变化应远小于 20dB（旧实现会瞬间跳满）
    assert max_step < 8.0, f"max_step={max_step:.1f}dB"
    print(f"  语音包络不抽吸: 单帧最大变化 {max_step:.1f}dB  OK")


def test_soft_clip_bounded():
    x = np.array([-5.0, -1.0, -0.5, 0.0, 0.5, 1.0, 5.0], dtype=np.float32)
    y = _soft_clip(x)
    assert np.all(np.abs(y) <= 1.0), y
    assert abs(y[3]) < 1e-6 and abs(y[2] - (-0.5)) < 1e-6 and abs(y[4] - 0.5) < 1e-6
    print("  soft clip 有界/线性段保持  OK")


if __name__ == "__main__":
    print("AGC 合成测试:")
    test_steady_converges_to_target()
    test_silence_no_boost()
    test_below_floor_no_boost()
    test_gain_limits_clamped()
    test_max_gain_slider()
    test_release_returns_to_unity()
    test_no_pumping_on_speech_like()
    test_soft_clip_bounded()
    print("全部通过")
