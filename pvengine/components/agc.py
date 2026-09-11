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

"""AGC 自动增益控制组件（峰值判定）。

目标 -10 dBFS peak、瞬时更新、增益限幅 ±30 dB。
无用户可调参数，开箱即用。
"""

import math
import numpy as np

from pvengine.components.effect_base import Effect

_TARGET_LINEAR = 10.0 ** (-10.0 / 20.0)
_GAIN_MIN = 10.0 ** (-30.0 / 20.0)
_GAIN_MAX = 10.0 ** (30.0 / 20.0)
_RMS_FLOOR = 10.0 ** (-60.0 / 20.0)
_DT = 0.01
_DECAY_FACTOR = 0.5 ** _DT
_DEAD_ZONE = 10.0 ** (0.5 / 20.0)

_KNEE = 0.8
_SOFT_SCALE = 1.0 / math.tanh(_KNEE)


def _soft_clip(frame: np.ndarray) -> np.ndarray:
    """平滑饱和：0~knee 线性，knee~1 渐压，>1 饱和。"""
    out = np.empty_like(frame)
    lo = frame > -_KNEE
    hi = frame < _KNEE
    mid = lo & hi
    out[mid] = frame[mid]
    over = ~hi
    out[over] = np.tanh(frame[over] * _SOFT_SCALE)
    under = ~lo
    out[under] = np.tanh(frame[under] * _SOFT_SCALE)
    return out


class AgcPlugin(Effect):
    """AGC 自动增益插件——峰值判定，目标 -10 dBFS，无用户参数。"""

    NAME = "agc"
    LABEL = "自动增益 AGC"
    PARAMS = {}

    def __init__(self, params=None, engine_cache=None):
        self._gain = 1.0
        self._initialized = False
        self._frame_count = 0
        self._last_peak = 0.0
        super().__init__(params)

    def on_params_changed(self):
        pass

    def process(self, frame, ctx):
        self._frame_count += 1
        n = len(frame)
        if n == 0:
            return frame
        peak = float(np.max(np.abs(frame)))
        self._last_peak = peak
        if peak > _RMS_FLOOR:
            target = _TARGET_LINEAR / peak
            target = min(max(target, _GAIN_MIN), _GAIN_MAX)
            if not self._initialized:
                self._initialized = True
                self._gain = target
            else:
                ratio = target / self._gain
                if not (_DEAD_ZONE <= ratio <= 1.0 / _DEAD_ZONE):
                    self._gain = target
        elif self._gain > 1.0:
            self._gain *= _DECAY_FACTOR
            if self._gain < 1.0:
                self._gain = 1.0
        g = self._gain
        if g != 1.0:
            frame = frame * np.float32(g)
        frame = _soft_clip(frame)
        return frame

    def get_agc_gain_db(self) -> float:
        """当前增益 dB（正值=增强），供 UI 读取。"""
        return 20.0 * math.log10(max(self._gain, 1e-10))

    def get_debug_info(self) -> dict:
        return {"frames": self._frame_count, "gain": self._gain, "peak": self._last_peak}

    def reset(self):
        self._gain = 1.0
        self._initialized = False
