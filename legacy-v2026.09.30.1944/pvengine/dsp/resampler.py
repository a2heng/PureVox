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

"""流式重采样器——替代 libsamplerate（SRC_SINC_FASTEST）的纯 Python 实现。

三次 Hermite 插值 + 输入历史保持，支持任意 src_ratio 的流式处理；
质量足以满足 AEC far-end 参考与 TSE 参考音频的重采样场景。
src_ratio 语义与 libsamplerate 一致：目标采样率 / 源采样率。
"""

import numpy as np


class Resampler:
    def __init__(self):
        self.reset()

    def reset(self):
        self._hist = np.zeros(0, dtype=np.float32)   # 待插值输入缓冲（含跨块历史）
        self._pos = 0.0                              # 下一个输出样本在 _hist 中的浮点位置

    def process(self, input, src_ratio: float, end_of_input: bool = False):
        """推入输入并返回本次产生的全部输出样本（list[float]）。

        end_of_input=True 时冲刷滤波器尾部的剩余输出（此后再调用产生空结果）。
        """
        if src_ratio <= 0.0:
            raise ValueError("Resampler: src_ratio must be positive")
        x = np.asarray(input, dtype=np.float32).reshape(-1)
        self._hist = np.concatenate([self._hist, x]) if len(x) else self._hist
        out = []
        step = 1.0 / float(src_ratio)
        last = len(self._hist) - 1
        # 三次插值需要 i-1..i+2 共 4 个点
        while True:
            i = int(self._pos)
            if i + 2 > last:
                break
            t = np.float32(self._pos - i)
            p0 = self._hist[i - 1] if i >= 1 else np.float32(0.0)
            p1, p2, p3 = self._hist[i], self._hist[i + 1], self._hist[min(i + 2, last)]
            y = (p1 + np.float32(0.5) * t * (p2 - p0
                 + t * (np.float32(2.0) * p0 - np.float32(5.0) * p1 + np.float32(4.0) * p2 - p3
                        + t * (np.float32(3.0) * (p1 - p2) + p3 - p0))))
            out.append(float(y))
            self._pos += step
        if end_of_input:
            # 冲刷：补 3 个零点把剩余位置走完
            self._hist = np.concatenate([self._hist, np.zeros(3, dtype=np.float32)])
            while self._pos + 1 < len(self._hist) - 1:
                i = int(self._pos)
                t = np.float32(self._pos - i)
                p0 = self._hist[i - 1] if i >= 1 else np.float32(0.0)
                p1, p2, p3 = self._hist[i], self._hist[i + 1], self._hist[i + 2]
                a = np.float32(0.5) * t * (p2 - p0)
                b = t * (np.float32(2.0) * p0 - np.float32(5.0) * p1 + np.float32(4.0) * p2 - p3)
                c = t * t * (np.float32(3.0) * (p1 - p2) + p3 - p0)
                out.append(float(p1 + a + np.float32(0.5) * (b + c)))
                self._pos += step
            self.reset()
        else:
            # 保留最后 4 个样本作为下一次的插值历史
            keep = min(4, len(self._hist))
            drop = len(self._hist) - keep
            self._hist = self._hist[drop:]
            self._pos -= drop
        return out
