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

"""系统 libpulse 的最小 ctypes 绑定（纯 Python，无自编译二进制）。

为什么自建而不用 pulsectl：流式数据面（pa_stream 读写回调）在 pulsectl
的全部 PyPI 版本中都不存在——旧桥调用的 connect_recording/connect_playback
来自一个未记录的 fork，干净环境（CI 打包的 deb/rpm/AppImage）必现
AttributeError。本绑定只含数据面所需的最小集合，依赖面显式化。

模型：pa_threaded_mainloop（libpulse 官方 C 线程主循环）承载 context 与
全部流；读/写回调由主循环线程派发（GIL 下执行 Python，短平快）。
主线程调用任何 pa_* API 必须持主循环锁（lock/unlock 包裹）。

延迟策略（F32 单声道 48kHz）：
- 播放：tlength=100ms、minreq=20ms（写回调节拍），prebuf 默认（启动
  预缓冲防欠载，配合上层 PlaybackSink 预热期静音喂流，水位恒定）；
- 录制：fragsize=20ms（读回调节拍），上层环形缓冲吸收抖动。
"""

import ctypes
import ctypes.util
import threading
import time
from ctypes import (CFUNCTYPE, POINTER, byref, c_char_p, c_int, c_int64,
                    c_size_t, c_uint8, c_uint32, c_uint64, c_void_p, string_at)

# ── libpulse 常量（pulse/def.h · enums.h · sample.h）──
# pa_sample_format_t：U8=0 ALAW=1 ULAW=2 S16LE=3 S16BE=4 FLOAT32LE=5
# FLOAT32BE=6 S32LE=7 S32BE=8 —— 必须用 5，3 是 S16LE（会按 float32 解读成
# denormal/NaN，表现为 VU 非空即满、无声音）。
PA_SAMPLE_FLOAT32LE = 5
PA_CONTEXT_UNCONNECTED = 0
PA_CONTEXT_CONNECTING = 1
PA_CONTEXT_AUTHORIZING = 2
PA_CONTEXT_SETTING_NAME = 3
PA_CONTEXT_READY = 4
PA_CONTEXT_FAILED = 5
PA_CONTEXT_TERMINATED = 6
PA_STREAM_UNCONNECTED = 0
PA_STREAM_CREATING = 1
PA_STREAM_READY = 2
PA_STREAM_FAILED = 3
PA_STREAM_TERMINATED = 4
PA_STREAM_NOFLAGS = 0
PA_SEEK_RELATIVE = 0
# PA_BUFFER_ATTR 的 -1（u32）
U32_MINUS1 = 0xFFFFFFFF

_FLOAT32 = '<f4'


class PaSampleSpec(ctypes.Structure):
    _fields_ = [("format", c_int), ("rate", c_uint32), ("channels", c_uint8)]


class PaBufferAttr(ctypes.Structure):
    _fields_ = [("maxlength", c_uint32), ("tlength", c_uint32),
                ("prebuf", c_uint32), ("minreq", c_uint32),
                ("fragsize", c_uint32)]


# 回调原型
PA_STATE_CB_T = CFUNCTYPE(None, c_void_p, c_void_p)             # (obj, userdata)
PA_REQUEST_CB_T = CFUNCTYPE(None, c_void_p, c_size_t, c_void_p)  # (stream, nbytes, userdata)


def _bind(lib, name, restype, argtypes):
    fn = getattr(lib, name)
    fn.restype = restype
    fn.argtypes = argtypes
    return fn


class LibPulseFuncs:
    """单个 libpulse 实例的函数绑定（进程内共享一份）。"""

    def __init__(self):
        path = ctypes.util.find_library("libpulse") or "libpulse.so.0"
        self.lib = ctypes.CDLL(path)

        L = self.lib
        # threaded mainloop
        self.ml_new = _bind(L, "pa_threaded_mainloop_new", c_void_p, [])
        self.ml_start = _bind(L, "pa_threaded_mainloop_start", c_int, [c_void_p])
        self.ml_stop = _bind(L, "pa_threaded_mainloop_stop", None, [c_void_p])
        self.ml_free = _bind(L, "pa_threaded_mainloop_free", None, [c_void_p])
        self.ml_lock = _bind(L, "pa_threaded_mainloop_lock", None, [c_void_p])
        self.ml_unlock = _bind(L, "pa_threaded_mainloop_unlock", None, [c_void_p])
        self.ml_get_api = _bind(L, "pa_threaded_mainloop_get_api", c_void_p,
                                [c_void_p])
        # proplist
        self.pl_new = _bind(L, "pa_proplist_new", c_void_p, [])
        self.pl_sets = _bind(L, "pa_proplist_sets", c_int,
                             [c_void_p, c_char_p, c_char_p])
        self.pl_free = _bind(L, "pa_proplist_free", None, [c_void_p])
        # context
        self.ctx_new = _bind(L, "pa_context_new", c_void_p,
                             [c_void_p, c_char_p])
        self.ctx_state_cb = _bind(L, "pa_context_set_state_callback", None,
                                  [c_void_p, PA_STATE_CB_T, c_void_p])
        self.ctx_connect = _bind(L, "pa_context_connect", c_int,
                                 [c_void_p, c_char_p, c_int, c_void_p])
        self.ctx_get_state = _bind(L, "pa_context_get_state", c_int, [c_void_p])
        self.ctx_errno = _bind(L, "pa_context_errno", c_int, [c_void_p])
        self.ctx_disconnect = _bind(L, "pa_context_disconnect", None, [c_void_p])
        self.ctx_unref = _bind(L, "pa_context_unref", None, [c_void_p])
        # stream
        self.s_new = _bind(L, "pa_stream_new", c_void_p,
                           [c_void_p, c_char_p,
                            POINTER(PaSampleSpec), c_void_p])
        self.s_state_cb = _bind(L, "pa_stream_set_state_callback", None,
                                [c_void_p, PA_STATE_CB_T, c_void_p])
        self.s_read_cb = _bind(L, "pa_stream_set_read_callback", None,
                               [c_void_p, PA_REQUEST_CB_T, c_void_p])
        self.s_write_cb = _bind(L, "pa_stream_set_write_callback", None,
                                [c_void_p, PA_REQUEST_CB_T, c_void_p])
        self.s_connect_record = _bind(L, "pa_stream_connect_record", c_int,
                                      [c_void_p, c_char_p,
                                       POINTER(PaBufferAttr), c_int])
        self.s_connect_playback = _bind(L, "pa_stream_connect_playback", c_int,
                                        [c_void_p, c_char_p,
                                         POINTER(PaBufferAttr), c_int,
                                         c_void_p, c_void_p])
        self.s_get_state = _bind(L, "pa_stream_get_state", c_int, [c_void_p])
        self.s_get_time = _bind(L, "pa_stream_get_time", c_int,
                                [c_void_p, POINTER(c_uint64)])
        self.s_get_latency = _bind(L, "pa_stream_get_latency", c_int,
                                   [c_void_p, POINTER(c_uint64),
                                    POINTER(c_int)])
        self.s_writable = _bind(L, "pa_stream_writable_size", c_size_t,
                                [c_void_p])
        self.s_write = _bind(L, "pa_stream_write", c_int,
                             [c_void_p, c_void_p, c_size_t, c_void_p,
                              c_int64, c_int])
        self.s_peek = _bind(L, "pa_stream_peek", c_int,
                            [c_void_p, POINTER(c_void_p), POINTER(c_size_t)])
        self.s_drop = _bind(L, "pa_stream_drop", c_int, [c_void_p])
        self.s_disconnect = _bind(L, "pa_stream_disconnect", c_int, [c_void_p])
        self.s_unref = _bind(L, "pa_stream_unref", None, [c_void_p])
        self.strerror = _bind(L, "pa_strerror", c_char_p, [c_int])


# 进程级共享绑定（libpulse 的 C 函数无实例状态，可安全共用）
_funcs: LibPulseFuncs = None
_funcs_lock = threading.Lock()


def _get_funcs() -> LibPulseFuncs:
    global _funcs
    with _funcs_lock:
        if _funcs is None:
            _funcs = LibPulseFuncs()
        return _funcs


def libpulse_available() -> bool:
    """系统 libpulse 是否可加载（pipewire-pulse 环境恒有）。"""
    try:
        _get_funcs()
        return True
    except Exception:
        return False


class _StreamHandler:
    """单条流的 Python 侧回调挂载点。回调按流指针从 _Link 查到本对象。"""
    __slots__ = ("on_read", "on_write", "on_state", "cbs")

    def __init__(self):
        self.on_read = None    # f(nbytes)
        self.on_write = None   # f(nbytes)
        self.on_state = None   # f(state:int)
        self.cbs = []          # CFUNCTYPE 引用（防 GC）


class _Link:
    """一条 libpulse 连接（threaded mainloop + context）+ 挂在其上的流。

    用法：
        link = _Link("PureVox")            # 启动主循环并连接（阻塞至 READY）
        s = link.add_stream("out", spec, record=False, dev="...",
                            attr=..., on_write=cb)   # 流回调在主循环线程
        ...
        link.close()
    """

    def __init__(self, app_name: str, timeout: float = 5.0):
        self._f = _get_funcs()
        self._err = ""
        self._handlers = {}       # int(指针) -> _StreamHandler（含回调引用）
        self._cb_refs = []        # 所有 ctypes 回调闭包：活到 link 彻底销毁，
                                  # 防 libpulse 在断流/断 context 时调用已 GC 的闭包
        self._closed = False
        self._state_evt = threading.Event()
        self._stream_evts = {}    # int(指针) -> Event（建流就绪等待）

        self._c_state_cb = PA_STATE_CB_T(self._on_ctx_state)
        self._ml = self._f.ml_new()
        if not self._ml:
            raise OSError("pa_threaded_mainloop_new 失败")
        api = self._f.ml_get_api(self._ml)
        self._ctx = self._f.ctx_new(api, app_name.encode())
        if not self._ctx:
            self._teardown_ml()
            raise OSError("pa_context_new 失败")
        self._f.ctx_state_cb(self._ctx, self._c_state_cb, None)
        if self._f.ml_start(self._ml) < 0:
            self._teardown_ml()
            raise OSError("pa_threaded_mainloop_start 失败")
        rc = self._f.ctx_connect(self._ctx, None, PA_STREAM_NOFLAGS, None)
        if rc < 0:
            self._save_errno()
            self.close()
            raise OSError(f"pa_context_connect 失败: {self._err}")
        # 事件在首个状态变更（CONNECTING）即置位，未必已 READY：轮询状态
        # 到 READY/FAILED/TERMINATED 再判定（state() 持 mainloop 锁，避免竞态）
        deadline = time.monotonic() + timeout
        while True:
            if self.state() in (PA_CONTEXT_READY, PA_CONTEXT_FAILED,
                                PA_CONTEXT_TERMINATED):
                break
            remaining = deadline - time.monotonic()
            if remaining <= 0:
                break
            self._state_evt.wait(min(remaining, 0.1))
        if self.state() != PA_CONTEXT_READY:
            if not self._err:
                self._save_errno()
            self.close()
            raise OSError(f"libpulse 连接失败: {self._err}")

    # ── context 状态 ──

    def _on_ctx_state(self, ctx_ptr, userdata):
        self._state_evt.set()

    def state(self) -> int:
        if not self._ctx or self._closed:
            return PA_CONTEXT_FAILED
        # 持 mainloop 锁读取：context 由主循环线程修改，无锁读会竞态
        self._f.ml_lock(self._ml)
        try:
            return self._f.ctx_get_state(self._ctx)
        finally:
            self._f.ml_unlock(self._ml)

    def last_error(self) -> str:
        return self._err

    def _save_errno(self):
        try:
            msg = self._f.strerror(self._f.ctx_errno(self._ctx))
            self._err = msg.decode(errors="replace") if msg else "未知错误"
        except Exception:
            self._err = "未知错误"

    # ── 流管理 ──

    def add_stream(self, stream_name: str, rate: int, channels: int,
                   record: bool, dev: str, attr: PaBufferAttr,
                   on_read=None, on_write=None, on_state=None,
                   timeout: float = 5.0) -> c_void_p:
        """创建并连接一条流；返回流指针（close 时统一回收）。

        on_read(nbytes)/on_write(nbytes)/on_state(state) 在主循环线程调用。
        """
        spec = PaSampleSpec(PA_SAMPLE_FLOAT32LE, int(rate), int(channels))
        self._f.ml_lock(self._ml)
        try:
            s = self._f.s_new(self._ctx, stream_name.encode(), byref(spec), None)
            if not s:
                raise OSError("pa_stream_new 失败")
            h = _StreamHandler()
            h.on_read, h.on_write, h.on_state = on_read, on_write, on_state
            h.cbs = [PA_STATE_CB_T(self._make_state_cb(s)),
                     PA_REQUEST_CB_T(self._make_request_cb(s))]
            self._cb_refs.extend(h.cbs)
            self._handlers[int(s)] = h
            ev = threading.Event()
            self._stream_evts[int(s)] = ev
            self._f.s_state_cb(s, h.cbs[0], None)
            if record:
                self._f.s_read_cb(s, h.cbs[1], None)
                rc = self._f.s_connect_record(s, dev.encode() if dev else None,
                                              byref(attr), PA_STREAM_NOFLAGS)
            else:
                self._f.s_write_cb(s, h.cbs[1], None)
                rc = self._f.s_connect_playback(
                    s, dev.encode() if dev else None, byref(attr),
                    PA_STREAM_NOFLAGS, None, None)
            if rc < 0:
                self._save_errno()
                self._f.s_unref(s)
                self._handlers.pop(int(s), None)
                self._stream_evts.pop(int(s), None)
                raise OSError(f"流连接失败: {self._err}")
            return s
        finally:
            self._f.ml_unlock(self._ml)

    def wait_stream_ready(self, s: c_void_p, timeout: float = 5.0) -> bool:
        ev = self._stream_evts.get(int(s))
        if ev is None:
            return False
        deadline = time.monotonic() + timeout
        while True:
            st = self.stream_state(s)
            if st in (PA_STREAM_READY, PA_STREAM_FAILED, PA_STREAM_TERMINATED):
                break
            remaining = deadline - time.monotonic()
            if remaining <= 0 or not ev.wait(min(remaining, 0.2)):
                break
        st = self.stream_state(s)
        if st != PA_STREAM_READY:
            if not self._err:
                self._err = "流进入 FAILED/TERMINATED"
            return False
        return True

    def stream_state(self, s: c_void_p) -> int:
        self._f.ml_lock(self._ml)
        try:
            return self._f.s_get_state(s)
        finally:
            self._f.ml_unlock(self._ml)

    def drop_stream(self, s: c_void_p) -> None:
        """断开并回收一条流（其余流不受影响）。"""
        if not s or self._closed:
            return
        self._f.ml_lock(self._ml)
        try:
            self._f.s_disconnect(s)
            self._f.s_unref(s)
        finally:
            self._f.ml_unlock(self._ml)
        self._handlers.pop(int(s), None)
        self._stream_evts.pop(int(s), None)

    # ── 回调派发（主循环线程）──

    def _make_state_cb(self, s):
        def cb(_ptr, _userdata):
            st = self._f.s_get_state(s)
            ev = self._stream_evts.get(int(s))
            if ev is not None:
                ev.set()
            h = self._handlers.get(int(s))
            if h is not None and h.on_state is not None:
                h.on_state(st)
        return cb

    def _make_request_cb(self, s):
        def cb(_ptr, nbytes, _userdata):
            h = self._handlers.get(int(s))
            if h is None:
                return
            try:
                if h.on_write is not None:
                    h.on_write(s, nbytes)
                elif h.on_read is not None:
                    h.on_read(s, nbytes)
            except Exception as e:
                # 回调异常不能逃出 libpulse 主循环线程
                self._err = f"流回调异常: {e}"
        return cb

    # ── 生命周期 ──

    def close(self) -> None:
        if self._closed:
            return
        self._closed = True
        # 1. mainloop 运行中持锁断流 + 断 context。关键：此时**不可**释放
        #    Python 回调引用——ctx_disconnect 会触发 libpulse 回调，回调闭包
        #    若已被 GC（callable=NULL）会段错误。
        self._f.ml_lock(self._ml)
        try:
            for s in list(self._handlers.keys()):
                try:
                    self._f.s_disconnect(s)
                except Exception:
                    pass
            if self._ctx:
                self._f.ctx_disconnect(self._ctx)
        finally:
            self._f.ml_unlock(self._ml)
        # 2. 停 mainloop（等待回调线程退出）
        self._f.ml_stop(self._ml)
        # 3. mainloop 已停，安全回收资源；回调引用最后释放
        self._f.ml_lock(self._ml)
        try:
            for s in list(self._handlers.keys()):
                try:
                    self._f.s_unref(s)
                except Exception:
                    pass
            if self._ctx:
                self._f.ctx_unref(self._ctx)
                self._ctx = None
            self._handlers.clear()
            self._stream_evts.clear()
        finally:
            self._f.ml_unlock(self._ml)
        self._f.ml_free(self._ml)
        self._ml = None
        # 所有 C 资源已销毁，回调闭包此刻才可释放
        self._cb_refs.clear()

    def _teardown_ml(self):
        try:
            if getattr(self, "_ml", None):
                self._f.ml_free(self._ml)
                self._ml = None
        except Exception:
            pass


def stream_capture_ts(s: c_void_p) -> float:
    """记录流缓冲内最旧样本的采集时刻（perf 主时钟秒）。

    libpulse 的 pa_stream_get_latency 返回缓冲中最早数据相对「现在」的延迟
    （微秒），故 采集时刻 ≈ now − latency。失败/负值回退 now。与 Windows
    PortAudio input_buffer_adc_time→LinearClock 等价：给 AEC far/mic 提供
    真实采集时间戳（而非引擎读取瞬间）。
    """
    now = time.perf_counter()
    try:
        f = _get_funcs()
        usec = c_uint64(0)
        neg = c_int(0)
        if f.s_get_latency(s, byref(usec), byref(neg)) >= 0 and not neg.value:
            ts = now - usec.value / 1e6
            if ts > 0.0:
                return ts
    except Exception:
        pass
    return now


def read_float32(ptr, nbytes: int) -> list:
    """libpulse 缓冲指针 → float32 样本列表。"""
    import numpy as np
    return np.frombuffer(string_at(ptr, nbytes), dtype=np.float32).tolist()
