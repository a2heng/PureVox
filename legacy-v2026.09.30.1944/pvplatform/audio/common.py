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

"""
平台音频后端共享公共件。

放置各平台后端与上层模块共同依赖的常量、RingBuffer 与日志钩子，
避免 platform.audio 与 audio_processor 之间产生循环导入。
"""

import threading
from typing import List, Optional, Tuple

SAMPLE_RATE = 48000
HOP_LENGTH = SAMPLE_RATE // 100   # 10ms @48kHz = 480（202609 模型契约，与 pvengine.context 一致）

# 模块级日志钩子（由 UI 经 audio_processor.set_module_log 注入，同时转发到此）
_module_log = print


def set_module_log(func):
    """设置全局日志钩子，同步到 common 与 audio_processor。"""
    global _module_log
    _module_log = func
    try:
        import audio_processor
        audio_processor._module_log = func
    except Exception:
        pass


class LinearClock:
    """把某采集栈的内部时钟（如 PortAudio paTime）映射到主时钟（perf/QPC 秒）。

    Windows 上 PortAudio 的 input_buffer_adc_time 与 QPC 速率≈1、只差常量
    （实测 1s 内误差 0.000s），用增量最小二乘拟合 y=slope·x+offset 后即可把
    mic 采集时刻统一到主时钟，长期速率漂移由 slope 吸收。
    """

    def __init__(self):
        self._n = 0
        self._sx = self._sy = self._sxx = self._sxy = 0.0
        self._slope = 1.0
        self._offset = 0.0

    def add(self, x: float, y: float) -> None:
        self._n += 1
        self._sx += x
        self._sy += y
        self._sxx += x * x
        self._sxy += x * y
        if self._n >= 2:
            denom = self._n * self._sxx - self._sx * self._sx
            if abs(denom) > 1e-12:
                s = (self._n * self._sxy - self._sx * self._sy) / denom
                if 0.5 < s < 2.0:
                    self._slope = s
            self._offset = (self._sy - self._slope * self._sx) / self._n

    def map(self, x: float) -> float:
        return self._slope * x + self._offset


class TimedFifo:
    """带采集时间戳的 FIFO：存 (首样本主时钟秒, samples) 块。

    push 任意长块（块内采样率 = dev_sr，第 k 样本时刻 = ts0 + k/dev_sr）；
    read_ts(n) 跨块取 n 个样本，返回 (首样本时刻, samples)；不足返回 None。
    满额自动丢最旧（drop 计数供诊断）。
    """

    def __init__(self, dev_sr: int, capacity_samples: int):
        from collections import deque
        self._dev_sr = max(1, int(dev_sr or 48000))
        self._cap = max(480, int(capacity_samples))
        self._blocks: deque = deque()   # (ts0, list[float])
        self._lock = threading.Lock()
        self._drops = 0

    def _drop_old(self) -> None:
        have = sum(len(b) for _, b in self._blocks)
        while have > self._cap and self._blocks:
            ts0, b = self._blocks[0]
            need = have - self._cap
            if len(b) <= need:
                self._blocks.popleft()
                have -= len(b)
                self._drops += len(b)
            else:
                self._blocks[0] = (ts0 + need / self._dev_sr, b[need:])
                have -= need
                self._drops += need

    def write_ts(self, ts0: float, data) -> None:
        """写入以 ts0（主时钟秒）为首样本的一整块。data 可列表/ndarray。"""
        if not data:
            return
        samples = [float(s) for s in data]
        with self._lock:
            self._blocks.append((float(ts0), samples))
            self._drop_old()

    def available(self) -> int:
        with self._lock:
            return sum(len(b) for _, b in self._blocks)

    def read_ts(self, n: int) -> Optional[Tuple[float, List[float]]]:
        """取 n 个 FIFO 样本 → (首样本主时钟秒, samples)；不足返回 None。"""
        with self._lock:
            if sum(len(b) for _, b in self._blocks) < n:
                return None
            out = []
            first_ts = None
            while n > 0 and self._blocks:
                ts0, b = self._blocks[0]
                take = min(n, len(b))
                if first_ts is None:
                    first_ts = ts0
                out.extend(b[:take])
                n -= take
                if take == len(b):
                    self._blocks.popleft()
                else:
                    self._blocks[0] = (ts0 + take / self._dev_sr, b[take:])
            return first_ts, out

    def flush(self) -> None:
        with self._lock:
            self._blocks.clear()

    def diag(self) -> dict:
        with self._lock:
            return {"q": sum(len(b) for _, b in self._blocks),
                    "drops": self._drops}


class RingBuffer:
    """线程安全环形缓冲区，满时自动丢弃旧数据。"""

    def __init__(self, capacity_samples: int) -> None:
        self._capacity: int = capacity_samples
        self._buffer: List[float] = [0.0] * capacity_samples
        self._write_pos: int = 0
        self._read_pos: int = 0
        self._count: int = 0
        self._lock: threading.Lock = threading.Lock()

    def write(self, data: List[float]) -> None:
        """线程安全地写入数据。"""
        with self._lock:
            data_len = len(data)
            if data_len >= self._capacity:
                start = data_len - self._capacity
                self._buffer[:] = data[start:]
                self._write_pos = 0
                self._read_pos = 0
                self._count = self._capacity
                return

            discard = max(0, self._count + data_len - self._capacity)
            if discard > 0:
                self._read_pos = (self._read_pos + discard) % self._capacity
                self._count -= discard

            first_part = min(data_len, self._capacity - self._write_pos)
            self._buffer[self._write_pos:self._write_pos + first_part] = data[:first_part]

            if first_part < data_len:
                self._buffer[:data_len - first_part] = data[first_part:]

            self._write_pos = (self._write_pos + data_len) % self._capacity
            self._count = min(self._count + data_len, self._capacity)

    def read(self, n_samples: int) -> Optional[List[float]]:
        """线程安全地读取 n_samples 个采样。"""
        with self._lock:
            if self._count < n_samples:
                return None

            first_part = min(n_samples, self._capacity - self._read_pos)
            result = self._buffer[self._read_pos:self._read_pos + first_part]

            if first_part < n_samples:
                result = result + self._buffer[:n_samples - first_part]

            self._read_pos = (self._read_pos + n_samples) % self._capacity
            self._count -= n_samples
            return result

    def available(self) -> int:
        """线程安全地获取可用采样数。"""
        with self._lock:
            return self._count

    def read_latest(self, n_samples: int) -> Optional[List[float]]:
        """读取最新 n_samples 个采样，丢弃更旧数据；无数据时返回 None。"""
        with self._lock:
            if self._count == 0:
                return None
            skip = max(0, self._count - n_samples)
            if skip > 0:
                self._read_pos = (self._read_pos + skip) % self._capacity
                self._count -= skip
            to_read = min(n_samples, self._count)
            if to_read == 0:
                return None
            first_part = min(to_read, self._capacity - self._read_pos)
            result = self._buffer[self._read_pos:self._read_pos + first_part]
            if first_part < to_read:
                result = result + self._buffer[:to_read - first_part]
            self._read_pos = (self._read_pos + to_read) % self._capacity
            self._count -= to_read
            return result
