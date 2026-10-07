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

"""AGC 自动增益控制组件（峰值包络 + 非对称平滑）。

目标 -12 dBFS peak、衰减下限 -30 dB、最大提升由「最大增益」滑杆设定
（0~30 dB，默认 30）。基于峰值包络（attack ~3ms / release ~80ms）计算
目标增益，再以非对称弹道平滑增益（降快 attack ~4ms 防削波、升慢
release ~300ms 防抽吸）。静音门（~-55 dBFS）之下不补增益、增益缓慢回落
到 1.0，避免词间底噪被抬高。输出仍走 soft clip 防削顶。
"""

import math
import numpy as np

from pvengine.components.effect_base import Effect

_SAMPLE_RATE = 48000
_TARGET_DB = -12.0
_GAIN_MIN_DB = -30.0
_GAIN_MAX_DB = 30.0
_FLOOR_DB = -55.0
_MAX_GAIN_DEFAULT = 30.0

# 峰值包络弹道：快跟峰、慢释放（人声 10ms 帧峰值起伏大，直接判定会抽吸）
_ENV_ATTACK_TAU = 0.003
_ENV_RELEASE_TAU = 0.080
# 增益弹道：降增益快（防削波）、升增益慢（防底噪/喘息）
_GAIN_ATTACK_TAU = 0.004
_GAIN_RELEASE_TAU = 0.300

_KNEE = 0.8
_SOFT_SCALE = 1.0 / math.tanh(_KNEE)


def _db_to_lin(db: float) -> float:
    return 10.0 ** (db / 20.0)


def _coeff(dt: float, tau: float) -> float:
    """一阶平滑系数：dt 秒内逼近目标的比例（1 - e^{-dt/tau}）。"""
    return 1.0 - math.exp(-dt / max(tau, 1e-4))


def _soft_clip(frame: np.ndarray) -> np.ndarray:
    """平滑饱和：|x|≤knee 线性，之外 tanh 渐压至 ±1 以内。"""
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
    """AGC 自动增益插件——峰值包络 + 非对称平滑，目标 -12 dBFS，无用户参数。"""

    NAME = "agc"
    LABEL = "自动增益 AGC"
    PARAMS = {
        # 手动上限：自动提升最多到该值（0 = 只衰减不提升），衰减下限固定 -30dB
        "max_gain_db": ("最大增益 dB", 0.0, _GAIN_MAX_DB,
                        _MAX_GAIN_DEFAULT, 1.0),
    }

    def __init__(self, params=None, engine_cache=None):
        self._env = 0.0
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
        x = np.asarray(frame, dtype=np.float32)
        peak = float(np.max(np.abs(x)))
        self._last_peak = peak

        dt = min(0.1, max(0.001, n / float(_SAMPLE_RATE)))

        # 峰值包络跟随（快 attack / 慢 release）
        ca = _coeff(dt, _ENV_ATTACK_TAU)
        cr = _coeff(dt, _ENV_RELEASE_TAU)
        if peak >= self._env:
            self._env += ca * (peak - self._env)
        else:
            self._env += cr * (peak - self._env)
        env = self._env

        target_lin = _db_to_lin(_TARGET_DB)
        floor_lin = _db_to_lin(_FLOOR_DB)
        gmin = _db_to_lin(_GAIN_MIN_DB)
        max_db = float(self.params.get("max_gain_db", _MAX_GAIN_DEFAULT))
        gmax = _db_to_lin(min(max(max_db, 0.0), _GAIN_MAX_DB))

        if env > floor_lin:
            desired = target_lin / env
            desired = min(max(desired, gmin), gmax)
        else:
            # 静音门之下：不补增益，缓慢回落到 unity（避免抬高底噪）
            desired = 1.0

        if not self._initialized:
            if peak > floor_lin:
                # 首个有声帧直接就位，避免开头上冲
                self._gain = desired
                self._initialized = True
        elif desired < self._gain:
            self._gain += _coeff(dt, _GAIN_ATTACK_TAU) * (desired - self._gain)
        else:
            self._gain += _coeff(dt, _GAIN_RELEASE_TAU) * (desired - self._gain)

        g = self._gain
        if g != 1.0:
            x = x * np.float32(g)
        return _soft_clip(x)

    def get_agc_gain_db(self) -> float:
        """当前增益 dB（正值=增强），供 UI 读取。"""
        return 20.0 * math.log10(max(self._gain, 1e-10))

    def get_debug_info(self) -> dict:
        return {"frames": self._frame_count, "gain": self._gain,
                "peak": self._last_peak, "env": self._env}

    def reset(self):
        self._env = 0.0
        self._gain = 1.0
        self._initialized = False
        self._frame_count = 0
        self._last_peak = 0.0
