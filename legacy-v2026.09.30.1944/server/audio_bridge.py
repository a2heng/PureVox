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

import threading
import time
from typing import List, Optional


class RemoteAudioSource:
    """接收 WSS 客户端解码后的 PCM，送入音频处理管线。

    ring_cls：内部环形缓冲实现（须有 write/read/available；可选 read_latest）。
    缺省时在函数内延迟导入主线 `audio_processor.RingBuffer`（避免 server 顶层
    把整个音频栈拖进精简调用方）；精简调用方可注入自带的 ring 实现。
    """

    def __init__(self, sample_rate: int = 48000, buffer_seconds: float = 0.5,
                 ring_cls=None):
        if ring_cls is None:
            from audio_processor import RingBuffer as ring_cls
        self._ring_cls = ring_cls
        self._sample_rate = sample_rate
        self._buffer = ring_cls(int(sample_rate * buffer_seconds))
        self._active_clients: int = 0
        self._lock = threading.Lock()
        self._log = print
        self.flush_event = threading.Event()  # 网络循环检测此事件清空处理缓冲
        self._discard_until: float = 0.0  # flush 后的丢弃窗口截止时间

    def set_logger(self, log_func):
        self._log = log_func

    def write_pcm(self, pcm_data: List[float]):
        # flush 后在途残留数据直接丢弃，防止"已停止但还嗞嗞嗞"
        if time.time() < self._discard_until:
            return
        if pcm_data:
            self._buffer.write(pcm_data)

    def read(self, n_samples: int) -> Optional[List[float]]:
        return self._buffer.read(n_samples)

    def read_latest(self, n_samples: int) -> Optional[List[float]]:
        fn = getattr(self._buffer, "read_latest", None)
        return fn(n_samples) if fn else None

    def available(self) -> int:
        return self._buffer.available()

    def client_connected(self):
        with self._lock:
            self._active_clients += 1
            self._log(f"[RemoteMic] 客户端连接 ({self._active_clients} 个活跃)")

    def client_disconnected(self):
        with self._lock:
            self._active_clients = max(0, self._active_clients - 1)
            self._log(f"[RemoteMic] 客户端断开 ({self._active_clients} 个活跃)")
            should_flush = (self._active_clients == 0)
        if should_flush:
            self.flush()  # 所有客户端断开 → 清空缓冲，防止循环最后一帧有声数据

    def flush(self):
        """清空缓冲 — 处理重启时丢弃过期音频。"""
        self._buffer = self._ring_cls(int(self._sample_rate * 2.0))
        self._discard_until = time.time() + 0.3  # 丢弃 300ms 内在途残留数据
        self.flush_event.set()  # 通知 network_loop 清空处理缓冲

    @property
    def active_clients(self) -> int:
        with self._lock:
            return self._active_clients

    @property
    def sample_rate(self) -> int:
        return self._sample_rate
