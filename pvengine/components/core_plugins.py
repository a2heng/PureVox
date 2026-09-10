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

"""核心处理插件——前增益/AGC/噪声门/EQ/压缩器/AI 降噪/AEC/TSE 统一为相同的
插件接口（NAME/LABEL/PARAMS/set_params/process/reset），
使整条管线完全由用户插件链驱动，不再有固定模式。

AI 类插件的模型引擎由 AudioProcessor 的 engine_cache 共享（按类型缓存），
避免链重建时重复加载模型。
"""

import numpy as np

from pvengine.components.effect_base import Effect


class GainPlugin(Effect):
    NAME = "gain"
    LABEL = "增益"
    PARAMS = {"gain_db": ("dB", -30.0, 30.0, 0.0, 1.0)}

    def __init__(self, params=None, stage_cache=None):
        from pvengine.components.gain import GainStage
        tmp = {"gain_db": (params or {}).get("gain_db", 0.0)}
        self.stage = GainStage(float(tmp["gain_db"]))
        super().__init__(params)

    def on_params_changed(self):
        self.stage.set_pre_gain_db(self.params["gain_db"])

    def process(self, frame, ctx):
        return self.stage.process(frame, ctx)

    def reset(self):
        self.stage.reset()


class AgcPlugin(Effect):
    """AGC 自动增益（自包含）：测本插件输入 RMS → 平滑增益 → 施加。"""

    NAME = "agc"
    LABEL = "自动增益 AGC"
    PARAMS = {"target_db": ("目标 dBFS", -40.0, 40.0, -20.0, 1.0)}

    def __init__(self, params=None, engine_cache=None):
        # 必须先建 agc 控制器再调 super().__init__——基类在 params 非空时会
        # 触发 on_params_changed()，此时访问 self.agc，若控制器尚未创建会抛
        # AttributeError 被 set_plugins 的 except 吞掉，导致 AGC 节点静默缺失。
        # （与 GainPlugin / CompressorPlugin 同模式：先建 stage 再调 super）
        from pvengine.components.gain import AgcController
        from pvengine.context import HOP_LENGTH, SAMPLE_RATE
        # 帧时长 = HOP_LENGTH / SAMPLE_RATE（1024@48k ≈ 21.33ms），用实际值算
        # attack/release/rms_ema 时间常数；不能用 AgcController 默认的 10ms，
        # 否则全部包络偏慢约 2 倍。
        dt_ms = HOP_LENGTH / SAMPLE_RATE * 1000.0
        self.agc = AgcController(call_interval_ms=dt_ms)
        self.agc.set_enabled(True, 0.0)
        super().__init__(params)

    def on_params_changed(self):
        self.agc.target_dbfs = self.params["target_db"]
        self.agc.target_linear = 10.0 ** (self.params["target_db"] / 20.0)

    def process(self, frame, ctx):
        rms = float(np.sqrt(np.mean(np.square(frame, dtype=np.float64)))) if len(frame) else 0.0
        self.agc.update_rms(rms)
        g = self.agc.tick()
        return frame * np.float32(g)

    def reset(self):
        self.agc.reset()


class GatePlugin(Effect):
    """噪声门（原 VAD 硬门升级为带参数的门）。"""

    NAME = "gate"
    LABEL = "噪声门 VAD"
    PARAMS = {
        "threshold_db": ("门限 dBFS", -80.0, -20.0, -45.0, 1.0),
        "attack_ms": ("开启 ms", 1.0, 100.0, 20.0, 5.0),
        "hold_ms": ("保持 ms", 0.0, 500.0, 250.0, 25.0),
        "release_ms": ("关闭 ms", 10.0, 1000.0, 250.0, 25.0),
    }

    def __init__(self, params=None, engine_cache=None):
        super().__init__(params)
        self._env = 0.0
        self._open_cnt = 0
        self._close_cnt = 0
        self._active = False

    def process(self, frame, ctx):
        rms = float(np.sqrt(np.mean(np.square(frame, dtype=np.float64)))) if len(frame) else 0.0
        thr = 10.0 ** (self.params["threshold_db"] / 20.0)
        onset = max(1, int(self.params["attack_ms"] / 21.3))
        hang = max(1, int(self.params["hold_ms"] / 21.3))
        rel_frames = max(1, int(self.params["release_ms"] / 21.3))
        is_voice = rms > thr
        if is_voice:
            self._open_cnt += 1
            self._close_cnt = 0
        else:
            self._close_cnt += 1
            self._open_cnt = 0
        if not self._active and self._open_cnt >= onset:
            self._active = True
        elif self._active and self._close_cnt >= hang:
            self._active = False
        if not self._active:
            # 平滑关断而非硬切（爆音抑制）
            self._env *= 0.85
        else:
            self._env = min(1.0, self._env + 0.3)
        return frame * np.float32(self._env)

    def reset(self):
        self._env = 0.0
        self._open_cnt = self._close_cnt = 0
        self._active = False


class _EqPluginBase(Effect):
    """均衡器基类：增益列表与高切/低切全部存节点 params（随链持久化），
    经 set_params 热更到运行中的 Stage。三种规格（10/31/61 段）只是
    频点栅格与匹配 Q 不同，处理路径完全同一份 EqStage 实现。"""

    NAME = "eq"
    LABEL = "均衡器"
    PARAMS = {}
    FREQS = None   # 子类指定（pvengine.components.eq 的栅格常量）
    Q = 0.0

    def __init__(self, params=None, engine_cache=None):
        from pvengine.components.eq import EqStage
        # 先建 stage 再走基类初始化——基类 set_params 会触发 _apply
        self.stage = EqStage(self.FREQS, self.Q)
        super().__init__(params)
        self._apply()

    def set_params(self, params: dict):
        """覆盖基类：EQ 参数含增益列表/布尔，不走基类的 float 通道。"""
        for k, v in (params or {}).items():
            if k == "gains":
                try:
                    self.params["gains"] = [float(g) for g in v]
                except (TypeError, ValueError):
                    pass
            elif k in ("hp_enabled", "lp_enabled"):
                self.params[k] = bool(v)
            elif k in ("hp_hz", "lp_hz"):
                try:
                    self.params[k] = float(v)
                except (TypeError, ValueError):
                    pass
        self.on_params_changed()

    def _apply(self):
        p = self.params
        self.stage.set_gains(p.get("gains") or [])
        self.stage.set_highpass(bool(p.get("hp_enabled", False)),
                                float(p.get("hp_hz", 80.0)))
        self.stage.set_lowpass(bool(p.get("lp_enabled", False)),
                               float(p.get("lp_hz", 16000.0)))

    def on_params_changed(self):
        self._apply()

    def process(self, frame, ctx):
        return self.stage.process(frame, ctx)

    def reset(self):
        self.stage.reset()


class Eq10Plugin(_EqPluginBase):
    """均衡器 10 段（1 倍频程）。"""

    NAME = "eq10"
    LABEL = "均衡器 EQ · 10 段"

    def __init__(self, params=None, engine_cache=None):
        from pvengine.components.eq import EQ10_FREQS, EQ_Q10
        self.FREQS, self.Q = EQ10_FREQS, EQ_Q10
        super().__init__(params, engine_cache)


class Eq31Plugin(_EqPluginBase):
    """均衡器 31 段（1/3 倍频程，硬件图示 EQ 通用规格）。"""

    NAME = "eq31"
    LABEL = "均衡器 EQ · 31 段"

    def __init__(self, params=None, engine_cache=None):
        from pvengine.components.eq import EQ31_FREQS, EQ_Q31
        self.FREQS, self.Q = EQ31_FREQS, EQ_Q31
        super().__init__(params, engine_cache)


class Eq61Plugin(_EqPluginBase):
    """均衡器 61 段（1/6 倍频程）。"""

    NAME = "eq61"
    LABEL = "均衡器 EQ · 61 段"

    def __init__(self, params=None, engine_cache=None):
        from pvengine.components.eq import EQ61_FREQS, EQ_Q61
        self.FREQS, self.Q = EQ61_FREQS, EQ_Q61
        super().__init__(params, engine_cache)


class CompressorPlugin(Effect):
    NAME = "compressor"
    LABEL = "压缩器"
    PARAMS = {
        "threshold_db": ("阈值 dB", -50.0, 0.0, -20.0, 1.0),
        "ratio": ("压缩比", 1.0, 12.0, 3.0, 0.5),
        "makeup_db": ("补偿 dB", 0.0, 18.0, 4.0, 1.0),
    }

    def __init__(self, params=None, stage_cache=None):
        from pvengine.components.misc import CompressorStage
        p = {"threshold_db": -20.0, "ratio": 3.0, "makeup_db": 4.0}
        p.update(params or {})
        self.stage = CompressorStage(threshold_db=float(p["threshold_db"]),
                                     ratio=float(p["ratio"]),
                                     makeup_db=float(p["makeup_db"]))
        super().__init__(params)

    def on_params_changed(self):
        self.stage.threshold_db = self.params["threshold_db"]
        self.stage.ratio = max(float(self.params["ratio"]), 1.0)
        self.stage.makeup_db = self.params["makeup_db"]

    def process(self, frame, ctx):
        return self.stage.process(frame, ctx)

    def reset(self):
        self.stage.reset()


class _AiPluginBase(Effect):
    PARAMS = {}
    _KIND = ""

    def __init__(self, params=None, stage_cache=None):
        super().__init__(params)
        cache = stage_cache or {}
        if self._KIND not in cache:
            cache[self._KIND] = _make_stage(self._KIND)
        self.stage = cache[self._KIND]

    def process(self, frame, ctx):
        return self.stage.process(frame, ctx)

    def reset(self):
        self.stage.reset()


class DenoiserPlugin(_AiPluginBase):
    """AI 智能降噪（202609 模型）。引擎经 cache 共享，重建链不重复加载。"""

    NAME = "denoiser"
    LABEL = "AI 降噪"
    _KIND = "denoise"


class DenoiserVadPlugin(_AiPluginBase):
    """AI 智能降噪 VAD（202609 VAD 变体模型）。引擎经 cache 共享，重建链不重复加载。"""

    NAME = "denoiser_vad"
    LABEL = "AI 降噪 VAD"
    _KIND = "denoise_vad"


class TsePlugin(_AiPluginBase):
    """目标说话人提取（202609 TSE）。需先加载参考音频；无参考时直通。"""

    NAME = "tse"
    LABEL = "目标说话人 TSE"
    _KIND = "tse"

    @property
    def has_reference(self):
        return self.stage.has_reference

    def set_reference(self, ref, ref_key=None):
        return self.stage.set_reference(ref, ref_key=ref_key)

# ── 工具函数 ──
def _model_file(name):
    """按模型相对路径定位：PyInstaller 资源目录 → 仓库根 → CWD。

    name 形如 "models/xxx.onnx"（见 model_config.py）；各打包形态
    （源码 / deb / rpm / AppImage / PyInstaller）均把 models/ 放在应用根，
    PyInstaller 冻结态资源在 sys._MEIPASS。全部落空时原样返回
    （保持旧行为：交给调用方按异常报告）。
    """
    import os, sys
    here = os.path.dirname(os.path.abspath(__file__))
    bases = []
    meipass = getattr(sys, "_MEIPASS", None)
    if meipass:
        bases.append(meipass)
    # 源码态仓库根 = pvengine/components 上三级；打包态为应用根附近目录
    bases.append(os.path.dirname(os.path.dirname(os.path.dirname(here))))
    bases.append(here)
    bases.append(os.path.dirname(here))
    for base in bases:
        cand = os.path.join(base, name)
        if os.path.isfile(cand):
            return cand
    return name


def _make_stage(kind):
    """按类型构建并缓存完整 AI Stage——TseStage 自带共享 STFT。
    AEC 不在此：行级 AEC 走 pvengine/aec_row.py（AecRow，一行一状态，
    会话多行共享），不进 fx 链。"""
    import model_config as _mc
    if kind == "denoise":
        from pvengine.components.denoise import DenoiseStage
        return DenoiseStage(_model_file(_mc.DENOISE_MODEL))
    if kind == "denoise_vad":
        from pvengine.components.denoise import DenoiseVadStage
        return DenoiseVadStage(_model_file(_mc.DENOISE_VAD_MODEL))
    if kind == "tse":
        from pvengine.components.tse import TseStage
        return TseStage(_model_file(_mc.TSE_MODEL))
    raise ValueError(kind)
