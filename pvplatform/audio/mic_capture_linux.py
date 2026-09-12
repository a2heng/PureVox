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

"""Linux 麦克风专用采集（AEC far 选麦克风时的数据源）。

在已有 PwBridge 上开一路 far 专用真源流（monitor=False，直录麦克风
节点，不经过 monitor），AEC 行自建自停，不进主混音，样本直达行内 AecRow
（far 严格配对，满 hop 出队）。采样率恒 F32 单声道 48000Hz。

接口契约（与 SpeakerCapture 一致）：
    start() -> bool / stop() / read(n) / available() / flush()
    dev_sr (int) / active (bool)
"""

from typing import Optional

from .common import _module_log
from .pwpipe_client import pw_available, PwBridge


class MicCaptureLinux:
    """指定麦克风的独立真源流（Linux 专用，far=mic 时一行一路）。"""

    AEC_FAR_SR = 48000

    def __init__(self, bridge: Optional[PwBridge] = None,
                 source_name: str = ""):
        self._active = False
        self._bridge = bridge
        self._own_bridge = bridge is None
        self._source_name = source_name or ""
        self._dev_sr = self.AEC_FAR_SR
        self._far_handle = -1

    @property
    def active(self) -> bool:
        return self._active

    @property
    def dev_sr(self) -> int:
        return self._dev_sr

    def start(self) -> bool:
        if not pw_available():
            return False
        if self._bridge is None:
            self._bridge = PwBridge()
        if not self._bridge.available:
            return False
        if not self._source_name:
            _module_log("[AEC] 麦克风 far 未指定源设备")
            return False
        self._far_handle = self._bridge.open_far(self._source_name,
                                                 monitor=False)
        if self._far_handle < 0:
            _module_log(f"[AEC] 麦克风 far 采集流打开失败: {self._source_name}")
            return False
        self._active = True
        _module_log(f"[AEC] 麦克风 far 采集: {self._source_name} "
                    f"({self._dev_sr}Hz, F32 单声道)")
        return True

    def stop(self) -> None:
        self._active = False
        if self._bridge is not None:
            if self._far_handle >= 0:
                self._bridge.close_far(self._far_handle)
                self._far_handle = -1
            if self._own_bridge:
                self._bridge.close()
                self._bridge = None

    def available(self) -> int:
        if not self._active or self._bridge is None or self._far_handle < 0:
            return 0
        return self._bridge.far_available(self._far_handle)

    def read(self, n_samples: int) -> Optional[list]:
        if not self._active or self._bridge is None or self._far_handle < 0:
            return None
        data = self._bridge.read_far_h(self._far_handle, n_samples)
        return list(data) if data else None

    def flush(self) -> None:
        pass

    def read_ts(self, n_samples: int):
        """读取 n 样本 → (首样本主时钟秒, samples)；不足返回 None。

        时间戳为流写入时的真实采集时刻（libpulse 流延迟），与 Windows
        MicCapture 的 TimedFifo 同一语义（far 与 mic 同一外部时钟域）。"""
        if not self._active or self._bridge is None or self._far_handle < 0:
            return None
        return self._bridge.read_far_ts(self._far_handle, n_samples)
