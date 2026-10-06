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

"""Windows 麦克风专用采集（AEC far 选麦克风时的数据源）。

与扬声器 loopback（SpeakerCaptureWin）对偶：PortAudio 输入流直采
指定麦克风。输入自适应（与主输入同一机制）：按设备原生采样率/声道
打开，回调内下混单声道后经 pvengine.Resampler 转 48k；对外恒 48k
单声道，dev_sr 恒报 48000（AecRow 无需二次重采样）。回调写环形缓冲
（200ms）。AEC 行自建自停，不进主混音，样本直达行内 AecRow（far
严格配对）。

接口契约（与 SpeakerCapture 一致）：
    start() -> bool / stop() / read(n) / available() / flush()
    dev_sr (int) / active (bool)
"""

import struct
import threading
import time
from typing import Optional, Tuple

from .common import LinearClock, TimedFifo, _module_log

_SAMPLE_RATE = 48000
_RING_CAP = _SAMPLE_RATE // 5    # 200ms（吸收调度抖动）


class MicCaptureWin:
    """指定麦克风的独立输入流（Windows 专用，far=mic 时一行一路）。

    采集时间戳：PortAudio input_buffer_adc_time 经 LinearClock 映射到主时钟
    （QPC/perf 秒），写入 TimedFifo（far 与 mic 同一外部时钟域）。
    """

    def __init__(self, device_id: Optional[int] = None):
        self._device_id = device_id
        self._p = None
        self._stream = None
        self._buffer = TimedFifo(_SAMPLE_RATE, _RING_CAP)
        self._clock = LinearClock()
        self._active = False
        self._lock = threading.Lock()
        self._dev_sr = _SAMPLE_RATE   # 对外恒 48k（内部重采样已消化速率差）
        self._native_sr = _SAMPLE_RATE
        self._native_ch = 1
        self._ratio = 1.0
        self._rs = None               # 需重采样时持有 Resampler，否则直通

    @property
    def active(self) -> bool:
        return self._active

    @property
    def dev_sr(self) -> int:
        return self._dev_sr

    def start(self) -> bool:
        import pyaudio
        from pvplatform.audio.pa_backend import native_hop_len
        with self._lock:
            if self._active:
                return True
            try:
                self._p = pyaudio.PyAudio()
                native_sr, native_ch = _SAMPLE_RATE, 1
                try:
                    info = self._p.get_device_info_by_index(
                        self._device_id)
                    native_sr = int(round(float(
                        info.get('defaultSampleRate') or _SAMPLE_RATE)))
                    native_ch = max(1, int(
                        info.get('maxInputChannels') or 1))
                except Exception:
                    pass
                self._native_sr, self._native_ch = native_sr, native_ch
                self._ratio = _SAMPLE_RATE / float(native_sr) \
                    if native_sr != _SAMPLE_RATE else 1.0
                self._rs = None
                if self._ratio != 1.0:
                    from pvengine.dsp.resampler import Resampler
                    self._rs = Resampler()
                    self._rs.process(
                        [0.0] * native_hop_len(native_sr), self._ratio)
                self._stream = self._p.open(
                    format=pyaudio.paFloat32, channels=native_ch,
                    rate=native_sr, input=True,
                    input_device_index=self._device_id,
                    frames_per_buffer=native_hop_len(native_sr),
                    stream_callback=self._callback)
                self._stream.start_stream()
            except Exception as e:
                _module_log(f"[AEC] 麦克风 far 采集打开失败: {e}")
                self.stop()
                return False
            self._active = True
            if self._rs is not None:
                _module_log(f"[AEC] 麦克风 far 采集: 设备 #{self._device_id} "
                            f"({native_ch}ch {native_sr}Hz → 48kHz 自适应重采样)")
            else:
                _module_log(f"[AEC] 麦克风 far 采集: 设备 #{self._device_id} "
                            f"({_SAMPLE_RATE}Hz, 单声道)")
            return True

    def stop(self) -> None:
        with self._lock:
            self._active = False
            if self._stream is not None:
                try:
                    self._stream.stop_stream()
                except Exception:
                    pass
                try:
                    self._stream.close()
                except Exception:
                    pass
                self._stream = None
            if self._p is not None:
                try:
                    self._p.terminate()
                except Exception:
                    pass
                self._p = None

    def _callback(self, in_data, frame_count, time_info, status):
        import pyaudio
        if not self._active:
            return (None, pyaudio.paComplete)
        try:
            from pvplatform.audio.pa_backend import downmix_mono
            ch = max(1, int(self._native_ch))
            raw = struct.unpack(f"{frame_count * ch}f", in_data)
            data = downmix_mono(raw, ch)
            if self._rs is not None:
                data = self._rs.process(data, self._ratio)
                if not data:
                    return (None, pyaudio.paContinue)
            adc = time_info.get('input_buffer_adc_time')
            now = time.perf_counter()
            if adc is not None:
                self._clock.add(adc, now)
                ts0 = self._clock.map(adc)
            else:
                ts0 = now
            self._buffer.write_ts(ts0, data)
        except Exception:
            pass
        return (None, pyaudio.paContinue)

    def available(self) -> int:
        return self._buffer.available()

    def read_ts(self, n_samples: int) -> Optional[Tuple[float, list]]:
        return self._buffer.read_ts(n_samples)

    def read(self, n_samples: int) -> Optional[list]:
        got = self._buffer.read_ts(n_samples)
        return got[1] if got is not None else None

    def flush(self) -> None:
        self._buffer.flush()
