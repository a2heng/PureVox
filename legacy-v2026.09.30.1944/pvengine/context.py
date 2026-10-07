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

"""组件间数据契约：处理模式常量与帧上下文。

纯 Python 音频引擎的数据流约定：
- 音频帧一律 numpy.float32 一维数组，单声道；
- 处理 hop = 10ms（48kHz → 480 样本；NFFT = 2×hop = 960）——
  按时间派生而非固定样本数，未来多采样率/重采样时契约不变；
- AI 模型 (202609 三件套) 同契约：波形 hop 进出，STFT 在模型图内，
  enh_hop 滞后 1 hop (10ms)；
- 每个组件实现 Stage 接口，按链路顺序消费/产出帧。
"""

from dataclasses import dataclass

SAMPLE_RATE = 48000
HOP_LENGTH = SAMPLE_RATE // 100   # 10ms @48kHz = 480 样本
NFFT = 2 * HOP_LENGTH             # 960
FREQ = NFFT // 2 + 1              # 481


@dataclass
class FrameContext:
    """一帧音频的处理上下文（同一帧在整条链路中共享同一 ctx）。

    当前无字段：保留该类型以固定 Stage.process(frame, ctx) 契约；
    需要跨组件传递的旁路信息由调用方按需扩展。
    """
