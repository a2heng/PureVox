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

"""AEC 会话——purevox_aec_202609_cpx 模型（回声消除），多行共享。

模型契约（202609 三件套统一，STFT 在模型图内，引擎零 DSP）：
  输入  mic_hop [1,480] + far_hop [1,480] + cache_in [1,215504]
  输出  enh_hop [1,480]（滞后 1 hop）+ cache_out [1,215504]

本文件只持有无状态的 ONNX 会话（多 EC 行共享一份）；行级状态
（cache / far 对齐 / mic 增益）归 `pvengine/ec.py` 的 EcRow 所有。
"""

import numpy as np

from pvengine.dsp.core import make_session, state_dim

_CACHE_FALLBACK = 215504


class AecEngine:
    """无状态 AEC 会话：cache 由调用方持有传入，多行共享同一 sess。"""

    def __init__(self, model_path: str):
        self.sess = make_session(model_path)
        self.in_names = [i.name for i in self.sess.get_inputs()]
        self.out_names = [o.name for o in self.sess.get_outputs()]
        self.state_dim = state_dim(self.sess, "cache_in", _CACHE_FALLBACK)

    def new_state(self) -> np.ndarray:
        """一行一份的零 cache（行启停/重建时重取）。"""
        return np.zeros((1, self.state_dim), dtype=np.float32)

    def process_frame(self, mic: np.ndarray, far: np.ndarray,
                      cache: np.ndarray) -> tuple:
        """mic/far 各 480 样本（48kHz，10ms）→ (输出 480, 新 cache)。"""
        outs = self.sess.run(self.out_names, {
            "mic_hop": np.asarray(mic, dtype=np.float32).reshape(1, -1),
            "far_hop": np.asarray(far, dtype=np.float32).reshape(1, -1),
            "cache_in": np.asarray(cache, dtype=np.float32).reshape(1, -1),
        })
        od = dict(zip(self.out_names, outs))
        return (np.asarray(od["enh_hop"], dtype=np.float32).reshape(-1),
                np.asarray(od["cache_out"], dtype=np.float32).reshape(1, -1))

    def release(self):
        self.sess = None
