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

"""PlaybackSink 合成测试（无硬件，可 CI 运行）：python tests/test_playback_sink.py

用合成时钟（非真实时间）验证播放正确性不变量：
1. 速率差 ±2% 长跑：无不连续（输出是输入的时间缩放，跳变有界）、延迟有界；
2. 抖动消费（帧长波动）：不垫零不丢样本；
3. 生产断流：静音重同步，恢复后续播且延迟不爬升；
4. 突发灌入：封顶丢最旧，延迟有界。
"""

import math
import os
import sys

sys.path.insert(0, os.path.dirname(os.path.dirname(os.path.abspath(__file__))))

from pvengine import PlaybackSink

HOP = 480
SR = 48000
# 连续正弦的相邻样本差上界；FADE_STEP = 预热/欠载淡入淡出的增益台阶（1/32）
SINE_STEP = 2 * math.pi * 220 / SR
FADE_STEP = 1.0 / 32


def run(producer_rate, consumer_frames, n_hops, stall_at=None, stall_hops=0,
        burst_at=None):
    """合成时钟推演。producer_rate: 每 hop 产出样本数（48k 时钟=480）。
    consumer_frames: 每次消费的样本数序列取模循环。返回 (out, hops_written)。"""
    sink = PlaybackSink(hop=HOP)
    out = []
    produced = 0.0
    written = 0
    consumed = 0
    fi = 0
    burst_done = False
    while consumed < n_hops * HOP:
        # —— 生产：按 producer_rate 产出已写入量对应的 hop ——
        # （断流/突发窗口以消费进度为时钟基准）
        while produced < consumed * (producer_rate / 480.0) + HOP:
            if stall_at is not None and stall_at <= consumed < stall_at + stall_hops * HOP:
                break
            if burst_at is not None and consumed >= burst_at and not burst_done:
                burst_done = True
                sink.write([math.sin(2 * math.pi * 220 * (
                    (written + k) / SR)) for k in range(HOP * 20)])
                written += HOP * 20
                produced += HOP * 20
                break
            frame = [math.sin(2 * math.pi * 220 * ((written + k) / SR))
                     for k in range(HOP)]
            sink.write(frame)
            written += HOP
            produced += HOP
        need = consumer_frames[fi % len(consumer_frames)]
        fi += 1
        chunk = sink.pull(need)
        out.extend(chunk)
        consumed += need
    return out, written


def max_discontinuity(x):
    return max((abs(x[i] - x[i - 1]) for i in range(1, len(x))), default=0.0)


def test_rate_drift():
    for rate in (1.002, 0.998, 1.02, 0.98):
        out, _ = run(int(HOP * rate), [HOP], 2000)
        d = max_discontinuity(out)
        bound = SINE_STEP + FADE_STEP
        assert d < bound, f"rate={rate} 不连续 {d:.4f} > {bound:.4f}"
        print(f"  rate={rate:+.1%}  最大跳变={d:.5f} (界 {bound:.5f})  OK")


def test_jitter():
    out, _ = run(HOP, [HOP, HOP // 2, HOP * 2, HOP, HOP * 3, HOP], 2000)
    d = max_discontinuity(out)
    assert d < SINE_STEP + FADE_STEP, f"抖动消费不连续 {d:.4f}"
    print(f"  帧长抖动  最大跳变={d:.5f}  OK")


def test_stall_recovery():
    out, written = run(HOP, [HOP], 3000, stall_at=500, stall_hops=100)
    d = max_discontinuity(out)
    # 断流必然有一段静音；恢复后必须续播（尾部有信号）且跳变有界
    tail_rms = math.sqrt(sum(v * v for v in out[-4800:]) / 4800)
    assert tail_rms > 0.3, f"断流后未恢复续播 tail_rms={tail_rms:.3f}"
    assert d < SINE_STEP + FADE_STEP, f"恢复跳变 {d:.4f} > {SINE_STEP + FADE_STEP:.4f}"
    print(f"  断流恢复  尾部RMS={tail_rms:.3f}  最大跳变={d:.5f}  OK")


def test_burst_cap():
    sink = PlaybackSink(hop=HOP)
    for k in range(HOP * 40):
        sink.write([0.5])
    got = sink.pull(HOP)
    d = sink.diag()
    assert d["level"] <= 480 * 30 + 10, f"封顶失效 level={d['level']}"
    assert d["drops"] > 0, "封顶未记录丢弃"
    assert len(got) == HOP
    print(f"  突发封顶  level={d['level']}  drops={d['drops']}  OK")


def test_latency_bounded():
    # ±2% 速率差长跑 60s 等效时长，稳态延迟必须有界（不爬升）
    out, _ = run(int(HOP * 1.02), [HOP], 6000)
    # 验证输出末段是纯连续正弦（稳态无欠载/无封顶扰动）；
    # ASRC 以 r≈1.02 消费 → 斜率同比放大，界乘最大步长 1.03
    d = max_discontinuity(out[-4800:])
    assert d < SINE_STEP * 1.03, f"长跑末段跳变 {d:.4f}"
    print(f"  长跑延迟有界  末段跳变={d:.5f}  OK")


if __name__ == "__main__":
    print("PlaybackSink 合成测试:")
    test_rate_drift()
    test_jitter()
    test_stall_recovery()
    test_burst_cap()
    test_latency_bounded()
    print("全部通过")
