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

"""纯媒体播放会话（无设备输入的本地媒体：文件/音效板/桌面声音）。

唯一实现：miniaudio PlaybackDevice（自包含 C，WASAPI/Pulse 直开）拉模型。
播放正确性收敛到 PlaybackSink（make_sink 注入，pvengine 持有实现）：
主设备回调驱动引擎帧源（read_frame → 写全部 sink），每个设备——
主/从地位对等——各自从自己的 sink pull 帧输出。设备间速率差由
sink 变速消化（此前从设备走队列+垫零，速率差必周期性咔哒）。

输出设备按名称精确匹配 miniaudio 枚举；未选或未匹配 = 系统默认输出
（Linux 走 pipewire-pulse 时 sink name 即 pw-dump 的 node.name）。
格式恒 F32 立体声 48kHz（miniaudio 负责与设备混音格式的转换）。
"""

import numpy as np

_TARGET_SR = 48000
_HOP = 480              # 混合粒度（10ms @48kHz，与 pvengine.context.HOP_LENGTH 一致）
_CHANNELS = 2            # 立体声交错输出
_BUFFER_MS = 60          # 设备周期：延迟/抗抖动平衡


class MediaSession:
    """read_frame(n) 引擎帧源；每输出设备一个 sink（write/pull 注入）。"""

    def __init__(self, read_frame, out_names, make_sink):
        self._read = read_frame
        self._names = [str(n) for n in (out_names or []) if n]
        self._make_sink = make_sink   # () -> sink 对象（write/pull）
        self._sinks = []              # 与设备一一对应
        self._devs = None             # Devices 上下文（设备 id 生命周期随它）
        self._devices = []            # [PlaybackDevice]
        self._err = None

    @property
    def error(self):
        return self._err

    def start(self) -> bool:
        try:
            import miniaudio
            self._devs = miniaudio.Devices()
            plays = self._devs.get_playbacks()
            ids = [self._match(plays, n) for n in self._names]
            ids = [i for i in ids if i is not None]
            if not ids:
                ids = [None]       # 未选/未匹配输出 = 系统默认输出
            self._sinks = [self._make_sink() for _ in ids]
            for i, dev_id in enumerate(ids):
                gen = self._master_gen() if i == 0 else self._slave_gen(i)
                next(gen)          # 生成器须先启动再交给设备
                dev = miniaudio.PlaybackDevice(
                    output_format=miniaudio.SampleFormat.FLOAT32,
                    nchannels=_CHANNELS, sample_rate=_TARGET_SR,
                    buffersize_msec=_BUFFER_MS, device_id=dev_id)
                dev.start(gen)
                self._devices.append(dev)
            return True
        except Exception as e:
            self._err = f"媒体输出设备打开失败: {e}"
            self.stop()
            return False

    @staticmethod
    def _match(plays, name):
        want = name.strip().casefold()
        for d in plays:
            if str(d.get("name", "")).strip().casefold() == want:
                return d["id"]
        return None

    def _write_all(self, frame) -> None:
        """引擎产出的混合帧写全部 sink（主设备回调驱动）。"""
        if not frame:
            return
        for s in self._sinks:
            s.write(frame)

    def _master_gen(self):
        """主设备回调生成器：驱动引擎帧源，再从自己 sink 拉满请求数。

        每轮 read 一个 hop → 写全部 sink → 消费等量（主时钟收支恒平，
        sink 只做 hop→need 的粒度适配与预热）。
        """
        need = yield b""
        while True:
            data = []
            while len(data) < need:
                try:
                    frame = self._read(_HOP)
                except Exception:
                    frame = None
                if not frame or len(frame) < _HOP:
                    frame = [0.0] * _HOP
                self._write_all(frame)
                data.extend(self._sinks[0].pull(min(_HOP, need - len(data))))
            need = yield _stereo_bytes(data, need)

    def _slave_gen(self, i: int):
        """从设备回调生成器：从自己的 sink 拉帧（sink 消化设备间速率差）。"""
        need = yield b""
        while True:
            data = self._sinks[i].pull(need)
            need = yield _stereo_bytes(data, need)

    def stop(self):
        for d in self._devices:
            try:
                d.close()
            except Exception:
                pass
        self._devices = []
        self._sinks = []
        self._devs = None


def _stereo_bytes(data, n: int) -> bytes:
    """单声道 n 样本 → F32 立体声交错 bytes。"""
    x = np.asarray(data[:n], dtype=np.float32)
    return np.repeat(x, _CHANNELS).tobytes()
