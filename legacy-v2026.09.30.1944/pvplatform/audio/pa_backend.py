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

"""Windows PortAudio 传输后端（哑传输，与 PwBridge 同形契约）。

拓扑（与 Linux 桥同一模型，取代旧的"全双工单流内联处理 + 三套回调"）：
- 输入：单路 input-only 回调流（原生采样率/声道打开 → 下混单声道 →
  pvengine.Resampler 转 48k）→ 环形缓冲（200ms，对外恒 48k）；
- 输出：每个设备一条 output-only 回调流（原生采样率打开；回调经
  OutputRateAdapter 把 48k sink 帧重采样到设备域，需逐回调精确帧数），
  回调（设备时钟）→ `out_pull[i](n)` 拉帧（PlaybackSink，跨时钟域变速消化）
  → 上混声道 → 设备。

本后端零缓冲策略、零时钟逻辑——正确性全部在 pvengine.PlaybackSink，
"一个功能只有一条规范实现路径"。输入与输出分属独立流，主输出与额外
输出地位对等（各自 sink 各自时钟域）。
"""

import math
import struct
import threading
from typing import Callable, List, Optional

from pvplatform.audio.common import TimedFifo, LinearClock, _module_log

SAMPLE_RATE = 48000
HOP_LENGTH = SAMPLE_RATE // 100       # 10ms @48kHz = 480

_RING_CAP = SAMPLE_RATE // 5          # 输入环 200ms（吸收调度抖动）


def native_hop_len(dev_sr: int) -> int:
    """设备原生 10ms hop 帧数（按时间派生：round(sr/100），守 10ms 网格）。

    Windows 输入自适应用：流按设备原生采样率打开，回调块取原生 hop，
    再经 pvengine.Resampler 转 48k。纯函数，无硬件依赖，可单测。
    """
    return max(1, int(round(max(1, int(dev_sr or SAMPLE_RATE)) / 100.0)))


def downmix_mono(interleaved, ch: int):
    """交织多声道块 → 单声道等权平均（纯搬运，无滤波）。

    ch<=1 时原样转列表返回；纯函数，可单测。
    """
    if int(ch) <= 1:
        return [float(s) for s in interleaved]
    n = len(interleaved) // int(ch)
    out = [0.0] * n
    for i in range(n):
        s = 0.0
        base = i * int(ch)
        for c in range(int(ch)):
            s += interleaved[base + c]
        out[i] = s / int(ch)
    return out


class OutputRateAdapter:
    """48k 引擎帧 → 设备原生采样率（输出回调用，需逐回调精确帧数）。

    PortAudio 输出回调必须返回恰好 frame_count 个设备域样本，而流式
    Resampler 逐块产出有 ±1 抖动：此处按需从 48k sink 拉帧、重采样后
    经小 FIFO 凑整，多余留给下次回调。ratio = 设备采样率 / 48000。
    纯逻辑（pull 可注入桩），可单测。
    """

    def __init__(self, pull_48k, ratio: float):
        from pvengine.dsp.resampler import Resampler
        self._pull = pull_48k
        self._ratio = float(ratio)
        self._rs = Resampler()
        self._rs.process([0.0] * HOP_LENGTH, self._ratio)  # 预热
        self._buf: list = []

    def get(self, need: int):
        """取 need 个设备域样本（恒定长度；sink 不足时垫零，不抛异常）。"""
        need = max(0, int(need))
        while len(self._buf) < need:
            take48 = max(HOP_LENGTH,
                         int(math.ceil((need - len(self._buf))
                                        / self._ratio)))
            try:
                chunk = self._pull(take48) or []
            except Exception:
                chunk = []
            if len(chunk) < take48:
                chunk = list(chunk) + [0.0] * (take48 - len(chunk))
            self._buf.extend(self._rs.process(chunk, self._ratio))
        out = self._buf[:need]
        del self._buf[:need]
        return out


class PaBridge:
    """PortAudio（WASAPI/MME）后端：1 路输入采集 + N 路输出播放。

    open(in_id, out_ids, out_pull)：out_pull[i] = 输出 i 的帧供给
    （PlaybackSink.pull，设备回调线程调用）。
    """

    def __init__(self):
        self._p = None                 # PyAudio 实例（open 传入则不拥有）
        self._owns_p = False
        self._in_stream = None
        self._in_ring = TimedFifo(SAMPLE_RATE, _RING_CAP)
        self._in_clock = LinearClock()
        # ── 输入自适应（Windows 本地输入任意采样率/声道 → 48k 单声道）──
        # 流按设备原生采样率/声道打开（WASAPI 共享模式只接受 MixFormat 附近
        # 参数，硬开 48k 在 44.1k 设备上报 -9997）；回调内下混单声道后经
        # pvengine.Resampler 转 48k 再入环。对外（read_each/read）恒为 48k。
        self._in_sr = SAMPLE_RATE      # 设备原生采样率（open 时按设备信息填写）
        self._in_ch = 1                # 设备原生声道数
        self._in_ratio = 1.0           # 48000 / _in_sr
        self._in_rs = None             # Resampler（需重采样时持有，否则 None=直通）
        self._in_name = ""             # 输入设备名（诊断/UI 状态行用）
        self._out_streams: List = []
        self._out_pull: List[Callable] = []
        self._out_adapters: list = []   # 与输出流下标对齐：需重采样时持有
                                        # OutputRateAdapter，否则 None=直通
        self._out_specs: list = []      # [{dev, ch, dev_sr, adaptive}]（状态行用）
        self._error: str = ""
        self._lock = threading.Lock()
        self._stopped = False

    # ── 连接管理 ──

    def open(self, in_id: Optional[int], out_ids: List[Optional[int]],
             out_pull: List[Callable], p=None) -> bool:
        """打开输入 + 多路输出。p 传入已验证的 PyAudio 实例则不接管其生命周期。

        out_ids 首路允许 None（= 系统默认输出）；其余须为有效设备索引。
        """
        import pyaudio
        self._p = p if p is not None else pyaudio.PyAudio()
        self._owns_p = p is None
        outs: List[Optional[int]] = []
        for dev in (out_ids or []):
            if dev is None:
                if not outs:
                    outs.append(None)       # 首路 None = 系统默认输出
            elif isinstance(dev, int) and dev >= 0 and dev not in outs:
                outs.append(dev)
        self._out_pull = list(out_pull or [])

        try:
            if in_id is not None:
                self._open_input(in_id)
            for i, dev in enumerate(outs):
                self._open_output(i, dev)
        except (OSError, ValueError) as e:
            self._error = str(e)
            self.close()
            return False
        if self._in_stream is None and not self._out_streams:
            self._error = "未指定任何输入/输出设备"
            self.close()
            return False
        # 缓冲/回调就绪后统一启动（回调开流即触发，避免竞态）
        try:
            if self._in_stream is not None:
                self._in_stream.start_stream()
            for s in self._out_streams:
                s.start_stream()
        except (OSError, ValueError) as e:
            self._error = str(e)
            self.close()
            return False
        return True

    def _open_input(self, dev: int) -> None:
        import pyaudio
        dev_sr, dev_ch, dev_name = SAMPLE_RATE, 1, f"#{dev}"
        try:
            info = self._p.get_device_info_by_index(dev)
            dev_sr = int(round(float(info.get('defaultSampleRate')
                                     or SAMPLE_RATE)))
            dev_ch = max(1, int(info.get('maxInputChannels') or 1))
            dev_name = str(info.get('name') or dev_name)
        except Exception:
            pass
        self._in_sr, self._in_ch = dev_sr, dev_ch
        self._in_name = dev_name
        self._in_ratio = SAMPLE_RATE / float(dev_sr) \
            if dev_sr != SAMPLE_RATE else 1.0
        self._in_rs = None
        if self._in_ratio != 1.0:
            from pvengine.dsp.resampler import Resampler
            self._in_rs = Resampler()
            # 预热：让插值历史就绪，避免首块毛刺（与回环行同做法）
            self._in_rs.process([0.0] * native_hop_len(dev_sr),
                                self._in_ratio)
        hop = native_hop_len(dev_sr)
        self._in_stream = self._p.open(
            format=pyaudio.paFloat32, channels=dev_ch,
            rate=dev_sr, input=True,
            input_device_index=dev,
            frames_per_buffer=hop,
            stream_callback=self._input_callback)
        if self._in_rs is not None:
            _module_log(f"[PaBridge] 输入设备 #{dev} "
                        f"({dev_ch}ch {dev_sr}Hz → 48kHz 自适应重采样)")
        else:
            _module_log(f"[PaBridge] 输入设备 #{dev} (mono 48kHz)")

    def _open_output(self, idx: int, dev: Optional[int]) -> None:
        import pyaudio
        ch, dev_sr = 2, SAMPLE_RATE
        if dev is not None:
            try:
                info = self._p.get_device_info_by_index(dev)
                ch = max(1, int(info.get('maxOutputChannels', 2)))
                dev_sr = int(round(float(info.get('defaultSampleRate')
                                           or SAMPLE_RATE)))
            except Exception:
                ch, dev_sr = 2, SAMPLE_RATE
        ratio = dev_sr / float(SAMPLE_RATE) \
            if dev_sr != SAMPLE_RATE else 1.0
        hop = native_hop_len(dev_sr)
        pull = self._out_pull[idx] if idx < len(self._out_pull) else None
        adapter = OutputRateAdapter(pull, ratio) \
            if (ratio != 1.0 and pull is not None) else None
        # adapters 与 out_streams 下标对齐（close 时同步清空）
        while len(self._out_adapters) <= idx:
            self._out_adapters.append(None)
        self._out_adapters[idx] = adapter
        s = self._p.open(
            format=pyaudio.paFloat32, channels=ch,
            rate=dev_sr, output=True,
            output_device_index=dev,
            frames_per_buffer=hop,
            stream_callback=self._make_output_callback(idx, ch))
        self._out_streams.append(s)
        self._out_specs.append({"dev": dev, "ch": ch, "dev_sr": dev_sr,
                                "adaptive": adapter is not None})
        if adapter is not None:
            _module_log(f"[PaBridge] 输出设备 "
                        f"#{dev if dev is not None else '(系统默认)'} "
                        f"({ch}ch 48kHz → {dev_sr}Hz 自适应重采样)")
        else:
            _module_log(f"[PaBridge] 输出设备 "
                        f"#{dev if dev is not None else '(系统默认)'} ({ch}ch)")

    def close(self) -> None:
        self._stopped = True
        streams = []
        with self._lock:
            if self._in_stream is not None:
                streams.append(self._in_stream)
            streams.extend(self._out_streams)
            self._in_stream = None
            self._out_streams = []
            self._out_pull = []
            self._out_adapters = []
            self._out_specs = []
        for s in streams:
            try:
                s.stop_stream()
            except Exception:
                pass
            try:
                s.close()
            except Exception:
                pass
        if self._p is not None and self._owns_p:
            try:
                self._p.terminate()
            except Exception:
                pass
        self._p = None

    def active(self) -> bool:
        with self._lock:
            streams = ([self._in_stream] if self._in_stream is not None else []) \
                + list(self._out_streams)
        if not streams:
            return False
        for s in streams:
            try:
                if not s.is_active():
                    return False
            except OSError:
                return False
        return True

    def last_error(self) -> str:
        return self._error or "未知错误"

    def sample_rate(self) -> int:
        return SAMPLE_RATE if self.active() else 0

    def input_info(self) -> dict:
        """输入端实际状态（UI 状态行用）：原生采样率/声道 + 是否重采样。

        未建流时返回直通默认值，不抛异常。
        """
        return {"name": self._in_name, "dev_sr": self._in_sr,
                "dev_ch": self._in_ch, "ratio": self._in_ratio,
                "adaptive": self._in_rs is not None,
                "active": self._in_stream is not None}

    def output_count(self) -> int:
        return len(self._out_streams)

    def output_info(self) -> list:
        """各输出端实际状态（UI 状态行用）：[{dev, ch, dev_sr, adaptive}]。

        未建流返回 []，不抛异常。
        """
        try:
            return [dict(s) for s in self._out_specs]
        except Exception:
            return []

    # ── 数据面 ──

    def read_each_ts(self, n: int):
        """逐路读取输入环并带回采时间戳（单路恒一路；无数据返回 None）。

        返回 [ (首样本主时钟秒, samples) | None ]，与 PwBridge 同形；
        AEC 行按时间戳与 far 配对。"""
        got = self._in_ring.read_ts(n)
        return [got] if got is not None else None

    def read_each(self, n: int) -> Optional[List[Optional[List[float]]]]:
        """逐路读取输入环（单路后端恒返回一路；无数据返回 None）。"""
        got = self._in_ring.read_ts(n)
        return [got[1]] if got is not None else None

    def read(self, n: int) -> Optional[List[float]]:
        """读取输入（单路，等权混合退化为直读；无数据返回 None）。"""
        got = self._in_ring.read_ts(n)
        return got[1] if got is not None else None

    # ── 回调（PortAudio 设备线程）──

    def _input_callback(self, in_data, frame_count, time_info, status):
        if self._stopped:
            return (None, pyaudio_paComplete())
        try:
            ch = max(1, int(self._in_ch))
            raw = struct.unpack(f'{frame_count * ch}f', in_data)
            mono = downmix_mono(raw, ch)
            if self._in_rs is not None:
                mono = self._in_rs.process(mono, self._in_ratio)
                if not mono:
                    return (None, pyaudio_paContinue())
            adc = time_info.get('input_buffer_adc_time')
            now = __import__('time').perf_counter()
            if adc is not None:
                self._in_clock.add(adc, now)
                ts0 = self._in_clock.map(adc)
            else:
                ts0 = now
            self._in_ring.write_ts(ts0, mono)
        except Exception as e:
            _module_log(f"[PaBridge] 输入回调异常: {e}")
        return (None, pyaudio_paContinue())

    def _make_output_callback(self, idx: int, ch: int):
        pull = self._out_pull[idx] if idx < len(self._out_pull) else None

        def callback(in_data, frame_count, time_info, status):
            if self._stopped:
                return (None, pyaudio_paComplete())
            try:
                adapter = self._out_adapters[idx] \
                    if idx < len(self._out_adapters) else None
                if adapter is not None:
                    mono = adapter.get(frame_count)
                else:
                    mono = pull(frame_count) if pull is not None else None
                    if mono is None or len(mono) < frame_count:
                        mono = list(mono or []) + \
                            [0.0] * (frame_count - len(mono or []))
                    else:
                        mono = list(mono[:frame_count])
                if ch > 1:
                    out = [0.0] * (frame_count * ch)
                    pos = 0
                    for v in mono:
                        for _c in range(ch):
                            out[pos] = v
                            pos += 1
                else:
                    out = mono
                return (struct.pack(f'{len(out)}f', *out),
                        pyaudio_paContinue())
            except Exception as e:
                _module_log(f"[PaBridge] 输出回调异常: {e}")
                return (struct.pack(f'{frame_count * ch}f',
                                    *([0.0] * frame_count * ch)),
                        pyaudio_paContinue())
        return callback


def pyaudio_paContinue():
    import pyaudio
    return pyaudio.paContinue


def pyaudio_paComplete():
    import pyaudio
    return pyaudio.paComplete
