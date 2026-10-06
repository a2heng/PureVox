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

"""PlaybackSink——跨时钟域播放缓冲（播放正确性的唯一实现点）。

原理（全部播放路径共用的时钟域翻译器）：
- **设备时钟是唯一主时钟**。生产者（处理线程/解码线程/网络）按自己的
  节奏 write(hop 帧)，消费者（设备回调/播放库）按设备真实节奏 pull(n)；
  两侧速率差（实测 ±2%）与调度抖动由本组件消化。
- **变速（ASRC）而非丢弃**：按缓冲水位伺服微调消费步长（线性插值变率
  重采样 ±3%）。PI 环——积分项慢速收敛到真实速率差（步长扰动 ≤±0.1%，
  音调恒定），比例项小系数阻尼；饥饿边缘温和加速。稳态速率差被平滑
  消化，不垫零不丢样本，延迟恒定。
- **欠载 = 静音 + 重同步**：缓冲耗尽输出静音并退回预热态，攒够水位再
  续播——绝不适配"复用上一帧"（冻结伪影）或周期性垫零（咔哒）。
- **过载封顶**：极端漂移丢最旧，防延迟爬升（仅兜瞬态，稳态靠伺服）。

write() 在引擎线程调，pull() 在设备线程调（线程安全）。
Windows 额外输出 / Linux 播放流 / 网络输出 / 媒体会话从设备全部经由
本组件——时钟域处理只此一份，禁止在任何回调里重写平行实现。
"""

import threading
from collections import deque
from typing import List


class PlaybackSink:
    """生产者 write / 设备 pull 的自适应播放缓冲。

    参数均为样本数（48kHz 下 1 hop = 480 = 10ms）：
    - prime：预热水位（攒够才开始出声，欠载后重同步也回到此态）
    - target：伺服目标水位（稳态延迟 ≈ target/48000 秒）
    - cap：封顶水位（超出丢最旧，防延迟爬升）
    """

    def __init__(self, hop: int = 480, prime: int = None, target: int = None,
                 cap: int = None):
        self._hop = max(1, int(hop))
        self._prime = prime if prime is not None else self._hop * 4
        self._target = target if target is not None else self._hop * 6
        self._cap = cap if cap is not None else self._hop * 30
        self._fade = 32        # 欠载淡出/续播淡入长度（0.67ms，消除边界阶跃）
        self._fade_in = 0      # >0 时正在淡入（剩余淡入样本数）
        self._lock = threading.Lock()
        self._buf: deque = deque()
        self._pos = 0.0          # 下一个输出样本在 buf[0..1] 间的小数位
        self._primed = False
        self._integ = 0.0        # 积分项（真实速率差，稳态恒定）
        # 诊断计数
        self.n_pads = 0          # 欠载垫零样本数
        self.n_drops = 0         # 封顶丢最旧样本数
        self.n_underruns = 0     # 欠载事件数

    # ── 生产侧（引擎线程）──

    def write(self, frame) -> None:
        """推入一帧产出（任意长度，常态 = hop）。"""
        if not frame:
            return
        with self._lock:
            self._buf.extend(frame)
            level = len(self._buf) - self._pos
            if level > self._cap:
                drop = int(level - self._cap)
                for _ in range(drop):
                    self._buf.popleft()
                # pos 随左移回退（跨过当前插值段则归零，跳变 < 1 样本）
                self._pos = max(0.0, self._pos - drop)
                self.n_drops += drop
            if not self._primed and self._level() >= self._prime:
                self._primed = True
                self._fade_in = self._fade   # 预热满 → 下次 pull 淡入续播

    def reset(self) -> None:
        """清空并回到预热态（停止/flush 时调）。"""
        with self._lock:
            self._buf.clear()
            self._pos = 0.0
            self._primed = False
            self._integ = 0.0

    # ── 消费侧（设备线程）──

    def pull(self, n: int) -> List[float]:
        """按设备节奏取 n 个样本；恒返回恰好 n 个（不足垫零并重同步）。

        消费步长按水位伺服微调（±3%）：水位低放慢/高加快，稳态贴 1.0，
        速率差被平滑消化，音调恒定。
        """
        out = [0.0] * max(0, int(n))
        n = len(out)
        if n == 0:
            return out
        with self._lock:
            if not self._primed:
                if self._level() < self._prime:
                    return out          # 预热期静音
                self._primed = True
                self._fade_in = self._fade   # 预热满 → 淡入续播

            level = self._level()
            err = (level - self._target) / float(self._target)
            # 积分项收敛 ~0.3s（须远快于 target 水位按最大速率差的耗尽
            # 时间 ~3s，否则水位先见底）；比例项小系数阻尼
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
                    # 临界水位淡出（欠载前平滑归零）与续播淡入：
                    # 只在边界区（±32 样本）生效，稳态零开销
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

            # 消费后再查封顶（写侧与读侧任一侧增长都受控）
            level = self._level()
            if level > self._cap:
                drop = int(level - self._cap)
                for _ in range(drop):
                    buf.popleft()
                self._pos = max(0.0, self._pos - drop)
                self.n_drops += drop
        return out

    # ── 观测 ──

    def _level(self) -> int:
        """当前水位（未消费样本数）。调用方须持锁。"""
        return max(0, len(self._buf) - int(self._pos))

    def level(self) -> int:
        with self._lock:
            return self._level()

    def rate(self) -> float:
        """当前消费步长（诊断用；稳态 ≈1.0）。"""
        with self._lock:
            return 1.0 + self._integ

    def diag(self) -> dict:
        with self._lock:
            return {"level": self._level(), "pads": self.n_pads,
                    "drops": self.n_drops, "underruns": self.n_underruns,
                    "rate": 1.0 + self._integ}
