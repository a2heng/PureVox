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

"""启动/停止提示音合成（纯 numpy，离线合成，不占用音频引擎）。

设计目标：短促（≤200ms）、悦耳、成对——start 取上行音程、stop 取下行，
同一套预设听感对称。全部为确定性合成（固定 RNG 种子），给定预设/方向
即得到同一段波形。播放侧在 uitk/cues.py（系统默认输出设备），本模块只
产出波形与 WAV 字节，不做任何设备 I/O。

新增一套提示音 = 在 _VOICES 里加一个 `_voice_*` 函数并登记进 PRESETS。
"""

import io
import wave

import numpy as np

SAMPLE_RATE = 48000
PEAK = 0.6

# 预设：id → 中文名（id 持久化在配置里，勿随意改名）
PRESETS = (
    ("soft", "柔和"),
    ("crisp", "清脆"),
    ("pop", "气泡"),
    ("blip", "电子"),
    ("wood", "木鱼"),
    ("chime", "风铃"),
)
DEFAULT_PRESET = "soft"


def _n(ms: float) -> int:
    return max(1, int(SAMPLE_RATE * ms / 1000.0))


def _t(n: int) -> np.ndarray:
    return np.arange(n, dtype=np.float32) / np.float32(SAMPLE_RATE)


def _env(n: int, attack_ms: float = 4.0, tau_ms: float = 45.0) -> np.ndarray:
    """快起慢落包络：线性起音 + 指数衰减（无咔哒）。"""
    env = np.exp(-_t(n) / np.float32(tau_ms / 1000.0))
    a = min(n, _n(attack_ms))
    env[:a] *= np.linspace(0.0, 1.0, a, dtype=np.float32)
    return env.astype(np.float32)


def _tone(freq: float, ms: float, tau_ms: float = 45.0,
          attack_ms: float = 4.0, partials=(1.0,)) -> np.ndarray:
    n = _n(ms)
    t = _t(n)
    out = np.zeros(n, dtype=np.float32)
    for i, amp in enumerate(partials):
        ratio = (1.0, 2.76, 5.40)[i] if i < 3 else float(i + 1)
        out += np.float32(amp) * np.sin(
            np.float32(2.0 * np.pi * freq * ratio) * t)
    return (out * _env(n, attack_ms, tau_ms)).astype(np.float32)


def _voice_soft(kind: str) -> np.ndarray:
    pair = (587.33, 880.0) if kind == "start" else (880.0, 587.33)
    return np.concatenate([_tone(pair[0], 70, 40),
                           _tone(pair[1], 80, 45)])


def _voice_crisp(kind: str) -> np.ndarray:
    f = 1318.5 if kind == "start" else 880.0
    return _tone(f, 95, 26, 2.0, partials=(1.0, 0.45, 0.18))


def _voice_pop(kind: str) -> np.ndarray:
    f0, f1 = (1200.0, 520.0) if kind == "start" else (700.0, 300.0)
    n = _n(70)
    t = _t(n)
    f = f0 * (f1 / f0) ** (t / t[-1])
    phase = np.cumsum(2.0 * np.pi * f / SAMPLE_RATE, dtype=np.float32)
    return (np.sin(phase) * _env(n, 2.0, 22.0)).astype(np.float32)


def _voice_blip(kind: str) -> np.ndarray:
    if kind == "start":
        return _square(1568.0, 45)
    return np.concatenate([_square(1568.0, 45), _square(1046.5, 55)])


def _square(freq: float, ms: float) -> np.ndarray:
    n = _n(ms)
    w = np.sign(np.sin(np.float32(2.0 * np.pi * freq) * _t(n)))
    return (w.astype(np.float32) * 0.5 * _env(n, 2.0, 24.0)).astype(np.float32)


def _voice_wood(kind: str) -> np.ndarray:
    f = 760.0 if kind == "start" else 520.0
    n = _n(65)
    rng = np.random.default_rng(0)
    click = rng.standard_normal(n).astype(np.float32) * _env(n, 0.5, 5.0)
    body = _tone(f, 65, 20, 1.0, partials=(1.0, 0.35, 0.12))
    return (0.35 * click + body).astype(np.float32)


def _voice_chime(kind: str) -> np.ndarray:
    f = 784.0 if kind == "start" else 587.33
    return _tone(f, 180, 80, 6.0, partials=(1.0, 0.30, 0.14))


_VOICES = {
    "soft": _voice_soft,
    "crisp": _voice_crisp,
    "pop": _voice_pop,
    "blip": _voice_blip,
    "wood": _voice_wood,
    "chime": _voice_chime,
}

_CACHE: dict = {}


def render(pid: str, kind: str) -> np.ndarray:
    """合成一段提示音（mono float32 @48kHz，峰值 PEAK）。"""
    if pid not in _VOICES:
        pid = DEFAULT_PRESET
    if kind not in ("start", "stop"):
        kind = "start"
    key = (pid, kind)
    got = _CACHE.get(key)
    if got is not None:
        return got
    sig = np.asarray(_VOICES[pid](kind), dtype=np.float32)
    peak = float(np.max(np.abs(sig))) or 1.0
    sig = (sig * np.float32(PEAK / peak)).astype(np.float32)
    _CACHE[key] = sig
    return sig


def wav_bytes(pid: str, kind: str) -> bytes:
    """提示音的 16bit PCM WAV 字节（mono 48kHz，供播放器直接吃内存）。"""
    sig = render(pid, kind)
    pcm = np.clip(sig * 32767.0, -32768.0, 32767.0).astype("<i2")
    buf = io.BytesIO()
    with wave.open(buf, "wb") as w:
        w.setnchannels(1)
        w.setsampwidth(2)
        w.setframerate(SAMPLE_RATE)
        w.writeframes(pcm.tobytes())
    return buf.getvalue()
