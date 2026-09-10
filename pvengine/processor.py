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

"""AudioProcessor——全链驱动门面。

整条管线 = 用户插件序列（pvengine.plugins 注册表），不再有固定模式。
基础设施（可视化抽头 / 录制抽头 / 末端限幅）固定挂在首尾，不进插件列表。
EQ 增益/高低切走插件 params（update_plugin_param 热更）；频谱预览走
process_eq_only 的独立状态副本。
"""

import numpy as np

from pvengine.context import FrameContext, HOP_LENGTH, SAMPLE_RATE
from pvengine.pipeline import Pipeline
from pvengine.plugins import create_plugin, DEFAULT_CHAIN
from pvengine.components.misc import (BufferTapStage, ClipStage,
                                      OutputTapStage, RecorderTapStage)

_VIZ_CAP = 1 << 16


class _EffectAdapter:
    """插件 Effect → Stage 协议适配（Pipeline 只认 accepts/process/reset）。"""

    name = "fx"

    def __init__(self, eff, enabled=True):
        self.eff = eff
        self.enabled = enabled
        # 媒体源类插件（音乐/音效板/桌面声音）声明 FADE_THROUGH：
        # 复选框关断不旁路，插件在 process 内自行淡出（衔接无缝）
        self.fade_through = bool(getattr(eff, "FADE_THROUGH", False))

    def accepts(self, ctx) -> bool:
        return self.enabled or self.fade_through

    def process(self, frame, ctx):
        return self.eff.process(frame.astype(np.float32, copy=False), ctx)

    def reset(self):
        self.eff.reset()

    @property
    def params(self):
        return getattr(self.eff, "params", {})


class AudioProcessor:
    """插件链音频处理器。链配置：[{"type","enabled","params"}, ...]，
    顺序即信号流顺序；AI 插件模型引擎跨重建共享（不重复加载）。"""

    def __init__(self, pre_gain_db: float = 0.0):
        self._stage_cache = {}      # AI Stage 缓存（denoise/tse；AEC 行级共享另走）
        self._viz_in = BufferTapStage()
        self._viz_out = BufferTapStage()
        self._viz_taps = []   # 位置抽头（链内 viz 节点），见 set_plugins
        self._out_taps = []   # 输出位置抽头（链内 output 节点），按链序
        self._viz_in.enabled = False
        self._viz_out.enabled = False
        self._recorder = RecorderTapStage()
        self._clip = ClipStage()
        self._far_sr = SAMPLE_RATE
        # 频谱预览专用 EQ（独立 IIR 状态，懒建）——绝不能用链内 eq 实例：
        # 两路不同信号轮流推进同一份 zi 会在每个帧边界产生不连续（可闻杂音）
        self._eq_preview = None

        self._typed = []            # [(type_str, obj)]，obj 为 Stage 或 EffectAdapter
        self._entries = []          # [(type, stage|None, params, enabled)] 与 UI 行 1:1
        self.pipeline = Pipeline([])
        self.plugin_errors: list[str] = []

        cfg = [dict(e) for e in DEFAULT_CHAIN]
        if pre_gain_db:
            for e in cfg:
                if e["type"] == "gain":
                    e["params"] = {"gain_db": float(pre_gain_db)}
        self.set_plugins(cfg)

    # ── 链构建 ──
    def set_plugins(self, chain_cfg):
        """整体替换插件链。单项失败（如模型缺失）跳过并记入 plugin_errors。

        系统节点（输入/输出）不入管线，但保留占位——
        _entries 与 UI 节点行索引 1:1 对齐，行级操作按此路由。
        viz 节点是**位置抽头**：在链中它所在的精确位置插入
        BufferTapStage（线性语义——抽到的是"到此为止"的信号），
        按 UI 行序编号，经 take_viz_tap(ordinal) 读取。
        """
        stages = []
        typed = []
        entries = []
        self._viz_taps = []          # [(ptype, BufferTapStage)] 按链序
        self._out_taps: list[OutputTapStage] = []   # 输出位置抽头，按链序
        self.plugin_errors = []

        def _spec_kind(ptype):
            try:
                from pvengine.plugins import get_spec
                sp = get_spec(ptype)
                return sp.kind if sp else None
            except Exception:
                return None

        for item in (chain_cfg or []):
            ptype = str(item.get("type", ""))
            enabled = bool(item.get("enabled", True))
            params = item.get("params") or {}
            if _spec_kind(ptype) == "viz":
                if enabled:
                    tap = BufferTapStage(48000 * 5)
                    self._viz_taps.append((ptype, tap))
                    stages.append(tap)
                    typed.append((ptype, tap))
                    entries.append((ptype, tap, dict(params), enabled))
                else:
                    entries.append((ptype, None, dict(params), enabled))
                continue
            if _spec_kind(ptype) == "output":
                if enabled:
                    tap = OutputTapStage()
                    self._out_taps.append(tap)
                    stages.append(tap)
                    typed.append((ptype, tap))
                    entries.append((ptype, tap, dict(params), enabled))
                else:
                    entries.append((ptype, None, dict(params), enabled))
                continue
            try:
                obj = create_plugin(ptype, params, self._stage_cache)
            except Exception as e:
                self.plugin_errors.append(f"{ptype}: {e}")
                obj = None
            if obj is None:
                entries.append((ptype, None, dict(params), enabled))
                continue
            obj.enabled = enabled
            if hasattr(obj, "accepts"):
                stage = obj
            else:
                stage = _EffectAdapter(obj, enabled)
            stages.append(stage)
            typed.append((ptype, stage))
            entries.append((ptype, stage, dict(params), enabled))
        self._entries = entries
        self._typed = typed
        self.pipeline = Pipeline(
            [self._viz_in] + stages + [self._recorder, self._clip, self._viz_out])

    def take_viz_tap(self, ordinal: int, cap: int = 4096):
        """取第 ordinal 个 viz 位置抽头的新样本（UI 线程定期调用即排空）。"""
        try:
            return self._viz_taps[ordinal][1].take(cap)
        except Exception:
            return []

    def take_output_frames(self) -> list:
        """取全部输出位置抽头的最新帧（按链序；空列表=无输出节点）。

        音频线程每处理一帧后调用并逐路写入播放流——线性多出：
        每路输出拿到自己链位置上的信号。
        """
        return [t.take_latest() for t in self._out_taps]

    def _entries_cfg(self) -> list:
        """全量链配置（含系统节点占位），顺序与 UI 行一致。"""
        return [{"type": t, "enabled": en, "params": dict(p)}
                for t, _s, p, en in self._entries]

    def get_plugins(self) -> list:
        out = []
        for ptype, st in self._typed:
            obj = getattr(st, "eff", st)
            params = dict(getattr(obj, "params", {}) or {})
            out.append({"type": ptype, "enabled": bool(st.enabled), "params": params})
        return out

    def get_live_agc_gain(self, index: int) -> float | None:
        """取 AGC 节点当前实时增益（dB）。index 为 UI 行索引。
        返回 None 表示该索引不是 AGC 或插件未就绪。"""
        if not (0 <= index < len(self._entries)):
            return None
        t, st, _p, _en = self._entries[index]
        if t != "agc" or st is None:
            return None
        obj = getattr(st, "eff", st)
        agc = getattr(obj, "agc", None)
        if agc is None:
            return None
        try:
            return float(agc.gain_db)
        except Exception:
            return None

    def update_plugin_param(self, index: int, key: str, value):
        if 0 <= index < len(self._entries):
            t, st, p, _en = self._entries[index]
            p[key] = value
            if st is not None:
                obj = getattr(st, "eff", st)
                if hasattr(obj, "set_params"):
                    obj.set_params({key: value})
                # 非滑杆的结构化参数（pads/path/loop 等）经插件自定义钩子
                if hasattr(obj, "on_struct_param"):
                    obj.on_struct_param(key, value)

    def _first_soundpad(self):
        for t, st, _p, en in self._entries:
            if t == "soundpad" and st is not None and st.enabled:
                return getattr(st, "eff", st)
        return None

    def soundpad_play(self, index: int):
        """播放音效板第 index 个垫子（无音效板节点时静默）。"""
        sp = self._first_soundpad()
        if sp is not None:
            sp.play(index)

    def soundpad_stop(self, index: int):
        sp = self._first_soundpad()
        if sp is not None:
            sp.stop(index)

    def soundpad_stop_all(self):
        sp = self._first_soundpad()
        if sp is not None:
            sp.stop_all()

    def music_status(self, index: int) -> dict:
        """音乐播放器状态（playing/pos秒/dur秒；非该节点返回默认）。"""
        if 0 <= index < len(self._entries):
            t, st, _p, en = self._entries[index]
            if t == "music_player" and st is not None:
                obj = getattr(st, "eff", st)
                if hasattr(obj, "status"):
                    return obj.status()
        return {"playing": False, "pos": 0.0, "dur": 0.0}

    def media_read(self, n: int) -> list:
        """纯媒体会话的帧源：静音帧过全链（媒体节点在链位置注入）。

        与主线同一条 process_pipeline：viz 位置抽头（VU/频谱）、输出抽头、
        fx 与录制全部生效——可视化随播放出图，链语义与有麦克风时一致。
        由 pvplatform MediaSession 的播放库回调生成器调用（设备时钟定节拍）。
        """
        return self.process_pipeline([0.0] * n)

    def set_plugin_enabled(self, index: int, enabled: bool):
        if 0 <= index < len(self._entries):
            t, st, p, _en = self._entries[index]
            self._entries[index] = (t, st, p, bool(enabled))
            if st is not None:
                st.enabled = bool(enabled)
                # 同步镜像到 Effect（FADE_THROUGH 插件据此自行淡出/淡入）
                eff = getattr(st, "eff", None)
                if eff is not None:
                    eff.enabled = bool(enabled)

    def move_plugin(self, index: int, direction: int) -> bool:
        j = index + direction
        if not (0 <= index < len(self._entries) and 0 <= j < len(self._entries)):
            return False
        self._entries[index], self._entries[j] = self._entries[j], self._entries[index]
        self.set_plugins(self._entries_cfg())
        return True

    def add_plugin(self, ptype: str, params: dict | None = None) -> bool:
        cfg = self._entries_cfg() + [{"type": ptype, "enabled": True,
                                      "params": params or {}}]
        before = len(self.plugin_errors)
        self.set_plugins(cfg)
        return len(self.plugin_errors) == before

    def remove_plugin(self, index: int):
        cfg = self._entries_cfg()
        if 0 <= index < len(cfg):
            del cfg[index]
        self.set_plugins(cfg)

    def _find(self, ptype: str):
        """返回当前链中该类型的活动实例（未启用也返回，供开关切换）。"""
        for t, st in self._typed:
            if t == ptype:
                return getattr(st, "eff", st)
        return None

    def tse_needs_reference(self) -> bool:
        return any(t == "tse" and st.enabled and
                   not getattr(getattr(st, "eff", st), "has_reference", True)
                   for t, st in self._typed)

    # ── 主处理 ──
    def _run_chain(self, mic: np.ndarray) -> np.ndarray:
        ctx = FrameContext()
        return self.pipeline.process(mic, ctx)

    def process(self, mic):
        if len(mic) != HOP_LENGTH:
            raise RuntimeError(f"Input audio chunk length must be equal to hop length ({HOP_LENGTH})")
        out = self._run_chain(np.asarray(mic, dtype=np.float32))
        return out.tolist()

    # ── 流式 pipeline（网络模式）──
    def process_pipeline(self, raw_input):
        if not len(raw_input):
            return []
        acc = np.asarray(raw_input, dtype=np.float32).reshape(-1)
        out_acc: list[float] = []
        self._viz_in.enabled = True
        self._viz_out.enabled = True
        try:
            while len(acc) >= HOP_LENGTH:
                chunk, acc = acc[:HOP_LENGTH], acc[HOP_LENGTH:]
                out_acc.extend(self._run_chain(chunk).tolist())
            if len(acc) >= HOP_LENGTH * 3 // 4:
                orig = len(acc)
                chunk = np.zeros(HOP_LENGTH, dtype=np.float32)
                chunk[:orig] = acc
                out_acc.extend(self._run_chain(chunk).tolist())
        finally:
            self._viz_in.enabled = False
            self._viz_out.enabled = False
        return out_acc

    # ── 可视化抽头 ──
    def get_and_clear_viz_input(self):
        return self._viz_in.take()

    def get_and_clear_viz_output(self):
        return self._viz_out.take()

    def process_eq_only(self, in_samples):
        """前置预览（频谱输入侧）：gain + eq 的**独立状态副本**。

        增益读链内 gain 插件的实时参数；EQ 系数经 mirror() 从链内
        eq 插件拷贝到私有预览实例——滤波状态（zi）独立连续，
        与主链互不干扰（共享实例会让主链每帧出现状态跳变 → 杂音）。
        """
        x = np.asarray(in_samples, dtype=np.float32).reshape(-1).copy()
        g = self._find("gain")
        if g is not None and getattr(g, "enabled", True):
            db = float((getattr(g, "params", {}) or {}).get("gain_db", 0.0))
            x = x * np.float32(10.0 ** (db / 20.0))
        eq = self._find("eq10") or self._find("eq31") or self._find("eq61")
        if eq is not None and getattr(eq, "enabled", True):
            st = getattr(getattr(eq, "eff", eq), "stage", None)
            if st is not None:
                if self._eq_preview is None:
                    from pvengine.components.eq import EqStage
                    self._eq_preview = EqStage()
                self._eq_preview.mirror(st)
                x = self._eq_preview.process(x, FrameContext()).astype(np.float32)
        return x.tolist()

    # ── TSE ──
    def set_tse_reference(self, ref, ref_key=None):
        for t, st in self._typed:
            if t == "tse":
                obj = getattr(st, "eff", st)
                if ref:
                    obj.set_reference(np.asarray(ref, dtype=np.float32),
                                      ref_key=ref_key)

    def is_tse_reference_loaded(self):
        return any(t == "tse" and getattr(getattr(s, "eff", s), "has_reference", False)
                   for t, s in self._typed)

    def set_tse_enabled(self, enabled: bool):
        for t, st in self._typed:
            if t == "tse":
                st.enabled = bool(enabled)

    def get_tse_recording_audio(self):
        return list(self._recorder.frame)

    def set_recording_enabled(self, enabled: bool):
        self._recorder.recording_enabled = bool(enabled)

    def is_recording_enabled(self):
        return self._recorder.recording_enabled

    # ── 生命周期 ──
    def cleanup(self):
        for st in self._stage_cache.values():
            try:
                st.release()
            except Exception:
                pass
        self._stage_cache.clear()
