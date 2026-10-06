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

"""pvengine——PureVox 纯 Python 组件化音频引擎。

组件化原则：
- Stage 接口是唯一契约（stages/base.py）：process(frame, ctx) + reset()/release()；
- components/ 下每个文件一个关注点，满足接口即可随意增删替换重排；
- dsp/ 提供可独立复用的基础算法件（窗/STFT/环形缓冲/重采样/Mel 频谱）；
- Pipeline 只负责按序执行与模式旁路判断，不感知具体组件实现。
"""

from pvengine.context import (SAMPLE_RATE, HOP_LENGTH, NFFT, FREQ,
                              FrameContext)
from pvengine.pipeline import Pipeline
from pvengine.processor import AudioProcessor
from pvengine.dsp.ring_buffer import RingBuffer
from pvengine.dsp.resampler import Resampler
from pvengine.dsp.playback import PlaybackSink
from pvengine.dsp.hop_queue import GridHistory, grid_from_ts
from pvengine.dsp.spectrum import (SPECTRUM_NUM_BANDS, SPECTRUM_FFT,
                                   compute_spectrum, spectrum_warmup)

__all__ = [
    "AudioProcessor", "Pipeline", "FrameContext",
    "RingBuffer", "Resampler", "PlaybackSink",
    "GridHistory", "grid_from_ts",
    "compute_spectrum", "spectrum_warmup",
    "SAMPLE_RATE", "HOP_LENGTH", "NFFT", "FREQ",
    "SPECTRUM_NUM_BANDS", "SPECTRUM_FFT",
]
