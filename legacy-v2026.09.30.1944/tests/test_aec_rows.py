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

"""行级 AEC 合成测试（无硬件；模型会话用桩代替，不加载 onnx）：
python tests/test_aec_rows.py

验证（外部时钟时间戳配对，无隐藏缓冲/无 HopQueue）：
1. GridHistory：按 ts 入历史、可按网格取/头部 pop、空洞重锚、封顶丢旧；
2. AecRow：far 按 mic 时间戳  - far_delay 切片取回声源（桩回显 far）；
   far 历史不足 → 直通 mic（不丢人声、不进模型）；
3. far 手动延迟方向与参考音量缩放。
"""

import os
import sys

sys.path.insert(0, os.path.dirname(os.path.dirname(os.path.abspath(__file__))))

import numpy as np

import pvengine.aec_row as aec_row_mod
from pvengine.aec_row import AecRow
from pvengine.dsp.hop_queue import GridHistory

HOP = 480
SR = 48000


class _FakeEngine:
    """桩会话：输出 = mic（验证通路），cache 原样返回。"""

    def new_state(self):
        return np.zeros((1, 8), dtype=np.float32)

    def process_frame(self, mic, far, cache):
        assert len(mic) == HOP and len(far) == HOP
        return np.asarray(mic, dtype=np.float32), np.asarray(
            cache, dtype=np.float32)


class _EchoFar:
    """桩会话：输出 = far（验证 far 直达 / 切片延迟 / 增益）。"""

    def new_state(self):
        return np.zeros((1, 8), dtype=np.float32)

    def process_frame(self, mic, far, cache):
        return np.asarray(far, dtype=np.float32), cache


def ts_of(h: int) -> float:
    return h * HOP / SR


def _push_hops(row, n, value, start_h=0):
    for h in range(start_h, start_h + n):
        row.push_far_ts(ts_of(h), [value] * HOP)


def test_grid_history():
    gh = GridHistory(HOP * 50)
    gh.push_ts(ts_of(0), [0.1] * HOP)
    gh.push_ts(ts_of(1), [0.2] * HOP)
    assert gh.start_grid() == 0 and gh.end_grid() == HOP * 2
    w = gh.window(0, HOP)
    assert w is not None and abs(w[240] - 0.1) < 1e-9
    p = gh.pop_hop(HOP)
    assert p is not None and len(p) == HOP
    assert gh.start_grid() == HOP
    # 空洞重锚
    gh.push_ts(ts_of(10), [0.5] * HOP)
    assert gh.start_grid() == HOP * 10
    # 不足 pop → None
    assert gh.pop_hop(HOP * 5) is None
    print("  GridHistory 时间戳入历史/取窗/头部 pop/空洞重锚  OK")


def test_aec_row_passthrough():
    """mic 直通：桩回显 mic，输出恒满帧。far 同步足够（far_delay=0）。"""
    aec_row_mod.get_shared_engine = lambda p: _FakeEngine()
    row = AecRow("dummy.onnx", far_sample_rate=SR)
    _push_hops(row, 20, 0.5)
    mic = [0.1] * HOP
    out = row.process_mic(mic, ts_of(10))
    assert out is not None and len(out) == HOP
    assert abs(float(out[240]) - 0.1) < 1e-6, out[240]
    assert len(row.last_far) == HOP
    print("  AecRow mic 直通、恒满帧  OK")


def test_aec_row_far_hist_slice():
    """far 按外部时钟切片：mic 时间戳对应 far_delay 前的 far（回声源）。

    far[0]=0.1, far[1]=0.2, ...；设 far_delay=20ms(960 样本)。mic 时间戳落在
    far[2] 处 → 切片取 far[2]−960 = far[0]，桩回显 far 应看到 0.1。
    """
    aec_row_mod.get_shared_engine = lambda p: _EchoFar()
    row = AecRow("dummy.onnx", far_sample_rate=SR, far_gain_db=0.0)
    row.set_delay_ms(20.0)
    for h in range(20):
        row.far_hist.push_ts(ts_of(h), [h * 0.1] * HOP)
    # mic 时刻 t=2 hop（960 样本），far_ref = t − 960 → far[0] = 0.0
    out = row.process_mic([0.0] * HOP, ts_of(2))
    assert out is not None and abs(float(out[240]) - 0.0) < 1e-6, out[240]
    print("  AecRow far 按时间戳  - far_delay 切片  OK")


def test_aec_row_far_delay():
    """far_delay 方向：把 far 参考整体后挪 d 样本（取更早历史）。"""
    aec_row_mod.get_shared_engine = lambda p: _EchoFar()
    row = AecRow("dummy.onnx", far_sample_rate=SR, far_gain_db=0.0)
    row.set_delay_ms(10.0)  # 480 样本
    for h in range(30):
        row.far_hist.push_ts(ts_of(h), [0.1 * h] * HOP)
    # mic 在 t=10 hop：far_delay=1 hop → 取 far[9]，值 0.9
    out = row.process_mic([0.0] * HOP, ts_of(10))
    assert out is not None and abs(float(out[240]) - 0.9) < 1e-3, out[240]
    print("  AecRow far_delay 生效：参考取更早一 hop 的 far  OK")


def test_aec_row_far_missing_passthrough():
    """far 完全无历史 → 直通 mic；far 有历史但理想窗口未就绪 → 喂最近一段。"""
    aec_row_mod.get_shared_engine = lambda p: _EchoFar()
    row = AecRow("dummy.onnx", far_sample_rate=SR, far_gain_db=0.0)
    # far 无任何历史 → 直通 mic（不丢人声、不进模型）
    out = row.process_mic([0.11] * HOP, ts_of(10))
    assert out is not None and len(out) == HOP
    assert abs(float(out[240]) - 0.11) < 1e-6, out[240]
    # far 只有 2 hop，且 mic 更远（far_delay=0 需要未来 far）→ 喂最近一段 far[1]
    _push_hops(row, 2, 0.3)
    out = row.process_mic([0.12] * HOP, ts_of(20))
    assert out is not None and abs(float(out[240]) - 0.3) < 1e-3, out[240]
    print("  AecRow far 无历史直通 mic；有历史则最近一段兜底  OK")


def test_aec_row_far_fallback_recent():
    """理想窗口未就绪但有 far 历史 → 喂最近一段（AEC 先跑，不全程直通）。"""
    aec_row_mod.get_shared_engine = lambda p: _EchoFar()
    row = AecRow("dummy.onnx", far_sample_rate=SR, far_gain_db=0.0)
    _push_hops(row, 3, 0.7)          # far[0..2]
    # mic 在 far 之后很远（far_delay=0 需要未来 far）→ 走最近一段兜底（far[2]=0.7）
    out = row.process_mic([0.0] * HOP, ts_of(20))
    assert out is not None and abs(float(out[240]) - 0.7) < 1e-3, out[240]
    assert row.diag()["fallback"] >= 1
    print("  AecRow far 理想窗口未就绪 → 喂最近一段兜底  OK")


def test_aec_row_far_gain():
    """参考音量：只缩放进模型的 far 帧（桩回显 far 即看到缩放）。"""
    aec_row_mod.get_shared_engine = lambda p: _EchoFar()
    row = AecRow("dummy.onnx", far_sample_rate=SR, far_gain_db=-6.0)
    for h in range(30):
        row.far_hist.push_ts(ts_of(h), [0.4] * HOP)
    out = row.process_mic([0.0] * HOP, ts_of(10))
    assert out is not None
    assert abs(float(out[240]) - 0.4 * 0.5011872) < 1e-3, out[240]
    row.set_far_gain_db(0.0)
    out = row.process_mic([0.0] * HOP, ts_of(12))
    assert out is not None and abs(float(out[240]) - 0.4) < 1e-3, out[240]
    print("  AecRow 参考音量只缩放 far  OK")


if __name__ == "__main__":
    print("行级 AEC 合成测试:")
    test_grid_history()
    test_aec_row_passthrough()
    test_aec_row_far_hist_slice()
    test_aec_row_far_delay()
    test_aec_row_far_missing_passthrough()
    test_aec_row_far_fallback_recent()
    test_aec_row_far_gain()
    print("全部通过")
