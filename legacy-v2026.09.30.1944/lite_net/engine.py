# PureVox Lite Denoise Only — 纯 Python 推理引擎
# Copyright (C) 2024-2026 a2heng <752848283@qq.com>
# SPDX-License-Identifier: GPL-3.0-or-later
#
# 直接用 onnxruntime + numpy 实现（与主程序 pvengine 同契约、独立精简副本）
# 契约 (202609 三件套统一): 波形 hop 进出, STFT 在模型图内, 引擎零 DSP
#   模型: purevox_denoise_202609 (输入 mix_hop [1,480] + cache_in [1,26080],
#         输出 enh_hop [1,480] + cache_out [1,26080])
#   hop = 10ms @48kHz = 480 样本; enh_hop 滞后 1 hop (10ms, 模型内部 tail 语义)
#   从零缓存起步即与训练流式契约一致 (无需静音预热)

import os
import numpy as np
import onnxruntime as ort

HOP = 480
CACHE_FALLBACK = 26080


class LiteDenoiseEngine:
    def __init__(self, model_path):
        if not os.path.isfile(model_path):
            raise FileNotFoundError(model_path)
        so = ort.SessionOptions()
        so.log_severity_level = 3
        so.intra_op_num_threads = 1
        so.inter_op_num_threads = 1
        # 保持默认 BFC arena：实测关闭仅省 ~6MB Commit，却让每帧推理 +33% CPU
        # （分配器逐帧搬运），得不偿失
        so.execution_mode = ort.ExecutionMode.ORT_SEQUENTIAL
        so.graph_optimization_level = ort.GraphOptimizationLevel.ORT_ENABLE_ALL
        self.sess = ort.InferenceSession(model_path, sess_options=so, providers=["CPUExecutionProvider"])
        self.in_names = [i.name for i in self.sess.get_inputs()]
        self.out_names = [o.name for o in self.sess.get_outputs()]
        # 缓存尺寸从模型输入形状推断（动态维回退历史常量）
        self.cache = np.zeros((1, self._cache_dim()), dtype=np.float32)

    def _cache_dim(self):
        for inp in self.sess.get_inputs():
            if inp.name == "cache_in":
                shape = inp.shape
                if len(shape) == 2 and isinstance(shape[1], int) and shape[1] > 0:
                    return shape[1]
                return CACHE_FALLBACK
        return CACHE_FALLBACK

    def process(self, hop480):
        # hop480: np array 480 float32 (10ms)
        if hop480.shape[0] != HOP:
            raise ValueError(f"chunk must be {HOP}")
        outs = self.sess.run(self.out_names, {
            "mix_hop": np.asarray(hop480, dtype=np.float32).reshape(1, -1),
            "cache_in": self.cache,
        })
        od = dict(zip(self.out_names, outs))
        self.cache = np.asarray(od["cache_out"], dtype=np.float32).reshape(1, -1)
        return np.asarray(od["enh_hop"], dtype=np.float32).reshape(-1)

    def reset(self):
        self.cache[:] = 0.0
