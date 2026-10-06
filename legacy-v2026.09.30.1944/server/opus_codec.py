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

import os
import struct
from typing import List, Optional

# opuslib (ctypes wrapper) needs opus.dll on PATH at load time
_this_dir = os.path.dirname(os.path.abspath(__file__))
if _this_dir not in os.environ.get('PATH', ''):
    os.environ['PATH'] = _this_dir + os.pathsep + os.environ.get('PATH', '')

try:
    import opuslib
    OPUS_AVAILABLE = True
except Exception:
    # ImportError = pip 包缺失；普通 Exception = 系统缺 libopus（Linux）。
    # 缺库只降级 Opus 解码（网络推流不可用），主程序其余功能不受影响
    OPUS_AVAILABLE = False

from logger import Logger
logger = Logger()


class OpusDecoder:
    def __init__(self, sample_rate: int = 48000, channels: int = 1,
                 frame_duration_ms: int = 10):   # 10ms 帧 = 480 样本 @48k（与模型 hop 一致）
        self._sample_rate = sample_rate
        self._channels = channels
        self._frame_size = int(sample_rate * frame_duration_ms / 1000)
        self._decoder = None
        if OPUS_AVAILABLE:
            self._decoder = opuslib.Decoder(sample_rate, channels)

    def decode(self, opus_data: bytes) -> Optional[List[float]]:
        if not self._decoder or not opus_data:
            return None
        try:
            pcm_bytes = self._decoder.decode(opus_data, self._frame_size)
            samples = struct.unpack(f'{len(pcm_bytes)//2}h', pcm_bytes)
            return [s / 32767.0 for s in samples]
        except Exception as e:
            logger.warn(f"Opus 解码失败: {e}")
            return None

    @property
    def available(self) -> bool:
        return self._decoder is not None
