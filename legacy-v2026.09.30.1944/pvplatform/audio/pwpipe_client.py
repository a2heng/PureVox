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

"""Linux 音频桥（纯 Python，pipewire-pulse 兼容层）。

数据面 = 自研 ctypes 绑定（`_libpulse`，系统 libpulse 的 threaded
mainloop + pa_stream 读写回调）。历史：曾用自编 C 库 libpvpipe.so，
纯 Python 化时一度用 pulsectl 高层流 API——但那些 API（connect_recording
等）在 PyPI 全版本中不存在（来自未记录 fork），干净安装必断，故收敛为
本文件内的显式绑定。设备枚举走 `pw-dump` 标准 introspection，与传输无关。

格式统一 F32 单声道 48000Hz，重采样与声道转换由 PipeWire 完成。

时钟模型（播放正确性关键）：
- **播放**：libpulse 写回调（设备时钟）→ `out_pull[i](n)` 拉帧 → 写流。
  速率差/抖动由上层 PlaybackSink（pvengine）消化，本桥零缓冲策略；
- **录制**：libpulse 读回调 → 各输入独立环形缓冲（200ms）→ 引擎线程
  read(hop) 混合消费。

设备列表 = `pw-dump` 解析的节点名（node.name 稳定）：
  - 输入：media.class=Audio/Source 的**物理麦克风**（排除 PureVox-* 流与
    一切 purevox* 虚拟源——purevox_mic / purevox_out.monitor 是 PureVox
    自身输出，当输入会回授）
  - 输出：media.class=Audio/Sink（扬声器 + purevox_out 虚拟麦克风 sink）
"""

import json
import subprocess
import sys
import threading
import time
from typing import Callable, List, Optional

import numpy as np

IS_LINUX = sys.platform.startswith("linux")

# 桥接数据面粒度：10ms @48kHz = 480（与 pvengine.context.HOP_LENGTH 一致）。
HOP = 480

from pvplatform.audio.common import TimedFifo
from pvplatform.audio._libpulse import (
    PA_STREAM_READY, U32_MINUS1, PaBufferAttr, _Link, libpulse_available)

# 录制块 20ms / 播放 tlength 100ms、minreq 20ms（见 _libpulse 模块注释）
_REC_FRAG = HOP * 2
_PLAY_TLENGTH = HOP * 10
_PLAY_MINREQ = HOP * 2
_RING_CAP = 48000 // 5          # 输入/far 环 200ms（吸收调度抖动）


def pw_available() -> bool:
    """音频桥是否可用（Linux + 系统 libpulse 就绪）。"""
    return IS_LINUX and libpulse_available()


def _list_nodes() -> List[dict]:
    """解析 `pw-dump`（PipeWire 标准全量 introspection），返回节点列表。

    每个节点含：id / name / description / media_class / api_alsa_path / state。
    """
    nodes: List[dict] = []
    try:
        out = subprocess.run(["pw-dump"], capture_output=True, text=True, timeout=5).stdout
        objs = json.loads(out)
    except Exception:
        return nodes
    for o in objs:
        if o.get("type") != "PipeWire:Interface:Node":
            continue
        info = o.get("info", {}) or {}
        p = info.get("props", {}) or {}
        nodes.append({
            "id": str(o.get("id", "")),
            "name": p.get("node.name", ""),
            "description": p.get("node.description", ""),
            "media_class": p.get("media.class", ""),
            "api_alsa_path": p.get("api.alsa.path", ""),
            "device_bus": p.get("device.bus", ""),
            "state": info.get("state", ""),
        })
    return nodes


def list_sources() -> List[str]:
    """枚举输入节点名（PureVox 自身的麦克风选项）。

    PureVox 的输入 = 真实物理麦克风（media.class=Audio/Source）。排除：
      - PureVox 自身流（PureVox-*）
      - PureVox 虚拟麦克风（purevox_out.monitor / purevox_mic）——那是 PureVox
        降噪后的**输出**（给别人软件当麦克风），拿它当输入会形成回授
      - error 状态的死节点

    注意：**不按 api.alsa.path 排除板载卡接口**（数字麦 Mic1 与模拟麦 Mic2 是
    同一块 sof-hda 声卡的两个接口、各对应一个真实物理麦克风），宽松枚举与
    旧实现一致。
    """
    nodes = _list_nodes()
    out = []
    for n in nodes:
        if n["media_class"] != "Audio/Source":
            continue
        name = n["name"]
        if not name or name.startswith("PureVox-"):
            continue
        if name.startswith("purevox"):
            continue
        if n.get("state") == "error":
            continue
        if name not in out:
            out.append(name)
    return out


def list_destinations() -> List[str]:
    """枚举输出节点名（media.class=Audio/Sink）。

    保留扬声器与 PureVox 虚拟麦克风 sink（purevox_out）。
    """
    out = []
    for n in _list_nodes():
        if n["media_class"] != "Audio/Sink":
            continue
        name = n["name"]
        if not name or name.startswith("PureVox-"):
            continue
        if name not in out:
            out.append(name)
    return out


def node_description(name: str) -> str:
    """节点名 → node.description（无则返回节点名）。"""
    if name == "purevox_out.monitor":
        return "PureVox out"
    for n in _list_nodes():
        if n["name"] == name:
            return n["description"] or name
    return name


def source_label(name: str) -> str:
    """输入节点显示名（标记职责）。"""
    if name == "purevox_mic":
        return "PureVox mic（虚拟麦克风）"
    if name.startswith("purevox"):
        return "PureVox out"
    return "麦克风 · " + (node_description(name) or name)


def dest_label(name: str) -> str:
    """输出节点显示名（标记职责）。"""
    if name == "purevox_out":
        return "PureVox out（默认）"
    return "播放 · " + (node_description(name) or name)


def default_mic_name() -> str:
    """默认输入：第一个物理麦克风节点名。"""
    for s in list_sources():
        if "source" in s and "purevox" not in s:
            return s
    for s in list_sources():
        if not s.startswith("purevox"):
            return s
    return ""


def default_sink_name() -> str:
    """默认输出：PureVox 虚拟麦克风 sink（无则第一个输出）。"""
    for d in list_destinations():
        if d == "purevox_out":
            return d
    for d in list_destinations():
        return d
    return ""


def speaker_sink_name() -> str:
    """物理扬声器 sink 名（AEC far 兜底目标）。

    优先非 PureVox 的真实输出；只有虚拟麦克风时返回 ""（无物理扬声器，AEC 静默降级）。
    """
    dsts = list_destinations()
    for d in dsts:
        if d != "purevox_out" and not d.startswith("purevox"):
            return d
    return ""


# ---------------------------------------------------------------------------
#  流回调：libpulse 主循环线程派发（GIL 下短平快）
# ---------------------------------------------------------------------------

_PLAY_ATTR = PaBufferAttr(U32_MINUS1, _PLAY_TLENGTH, U32_MINUS1,
                          _PLAY_MINREQ, _PLAY_TLENGTH)
_REC_ATTR = PaBufferAttr(U32_MINUS1, U32_MINUS1, U32_MINUS1,
                         U32_MINUS1, _REC_FRAG)


def _make_record_reader(fifo: TimedFifo):
    """读回调：pa_stream_peek 循环 → F32 样本带采集时间戳入 TimedFifo。

    时间戳来自 libpulse 流延迟（stream_capture_ts），与 Windows
    input_buffer_adc_time 等价——AEC far/mic 按真实采集时刻配对。
    """
    from pvplatform.audio._libpulse import read_float32, stream_capture_ts

    def on_read(s, nbytes: int) -> None:
        from pvplatform.audio._libpulse import _get_funcs
        f = _get_funcs()
        from ctypes import byref, c_size_t, c_void_p
        total = 0
        while total < 64:            # 单次回调上限防御
            data = c_void_p()
            size = c_size_t()
            if f.s_peek(s, byref(data), byref(size)) < 0:
                break
            if size.value == 0:
                break
            if data.value:           # data==NULL 且 size>0 = 洞，只 drop
                fifo.write_ts(stream_capture_ts(s),
                              read_float32(data.value, size.value))
            f.s_drop(s)
            total += 1
            if size.value == 0:
                break
    return on_read


class PwBridge:
    """PureVox 纯 Python 音频桥：N 路输入采集（自动混音/逐路可读）
    + M 路输出播放 + 多路 AEC far 专用采集。

    所有流以 F32 单声道 48000Hz 协商，PipeWire 负责重采样与声道转换。
    read() 对全部输入环取平均（缺席的路跳过），read_each() 逐路返回
    （AEC 行按路取 mic 用）；输出按设备时钟写回调从 `out_pull[i]`
    拉帧（PlaybackSink），本桥不做任何缓冲策略。
    far 专用流（open_far：monitor 源或麦克风真源）直达 AEC 行，
    不进混音。
    """

    def __init__(self):
        self._link: Optional[_Link] = None
        self._in_rings: List[TimedFifo] = []
        self._in_streams: List = []
        self._out_streams: List = []
        self._out_pull: List[Callable] = []
        self._far_streams: List = []    # AEC far 专用流（与输入环同构，多路）
        self._far_rings: List[TimedFifo] = []
        self._error: str = ""
        self._lock = threading.Lock()   # open/close/open_far 与回调的簿记互斥

    @property
    def available(self) -> bool:
        return pw_available()

    # ── 连接管理 ──

    def open(self, inputs: List[str], outputs: List[str],
             out_pull: Optional[List[Callable]] = None) -> bool:
        """打开 N 路采集 + M 路播放（node.name 列表，空串项忽略）。

        out_pull[i]：输出 i 的帧供给函数（libpulse 主循环线程调用，
        参数 = 需要的样本数，返回等长 float 列表；PlaybackSink.pull）。
        """
        if not self.available:
            self._error = "系统 libpulse 不可用（pipewire-pulse 环境）"
            return False
        inputs = [s for s in (inputs or []) if s]
        outputs = [s for s in (outputs or []) if s]
        if not (inputs or outputs):
            self._error = "未指定任何输入/输出节点"
            return False
        out_pull = list(out_pull or [])
        try:
            self._link = _Link("PureVox")
        except OSError as e:
            self._error = str(e)
            self._link = None
            return False

        deadline = time.time() + 5.0
        try:
            for i, name in enumerate(inputs):
                ring = TimedFifo(48000, _RING_CAP)
                s = self._link.add_stream(
                    f"PureVox-in{i}", 48000, 1, record=True, dev=name,
                    attr=_REC_ATTR, on_read=_make_record_reader(ring))
                self._in_rings.append(ring)
                self._in_streams.append(s)
            for i, name in enumerate(outputs):
                pull = out_pull[i] if i < len(out_pull) else None
                s = self._link.add_stream(
                    "PureVox-out" if i == 0 else f"PureVox-out{i}",
                    48000, 1, record=False, dev=name, attr=_PLAY_ATTR,
                    on_write=self._make_play_writer(pull))
                self._out_streams.append(s)
            for s in (*self._in_streams, *self._out_streams):
                wait = max(0.1, deadline - time.time())
                if not self._link.wait_stream_ready(s, wait):
                    raise OSError(self._link.last_error() or "流未就绪")
        except OSError as e:
            self._error = str(e)
            self.close()
            return False
        return True

    def _make_play_writer(self, pull: Optional[Callable]):
        """写回调：按设备时钟拉帧（PlaybackSink）→ F32LE bytes → 写流。"""
        def on_write(s, nbytes: int) -> None:
            n = nbytes // 4
            if n <= 0:
                return
            if pull is not None:
                mono = pull(n)
                if mono is None or len(mono) < n:
                    mono = list(mono or []) + [0.0] * (n - len(mono))
            else:
                mono = [0.0] * n
            data = np.asarray(mono, dtype=np.float32).tobytes()
            from pvplatform.audio._libpulse import _get_funcs
            f = _get_funcs()
            f.s_write(s, data, len(data), None, 0, 0)  # PA_SEEK_RELATIVE=0
        return on_write

    def close(self) -> None:
        with self._lock:
            link = self._link
            self._link = None
            self._in_rings = []
            self._in_streams = []
            self._out_streams = []
            self._out_pull = []
            self._far_streams = []
            self._far_rings = []
        if link is not None:
            try:
                link.close()
            except Exception:
                pass

    def active(self) -> bool:
        link = self._link
        if link is None:
            return False
        with self._lock:
            streams = (*self._in_streams, *self._out_streams)
        if not streams:
            return False
        for s in streams:
            if link.stream_state(s) != PA_STREAM_READY:
                return False
        return True

    def last_error(self) -> str:
        return self._error or (self._link.last_error()
                               if self._link is not None else "") or "未知错误"

    def sample_rate(self) -> int:
        return 48000 if self.active() else 0

    def output_count(self) -> int:
        """当前播放路数（线性多出对齐判断用）。"""
        return len(self._out_streams)

    # ── 数据面 ──

    def read_each(self, n: int) -> Optional[List[Optional[List[float]]]]:
        """逐路读取输入环（与 open 时 inputs 顺序对齐；缺席的路为 None；
        无任何数据返回 None）。AEC 行按路取本路 mic 用。"""
        if self._link is None:
            return None
        with self._lock:
            rings = list(self._in_rings)
        hops = []
        for ring in rings:
            got = ring.read_ts(n)
            hops.append(got[1] if got is not None else None)
        if not any(h is not None for h in hops):
            return None
        return hops

    def read_each_ts(self, n: int):
        """同 read_each，但每路带回采时间戳 [(ts0, hop)|None]。

        时间戳来自 libpulse 流延迟（写入时的真实采集时刻），与 Windows
        input_buffer_adc_time→LinearClock 同一外部钟量纲；AEC 行按时间戳
        与 far 网格配对。"""
        if self._link is None:
            return None
        with self._lock:
            rings = list(self._in_rings)
        out = [ring.read_ts(n) for ring in rings]
        if not any(o is not None for o in out):
            return None
        return out

    def read(self, n: int) -> Optional[List[float]]:
        """读取并混合全部输入路（等权平均；无数据返回 None）。"""
        hops = self.read_each(n)
        if not hops:
            return None
        chunks = [h for h in hops if h is not None]
        if not chunks:
            return None
        if len(chunks) == 1:
            return chunks[0]
        acc = [0.0] * len(chunks[0])
        for c in chunks:
            for i, v in enumerate(c):
                acc[i] += v
        k = 1.0 / len(chunks)
        return [v * k for v in acc]

    def open_far(self, dev_name: str, monitor: bool = True) -> int:
        """开一路 AEC far 专用采集流，返回句柄（<0 表示失败）。

        monitor=True 监听 <sink>.monitor（扬声器 far），False 直录
        真源（麦克风 far）。未 open 主流时也可独立使用（自建会话）。
        far 流直达 AEC 行，不进混音。
        """
        if not self.available:
            return -1
        with self._lock:
            if self._link is None:
                try:
                    self._link = _Link("PureVox-far")
                except OSError as e:
                    self._error = str(e)
                    return -1
            src = dev_name
            if monitor and not src.endswith(".monitor"):
                src = f"{src}.monitor"
            try:
                ring = TimedFifo(48000, _RING_CAP)
                s = self._link.add_stream(
                    f"PureVox-far{len(self._far_streams)}", 48000, 1,
                    record=True, dev=src, attr=_REC_ATTR,
                    on_read=_make_record_reader(ring))
                self._far_streams.append(s)
                self._far_rings.append(ring)
                return len(self._far_streams) - 1
            except OSError as e:
                self._error = str(e)
                return -1

    def close_far(self, handle: int) -> None:
        """关闭一路 far 专用流（句柄失效后读操作返回 None）。"""
        with self._lock:
            if self._link is not None and 0 <= handle < len(self._far_streams):
                s = self._far_streams[handle]
                if s is not None:
                    self._link.drop_stream(s)
                self._far_streams[handle] = None
                self._far_rings[handle] = None

    def read_far_h(self, handle: int, n: int) -> Optional[List[float]]:
        """从指定 far 流读 n 个样本（FIFO；无数据返回 None）。"""
        got = self.read_far_ts(handle, n)
        return got[1] if got is not None else None

    def read_far_ts(self, handle: int, n: int):
        """从指定 far 流读 n 样本 → (首样本采集时刻, samples)；无数据 None。

        时间戳为写入时的真实采集时刻（libpulse 流延迟），供 AEC 行网格配对，
        与 Windows SpeakerCapture/MicCapture 的 TimedFifo 语义一致。"""
        if self._link is None:
            return None
        with self._lock:
            ring = self._far_rings[handle] \
                if 0 <= handle < len(self._far_rings) else None
        if ring is None:
            return None
        return ring.read_ts(n)

    def far_available(self, handle: int) -> int:
        """指定 far 流当前可用样本数（句柄失效返回 0）。"""
        with self._lock:
            ring = self._far_rings[handle] \
                if 0 <= handle < len(self._far_rings) else None
        return ring.available() if ring is not None else 0
