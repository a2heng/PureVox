# PureVox Lite Denoise Only — 跨时钟域播放缓冲
# Copyright (C) 2024-2026 a2heng <752848283@qq.com>
# SPDX-License-Identifier: GPL-3.0-or-later
#
# Lite 本地实现（零复用主线；语义逐条对齐 pvengine.dsp.playback.PlaybackSink，
# 两份手动同步，改一处须改另一处）：
# - 设备时钟是唯一主时钟：生产者按自己节奏 write(hop)，消费侧按设备真实节奏
#   pull(n)；两侧速率差与调度抖动在此消化；
# - 变速（ASRC）而非丢弃：按缓冲水位 PI 伺服线性插值变率重采样（±3%），
#   不垫零不丢样本，音调恒定；
# - 欠载 = 静音 + 退回预热态重同步，绝不复用上一帧 / 周期性垫零（咔哒）；
# - 过载封顶丢最旧，防延迟爬升。
#
# 线程约束：write() 在生产者线程（引擎/解码），pull() 在设备回调线程。

import threading
from collections import deque


class PlaybackBuffer:
    """生产者 write / 设备 pull 的自适应播放缓冲（长度单位一律为样本）。"""

    def __init__(self, hop=480, prime=None, target=None, cap=None):
        self._hop = max(1, int(hop))
        self._prime = prime if prime is not None else self._hop * 4
        self._target = target if target is not None else self._hop * 6
        self._cap = cap if cap is not None else self._hop * 30
        self._fade = 32        # 欠载淡出 / 续播淡入长度（0.67ms，消边界阶跃）
        self._fade_in = 0
        self._lock = threading.Lock()
        self._buf = deque()
        self._pos = 0.0        # 下一输出样本在 buf[0..1] 间的小数位
        self._primed = False
        self._integ = 0.0      # 积分项（真实速率差，稳态恒定）
        # 诊断计数
        self.n_pads = 0
        self.n_drops = 0
        self.n_underruns = 0

    # ── 生产侧 ──

    def write(self, frame):
        """推入一帧产出（任意长度，常态 = hop；接受 list / numpy 数组）。"""
        if frame is None or len(frame) == 0:
            return
        with self._lock:
            self._buf.extend(frame)
            level = len(self._buf) - self._pos
            if level > self._cap:
                drop = int(level - self._cap)
                for _ in range(drop):
                    self._buf.popleft()
                self._pos = max(0.0, self._pos - drop)
                self.n_drops += drop
            if not self._primed and self._level() >= self._prime:
                self._primed = True
                self._fade_in = self._fade

    def reset(self):
        """清空并回到预热态（停止 / 重开设备时调）。"""
        with self._lock:
            self._buf.clear()
            self._pos = 0.0
            self._primed = False
            self._integ = 0.0

    # ── 消费侧（设备回调线程）──

    def pull(self, n):
        """按设备节奏取 n 个样本；恒返回恰好 n 个（不足垫零并重同步）。"""
        out = [0.0] * max(0, int(n))
        n = len(out)
        if n == 0:
            return out
        with self._lock:
            if not self._primed:
                if self._level() < self._prime:
                    return out          # 预热期静音
                self._primed = True
                self._fade_in = self._fade

            level = self._level()
            err = (level - self._target) / float(self._target)
            integ = self._integ + max(-0.3, min(0.3, err)) * 0.002
            self._integ = max(-0.03, min(0.03, integ))
            r = 1.0 + self._integ + max(-0.1, min(0.1, err)) * 0.01
            if level < self._hop:
                r += 0.01               # 饥饿边缘安全阀（温和加速）
            r = max(0.97, min(1.03, r))

            i = 0
            buf = self._buf
            while i < n:
                if len(buf) >= 2:
                    b0 = buf[0]
                    t = self._pos
                    v = b0 + (buf[1] - b0) * t
                    rem = len(buf) - 1 - int(self._pos)
                    if self._fade_in > 0:
                        v *= (self._fade - self._fade_in) / float(self._fade)
                        self._fade_in -= 1
                    elif rem < self._fade:
                        v *= rem / float(self._fade)
                    out[i] = v
                    self._pos += r
                    while self._pos >= 1.0 and len(buf) > 1:
                        buf.popleft()
                        self._pos -= 1.0
                    i += 1
                else:
                    # 欠载：垫零 + 退回预热态（重同步，续播前先攒水位）
                    if len(buf) <= 1:
                        buf.clear()
                        self._pos = 0.0
                    self.n_underruns += 1
                    self.n_pads += n - i
                    self._primed = False
                    self._fade_in = 0
                    break

            level = self._level()
            if level > self._cap:
                drop = int(level - self._cap)
                for _ in range(drop):
                    buf.popleft()
                self._pos = max(0.0, self._pos - drop)
                self.n_drops += drop
        return out

    # ── 观测 ──

    def _level(self):
        """当前水位（未消费样本数）。调用方须持锁。"""
        return max(0, len(self._buf) - int(self._pos))

    def level(self):
        with self._lock:
            return self._level()

    def rate(self):
        """当前消费步长（诊断用；稳态 ≈1.0）。"""
        with self._lock:
            return 1.0 + self._integ

    def diag(self):
        with self._lock:
            return {"level": self._level(), "pads": self.n_pads,
                    "drops": self.n_drops, "underruns": self.n_underruns,
                    "rate": 1.0 + self._integ}
