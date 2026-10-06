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
平台音频后端抽象层。

对外暴露统一接口（与 audio_processor 的调用方契约一致）：

    SpeakerCapture
        扬声器 loopback 采集（AEC far=扬声器时的数据源）。
        接口：start() -> bool, stop(), read(n) -> Optional[list],
              available() -> int, dev_sr(int), active(bool), flush()。

    MicCapture
        麦克风专用采集（AEC far=麦克风时的数据源，一行一路，
        不进主混音，直达行内 AecRow）。接口同 SpeakerCapture。

    create_speaker_capture(on_device_changed=None, pw_bridge=None, far_sink="", device_name="")
        工厂函数，按当前平台返回具体后端实例：
        - Windows: WASAPI loopback（COM）。device_name 传目标端点名（与主设备
          选择同一模糊匹配）；不传/未命中回退默认渲染端点。
        - Linux:   原生 PipeWire capture.sink（复用已有 PwBridge，far 专用流）
        - macOS:   预留
    pw_bridge / far_sink 仅 Linux 使用：pw_bridge 为已打开的 PwBridge；
    far_sink 为扬声器 sink 节点名（空则取物理扬声器兜底）。

    create_mic_capture(dev, pw_bridge=None)
        工厂函数：dev 为 Windows 设备索引（int）/ Linux 源节点名（str）。
"""

from .. import IS_WINDOWS, IS_LINUX, IS_MACOS


def create_speaker_capture(on_device_changed=None, pw_bridge=None, far_sink="",
                           device_name=""):
    """按平台创建 SpeakerCapture 实例。"""
    if IS_WINDOWS:
        from .speaker_capture_win import SpeakerCaptureWin
        return SpeakerCaptureWin(on_device_changed=on_device_changed,
                                 device_name=device_name)
    if IS_LINUX:
        from .speaker_capture_linux import SpeakerCaptureLinux
        return SpeakerCaptureLinux(on_device_changed=on_device_changed,
                                   bridge=pw_bridge, sink_name=far_sink)
    if IS_MACOS:
        from .speaker_capture_macos import SpeakerCaptureMacOS
        return SpeakerCaptureMacOS(on_device_changed=on_device_changed)
    raise RuntimeError(f"不支持的平台: {__import__('sys').platform}")


def create_mic_capture(dev=None, pw_bridge=None):
    """按平台创建 MicCapture 实例（dev：Windows 设备索引 / Linux 源节点名）。"""
    if IS_WINDOWS:
        from .mic_capture_win import MicCaptureWin
        return MicCaptureWin(device_id=dev)
    if IS_LINUX:
        from .mic_capture_linux import MicCaptureLinux
        return MicCaptureLinux(bridge=pw_bridge, source_name=dev or "")
    if IS_MACOS:
        from .speaker_capture_macos import SpeakerCaptureMacOS
        return SpeakerCaptureMacOS()
    raise RuntimeError(f"不支持的平台: {__import__('sys').platform}")


__all__ = ["create_speaker_capture", "create_mic_capture"]
