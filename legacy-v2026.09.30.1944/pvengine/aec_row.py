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

"""行级回声消除——一个 `echo_cancel` 输入行对应一个 AecRow。

信号流（该行 mic 专属，不经过 fx 链）：
  mic_hop → AEC（far 直达）→ 混音

far/mic 按**外部时钟（QPC/perf 秒）**配对：mic 每 hop 带采集时间戳，far
采集样本带时间戳入 `GridHistory`（48k 采样网格）。配对取的是时间：
模型 far 输入 = far 历史里 [mic网格 − far_delay, +hop) 的样本——即回声源
那一份 far；谁先到、谁晚到只影响是否有样本，不影响配对对齐，无隐藏缓冲。

far 参考天然在时间上早于回声：far 只要采集到 mic 时刻往前 d 的历史即可，
不需追上 mic。far 历史不足（缺配/刚启动/生产空洞）时本行直通 mic（不丢人声、
不进模型、不动 cache）。far_delay 在网格域直接切片，无需额外延迟环。

选型说明（先扩展再新建）：旧实现用计数/水位配对（HopQueue 或 FarSync），
时间原点依赖生产者同时起步，遇调度抖动/不同起步会错位；外部时钟配对用
时间戳网格对齐，故替换。AecEngine 会话多行共享（按模型路径缓存），cache
行私有、逐 hop 续传。
"""

import numpy as np

from pvengine.components.aec import AecEngine
from pvengine.context import HOP_LENGTH, SAMPLE_RATE
from pvengine.dsp.hop_queue import GridHistory, grid_from_ts

_engines: dict = {}


def get_shared_engine(model_path: str) -> AecEngine:
    """按模型路径缓存的多行共享 AEC 会话（进程内单例）。"""
    eng = _engines.get(model_path)
    if eng is None or getattr(eng, "sess", None) is None:
        eng = AecEngine(model_path)
        _engines[model_path] = eng
    return eng


def find_model_file(name: str) -> str:
    """按模型相对路径定位（PyInstaller 资源目录 → 仓库根 → CWD）。"""
    import os
    import sys
    here = os.path.dirname(os.path.abspath(__file__))
    bases = []
    meipass = getattr(sys, "_MEIPASS", None)
    if meipass:
        bases.append(meipass)
    bases.append(os.path.dirname(here))
    bases.append(here)
    for base in bases:
        cand = os.path.join(base, name)
        if os.path.isfile(cand):
            return cand
    return name


class AecRow:
    """一行 AEC 的行级状态：far 网格历史 + AEC cache + far 延迟(d)。"""

    MAX_DELAY_SAMPLES = 48000  # 1000ms @ 48kHz
    # far 历史上限：需覆盖 far_delay(≤500ms) + 少许余量；超出只丢最旧 far
    _HIST_CAP = SAMPLE_RATE * 2   # 2s

    def __init__(self, model_path: str, far_sample_rate: int = SAMPLE_RATE,
                 far_gain_db: float = 0.0):
        self.engine = get_shared_engine(model_path)
        self.cache = self.engine.new_state()
        self.far_sample_rate = int(far_sample_rate or SAMPLE_RATE)
        self.far_hist = GridHistory(self._HIST_CAP)
        if self.far_sample_rate != SAMPLE_RATE:
            from pvengine.dsp.resampler import Resampler
            self._resampler: object = Resampler()
            self._ratio = SAMPLE_RATE / float(self.far_sample_rate)
            self._resampler.process(np.zeros(HOP_LENGTH, dtype=np.float32),
                                    self._ratio)
        else:
            self._resampler = None
            self._ratio = 1.0
        self._far_gain = 10.0 ** (float(far_gain_db) / 20.0)
        self.last_far = np.zeros(HOP_LENGTH, dtype=np.float32)
        self._delay_samples = 0
        self._n_fallback = 0

    def set_far_gain_db(self, db: float) -> None:
        self._far_gain = 10.0 ** (float(db) / 20.0)

    def set_delay_ms(self, ms: float) -> None:
        """far_delay（毫秒）——网格域切片偏移，立即生效。"""
        self._delay_samples = max(0, min(self.MAX_DELAY_SAMPLES,
                                         int(round(ms * SAMPLE_RATE / 1000.0))))

    def push_far_ts(self, ts0: float, samples) -> None:
        """far 设备域新到样本入历史（ts0=首样本主时钟秒）。重采样到 48k。"""
        if not samples:
            return
        if self._resampler is not None:
            got = self._resampler.process(list(samples), self._ratio)
            self.far_hist.push_ts(ts0, got)
        else:
            self.far_hist.push_ts(ts0, samples)

    def process_mic(self, mic_hop, ts0: float = 0.0) -> np.ndarray:
        """本路 mic 一 hop（ts0=采集主时钟秒）→ AEC → 输出一 hop。

        模型 far 输入 = far 历史 [mic网格 − far_delay, +hop)，即回声源；
        far 历史不足时直通 mic（不丢人声、不进模型、不动 cache）。
        """
        mic = np.asarray(mic_hop, dtype=np.float32).reshape(-1)
        g0 = grid_from_ts(ts0) if ts0 else 0
        start = g0 - self._delay_samples
        far_win = self.far_hist.window(start, HOP_LENGTH)
        if far_win is None:
            # 理想窗口未就绪（启动/far_delay 未设/空洞）。若已有任何 far 历史，
            # 先喂“最近一段”让 AEC 运行（模型多抽头自找对齐），否则直通 mic。
            end_g = self.far_hist.end_grid()
            if end_g is not None and end_g >= HOP_LENGTH:
                far_win = self.far_hist.window(end_g - HOP_LENGTH, HOP_LENGTH)
            if far_win is None:
                return mic
            self._n_fallback += 1
        far_ref = np.asarray(far_win, dtype=np.float32).reshape(-1)
        if self._far_gain != 1.0:
            far_ref = far_ref * np.float32(self._far_gain)
        self.last_far = far_ref
        out, self.cache = self.engine.process_frame(mic, far_ref, self.cache)
        return np.asarray(out, dtype=np.float32).reshape(-1)

    def diag(self) -> dict:
        d = self.far_hist.diag()
        d["fallback"] = self._n_fallback
        d["cache_norm"] = float((self.cache * self.cache).sum() ** 0.5)
        return d
