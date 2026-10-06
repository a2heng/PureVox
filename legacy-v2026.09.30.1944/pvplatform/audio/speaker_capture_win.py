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

"""
Windows 平台扬声器 loopback 采集（AEC far-end 数据源）。

通过 Windows Core Audio WASAPI loopback 捕获指定渲染端点的播放音频
（AEC far=扬声器时选定端点；不传 device_name 时回退系统默认渲染端点）。
不走 PyAudio/PortAudio（其对 loopback 支持有限），直接用 IAudioClient
以 AUDCLNT_STREAMFLAGS_LOOPBACK 模式打开端点。

端点选择（device_name）与主设备选择一致：按名字相似度模糊匹配
（device_api.best_name_match），匹配不到才回退默认端点——这样 AEC
far=扬声器采集到的是行里所选扬声器（真实回声源），而非恒为系统默认。

接口契约（与 Linux 后端一致）：
    start() -> bool / stop() / read(n) / flush()
    dev_sr (int) / active (bool) / on_device_changed 回调
"""

import ctypes
import threading
import time
from ctypes import wintypes, POINTER, byref, cast, c_void_p
from typing import Optional, Callable, Tuple

from .common import TimedFifo, HOP_LENGTH, _module_log
from .device_api import best_name_match


class SpeakerCaptureWin:
    """半双工扬声器采集 — WASAPI loopback（Windows 专用，可按名选端点）。"""

    # COM 接口 GUID
    _CLSID_MMDeviceEnumerator = "{BCDE0395-E52F-467C-8E3D-C4579291692E}"
    _IID_IMMDeviceEnumerator    = "{A95664D2-9614-4F35-A746-DE8DB63617E6}"
    _IID_IMMDevice              = "{D666063F-1587-4E43-81F1-B948E807363F}"
    _IID_IAudioClient           = "{1CB9AD4C-DBFA-4C32-B178-C2F568A703B2}"
    _IID_IAudioCaptureClient    = "{C8ADBD64-E71E-48A0-A4DE-185C395CD317}"

    AUDCLNT_SHAREMODE_SHARED = 0
    AUDCLNT_STREAMFLAGS_LOOPBACK = 0x00020000
    AUDCLNT_BUFFERFLAGS_SILENT = 0x2   # 数据未定义：必须按静音处理，不能读
    CLSCTX_ALL = 0x17
    COINIT_MULTITHREADED = 0x0
    # 枚举掩码：DEVICE_STATE_ACTIVE
    DEVICE_STATE_ACTIVE = 0x1

    # 设备检查间隔（秒）
    DEVICE_CHECK_INTERVAL = 2.0

    AEC_FAR_SR = 48000  # AEC model requires far-end (speaker loopback) at 48kHz

    def __init__(self, on_device_changed: Optional[Callable[[int], None]] = None,
                 device_name: str = ""):
        self._wanted_name = device_name or ""
        self._buffer = TimedFifo(48000, HOP_LENGTH * 32)  # ~320ms，吸收回环抖动
        self._active = False
        self._lock = threading.Lock()
        self._capture_thread: Optional[threading.Thread] = None
        self._device_check_thread: Optional[threading.Thread] = None
        # COM objects (used in capture thread)
        self._audio_client: Optional[ctypes.c_void_p] = None
        self._capture_client: Optional[ctypes.c_void_p] = None
        self._dev_ch: int = 1
        self._dev_sr: int = 48000  # 强制 48kHz
        self._dev_name: str = "Unknown"
        self._current_device_name: Optional[str] = None
        self._endpoint_id: Optional[str] = None
        # Callback when device switches: called with new dev_sr
        self._on_device_changed = on_device_changed

    @property
    def active(self) -> bool:
        return self._active

    @property
    def dev_sr(self) -> int:
        """设备实际采样率（来自 WASAPI MixFormat）。"""
        return self._dev_sr

    @property
    def device_name(self) -> str:
        """当前实际采集端点名（诊断用）。"""
        return self._current_device_name or self._dev_name or ""

    # ── COM 小工具 ────────────────────────────────────────────────

    @staticmethod
    def _make_guid(s: str):
        ole32 = ctypes.windll.ole32
        buf = (ctypes.c_ubyte * 16)()
        ole32.CLSIDFromString(s, buf)
        return buf

    @staticmethod
    def _release_com(p: Optional[c_void_p]) -> None:
        """调用 COM 对象 vtable[2] Release。"""
        if p and p.value:
            try:
                vtbl = cast(p, POINTER(POINTER(c_void_p))).contents
                cast(vtbl[2], ctypes.WINFUNCTYPE(ctypes.c_ulong, c_void_p))(p)
            except Exception:
                pass

    def _friendly_name_of(self, ole32, p_device) -> str:
        """读端点友好名（IPropertyStore PKEY_Device_FriendlyName）。"""
        try:
            vtbl_dev = cast(p_device, POINTER(POINTER(c_void_p))).contents
            fn_OpenPS = cast(vtbl_dev[4], ctypes.WINFUNCTYPE(
                ctypes.c_long, c_void_p, wintypes.DWORD, POINTER(c_void_p)))
            p_store = c_void_p()
            if fn_OpenPS(p_device, 0, byref(p_store)) != 0 or not p_store:
                return "Unknown"
            try:
                pkey_fmtid = self._make_guid("{a45c254e-df1c-4efd-8020-67d146a850e0}")

                class PK(ctypes.Structure):
                    _fields_ = [("fmtid", ctypes.c_ubyte * 16),
                                ("pid", wintypes.DWORD)]
                pk = PK()
                ctypes.memmove(pk.fmtid, pkey_fmtid, 16)
                pk.pid = 14  # PKEY_Device_FriendlyName
                vtbl_store = cast(p_store, POINTER(POINTER(c_void_p))).contents
                fn_GetValue = cast(vtbl_store[5], ctypes.WINFUNCTYPE(
                    ctypes.c_long, c_void_p, ctypes.c_void_p, ctypes.c_void_p))
                pv = (ctypes.c_ubyte * 24)()
                if fn_GetValue(p_store, byref(pk), pv) == 0:
                    if ctypes.c_ushort.from_buffer(pv, 0).value == 31:  # VT_LPWSTR
                        ptr = c_void_p.from_buffer(pv, 8).value
                        if ptr:
                            return ctypes.wstring_at(ptr)
            finally:
                self._release_com(p_store)
        except Exception as e:
            _module_log(f"[AEC] 读端点名失败: {e}")
        return "Unknown"

    def _endpoint_id_of(self, ole32, p_device) -> str:
        """读端点设备 ID（IMMDevice::GetId，用于比对设备是否变更）。"""
        try:
            vtbl_dev = cast(p_device, POINTER(POINTER(c_void_p))).contents
            fn_GetId = cast(vtbl_dev[5], ctypes.WINFUNCTYPE(
                ctypes.c_long, c_void_p, POINTER(c_void_p)))
            p_id = c_void_p()
            if fn_GetId(p_device, byref(p_id)) == 0 and p_id.value:
                s = ctypes.wstring_at(p_id.value)
                ole32.CoTaskMemFree(p_id)
                return s
        except Exception:
            pass
        return ""

    def _enumerate_render(self, ole32):
        """枚举活动渲染端点，返回 [(friendly_name, IMMDevice*), ...]（须调用方释放）。"""
        p_enum = c_void_p()
        hr = ole32.CoCreateInstance(
            byref(self._make_guid(self._CLSID_MMDeviceEnumerator)),
            None, self.CLSCTX_ALL,
            byref(self._make_guid(self._IID_IMMDeviceEnumerator)),
            byref(p_enum))
        if hr < 0 or not p_enum:
            return []
        try:
            vtbl = cast(p_enum, POINTER(POINTER(c_void_p))).contents
            fn_Enum = cast(vtbl[3], ctypes.WINFUNCTYPE(
                ctypes.c_long, c_void_p, wintypes.DWORD, wintypes.DWORD,
                POINTER(c_void_p)))
            p_coll = c_void_p()
            hr = fn_Enum(p_enum, 0, self.DEVICE_STATE_ACTIVE, byref(p_coll))  # eRender
            if hr < 0 or not p_coll:
                return []
            try:
                vtbl_c = cast(p_coll, POINTER(POINTER(c_void_p))).contents
                fn_Count = cast(vtbl_c[3], ctypes.WINFUNCTYPE(
                    ctypes.c_long, c_void_p, POINTER(wintypes.DWORD)))
                fn_Item = cast(vtbl_c[4], ctypes.WINFUNCTYPE(
                    ctypes.c_long, c_void_p, wintypes.DWORD, POINTER(c_void_p)))
                n = wintypes.DWORD()
                if fn_Count(p_coll, byref(n)) < 0:
                    return []
                out = []
                for i in range(n.value):
                    p_dev = c_void_p()
                    if fn_Item(p_coll, i, byref(p_dev)) < 0 or not p_dev:
                        continue
                    name = self._friendly_name_of(ole32, p_dev)
                    if name and name != "Unknown":
                        out.append((name, p_dev))
                    else:
                        self._release_com(p_dev)
                return out
            finally:
                self._release_com(p_coll)
        finally:
            self._release_com(p_enum)

    def _resolve_endpoint(self, ole32) -> c_void_p:
        """按 device_name 模糊匹配活动渲染端点；未命中回退默认渲染端点。

        返回持有引用的 IMMDevice*（调用方负责 Release），失败返回空指针。
        """
        if self._wanted_name:
            try:
                ends = self._enumerate_render(ole32)
            except Exception as e:
                _module_log(f"[AEC] 端点枚举失败: {e}")
                ends = []
            if ends:
                names = [n for n, _ in ends]
                chosen = best_name_match(self._wanted_name, names)
                if chosen is not None:
                    for n, p_dev in ends:
                        if n == chosen:
                            return p_dev
                        self._release_com(p_dev)
                    # 理论不可达（chosen 来自 names）；继续回退默认
                else:
                    for _, p_dev in ends:
                        self._release_com(p_dev)
        # 默认端点
        p_enum = c_void_p()
        hr = ole32.CoCreateInstance(
            byref(self._make_guid(self._CLSID_MMDeviceEnumerator)),
            None, self.CLSCTX_ALL,
            byref(self._make_guid(self._IID_IMMDeviceEnumerator)),
            byref(p_enum))
        if hr < 0 or not p_enum:
            return c_void_p()
        try:
            vtbl = cast(p_enum, POINTER(POINTER(c_void_p))).contents
            fn_GetDefault = cast(vtbl[4], ctypes.WINFUNCTYPE(
                ctypes.c_long, c_void_p, wintypes.DWORD, wintypes.DWORD,
                POINTER(c_void_p)))
            p_dev = c_void_p()
            hr = fn_GetDefault(p_enum, 0, 0, byref(p_dev))  # eRender, eConsole
            if hr < 0 or not p_dev:
                _module_log("[AEC] 无法获取默认渲染设备")
                return c_void_p()
            return p_dev
        finally:
            self._release_com(p_enum)

    # ── 打开/重启 ────────────────────────────────────────────────

    def _open_impl(self) -> bool:
        """打开目标渲染端点的 WASAPI loopback 并开始采集（不启动线程）。

        start() 与 _restart_capture()（设备变更重连）共用此实现，避免
        start/restart 两套端点解析漂移。
        """
        try:
            ole32 = ctypes.windll.ole32
            ole32.CoInitializeEx(None, self.COINIT_MULTITHREADED)

            p_device = self._resolve_endpoint(ole32)
            if not p_device or not p_device.value:
                _module_log("[AEC] 无法解析回环端点")
                return False
            try:
                dev_name = self._friendly_name_of(ole32, p_device)
                dev_id = self._endpoint_id_of(ole32, p_device)

                vtbl_dev = cast(p_device, POINTER(POINTER(c_void_p))).contents

                # Activate IAudioClient
                fn_Activate = cast(vtbl_dev[3], ctypes.WINFUNCTYPE(
                    ctypes.c_long, c_void_p, c_void_p, wintypes.DWORD,
                    c_void_p, POINTER(c_void_p)))
                p_ac = c_void_p()
                hr = fn_Activate(p_device, byref(self._make_guid(self._IID_IAudioClient)),
                                 self.CLSCTX_ALL, None, byref(p_ac))
                if hr < 0 or not p_ac:
                    _module_log("[AEC] 无法激活 IAudioClient")
                    return False

                # GetMixFormat → 设备原生格式（采样率 + 声道数）
                vtbl_ac = cast(p_ac, POINTER(POINTER(c_void_p))).contents

                class WAVEFORMATEX(ctypes.Structure):
                    _fields_ = [
                        ("wFormatTag",      wintypes.WORD),
                        ("nChannels",       wintypes.WORD),
                        ("nSamplesPerSec",  wintypes.DWORD),
                        ("nAvgBytesPerSec", wintypes.DWORD),
                        ("nBlockAlign",     wintypes.WORD),
                        ("wBitsPerSample",  wintypes.WORD),
                        ("cbSize",          wintypes.WORD),
                    ]
                fn_GetMixFormat = cast(vtbl_ac[8], ctypes.WINFUNCTYPE(
                    ctypes.c_long, c_void_p, POINTER(ctypes.c_void_p)))
                p_wfx = ctypes.c_void_p()
                hr = fn_GetMixFormat(p_ac, byref(p_wfx))
                if hr < 0 or not p_wfx:
                    self._release_com(p_ac)
                    return False
                wfx = cast(p_wfx, POINTER(WAVEFORMATEX)).contents
                self._dev_sr = int(wfx.nSamplesPerSec)
                self._dev_ch = max(1, int(wfx.nChannels))
                _module_log(f"[AEC] 扬声器采集: {dev_name} ({self._dev_sr}Hz, "
                            f"ch={self._dev_ch})")
                self._buffer = TimedFifo(self._dev_sr, HOP_LENGTH * 32)

                # Initialize with AUDCLNT_STREAMFLAGS_LOOPBACK
                REFERENCE_TIME = 100000  # 10ms buffer
                fn_Initialize = cast(vtbl_ac[3], ctypes.WINFUNCTYPE(
                    ctypes.c_long, c_void_p, wintypes.DWORD, wintypes.DWORD,
                    ctypes.c_longlong, ctypes.c_longlong, c_void_p, c_void_p))
                hr = fn_Initialize(
                    p_ac, self.AUDCLNT_SHAREMODE_SHARED,
                    self.AUDCLNT_STREAMFLAGS_LOOPBACK,
                    REFERENCE_TIME, 0, p_wfx, None)
                ole32.CoTaskMemFree(p_wfx)
                if hr < 0:
                    _module_log(f"[AEC] IAudioClient::Initialize failed: 0x{hr:08X}")
                    self._release_com(p_ac)
                    return False

                # GetService → IAudioCaptureClient
                fn_GetService = cast(vtbl_ac[14], ctypes.WINFUNCTYPE(
                    ctypes.c_long, c_void_p, c_void_p, POINTER(c_void_p)))
                p_cc = c_void_p()
                hr = fn_GetService(p_ac, byref(self._make_guid(self._IID_IAudioCaptureClient)),
                                   byref(p_cc))
                if hr < 0 or not p_cc:
                    _module_log("[AEC] failed to get IAudioCaptureClient")
                    self._release_com(p_ac)
                    return False

                fn_Start = cast(vtbl_ac[10], ctypes.WINFUNCTYPE(
                    ctypes.c_long, c_void_p))
                if fn_Start(p_ac) < 0:
                    _module_log("[AEC] IAudioClient::Start failed")
                    self._release_com(p_cc)
                    self._release_com(p_ac)
                    return False

                self._audio_client = p_ac
                self._capture_client = p_cc
                self._dev_name = dev_name
                self._current_device_name = dev_name
                self._endpoint_id = dev_id
                return True
            finally:
                self._release_com(p_device)
        except Exception as e:
            import traceback
            _module_log(f"[AEC] speaker capture failed: {e}")
            _module_log(traceback.format_exc())
            return False

    def start(self) -> bool:
        """打开选定渲染设备 loopback（device_name 未传/未命中时用默认）。
        Returns True on success."""
        if self._active:
            return True
        if not self._open_impl():
            self._active = False
            return False
        self._active = True

        # 采集线程 + 设备监听线程（端点变更自动重连）
        self._capture_thread = threading.Thread(target=self._capture_loop, daemon=True)
        self._capture_thread.start()
        self._device_check_thread = threading.Thread(target=self._device_check_loop, daemon=True)
        self._device_check_thread.start()
        return True

    def _restart_capture(self) -> bool:
        """重新初始化采集（不启动新的监听线程，供设备变更重连调用）。"""
        try:
            with self._lock:
                self._stop_capture_internal()
            return self._open_impl()
        except Exception as e:
            _module_log(f"[AEC] restart capture failed: {e}")
            return False

    # ── 设备变更监听 ─────────────────────────────────────────────

    def _expected_endpoint_name(self) -> Optional[str]:
        """当前应采集的端点名（按 device_name 重新解析；无则读默认端点）。

        仅用于比对设备是否变更（device_name 语义下 = 目标名仍解析到
        同一端点；默认语义下 = 系统默认端点是否切换）。
        """
        try:
            ole32 = ctypes.windll.ole32
            ole32.CoInitializeEx(None, self.COINIT_MULTITHREADED)
            p_dev = self._resolve_endpoint(ole32)
            if not p_dev or not p_dev.value:
                return None
            try:
                return self._friendly_name_of(ole32, p_dev)
            finally:
                self._release_com(p_dev)
        except Exception:
            return None

    def _device_check_loop(self) -> None:
        """后台线程：定期检查目标端点是否变化，变化则自动重新连接。"""
        while self._active:
            try:
                time.sleep(self.DEVICE_CHECK_INTERVAL)
                if not self._active:
                    break

                new_device_name = self._expected_endpoint_name()
                if new_device_name and new_device_name != self._current_device_name:
                    _module_log(f"[AEC] device changed: {self._current_device_name} "
                                f"-> {new_device_name}")
                    if self._restart_capture():
                        _module_log(f"[AEC] device switched: {self._dev_name} "
                                    f"(sr={self._dev_sr}Hz)")
                        if self._on_device_changed:
                            try:
                                self._on_device_changed(self._dev_sr)
                            except Exception:
                                pass
                    else:
                        _module_log(f"[AEC] device switch failed")
            except Exception as e:
                _module_log(f"[AEC] device check error: {e}")
                time.sleep(1.0)

    # ── 数据面 ───────────────────────────────────────────────────

    def _stop_capture_internal(self) -> None:
        """内部方法：停止采集（不停监听线程）"""
        if self._audio_client:
            try:
                vtbl_ac = ctypes.cast(self._audio_client,
                                      ctypes.POINTER(ctypes.POINTER(ctypes.c_void_p))).contents
                fn_Stop = ctypes.cast(vtbl_ac[11], ctypes.WINFUNCTYPE(
                    ctypes.c_long, ctypes.c_void_p))
                fn_Stop(self._audio_client)
                self._release_com(self._audio_client)
            except Exception:
                pass
            self._audio_client = None
        self._capture_client = None

    def _capture_loop(self) -> None:
        """后台线程：持续从 IAudioCaptureClient 读取并写入 TimedFifo（带 QPC 时间戳）。

        每次循环从 self._capture_client 取当前采集客户端——设备变更重连
        （_restart_capture 换新 client）后自动跟随新指针，不在旧已释放
        对象上继续调用。
        """
        import struct

        last_cc = None
        fn_GetBuffer = fn_ReleaseBuffer = None

        def _bind(cc):
            vtbl_cc = cast(cc, POINTER(POINTER(c_void_p))).contents
            gb = cast(vtbl_cc[3], ctypes.WINFUNCTYPE(
                ctypes.c_long, c_void_p,
                POINTER(ctypes.POINTER(ctypes.c_ubyte)),     # BYTE **ppData
                POINTER(wintypes.DWORD),                      # UINT32 *pNumFramesToRead
                POINTER(wintypes.DWORD),                      # DWORD *pdwFlags
                POINTER(ctypes.c_uint64),                     # UINT64 *pu64DevicePosition
                POINTER(ctypes.c_uint64)))                    # UINT64 *pu64QPCPosition
            rb = cast(vtbl_cc[4], ctypes.WINFUNCTYPE(
                ctypes.c_long, c_void_p, wintypes.DWORD))
            return gb, rb

        qpf = ctypes.c_uint64()
        try:
            ctypes.windll.kernel32.QueryPerformanceFrequency(ctypes.byref(qpf))
        except Exception:
            qpf.value = 0
        qpf_s = float(qpf.value) or 1.0

        while self._active:
            try:
                cc = self._capture_client
                if cc is None or not cc.value:
                    time.sleep(0.005)
                    continue
                if cc is not last_cc:
                    last_cc = cc
                    fn_GetBuffer, fn_ReleaseBuffer = _bind(cc)
                # 一次把客户端里排队的包全部取空：只取一个包 + 1ms 轮询会在
                # 设备包率偏高/调度抖动时追不上，积压后产生不连续（爆音）
                drained = False
                while self._active:
                    p_data = ctypes.POINTER(ctypes.c_ubyte)()
                    num_frames = wintypes.DWORD()
                    flags = wintypes.DWORD()
                    dev_pos = ctypes.c_uint64()
                    qpc_pos = ctypes.c_uint64()
                    hr = fn_GetBuffer(cc, byref(p_data),
                                      byref(num_frames), byref(flags),
                                      byref(dev_pos), byref(qpc_pos))
                    if hr < 0 or num_frames.value == 0:
                        break
                    drained = True
                    frame_count = num_frames.value
                    ch = self._dev_ch
                    if flags.value & self.AUDCLNT_BUFFERFLAGS_SILENT:
                        # 静音包：数据未定义，按零填充（读原始字节=噪声爆音）
                        mono = [0.0] * frame_count
                    else:
                        n_floats = frame_count * ch
                        raw = list(struct.unpack(
                            f"{n_floats}f", bytes(p_data[:n_floats * 4])))
                        if ch > 1:
                            mono = [0.0] * frame_count
                            for i in range(frame_count):
                                s = 0.0
                                for c in range(ch):
                                    s += raw[i * ch + c]
                                mono[i] = s / ch
                        else:
                            mono = raw
                    fn_ReleaseBuffer(cc, num_frames)
                    # 外部时钟（QPC 秒）：包内首帧的 QPC 位置；缺失退回墙钟
                    ts0 = (qpc_pos.value / qpf_s) if qpc_pos.value \
                        else time.perf_counter()
                    self._buffer.write_ts(ts0, mono)
                if not drained:
                    time.sleep(0.001)

            except Exception:
                time.sleep(0.005)

    def stop(self) -> None:
        """停止 loopback 采集。"""
        self._active = False

        if self._device_check_thread:
            self._device_check_thread.join(timeout=1.0)
            self._device_check_thread = None
        if self._capture_thread:
            self._capture_thread.join(timeout=1.0)
            self._capture_thread = None

        with self._lock:
            self._stop_capture_internal()

    def available(self) -> int:
        """缓冲区当前可用采样数。"""
        return self._buffer.available()

    def read_ts(self, n_samples: int) -> Optional[Tuple[float, list]]:
        """读取 n 个 FIFO 采样 → (首样本主时钟秒, samples)；不足返回 None。"""
        return self._buffer.read_ts(n_samples)

    def read(self, n_samples: int) -> Optional[list]:
        """从缓冲区读取 n_samples 个 FIFO 采样；数据不足时返回 None。"""
        got = self._buffer.read_ts(n_samples)
        return got[1] if got is not None else None

    def read_latest(self, n_samples: int) -> Optional[list]:
        """从缓冲区读取最新 n_samples 个采样，丢弃旧数据；数据不足返回 None。"""
        n = int(n_samples)
        avail = self._buffer.available()
        if avail < n:
            return None
        if avail > n:
            self._buffer.read_ts(avail - n)
        got = self._buffer.read_ts(n)
        return got[1] if got is not None else None

    def flush(self) -> None:
        """清空缓冲区。"""
        self._buffer.flush()
