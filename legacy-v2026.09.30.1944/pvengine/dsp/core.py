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

"""基础 DSP 件：窗函数、限幅、ONNX 会话工厂。"""

import numpy as np
import onnxruntime as ort

from pvengine.context import NFFT


def sqrt_hann(n: int = NFFT) -> np.ndarray:
    """sqrt-Hann 分析窗（torch.hann_window(periodic=True).pow(0.5) 语义，
    含 1e-10 偏置防止零窗）。"""
    hann = np.float32(0.5) - np.float32(0.5) * np.cos(
        2.0 * np.pi * np.arange(n, dtype=np.float64) / n)
    return np.sqrt(hann.astype(np.float32) + np.float32(1e-10)).astype(np.float32)


def hann_window(n: int = NFFT) -> np.ndarray:
    """周期性 Hann 窗（= sqrt_hann 的平方，TSE 路径用）。"""
    w = sqrt_hann(n)
    return (w * w).astype(np.float32)


def clip_buffer(data: np.ndarray) -> np.ndarray:
    """±1 限幅；NaN/Inf 归零（对齐原 C clip_sample）。"""
    out = np.nan_to_num(data, nan=0.0, posinf=0.0, neginf=0.0).astype(np.float32)
    return np.clip(out, -1.0, 1.0)


def make_session(model_path: str) -> ort.InferenceSession:
    """统一 ONNX 会话：单线程 + 全量图优化（与原 C 引擎一致）。"""
    so = ort.SessionOptions()
    so.log_severity_level = 3
    so.intra_op_num_threads = 1
    so.inter_op_num_threads = 1
    so.execution_mode = ort.ExecutionMode.ORT_SEQUENTIAL
    so.graph_optimization_level = ort.GraphOptimizationLevel.ORT_ENABLE_ALL
    return ort.InferenceSession(model_path, sess_options=so,
                                providers=["CPUExecutionProvider"])


def state_dim(session: ort.InferenceSession, name: str, fallback: int = 0) -> int:
    """从会话输入元数据读取一维状态张量的维度（动态维返回 fallback）。"""
    for inp in session.get_inputs():
        if inp.name == name:
            shape = inp.shape
            if len(shape) >= 1 and isinstance(shape[-1], int) and shape[-1] > 0:
                return int(shape[-1])
            return fallback
    return fallback
