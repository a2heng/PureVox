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
macOS 平台扬声器 loopback 采集（占位实现）。

macOS 尚无系统的 loopback 采集方案（需 ScreenCaptureKit 的
AudioCaptureEngine 或虚拟声卡驱动 BlackHole 等）。本模块保持接口
契约一致，start() 恒返回 False，上层 AEC 自动降级（不阻塞主流程）。
"""

from typing import Optional, Callable

from .common import RingBuffer, HOP_LENGTH, _module_log


class SpeakerCaptureMacOS:
    """macOS 扬声器采集占位 — 无系统 loopback，AEC 不可用时降级。"""

    DEVICE_CHECK_INTERVAL = 2.0
    AEC_FAR_SR = 48000

    def __init__(self, on_device_changed: Optional[Callable[[int], None]] = None):
        self._buffer = RingBuffer(HOP_LENGTH * 2)
        self._active = False
        self._on_device_changed = on_device_changed

    @property
    def active(self) -> bool:
        return self._active

    @property
    def dev_sr(self) -> int:
        return 48000

    def start(self) -> bool:
        _module_log("[AEC] macOS: 尚无系统 loopback 采集，AEC 降级")
        return False

    def stop(self) -> None:
        self._active = False

    def read(self, n_samples: int) -> Optional[list]:
        return None

    def flush(self) -> None:
        self._buffer = RingBuffer(HOP_LENGTH * 2)

    def read_ts(self, n_samples: int):
        return None
