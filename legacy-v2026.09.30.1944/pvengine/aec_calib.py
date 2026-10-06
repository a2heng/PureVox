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

"""AEC far 延迟校准的纯信号件（探测音生成 + far↔mic 相对延迟估计）。

测量对象（与运行时 far 采集同一点）：
  播放 chirp → 采集 far 参考（far=扬声器为端点 loopback / far=麦克风为
  far mic）与目标麦克风；两路录音起点对齐后，互相关 far↔mic 直接给出
  运行时需要补偿的 far_delay——不再依赖「写播放时刻 → mic」的绝对链路
  计时，前导静音/预卷/播放缓冲等系统偏差在互相关里自动抵消。

本模块只做纯信号处理（numpy/scipy，收敛于 pvengine 内）；录音采集/
设备解析在上层（audio_processor 校准），这里不触碰任何平台 API。
"""

from typing import List, Optional, Tuple

import numpy as np

SAMPLE_RATE = 48000


def make_probe(fs: int = SAMPLE_RATE, n_repeat: int = 1,
               chirp_ms: float = 120.0, gap_ms: float = 250.0,
               head_ms: float = 150.0, tail_ms: float = 250.0,
               f0: float = 400.0, f1: float = 5000.0,
               amp: float = 0.85) -> np.ndarray:
    """生成校准探测音频：头静音 + 单发上扫 chirp + 尾静音（float32 48k）。

    **必须单发**：若重复相同 chirp，far↔mic 互相关会在「延迟 ± 重复间隔」
    处出现等强假峰（声道起点抖动时锁错峰 → 数百 ms 漂浮）；单发无此别名。
    chirp 用 hanning 包络、时长 ~120ms、400~5k 上扫，兼顾扬声器频响、
    房间响应与检测信噪比（mic 拾取回声通常很弱）。
    """
    fs = int(fs)
    chirp_n = int(fs * chirp_ms / 1000.0)
    t = np.linspace(0.0, chirp_ms / 1000.0, chirp_n, dtype=np.float32)
    chirp = np.sin(2 * np.pi * (f0 * t + (f1 - f0) * t * t / (2 * (chirp_ms / 1000.0)))) \
        * np.hanning(chirp_n).astype(np.float32) * float(amp)
    head = np.zeros(int(fs * head_ms / 1000.0), dtype=np.float32)
    gap = np.zeros(int(fs * gap_ms / 1000.0), dtype=np.float32)
    tail = np.zeros(int(fs * tail_ms / 1000.0), dtype=np.float32)
    parts = [head]
    for i in range(max(1, int(n_repeat))):
        parts.append(chirp)
        parts.append(gap)
    parts.append(tail)
    return np.concatenate(parts).astype(np.float32)


def _bandpass_zero_phase(x: np.ndarray, fs: int, band) -> Optional[np.ndarray]:
    """零相位带通（Butterworth sos + sosfiltfilt）。

    far/mic 两路用同一滤波器、零相位 ⇒ 不引入任何群延迟/相对位移，只把带外
    环境噪声与麦克风热噪声剔除，测到的延迟不受滤波影响。
    """
    if not band or len(x) < fs * 0.05:
        return x
    from scipy.signal import butter, sosfiltfilt
    try:
        low, high = float(band[0]), float(band[1])
        nyq = float(fs) / 2.0
        low = max(1.0, min(low, nyq * 0.99))
        high = min(max(low + 1.0, high), nyq * 0.99)
        sos = butter(4, [low, high], btype="bandpass", output="sos", fs=fs)
        return sosfiltfilt(sos, x)
    except Exception:
        return x


def _prepare(x: List[float], fs: int = SAMPLE_RATE,
             bandpass_hz=None) -> Optional[np.ndarray]:
    """转 float64、可选带通去噪、去 DC、单位 RMS（方便相关峰解读与阈值）。"""
    if not x or len(x) < 64:
        return None
    a = np.asarray(x, dtype=np.float64)
    if bandpass_hz:
        a = _bandpass_zero_phase(a, fs, bandpass_hz)
    a = a - a.mean()
    rms = float(np.sqrt(np.mean(a * a)))
    if rms < 1e-9:
        return None
    return a / rms


def estimate_far_delay_ms(far: List[float], mic: List[float],
                          fs: int = SAMPLE_RATE,
                          min_delay_ms: float = 0.5,
                          max_delay_ms: float = 450.0,
                          max_peaks: int = 6,
                          bandpass_hz=(300.0, 5500.0)
                          ) -> Optional[Tuple[float, dict]]:
    """估计 far 参考相对麦克风的回声延迟（far 先到、mic 滞后为正）。

    far/mic 为同一起点采集的样本序列（数组可不等长）。返回
    (delay_ms, diag) 或 None（数据不足 / 未检测到可靠回声峰）。

    bandpass_hz：互相关前对两路做**同一零相位带通**（默认 300~5500Hz，
    与探测 chirp 频带一致），剔除环境噪声与麦克风热噪声等带外污染；零相位
    不引入延迟，不影响估计结果。传 None 关闭。

    diag 含 corr（相关峰高度，近似归一化系数）、snr（峰/噪声底比）、
    lag、n_peaks，供上层打日志判断测量可信度。
    """
    from scipy.signal import find_peaks, fftconvolve

    fs = int(fs)
    far_n = _prepare(far, fs, bandpass_hz)
    mic_n = _prepare(mic, fs, bandpass_hz)
    if far_n is None or mic_n is None:
        return None
    if len(mic_n) < fs * 0.05:      # 至少 50ms 录音
        return None

    # corr[t] = Σ_m mic_n[m] · far_n[m - L]，L = t - (len(far)-1)；
    # mic 滞后 far 一个正 L：mic[m]≈far[m-L] 时峰出现在 L = 真延迟。
    corr = fftconvolve(mic_n, far_n[::-1], mode="full")
    lags = np.arange(len(corr)) - (len(far_n) - 1)

    lo = max(1, int(min_delay_ms * fs / 1000.0))
    hi = int(max_delay_ms * fs / 1000.0)
    if hi <= lo or hi >= len(lags):
        hi = len(lags) - 1
    mask = (lags >= lo) & (lags <= hi)
    if not mask.any():
        return None
    valid_lags = lags[mask]
    valid_corr = corr[mask]

    cmax = float(np.max(np.abs(valid_corr))) if len(valid_corr) else 0.0
    if cmax < 1e-6:
        return None
    y = np.abs(valid_corr) / cmax

    peaks, _ = find_peaks(
        y, height=0.25,
        distance=max(1, int(20 * fs / 1000.0)),
        prominence=0.15)
    if len(peaks) == 0:
        # 无独立峰：当作未检测到（不靠 argmax 强行取值，避免静音误报）
        return None

    # 按高度排序后取前几强峰，从中挑「最早且高度≥0.6×最强」的直接路径峰；
    # 否则用最强峰（房间多次反射可能叠加更强）。
    order = np.argsort(y[peaks])[::-1]
    strong = order[:max_peaks]
    best = int(peaks[order[0]])
    for idx in strong:
        if y[int(peaks[idx])] >= 0.6 * y[peaks[order[0]]]:
            cand = int(peaks[idx])
            if valid_lags[cand] < valid_lags[best]:
                best = cand
    lag = int(valid_lags[best])

    # 抛物线插值到亚样本
    if 0 < best < len(valid_corr) - 1:
        y0, ym, yp = float(valid_corr[best - 1]), float(valid_corr[best]), \
            float(valid_corr[best + 1])
        denom = 2.0 * (2.0 * ym - y0 - yp)
        if abs(denom) > 1e-12:
            off = max(-1.0, min(1.0, (y0 - yp) / denom))
            lag = lag + off
    else:
        off = 0.0

    delay_ms = max(0.0, lag * 1000.0 / fs)
    peak_raw = float(np.abs(valid_corr[best]))
    # 近似归一化系数：远/近端各为单位 RMS 时，完全对齐的峰 ≈ 重叠样本数
    overlap = float(max(1, min(len(mic_n), len(far_n))))
    coef = min(1.0, peak_raw / overlap)
    noise = float(np.percentile(np.abs(valid_corr), 95)) or 1e-9
    diag = {
        "corr": float(coef),
        "snr": float(peak_raw / noise),
        "lag": float(lag),
        "n_peaks": int(len(peaks)),
        "refine": float(off),
    }
    # 可靠度门：峰太弱（无实质回声）视为失败，交给上层保留原值
    if coef < 0.02 or diag["snr"] < 2.0:
        return None
    return delay_ms, diag
