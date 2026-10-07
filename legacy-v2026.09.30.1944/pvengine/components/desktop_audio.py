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

"""桌面声音输入插件——系统/应用音频经 loopback 捕获接入处理链。

复用 AEC 的 SpeakerCapture 平台工厂（Win=WASAPI loopback 默认渲染设备，
Linux=PipeWire monitor 源，macOS 同理），pull 模型：process 每帧读取
捕获环形缓冲的最新样本与信号相加。链位置语义=注入点：默认链尾=随全部
输出扇出；注意 AEC 与本节点会各自开一路 loopback（互不干扰）。

音量滑杆实时生效；禁用行（复选框关）时插件自行淡出并释放捕获
（FADE_THROUGH：适配层不旁路，衔接无缝），勾回原地淡入恢复。
"""

import threading

import numpy as np

from pvengine.components.effect_base import Effect


class DesktopAudioPlugin(Effect):
    NAME = "desktop_audio"
    LABEL = "桌面声音输入"
    # 复选框关断不旁路：插件自行淡出并释放捕获（衔接无缝）
    FADE_THROUGH = True
    PARAMS = {"volume_db": ("音量 dB", -30.0, 6.0, -6.0, 1.0)}

    def __init__(self, params=None, stage_cache=None):
        self._lock = threading.Lock()
        self._capture = None
        self._mix = 0.0        # 混入包络（0..1，每 hop ±1/n 步进）
        self._volume = 10.0 ** (-6.0 / 20.0)
        self.enabled = True
        super().__init__(params)

    def on_params_changed(self):
        self._volume = 10.0 ** (self.params["volume_db"] / 20.0)

    def _ensure_capture(self):
        """懒启动 loopback 捕获（失败静默，process 退化为直通）。"""
        with self._lock:
            if self._capture is not None:
                return
            try:
                from pvplatform.audio import create_speaker_capture
                cap = create_speaker_capture()
                if cap.start():
                    self._capture = cap
            except Exception:
                self._capture = None

    def _shutdown_capture(self):
        with self._lock:
            cap, self._capture = self._capture, None
        if cap is not None:
            try:
                cap.stop()
            except Exception:
                pass

    # ── 音频面 ──
    def process(self, frame, ctx):
        n = len(frame)
        active = bool(self.enabled)
        if self._mix <= 0.0 and not active:
            return frame                      # 禁用且已淡出：零开销直通
        if self._capture is None:
            if not active:
                return frame
            self._ensure_capture()
            if self._capture is None:
                return frame
        data = self._capture.read(n)
        m = min(n, len(data)) if data else 0
        # ── 混入包络：每 hop ±1/n 线性淡入淡出，启停衔接无缝 ──
        target = 1.0 if (active and m >= n) else 0.0
        step = 1.0 / max(1, n)
        if self._mix < target:
            self._mix = min(target, self._mix + step)
        elif self._mix > target:
            self._mix = max(target, self._mix - step)
        if self._mix <= 0.0:
            if not active:
                self._shutdown_capture()
            return frame
        out = frame.astype(np.float32, copy=True)
        if m:
            out[:m] += np.asarray(data[:m], dtype=np.float32) * \
                np.float32(self._volume * self._mix)
            np.clip(out, -1.0, 1.0, out=out)
        return out

    def reset(self):
        self._shutdown_capture()
