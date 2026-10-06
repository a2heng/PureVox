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

"""线程安全环形缓冲（满丢最旧）——对齐原 C RingBuffer 语义。

向量化读写；锁粒度为整次操作，多生产/多消费均安全。
"""

import threading
import numpy as np


class RingBuffer:
    """FIFO；write 满时丢弃最旧数据，read 返回 min(n, available) 个样本。"""

    def __init__(self, capacity: int):
        self._cap = max(int(capacity), 1)
        self._buf = np.zeros(self._cap, dtype=np.float32)
        self._lock = threading.Lock()
        self._w = 0            # 写游标
        self._r = 0            # 读游标
        self._count = 0        # 可读样本数

    def write(self, data) -> None:
        x = np.asarray(data, dtype=np.float32).reshape(-1)
        if not len(x):
            return
        with self._lock:
            n = len(x)
            if n >= self._cap:                      # 整段超容量：只留最新 cap 个
                self._buf[:] = x[-self._cap:]
                self._r = 0
                self._w = 0
                self._count = self._cap
                return
            end = self._w + n
            if end <= self._cap:
                self._buf[self._w:end] = x
            else:
                first = self._cap - self._w
                self._buf[self._w:] = x[:first]
                self._buf[:end - self._cap] = x[first:]
            self._w = end % self._cap
            if self._count + n > self._cap:         # 溢出：读游标被挤到与写游标重合
                self._r = self._w
                self._count = self._cap
            else:
                self._count += n

    def read(self, n: int):
        with self._lock:
            got = min(int(n), self._count)
            start = self._r
            end = start + got
            if end <= self._cap:
                out = self._buf[start:end].copy()
            else:
                first = self._cap - start
                out = np.concatenate([self._buf[start:], self._buf[:got - first]])
            self._r = (self._r + got) % self._cap
            self._count -= got
            return out.tolist()

    def read_latest(self, n: int):
        """读取最新 n 个样本（丢弃更旧数据）；无数据时返回 None。

        与 read() 的“返回可用部分”不同：这里只关心最新一段，供频谱等
        只取当前窗口的消费者使用。
        """
        with self._lock:
            if self._count == 0:
                return None
            skip = max(0, self._count - int(n))
            if skip:
                self._r = (self._r + skip) % self._cap
                self._count -= skip
            got = self._count
            start = self._r
            end = start + got
            if end <= self._cap:
                out = self._buf[start:end].copy()
            else:
                first = self._cap - start
                out = np.concatenate([self._buf[start:], self._buf[:got - first]])
            self._r = (self._r + got) % self._cap
            self._count = 0
            return out.tolist()

    def available(self) -> int:
        with self._lock:
            return self._count

    def clear(self) -> None:
        with self._lock:
            self._w = self._r = self._count = 0
            self._buf[:] = 0.0
