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
Linux 平台扬声器 loopback 采集（AEC far-end 数据源）。

原生 PipeWire 实现（取代旧 PyAudio/PulseAudio "Monitor of <sink>" 路径）：
  - 在已有 PwBridge 上开出第 4 条 far 流（"PureVox-far"），以 PipeWire 的
    stream.capture.sink 语义 tap 目标 sink 的播出输出 —— 不依赖预先存在的
    ".monitor" 源节点，也不走 pyaudio 枚举。
  - 采样率恒为 F32 单声道 48000Hz（与模型对齐），C++ 无需重采样。
  - 目标 sink 缺省时使用 speaker_sink_name()（物理输出兜底），只有虚拟麦克风
    时 start() 返回 False，上层 AEC 静默降级。

接口契约（与各平台后端一致）：
    start() -> bool / stop() / read(n) / flush()
    dev_sr (int) / active (bool) / on_device_changed 回调
"""

from typing import List, Optional, Callable

from .common import _module_log
from .pwpipe_client import pw_available, PwBridge, speaker_sink_name


class SpeakerCaptureLinux:
    """半双工扬声器采集 — 原生 PipeWire capture.sink（Linux 专用）。"""

    AEC_FAR_SR = 48000

    def __init__(self, on_device_changed: Optional[Callable[[int], None]] = None,
                 bridge: Optional[PwBridge] = None,
                 sink_name: str = ""):
        self._active = False
        self._bridge = bridge
        self._own_bridge = bridge is None   # 未给桥 = 自建（随 stop 释放）
        self._sink_name = sink_name or ""
        self._dev_sr = self.AEC_FAR_SR
        self._far_handle = -1               # 桥上 far 专用流句柄
        self._on_device_changed = on_device_changed

    @property
    def active(self) -> bool:
        return self._active

    @property
    def dev_sr(self) -> int:
        return self._dev_sr

    @property
    def sink_name(self) -> str:
        return self._sink_name

    def start(self) -> bool:
        if not pw_available():
            _module_log("[AEC] Linux: pvpipe 不可用，无法采集扬声器")
            return False
        if self._bridge is None:
            self._bridge = PwBridge()       # 独立会话（桌面声音等无桥场景）
        if not self._bridge.available:
            _module_log("[AEC] Linux: PipeWire 桥未就绪，AEC far 采集不可用")
            return False
        sink = self._sink_name or speaker_sink_name()
        if not sink:
            _module_log("[AEC] Linux: 无物理扬声器 sink，AEC 降级")
            return False
        self._far_handle = self._bridge.open_far(sink, monitor=True)
        if self._far_handle < 0:
            _module_log(f"[AEC] Linux 打开 far 采集流失败: {sink}")
            return False
        self._active = True
        _module_log(f"[AEC] Linux 扬声器采集 (PipeWire capture.sink): {sink} "
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
        SpeakerCapture 的 TimedFifo 同一语义；AEC 行按此与 mic 网格配对。"""
        if not self._active or self._bridge is None or self._far_handle < 0:
            return None
        return self._bridge.read_far_ts(self._far_handle, n_samples)