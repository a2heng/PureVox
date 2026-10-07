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

"""AEC far 延迟校准纯函数合成测试（无硬件）：python tests/test_aec_calib.py

验证 pvengine.aec_calib 的探测音生成与 far↔mic 相对延迟估计：
1. make_probe：有限长度、无削顶；
2. 估计器在合成回声（far 先到、mic 滞后 d + 噪声）下给出正确 d；
3. 无回声（纯噪声 mic）→ None；数据不足 → None。
"""

import os
import sys

sys.path.insert(0, os.path.dirname(os.path.dirname(os.path.abspath(__file__))))

import numpy as np

from pvengine.aec_calib import make_probe, estimate_far_delay_ms

SR = 48000


def check(name, cond, detail=""):
    print(("  PASS " if cond else "  FAIL ") + name + (" — " + detail if detail else ""))
    return cond


def synthetic_mic(far, delay_ms, echo_gain=0.05, noise=0.004, seed=7):
    d = int(delay_ms * SR / 1000)
    mic = np.concatenate([np.zeros(d), far * echo_gain])[:len(far)]
    rng = np.random.default_rng(seed)
    return mic + rng.normal(0.0, noise, len(mic))


def main():
    ok = True
    print("AEC far 延迟校准（纯函数合成）:")

    probe = make_probe(SR)
    ok &= check("探测音有限长度>0", len(probe) > SR * 0.5, f"len={len(probe)}")
    ok &= check("探测音无削顶", float(np.max(np.abs(probe))) <= 1.0,
                f"peak={float(np.max(np.abs(probe))):.2f}")
    ok &= check("探测音含能量", float(np.max(np.abs(probe))) > 0.1)

    far = probe.astype(float)
    for d_ms in (2.0, 15.0, 80.0, 250.0):
        mic = synthetic_mic(far, d_ms)
        res = estimate_far_delay_ms(far.tolist(), mic.tolist(), fs=SR)
        got = None if res is None else res[0]
        good = got is not None and abs(got - d_ms) <= 2.0
        ok &= check(f"延迟 {d_ms:.0f}ms 估计准确", good,
                    f"got={got}")
        if res is not None:
            ok &= check(f"延迟 {d_ms:.0f}ms 相关度可信", res[1]["corr"] > 0.1,
                        f"corr={res[1]['corr']:.2f} snr={res[1]['snr']:.1f}")

    # 带外污染鲁棒性：强 50Hz 工频 + 宽带热噪声叠加，估计仍准确
    far = probe.astype(float)
    t = np.arange(len(far)) / SR
    hum = 0.3 * np.sin(2 * np.pi * 50 * t)
    mic = synthetic_mic(far, 90.0, echo_gain=0.05, noise=0.004, seed=5) + hum
    res = estimate_far_delay_ms(far.tolist(), mic.tolist(), fs=SR)
    good = res is not None and abs(res[0] - 90.0) <= 2.0
    ok &= check("带外工频/热噪下 90ms 仍准确", good, f"got={None if res is None else res[0]:.1f}")

    # 纯带外信号（无探测音）→ None（带通后近乎静音，不误报）
    hum_only = hum + np.random.default_rng(9).normal(0, 0.01, len(far))
    res = estimate_far_delay_ms(far.tolist(), hum_only.tolist(), fs=SR)
    ok &= check("纯带外噪声 → None", res is None)

    # 纯噪声（无回声）→ None
    rng = np.random.default_rng(3)
    noise_mic = rng.normal(0.0, 1.0, len(far))
    res = estimate_far_delay_ms(far.tolist(), noise_mic.tolist(), fs=SR)
    ok &= check("无回声噪声 → None", res is None)

    # 数据不足 → None
    res = estimate_far_delay_ms([0.1, -0.1], [0.0, 0.0, 0.1], fs=SR)
    ok &= check("数据不足 → None", res is None)

    print("ALL PASS" if ok else "FAILURES PRESENT")
    return 0 if ok else 1


if __name__ == "__main__":
    sys.exit(main())
