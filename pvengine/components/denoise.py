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

"""Denoise 组件——purevox_denoise_202609 模型流式推理。

模型契约（202609 三件套统一，STFT 在模型图内，引擎零 DSP）：
  输入  mix_hop  [1,480]  波形 hop (10ms @48kHz)
        cache_in [1,26080] 扁平流式缓存（首帧零起）
  输出  enh_hop  [1,480]  增强波形 hop（滞后 1 hop = 10ms，模型内部 tail 语义）
        cache_out [1,26080]
从零缓存起步即与训练流式契约一致（无需静音预热）。
"""

import numpy as np

from pvengine.context import HOP_LENGTH
from pvengine.dsp.core import make_session, state_dim
from pvengine.stages.base import Stage

_CACHE_FALLBACK = 26080


class DenoiseEngine:
    def __init__(self, model_path: str):
        self.sess = make_session(model_path)
        self.in_names = [i.name for i in self.sess.get_inputs()]
        self.out_names = [o.name for o in self.sess.get_outputs()]
        dim = state_dim(self.sess, "cache_in", _CACHE_FALLBACK)
        self.cache = np.zeros((1, dim), dtype=np.float32)

    def process_chunk(self, block: np.ndarray) -> np.ndarray:
        if len(block) != HOP_LENGTH:
            raise ValueError(f"denoise: chunk must be {HOP_LENGTH} samples (10ms)")
        outs = self.sess.run(self.out_names, {
            "mix_hop": np.asarray(block, dtype=np.float32).reshape(1, -1),
            "cache_in": self.cache,
        })
        od = dict(zip(self.out_names, outs))
        self.cache = np.asarray(od["cache_out"], dtype=np.float32).reshape(1, -1)
        return np.asarray(od["enh_hop"], dtype=np.float32).reshape(-1)

    def reset(self):
        self.cache[:] = 0.0

    def release(self):
        self.sess = None


class DenoiseStage(Stage):
    """降噪模式组件：在 DENOISE/AEC/TSE 模式下生效（AEC/TSE 先降噪再处理）。"""

    name = "denoise"
    active_modes = frozenset({1, 2, 3})

    def __init__(self, model_path: str):
        super().__init__()
        self.engine = DenoiseEngine(model_path)

    def process(self, frame, ctx):
        return self.engine.process_chunk(frame)

    def reset(self):
        self.engine.reset()

    def release(self):
        self.engine.release()
