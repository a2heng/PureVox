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

"""TSE 组件——purevox_tse_202609c 模型（目标说话人提取，全帧 1001 key ISA）。

模型契约（202609 三件套统一，STFT 在模型图内，引擎零 DSP）：
  输入  mix_hop [1,480] + enr_tok [1,2,1001,180] + cache_in [1,513216]
  输出  enh_hop [1,480]（滞后 1 hop）+ cache_out [1,513216]
enr_tok 由 ref_encoder.onnx（无参数）对注册语音一次性预计算：
  10s 归一（不足平铺）→ 双端零 pad 480 → 1001 帧 × 960 sqrt-Hann →
  ref_spec [1,2,1001,481] → ref_encoder.onnx → enr_tok [1,2,1001,180]。
无参考时组件直通。
"""

import os
import re

import numpy as np

from pvengine.context import HOP_LENGTH, SAMPLE_RATE
from pvengine.dsp.core import make_session, state_dim
from pvengine.stages.base import Stage

_CACHE_FALLBACK = 513216
REF_SEC = 10.0                       # 训练注册时长 (tse202609c REF=10.0)
REF_SAMPLES = int(REF_SEC * SAMPLE_RATE)   # 480000
REF_FRAMES = REF_SAMPLES // HOP_LENGTH + 1  # 1001（双端 pad 480）
_WIN = 2 * HOP_LENGTH                # 960


def _sqrt_hann(n: int) -> np.ndarray:
    """sqrt-Hann（周期）——与训练侧 torch.hann_window(n).pow(0.5) 严格一致
    （注意不带 dsp.core.sqrt_hann 的 1e-10 偏置，保证 ref_spec 逐位对齐）。"""
    hann = np.float32(0.5) - np.float32(0.5) * np.cos(
        2.0 * np.pi * np.arange(n, dtype=np.float64) / n)
    return np.sqrt(hann.astype(np.float32)).astype(np.float32)


def _ref_encoder_path(model_path: str) -> str:
    """主模型路径 → 同目录 ref_encoder 路径（剥离 _epXXXX 段）。"""
    d, base = os.path.split(model_path)
    stem = re.sub(r"_ep\d+", "", base)
    stem = stem[:-5] if stem.endswith(".onnx") else stem
    return os.path.join(d, stem + "_ref_encoder.onnx")


class TseEngine:
    def __init__(self, model_path: str):
        self.sess = make_session(model_path)
        self._model_path = model_path
        self.in_names = [i.name for i in self.sess.get_inputs()]
        self.out_names = [o.name for o in self.sess.get_outputs()]
        dim = state_dim(self.sess, "cache_in", _CACHE_FALLBACK)
        self.cache = np.zeros((1, dim), dtype=np.float32)
        self.enr_tok: np.ndarray | None = None   # (1,2,1001,180)
        self._ref_sess = None

    @property
    def has_reference(self) -> bool:
        return self.enr_tok is not None

    def set_reference(self, ref: np.ndarray, ref_key: str | None = None) -> bool:
        """参考音频（48kHz float）→ 归一 10s → ref_spec → ref_encoder → enr_tok。

        ref_key 给出参考来源文件路径时启用 enr_tok 缓存 (<ref_key>_enrtok.npz)：
        键 = 录音 mtime/size + ref_encoder mtime/size —— 录音或模型版本未变时
        直接载入缓存（跳过 STFT+ref_encoder 处理）；任一变化自动失效重算。
        """
        ref = np.asarray(ref, dtype=np.float32).reshape(-1)
        if len(ref) < HOP_LENGTH:            # 过短（<10ms）拒绝
            return False
        # fix_ref: 不足 10s 平铺补足，超长截断（与训练侧一致）
        ref = np.tile(ref, REF_SAMPLES // len(ref) + 1)[:REF_SAMPLES]

        # ── 缓存键 ──
        cache_path = os.path.splitext(ref_key)[0] + "_enrtok.npz" if ref_key else None
        sig = None
        rp = _ref_encoder_path(self._model_path)
        if cache_path:
            try:
                sig = np.array([
                    int(os.path.getmtime(ref_key) * 1000), int(os.path.getsize(ref_key)),
                    int(os.path.getmtime(rp) * 1000) if os.path.isfile(rp) else 0,
                    int(os.path.getsize(rp)) if os.path.isfile(rp) else 0,
                    REF_SAMPLES], dtype=np.int64)
            except OSError:
                cache_path = None
        if cache_path and os.path.isfile(cache_path):
            try:
                z = np.load(cache_path)
                tok = z["enr_tok"]
                if tok.shape == (1, 2, REF_FRAMES, 180) and np.array_equal(z["sig"], sig):
                    self.enr_tok = tok.astype(np.float32)
                    return True
            except Exception:
                pass

        # ── 现算: 双端零 pad 480 → 1001 帧 × 960 sqrt-Hann → ref_spec ──
        x = np.zeros(REF_SAMPLES + 2 * HOP_LENGTH, dtype=np.float32)
        x[HOP_LENGTH:HOP_LENGTH + REF_SAMPLES] = ref
        win = _sqrt_hann(_WIN)
        spec = np.empty((REF_FRAMES, _WIN // 2 + 1), dtype=np.complex64)
        for k in range(REF_FRAMES):
            off = k * HOP_LENGTH
            spec[k] = np.fft.rfft(x[off:off + _WIN] * win, n=_WIN)
        ref_spec = np.empty((1, 2, REF_FRAMES, _WIN // 2 + 1), dtype=np.float32)
        ref_spec[0, 0] = spec.real
        ref_spec[0, 1] = spec.imag
        # ref_encoder.onnx（无参数）：drc + ERB + 全帧 key
        if self._ref_sess is None:
            if not os.path.isfile(rp):
                raise FileNotFoundError(rp)
            self._ref_sess = make_session(rp)
        tok = self._ref_sess.run(None, {"ref_spec": ref_spec})[0]
        self.enr_tok = np.asarray(tok, dtype=np.float32)
        # ── 落缓存 ──
        if cache_path and sig is not None:
            try:
                np.savez(cache_path, enr_tok=self.enr_tok, sig=sig)
            except Exception:
                pass
        return True

    def process_chunk(self, block: np.ndarray) -> np.ndarray:
        if len(block) != HOP_LENGTH:
            raise ValueError(f"tse: chunk must be {HOP_LENGTH} samples (10ms)")
        outs = self.sess.run(self.out_names, {
            "mix_hop": np.asarray(block, dtype=np.float32).reshape(1, -1),
            "enr_tok": self.enr_tok,
            "cache_in": self.cache,
        })
        od = dict(zip(self.out_names, outs))
        self.cache = np.asarray(od["cache_out"], dtype=np.float32).reshape(1, -1)
        return np.asarray(od["enh_hop"], dtype=np.float32).reshape(-1)

    def reset(self):
        self.cache[:] = 0.0

    def release(self):
        self.sess = None
        self._ref_sess = None


class TseStage(Stage):
    """TSE 模式组件：波形 hop 直入模型（无参考时直通）。"""

    name = "tse"

    def __init__(self, model_path: str):
        super().__init__()
        self.engine = TseEngine(model_path)

    @property
    def has_reference(self) -> bool:
        return self.engine.has_reference

    def set_reference(self, ref: np.ndarray, ref_key: str | None = None) -> bool:
        return self.engine.set_reference(ref, ref_key=ref_key)

    def process(self, frame, ctx):
        if not self.engine.has_reference:
            return frame
        return self.engine.process_chunk(frame)

    def reset(self):
        self.engine.reset()

    def release(self):
        self.engine.release()
