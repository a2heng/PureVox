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

"""时间戳网格历史（外部时钟配对的公共件）。

AEC far 与回环输入行共用：样本带采集时间戳（主时钟 QPC/perf 秒）push 进
48k 采样网格历史；消费按**时间**取段——far 参考取「mic 网格 − far_delay」
的历史，回环取头部整 hop——谁先到/晚到只看时间戳，不看到达顺序。因此
far/mic 时间原点严格一致，不存在任何“为吸收时钟差人为保持水位/内容滞后”
的隐藏缓冲。

空洞处理：生产者掉数据会在时间轴上留下空洞，本历史无法合成缺段，遇空洞
即重锚（丢弃旧历史、重新从当下起），期间上层短暂直通，避免错位累积。
"""

from typing import Optional


def grid_from_ts(ts: float, sr: int = 48000) -> int:
    """把外部时钟秒时间戳落到 48k 采样网格（far/mic 共用同一网格域）。"""
    return int(round(float(ts) * int(sr)))


class GridHistory:
    """48k 采样网格的连续历史（外部时钟配对用）。"""

    def __init__(self, cap_samples: int, sample_rate: int = 48000):
        self._sr = int(sample_rate)
        self._cap = max(480, int(cap_samples))
        self._start = None       # buf[0] 的网格索引
        self._buf = None         # list[float] 连续样本
        self._drops = 0
        self._resync = 0

    def push_ts(self, ts0: float, samples) -> None:
        if not samples:
            return
        seq = [float(s) for s in samples]
        g0 = grid_from_ts(ts0, self._sr)
        if self._buf is None:
            self._buf = seq
            self._start = g0
            return
        end = self._start + len(self._buf)
        tol = 2  # 时间戳舍入容差（样本）
        if g0 > end + tol:
            # 生产者真空洞 → 重锚（历史重来；配对期间上层短暂直通）
            self._buf = seq
            self._start = g0
            self._resync += 1
        elif g0 > end:
            # 极小正偏（±1 样本舍入）→ 跳过 seq 前 (g0-end) 个，保持连续
            skip = g0 - end
            if skip >= len(seq):
                return
            seq = seq[skip:]
            self._buf.extend(seq)
        else:
            if g0 < end:
                skip = end - g0
                if skip >= len(seq):
                    return
                seq = seq[skip:]
            self._buf.extend(seq)
        over = len(self._buf) - self._cap
        if over > 0:
            self._buf = self._buf[over:]
            self._start += over
            self._drops += over

    def push_contig(self, samples) -> None:
        """连续流式写入（不做时间戳对齐）。

        用于只求「连续」、不需跨时钟配对的输入（如播放输入回环行）：
        时间戳取整会让每次推入的网格推进与样本数差 ±1，push_ts 的补偿逻辑
        随即丢/补样本，形成周期性微小时基抖动（听感为轻微「嘀嗒」毛刺）。
        """
        seq = [float(s) for s in samples]
        if not seq:
            return
        if self._buf is None:
            self._buf = seq
            self._start = 0
        else:
            self._buf.extend(seq)
        over = len(self._buf) - self._cap
        if over > 0:
            self._buf = self._buf[over:]
            self._start += over
            self._drops += over

    def start_grid(self) -> Optional[int]:
        return self._start

    def end_grid(self) -> Optional[int]:
        return None if self._buf is None else self._start + len(self._buf)

    def window(self, start_grid: int, n: int) -> Optional[list]:
        """取 [start_grid, start_grid+n) 连续样本；不足返回 None。"""
        n = int(n)
        if self._buf is None or n <= 0:
            return None
        idx = start_grid - self._start
        if idx < 0 or idx + n > len(self._buf):
            return None
        return self._buf[idx:idx + n]

    def pop_hop(self, n: int) -> Optional[list]:
        """从历史头部连续取 n 样本（够才给），并前移头部；不足返回 None。"""
        n = int(n)
        if self._buf is None or len(self._buf) < n:
            return None
        out = self._buf[:n]
        self._buf = self._buf[n:]
        self._start += n
        return out

    def diag(self) -> dict:
        return {"g_start": self._start,
                "len": (0 if self._buf is None else len(self._buf)),
                "drops": self._drops, "resync": self._resync}
