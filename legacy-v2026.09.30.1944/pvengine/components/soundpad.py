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

"""音效板插件——链上注入式媒体输入（Soundpad 类功能）。

process(frame) 在自身链位置把当前播放中的音效帧与信号相加：
- 挂链尾（默认追加位置）= 后级直通：音效不经降噪/变声，随全部输出扇出；
- 用户可拖到降噪之前参与处理（位置语义与可视化/输出抽头一致）。

音源为库内 48k/mono WAV（添加时经 audio_decode.import_media 归一入库，
见 ~/.purevox/soundpad/），整段解码进内存（音效都是短音效，起播零延迟）；
懒加载：首次 play 才读文件，音效列表变更即清理失效缓存。控制面（UI/热键
线程）与音频面（处理线程）经锁分离。

重播语义：再次按下同一音效 = 旧声部淡出、新声部从**开头**起播（连按即
重新触发，不叠加多条并行声部）。

音量是**每个音效一个**的 `volume_db`（±10dB，文件本身已够响）：起播时取其
音量，滑杆拖动经 set_pads 同步更新正在播的声部（实时生效）。

起播/结束/手动停止/音效移除/复选框关断一律 hop 内线性淡变（1→0），
杜绝硬切咔哒；结束段（剩余 ≤1 hop）天然淡出收尾。
"""

import threading

import numpy as np

from pvengine.components.audio_decode import decode_to_mono_48k as _decode
from pvengine.components.effect_base import Effect


class _Voice:
    """单次播放状态（data 只读共享；pos 仅音频线程推进）。

    fade_at：淡出段起点（样本索引）；置位后该段以线性 1→0 包络混出后结束。
    """

    __slots__ = ("data", "pos", "vol", "ending", "fade_at")

    def __init__(self, data, vol):
        self.data = data
        self.pos = 0
        self.vol = vol
        self.ending = False
        self.fade_at = None


class SoundPadPlugin(Effect):
    NAME = "soundpad"
    LABEL = "音效板"
    # 复选框关断不旁路：正在播的音效自行淡出（衔接无缝）
    FADE_THROUGH = True
    # 音量是「每个音效一个」（音效 dict 里的 volume_db，±10dB），非节点级滑杆
    PARAMS = {}

    def __init__(self, params=None, stage_cache=None):
        self._lock = threading.Lock()
        self._voices = {}   # pad_index -> _Voice
        self._fading = []   # 结束/被打断的 voice（淡出后丢弃）
        self._cache = {}    # path -> np.ndarray | None
        self._pads = []
        self.enabled = True
        super().__init__(params)
        self.set_pads((params or {}).get("pads") or [])

    @staticmethod
    def _pad_gain(pad) -> float:
        try:
            db = float(pad.get("volume_db", 0.0))
        except (TypeError, ValueError):
            db = 0.0
        return float(10.0 ** (max(-10.0, min(10.0, db)) / 20.0))

    # ── 控制面（UI / 热键线程调用）──
    def set_pads(self, pads):
        with self._lock:
            self._pads = [dict(p) for p in (pads or [])]
            keep = {p.get("path") for p in self._pads}
            self._cache = {k: v for k, v in self._cache.items() if k in keep}
            # 正在播的音效跟随各自音量实时更新（拖动滑杆即时生效）
            for i, v in self._voices.items():
                if i < len(self._pads):
                    v.vol = self._pad_gain(self._pads[i])
            for i in [i for i in self._voices if i >= len(self._pads)]:
                v = self._voices.pop(i)
                v.ending = True
                self._fading.append(v)

    def play(self, index):
        with self._lock:
            if not (0 <= index < len(self._pads)):
                return
            path = self._pads[index].get("path") or ""
            if path not in self._cache:
                self._cache[path] = _decode(path)
            data = self._cache.get(path)
            if data is None or not len(data):
                return
            old = self._voices.pop(index, None)
            if old is not None:
                old.ending = True
                self._fading.append(old)
            self._voices[index] = _Voice(
                data, self._pad_gain(self._pads[index]))

    def stop(self, index):
        with self._lock:
            v = self._voices.pop(index, None)
            if v is not None:
                v.ending = True
                self._fading.append(v)

    def stop_all(self):
        with self._lock:
            for v in self._voices.values():
                v.ending = True
                self._fading.append(v)
            self._voices.clear()

    def on_struct_param(self, key, value):
        if key == "pads":
            self.set_pads(value)

    # ── 音频面（处理线程）──
    def process(self, frame, ctx):
        with self._lock:
            if not self.enabled:
                for v in self._voices.values():
                    v.ending = True
            voices = list(self._voices.items())
            fading = list(self._fading)
        if not (voices or fading):
            return frame
        out = frame.astype(np.float32, copy=True)
        n = len(out)
        finished = []
        for key, v in voices:
            if self._mix_voice(out, v, n):
                finished.append((key, v))
        still = []
        for v in fading:
            if not self._mix_voice(out, v, n):
                still.append(v)
        if finished or len(still) != len(fading):
            with self._lock:
                for key, v in finished:
                    # 仍是同一实例（未被新 play 替换）才移除
                    if self._voices.get(key) is v:
                        self._voices.pop(key, None)
                self._fading = still
        np.clip(out, -1.0, 1.0, out=out)
        return out

    @staticmethod
    def _mix_voice(out, v, n):
        """混一 hop；起始淡入（0→1）与结束淡出（1→0）均在 hop 内线性完成。

        返回 True = 本 voice 已播完（数据耗尽或淡出段结束，调用方移除）。
        """
        remaining = len(v.data) - v.pos
        if remaining <= 0:
            return True
        take = min(n, remaining)
        if v.fade_at is None and (v.ending or remaining <= n):
            v.fade_at = v.pos              # 结束段：本 hop 内 1→0
        seg = v.data[v.pos:v.pos + take]
        if v.fade_at is not None and v.pos == v.fade_at:
            w = np.linspace(1.0, 0.0, take, dtype=np.float32)
        elif v.pos == 0:
            w = np.linspace(0.0, 1.0, take, dtype=np.float32)
        else:
            w = None
        if w is None:
            out[:take] += seg * np.float32(v.vol)
        else:
            out[:take] += seg * (np.float32(v.vol) * w)
        v.pos += take
        return v.pos >= len(v.data) or \
            (v.fade_at is not None and v.pos > v.fade_at)

    def reset(self):
        self.stop_all()
