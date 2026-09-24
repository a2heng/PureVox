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
PureVox 音频处理核心模块。

提供以下功能：
- 音频常量（采样率、帧大小、hop 长度）
- 线程安全环形缓冲区
- 实时音频流处理线程
- 音频设备枚举（Windows WASAPI/MME / Linux pipewire-pulse）
"""

import io
import os
import struct
import threading
import time
import wave
from typing import Any, List, Optional, Callable, Tuple

import numpy as np

# PyAudio（PortAudio）仅 Windows/macOS 后端使用；Linux 走原生 PipeWire，
# 允许无 PyAudio 环境运行。引用点都在非 Linux 执行路径内。
try:
    import pyaudio
except ImportError:
    pyaudio = None  # type: ignore

# Module-level log hook — set by ui.py to sync console + UI log.
_module_log = print

# 平台抽象层：SpeakerCapture 工厂 + 公共件（RingBuffer/常量/日志，避免循环导入）
from pvplatform import IS_LINUX
from pvplatform.audio import create_speaker_capture
from pvplatform.audio.common import set_module_log as _set_common_log

if IS_LINUX:
    from pvplatform.audio.pwpipe_client import PwBridge as _PwBridge
    from pvplatform.audio.pwpipe_client import list_sources as _pw_sources
    from pvplatform.audio.pwpipe_client import list_destinations as _pw_dests
else:
    from pvplatform.audio.pa_backend import PaBridge as _PaBridge


def set_module_log(func):
    global _module_log
    _module_log = func
    _set_common_log(func)


try:
    from pvengine import AudioProcessor, PlaybackSink, RingBuffer, Resampler
    ENGINE_AVAILABLE = True
except ImportError:
    ENGINE_AVAILABLE = False
    _module_log("[音频] 引擎模块 pvengine 不可用（缺 numpy/onnxruntime？）")

SAMPLE_RATE = 48000
HOP_LENGTH = 480                # 10ms @48kHz（202609 模型契约：波形 hop 进出）


TSE_SAMPLE_RATE = 48000        # TSE 模型采样率 (48kHz)


#  Speaker loopback capture — 平台抽象（WASAPI / PulseAudio）
#  实现见 platform.audio.speaker_capture_{win,linux,macos}
# ═══════════════════════════════════════════════════════════════

SpeakerCapture = create_speaker_capture  # 工厂别名：按平台返回后端实例


class AudioThread(threading.Thread):
    """本地设备的音频处理线程。"""

    def __init__(self, input_id: Optional[int], output_id: int,
                 process_frame: Callable[[List[float]], List[float]],
                 hop_length: int,
                 processor: object = None,
                 network_source = None,
                 api_type: int = 13,
                 ready_msg: str = "",
                 extra_output_ids=None) -> None:
        super().__init__(name='AudioProcessor', daemon=True)
        self._ready_msg: str = ready_msg
        self._api_type: int = api_type
        self._input_id: Optional[int] = input_id
        self._output_id: int = output_id
        self._network_source = network_source  # RemoteAudioSource for network input mode
        # 传输后端插件（Linux=PwBridge / Windows=PaBridge）+ 每输出一个
        # PlaybackSink（跨时钟域播放正确性唯一实现点，见 pvengine.dsp.playback）。
        # 回调/流只做搬运；处理在本线程 _bridge_loop，速率差由 sink 消化。
        self._use_pw = False
        self._bridge = None                      # 后端实例（open 于 _create_stream）
        self._pw_ports: Tuple[List[str], List[str]] = ([], [])  # (输入列表, 输出列表)
        self._sinks: List[PlaybackSink] = []     # 与输出路一一对应
        # 网络读侧状态（_network_reader）
        self._net_acc: List[float] = []
        self._net_last_data: float = 0.0
        # 循环诊断（60s 汇总一行，见 _bridge_loop）
        self._diag = {"fc": 0, "t_sum": 0.0, "t_max": 0.0, "t0": time.time(),
                      "s_pads": 0, "s_drops": 0, "s_urs": 0}
        self._process_frame: Callable[[List[float]], List[float]] = process_frame
        self._hop_length: int = hop_length
        # 多输出扇出（原「监听」机制的泛化）：主输出之外的全部播放设备
        self._extra_out_ids: List[int] = [i for i in (extra_output_ids or [])
                                          if i is not None]
        self._stop_event: threading.Event = threading.Event()
        # 流就绪事件：_create_stream 成功后 set()，失败时记录 _start_error
        self._ready_event: threading.Event = threading.Event()
        self._start_error: Optional[str] = None
        self._p: Optional[Any] = None
        self._vu_peak: float = 0.0  # L4:VU峰值快照（最新帧dBFS）
        self._spectrum_in: RingBuffer = RingBuffer(SAMPLE_RATE * 2)   # L4:频谱输入 2s
        self._spectrum_out: RingBuffer = RingBuffer(SAMPLE_RATE * 2)  # L4:频谱输出 2s
        self._viz_enabled: bool = True  # 频谱/VU 开关（窗口最小化时暂停）
        self._lock: threading.Lock = threading.Lock()  # Lock for stream operations
        self.processor: object = processor  # Reference to processor for dynamic control
        self._tse_hook: Optional[Callable[[List[float]], None]] = None  # TSE audio hook
        self._recording_hook: Optional[Callable[[List[float]], None]] = None  # 录音捕获钩子

        # ── AEC（行级：一行 echo_cancel 输入对应一路 AEC）──
        # cfg: SessionPlan.aec_rows（{mic, far_gain_db, far_kind, far_device}）；
        # live: 建流后装配的 [{mic_index, capture, row}]，随流共存亡。
        # far 经行内 GridHistory 按外部时钟配对直达 AecRow，不经过任何 fx。
        self._aec_cfg: list = []
        self._aec_inputs: list = []   # 建桥时的输入顺序（与 read_each 对齐）
        self._aec_live: list = []
        self._aec_vu: dict = {}   # {mic_name: {"mic": float, "far": float, "out": float}}
        # ── 回环输入（一行 loopback 输入对应一路回采，进混音）──
        # cfg: SessionPlan.loopbacks（扬声器设备名）；live: [{capture, hist, rs, ratio}]。
        # 与 AEC 行 far=扬声器继承同一套回环采集机制。
        self._loopback_cfg: list = []
        self._loopback_live: list = []
        # ── AEC 诊断（每 100 帧=1s 打印一次，配合参考音量滑杆实时看效果）──
        self._aec_diag_count: int = 0

    def set_pw_ports(self, input_names: List[str], output_names: List[str]) -> None:
        """设置 Linux PipeWire 输入/输出节点名列表（node.name）。

        多输入自动混音，多输出扇出同一路降噪音频。须在 run() 之前调用；
        start_audio_stream 会据此选择原生 PipeWire 后端。
        """
        self._use_pw = bool(input_names or output_names) and IS_LINUX
        self._pw_ports = (list(input_names or []), list(output_names or []))

    def set_aec_rows(self, rows, input_names) -> None:
        """设置行级 AEC 配置（SessionPlan.aec_rows 原样 + 建桥输入顺序）。

        须在 run() 之前调用；AEC 行随流装配、随流释放，行配置变更走重启
        （与输入行一致，不做运行时热切换）。
        """
        self._aec_cfg = [dict(r) for r in (rows or [])]
        self._aec_inputs = list(input_names or [])

    def set_loopback_rows(self, devices) -> None:
        """设置回环输入行（SessionPlan.loopbacks 原样）。须在 run() 之前调用."""
        self._loopback_cfg = [d for d in (devices or []) if d]

    def wait_ready(self, timeout: float = 3.0) -> bool:
        """等待音频流创建完成。返回 True 表示成功，False 表示失败/超时。"""
        if not self._ready_event.wait(timeout):
            return False
        return self._start_error is None

    def set_viz_enabled(self, enabled: bool) -> None:
        """暂停/恢复频谱和 VU 更新（窗口最小化时调用）。"""
        self._viz_enabled = enabled
        if not enabled:
            # 清空积压数据，避免恢复时瞬间刷新大量旧数据
            for buf in (self._spectrum_in, self._spectrum_out):
                if buf: buf.read_latest(HOP_LENGTH)

    def _open_loopback_capture(self, dev: str):
        """开一路回环采集（扬声器播出直采），AEC far 与回环输入行共用。

        失败抛 OSError（大声失败，不静默降级）。
        """
        if IS_LINUX and self._use_pw:
            cap = SpeakerCapture(pw_bridge=self._bridge, far_sink=dev)
        else:
            # Windows/macOS：SpeakerCapture 按目标端点名开 loopback（与主设备
            # 选择同一模糊匹配；Windows 未匹配回退默认渲染端点）。
            cap = SpeakerCapture(device_name=dev)
        if not cap.start():
            raise OSError(f"回环采集启动失败: {dev}")
        return cap

    def _open_mic_capture(self, dev: str):
        """开一路麦克风专用采集（AEC far=mic 用，不进混音）。失败抛 OSError."""
        from pvplatform.audio import create_mic_capture
        if IS_LINUX and self._use_pw:
            cap = create_mic_capture(dev, pw_bridge=self._bridge)
        else:
            far_id = get_device_id(dev, True, api_type=self._api_type)
            if far_id is None:
                raise OSError(f"AEC far 麦克风无匹配设备: {dev}")
            cap = create_mic_capture(far_id)
        if not cap.start():
            raise OSError(f"麦克风 far 采集启动失败: {dev}")
        return cap

    def _build_aec_rows(self) -> None:
        """按 _aec_cfg 逐行装配 AEC（far 采集 + 行级 AecRow），建流后调用。

        far=扬声器继承回环采集机制（_open_loopback_capture），far=麦克风
        走麦克风专用采集；far 样本经行内 GridHistory 按外部时钟配对直达 AecRow，
        不经过任何 fx。
        """
        self._aec_live = []
        if not self._aec_cfg:
            return
        import model_config
        from pvengine.aec_row import AecRow, find_model_file
        model_path = find_model_file(model_config.AEC_MODEL)
        self._aec_vu = {}
        for cfg in self._aec_cfg:
            mic = cfg["mic"]
            mic_index = self._aec_inputs.index(mic) \
                if mic in self._aec_inputs else -1
            if mic_index < 0:
                raise OSError(f"AEC 行麦克风不在输入列表: {mic}")
            far_kind = cfg.get("far_kind", "speaker")
            far_device = cfg.get("far_device", "")
            if far_kind == "mic":
                cap = self._open_mic_capture(far_device)
            else:
                cap = self._open_loopback_capture(far_device)
            row = AecRow(model_path, far_sample_rate=cap.dev_sr,
                         far_gain_db=cfg.get("far_gain_db", -20.0))
            far_delay = cfg.get("far_delay_ms", 0.0)
            if far_delay:
                row.set_delay_ms(far_delay)
            self._aec_live.append({"mic_index": mic_index, "capture": cap,
                                     "row": row, "mic": mic,
                                     "far_kind": far_kind,
                                     "far_device": far_device})
            self._aec_vu[mic] = {"mic": 0.0, "far": 0.0, "out": 0.0}
            _module_log(f"[AEC] 行装配: mic={mic} far({far_kind})={far_device} "
                        f"sr={cap.dev_sr}Hz gain={cfg.get('far_gain_db', -20.0)}dB "
                        f"delay={far_delay:.0f}ms")

    def _stop_aec_rows(self) -> None:
        """释放全部 AEC 行采集（流关闭时调用）。"""
        for live in self._aec_live:
            try:
                live["capture"].stop()
            except Exception as e:
                _module_log(f"[AEC] 行采集关闭异常: {e}")
        self._aec_live = []
        self._aec_vu = {}

    def _build_loopback_rows(self) -> None:
        """按 _loopback_cfg 逐行装配回环输入（回环采集 + 外部时钟网格历史）。

        桌面声音与 AEC far 一样：样本带采集时间戳入 GridHistory，按 48k 网格
        逐 hop 出队进混音，时间对齐由时间戳保证（无隐藏缓冲）。
        """
        self._loopback_live = []
        if not self._loopback_cfg:
            return
        import numpy as _np
        from pvengine.dsp.hop_queue import GridHistory
        from pvengine.dsp.resampler import Resampler
        cap_samples = HOP_LENGTH * 150   # 1.5s 上限
        for dev in self._loopback_cfg:
            cap = self._open_loopback_capture(dev)
            hist = GridHistory(cap_samples)
            if cap.dev_sr != SAMPLE_RATE:
                rs = Resampler()
                ratio = SAMPLE_RATE / float(cap.dev_sr)
                rs.process(_np.zeros(HOP_LENGTH, dtype=_np.float32), ratio)
            else:
                rs, ratio = None, 1.0
            self._loopback_live.append({"capture": cap, "hist": hist,
                                        "rs": rs, "ratio": ratio,
                                        "device": dev, "primed": False})
            _module_log(f"[回环] 行装配: {dev} sr={cap.dev_sr}Hz")

    def _stop_loopback_rows(self) -> None:
        """释放全部回环输入采集（流关闭时调用）。"""
        for live in self._loopback_live:
            try:
                live["capture"].stop()
            except Exception as e:
                _module_log(f"[回环] 行采集关闭异常: {e}")
        self._loopback_live = []

    def set_aec_far_gain(self, mic: str, db: float) -> bool:
        """运行时实时改某 AEC 行的参考音量（只缩放进模型的 far 帧）。

        单 float 赋值，线程安全；返回 False 表示该行不在运行中。
        """
        for live in self._aec_live:
            if live["mic"] == mic:
                live["row"].set_far_gain_db(db)
                return True
        return False

    def get_aec_vu(self) -> dict:
        """返回各 AEC 行的最新 mic/far/out 峰值（UI 线程调用）。"""
        return dict(self._aec_vu)

    def get_aec_info(self) -> list:
        """各 AEC 行 far 端实际状态（UI 状态行用）：[{mic, far_kind,
        far_device, far_sr}]。未运行返回 []，不抛异常。"""
        out = []
        try:
            for live in self._aec_live:
                cap = live.get("capture")
                try:
                    sr = int(cap.dev_sr) if cap is not None else 0
                except Exception:
                    sr = 0
                out.append({"mic": live.get("mic", ""),
                            "far_kind": live.get("far_kind", ""),
                            "far_device": live.get("far_device", ""),
                            "far_sr": sr})
        except Exception:
            pass
        return out

    def get_input_info(self) -> dict:
        """输入端实际采样率状态（UI 状态行用）：原生采样率/声道 + 是否重采样。

        桥未就绪（Linux/网络模式无 input_info 时）返回 {}，不抛异常。
        """
        try:
            if self._bridge is not None \
                    and hasattr(self._bridge, "input_info"):
                return dict(self._bridge.input_info())
        except Exception:
            pass
        return {}

    def set_aec_delay_ms(self, mic: str, ms: float) -> bool:
        """运行时设置某 AEC 行 far 延迟（毫秒）。"""
        for live in self._aec_live:
            if live["mic"] == mic:
                live["row"].set_delay_ms(ms)
                return True
        return False

    @staticmethod
    def calibrate_aec_delay(mic_dev: str, far_dev: str,
                            far_kind: str = "speaker") -> Optional[float]:
        """离线校准 AEC far 延迟（须在音频处理停止时调用）。

        同一时点采集 far 参考（far=扬声器＝所选端点 loopback / far=麦克风＝
        far mic 输入）与麦克风后直接互相关，得到「扬声器端点 → 麦克风回声」
        的唯一延迟路径值 K，返回即滑杆要用的值：无任何隐含叠加。硬件不变
        则 K 基本恒定，故安静环境测一次、保存持续使用即可（模型自带多抽头
        对齐，远/近端残余错位由模型内部消化，不再额外加减）。

        失败返回 None（UI 保留原值）。

        输入自适应：mic 按设备原生采样率/声道录制（下混单声道后一次性
        重采样到 48k 再互相关），44.1k 麦克风也可直接校准；far 端
        far=扬声器走 loopback 原生 MixFormat（行内重采样）、far=麦克风
        走自适应 MicCaptureWin（对外恒 48k）。
        """
        if pyaudio is None or IS_LINUX:
            _module_log("[AEC] 校准当前仅支持 Windows 本地 WASAPI")
            return None
        import threading as _threading
        import time as _time
        from pvplatform import IS_WINDOWS as _IS_WINDOWS
        if not _IS_WINDOWS:
            _module_log("[AEC] 校准当前仅支持 Windows 本地 WASAPI")
            return None
        from pvengine.aec_calib import make_probe as _make_probe, \
            estimate_far_delay_ms as _estimate_delay

        # ── 1. 设备解析（与主设备选择同一模糊匹配）──
        api = default_api_type()
        mic_id = get_device_id(mic_dev, True, api)
        if mic_id is None:
            _module_log(f"[AEC] 校准失败：麦克风设备无法解析: {mic_dev!r}")
            return None
        if far_kind == "mic":
            far_id = get_device_id(far_dev, True, api)
            if far_id is None:
                _module_log(f"[AEC] 校准失败：far 麦克风设备无法解析: {far_dev!r}")
                return None
        else:
            far_kind = "speaker"
        # chirp 播到目标端点：far=扬声器用所选端点（loopback 同源）；
        # far=麦克风时端点未知，退回系统默认渲染设备。
        out_idx = get_device_id(far_dev, False, api) \
            if far_kind == "speaker" else None

        probe = _make_probe(SAMPLE_RATE)   # 单发上扫 chirp（无重复假峰）
        probe_sec = len(probe) / float(SAMPLE_RATE)

        pa = pyaudio.PyAudio()
        # ── mic 原生采样率/声道（与主输入同一自适应机制）──
        from pvplatform.audio.pa_backend import downmix_mono as _downmix, \
            native_hop_len as _native_hop
        mic_sr, mic_ch = SAMPLE_RATE, 1
        try:
            _minfo = pa.get_device_info_by_index(mic_id)
            mic_sr = int(round(float(
                _minfo.get('defaultSampleRate') or SAMPLE_RATE)))
            mic_ch = max(1, int(_minfo.get('maxInputChannels') or 1))
        except Exception:
            pass
        mic_ratio = SAMPLE_RATE / float(mic_sr) \
            if mic_sr != SAMPLE_RATE else 1.0
        if mic_ratio != 1.0:
            _module_log(f"[AEC] 校准 mic 自适应: {mic_ch}ch {mic_sr}Hz → "
                        f"48kHz 重采样")
        rec_samples = int(mic_sr * (probe_sec + 1.8))   # 设备域，前后各留余量
        mic_np = np.zeros(rec_samples, dtype=np.float32)
        mic_pos = [0]

        def _mic_cb(in_data, frame_count, time_info, status):
            mono = _downmix(np.frombuffer(in_data, dtype=np.float32),
                            mic_ch)
            n = min(len(mono), rec_samples - mic_pos[0])
            if n > 0:
                mic_np[mic_pos[0]:mic_pos[0] + n] = mono[:n]
                mic_pos[0] += n
            return (in_data, pyaudio.paContinue)

        mic_stream = None
        out_stream = None
        far_cap = None
        pump_thread = None
        try:
            # ── 2. far 采集（与运行时同一点）──
            if far_kind == "mic":
                from pvplatform.audio import create_mic_capture
                far_cap = create_mic_capture(far_id)
            else:
                far_cap = SpeakerCapture(device_name=far_dev)
            if far_cap is None or not far_cap.start():
                _module_log(f"[AEC] 校准失败：far 采集启动失败 ({far_dev!r})")
                return None

            # ── 3. 开输出/输入流（mic 先不启动）──
            mic_stream = pa.open(
                format=pyaudio.paFloat32, channels=mic_ch, rate=mic_sr,
                input=True, input_device_index=mic_id,
                frames_per_buffer=_native_hop(mic_sr),
                stream_callback=_mic_cb)
            if out_idx is not None:
                try:
                    out_stream = pa.open(
                        format=pyaudio.paFloat32, channels=1,
                        rate=SAMPLE_RATE, output=True,
                        output_device_index=out_idx,
                        frames_per_buffer=1024)
                except Exception as e:
                    _module_log(f"[AEC] 校准输出流打开失败，改用默认输出: {e}")
                    out_stream = None
            if out_stream is None:
                out_stream = pa.open(
                    format=pyaudio.paFloat32, channels=1, rate=SAMPLE_RATE,
                    output=True, frames_per_buffer=1024)

            # ── 4. 同步起点：flush far 环 → 立刻启动 mic 录音 → pump far ──
            _time.sleep(0.3)                 # far 采集先稳定运行
            far_cap.flush()
            mic_pos[0] = 0
            mic_np[:] = 0.0
            mic_stream.start_stream()
            far_list: list = []
            _stop_pump = _threading.Event()

            def _pump():
                while not _stop_pump.is_set():
                    try:
                        n = far_cap.available()
                        if n > 0:
                            got = far_cap.read(n)
                            if got:
                                far_list.extend(got)
                    except Exception:
                        pass
                    _time.sleep(0.005)
            pump_thread = _threading.Thread(target=_pump, daemon=True)
            pump_thread.start()

            _time.sleep(0.2)                 # 起点静音预卷
            out_stream.start_stream()
            step = 1024
            for i in range(0, len(probe), step):
                out_stream.write(probe[i:i + step].tobytes())
            # 等回声尾巴进 mic + pump 收完
            deadline = _time.time() + probe_sec + 1.6
            while mic_pos[0] < rec_samples and _time.time() < deadline:
                _time.sleep(0.01)
        except Exception as e:
            import traceback as _tb
            _module_log(f"[AEC] 校准采集失败: {e}")
            _module_log(_tb.format_exc())
            return None
        finally:
            _stop_pump.set()
            if pump_thread is not None:
                pump_thread.join(timeout=1.0)
            try:
                if mic_stream is not None:
                    mic_stream.stop_stream()
                    mic_stream.close()
            except Exception:
                pass
            try:
                if out_stream is not None:
                    out_stream.stop_stream()
                    out_stream.close()
            except Exception:
                pass
            pa.terminate()
            if far_cap is not None:
                try:
                    far_cap.stop()
                except Exception:
                    pass

        # ── 5. far↔mic 互相关求相对延迟（统一 48k 域）──
        mic_arr = np.asarray(mic_np[:mic_pos[0]], dtype=np.float32)
        if mic_ratio != 1.0:
            from pvengine import Resampler as _Resampler
            _mrs = _Resampler()
            _mrs.process([0.0] * _native_hop(mic_sr), mic_ratio)  # 预热
            mic48 = _mrs.process(mic_arr.tolist(), mic_ratio, True)  # 冲刷尾部
            mic_arr = np.asarray(mic48, dtype=np.float32)
        far_arr = np.asarray(far_list, dtype=np.float32)
        if len(mic_arr) < SAMPLE_RATE * 0.2 or len(far_arr) < SAMPLE_RATE * 0.1:
            _module_log("[AEC] 校准失败：录音数据不足 "
                        f"(mic={len(mic_arr)} far={len(far_arr)})")
            return None
        if far_cap is not None and far_cap.dev_sr != SAMPLE_RATE:
            from pvengine import Resampler as _Resampler
            ratio = SAMPLE_RATE / float(far_cap.dev_sr)
            rs = _Resampler()
            rs.process(np.zeros(480, dtype=np.float32), ratio)
            far48 = rs.process(list(far_arr), ratio)
            far_arr = np.asarray(far48, dtype=np.float32)
        _module_log(f"[AEC] 校准录音: mic={len(mic_arr)} far={len(far_arr)} "
                    f"mic_sr={mic_sr}Hz "
                    f"far_sr={far_cap.dev_sr if far_cap else '?'}Hz")

        res = _estimate_delay(far_arr.tolist(), mic_arr.tolist(),
                              fs=SAMPLE_RATE)
        if res is None:
            _module_log("[AEC] 校准失败：未检测到可靠回声峰（音量过低/设备错误？）")
            return None
        delay_ms, diag = res
        delay_ms = max(0.0, min(1000.0, delay_ms))
        _module_log(f"[AEC] 校准完成: far_delay={delay_ms:.1f}ms "
                    f"(回声路径唯一值，硬件不变则恒定；互相关前已按探测音"
                    f"频带做零相位带通去噪) corr={diag['corr']:.2f} "
                    f"snr={diag['snr']:.1f}")
        return round(delay_ms, 1)

    def _validate_and_fix_device(self, device_id: int, want_input: bool) -> int:
        """验证设备 ID 是否仍然存在；如已拔出则自动查找备选设备。
        
        返回原 ID（若有效）或备选设备 ID。若无任何可用设备则返回 None。
        
        NOTE: 使用重试机制避免 WASAPI 瞬态枚举失败误判设备拔出。
        """
        if self._p is None or device_id < 0:
            return None

        # 重试：PortAudio 在设备枚举期间可能瞬态失败，连续失败才判拔出
        RETRIES = 5
        RETRY_DELAY = 0.05  # 50ms
        for attempt in range(RETRIES):
            try:
                self._p.get_device_info_by_index(device_id)
                return device_id  # 设备仍在
            except Exception:
                if attempt < RETRIES - 1:
                    time.sleep(RETRY_DELAY)
        # 全部重试失败 — 设备确实已拔出 → 查找备选
        fallback = None
        try:
            for i in range(self._p.get_device_count()):
                try:
                    info = self._p.get_device_info_by_index(i)
                except Exception:
                    continue
                if want_input and info.get('maxInputChannels', 0) > 0:
                    fallback = i
                    break
                if not want_input and info.get('maxOutputChannels', 0) > 0:
                    fallback = i
                    break
        except Exception:
            pass
        if fallback is not None:
            try:
                name = self._p.get_device_info_by_index(fallback).get('name', str(fallback))
            except Exception:
                name = str(fallback)
            _module_log(
                f"[设备] {'输入' if want_input else '输出'}设备 {device_id} 已拔出，"
                f"自动切换到 {name}")
        return fallback

    def _create_stream(self) -> None:
        """打开传输后端并装配播放 sink（一个输出路一个 PlaybackSink）。

        后端为哑传输（搬运 + 设备时钟回调），见 pvplatform.audio；
        跨时钟域播放正确性在 PlaybackSink（pvengine.dsp.playback）。
        """
        hop = self._hop_length

        # Linux：PwBridge（libpulse 原生流），F32 单声道 48000Hz
        if IS_LINUX and self._use_pw:
            in_names, out_names = self._pw_ports
            self._sinks = [PlaybackSink(hop=hop) for _ in out_names]
            bridge = _PwBridge()
            if not bridge.open(in_names, out_names,
                               [s.pull for s in self._sinks]):
                err = bridge.last_error()
                bridge.close()
                raise OSError(f"PipeWire 连接失败: {err}")
            self._bridge = bridge
            _module_log(f"[PipeWire] 输入x{len(in_names)}: {','.join(in_names) or '(未选)'}  "
                        f"输出x{len(out_names)}: {','.join(out_names) or '(未选)'}")
            _module_log("[PipeWire] F32 单声道 48000Hz 协商（PipeWire 负责重采样/声道转换）")
            return

        if IS_LINUX:
            # Linux 强制原生 PipeWire：不落 PortAudio/PyAudio 备选
            raise OSError("Linux 音频仅支持原生 PipeWire（未配置输入/输出节点或 pvpipe 不可用）")

        # Windows：PaBridge（PortAudio 输入/输出独立流，每输出一个 pull）
        self._p = pyaudio.PyAudio()
        if self._p is None:
            raise OSError("PyAudio 初始化失败")

        in_id = self._input_id
        out_id = self._validate_and_fix_device(self._output_id, want_input=False)
        out_ids = [out_id]      # None = 系统默认输出（PaBridge 透传）
        for dev in self._extra_out_ids:
            fixed = self._validate_and_fix_device(dev, want_input=False)
            if fixed is not None and fixed not in out_ids:
                out_ids.append(fixed)

        if in_id is not None:
            in_id = self._validate_and_fix_device(in_id, want_input=True)

        self._sinks = [PlaybackSink(hop=hop) for _ in out_ids]
        bridge = _PaBridge()
        if not bridge.open(in_id, out_ids, [s.pull for s in self._sinks],
                           p=self._p):
            err = bridge.last_error()
            bridge.close()
            raise OSError(f"音频流打开失败: {err}")
        self._bridge = bridge
        if in_id is None:
            _module_log(f"[输出] x{len(out_ids)}（网络输入模式）")

    # ── 统一处理循环 ────────────────────────────────────────────────
    #
    # 时钟模型（2026-09 播放重构沉淀，全部路径唯一）：
    #   设备时钟是唯一主时钟——播放侧由设备回调经 PlaybackSink.pull 拉帧，
    #   速率差/调度抖动由 sink 变速消化（pvengine.dsp.playback：PI 伺服
    #   ASRC ±3%、预热、欠载静音重同步、封顶丢最旧）。处理线程按 hop 推进
    #   （read → process → sinks.write），不做任何缓冲策略。
    #   历史病灶（已根除）：全双工单流内联处理 + 主输出帧长硬对齐 +
    #   额外输出手写 ASRC + 网络输出 drop/pad —— 5 路径 4 种时钟策略，
    #   修一处漏三处；现收敛为后端插件（哑）+ 一个 PlaybackSink。


    def run(self) -> None:
        """运行音频处理线程。"""
        self._stop_event.clear()
        try:
            self._create_stream()
            self._build_aec_rows()
            self._build_loopback_rows()
        except Exception as e:
            _module_log(f"[音频] 音频流创建失败（线程将退出）: {e}")
            import traceback as _tb
            _module_log(f"[音频] 堆栈: {_tb.format_exc()}")
            self._start_error = str(e)
            self._stop_aec_rows()
            self._stop_loopback_rows()
            self._ready_event.set()  # 通知等待方：失败
            return

        self._ready_event.set()  # 通知等待方：成功

        if self._ready_msg:
            _module_log(f"[启动] {self._ready_msg}")

        # 本地/网络同一处理循环（平台差异只在后端插件；纯媒体会话
        # 无设备输入不经本线程——EngineController 走 MediaSession）
        is_network = self._input_id is None and self._network_source is not None
        self._bridge_loop(is_network)

        _module_log("[DEV] 音频线程已退出")

    # ── 统一处理循环（本地/网络同一实现）────────────────────────────

    def _network_reader(self) -> Optional[List[float]]:
        """网络源读侧：flush / 突发硬顶 / 断流垫零 → 返回一个 hop 或 None。

        速率差稳态由 PlaybackSink 伺服消化，这里只兜突发（硬顶截断）
        与断流（150ms 无数据：残帧淡出补零成 hop，交给链推进状态）。
        """
        MAX_ACC = HOP_LENGTH * 8            # 硬上限 ~80ms
        TARGET_ACC = HOP_LENGTH * 5         # 目标缓冲 ~50ms
        STALL_TIMEOUT = 0.15                # 断流判定
        src = self._network_source
        acc = self._net_acc
        if src and src.flush_event.is_set():
            src.flush_event.clear()
            acc.clear()
            self._flush_sinks()
            _module_log("[网络] 缓冲已清空 (flush)")
            self._net_last_data = time.time()
        if src:
            navail = src.available()
            if navail > 0:
                chunk = src.read(navail)
                if chunk:
                    acc.extend(chunk)
                    self._net_last_data = time.time()
        if len(acc) > MAX_ACC:
            acc[:] = acc[-TARGET_ACC:]      # 突发兜底（稳态漂移由 sink 消化）
        stall = time.time() - self._net_last_data
        if stall > STALL_TIMEOUT and 0 < len(acc) < HOP_LENGTH:
            fade_len = min(64, len(acc))
            for i in range(fade_len):
                acc[-fade_len + i] *= 1.0 - (i + 1) / (fade_len + 1)
            acc.extend([0.0] * (HOP_LENGTH - len(acc)))
        if len(acc) < HOP_LENGTH:
            return None
        hop = acc[:HOP_LENGTH]
        del acc[:HOP_LENGTH]
        return hop

    def _flush_sinks(self) -> None:
        """清空全部播放 sink（回预热态，续播前重同步）。"""
        for sink in self._sinks:
            sink.reset()

    def _deliver(self, out: List[float]) -> None:
        """一帧产出 → 全部播放 sink。

        线性多出：链内 output 抽头各取所在位置的信号
        （take_output_frames 与 sink 一一对应）；无抽头/抽头空回退
        最近非空帧，再兜底主输出帧——sink 恒有喂，不进饥饿态。
        """
        out_frames = self.processor.take_output_frames()
        if self._sinks and len(out_frames) == len(self._sinks):
            last = None
            for sink, f in zip(self._sinks, out_frames):
                frame = f if f else last
                if not frame:
                    frame = out
                last = frame
                sink.write(frame)
        else:
            for sink in self._sinks:
                sink.write(out)

    def _read_mix(self) -> Optional[List[float]]:
        """统一本地读侧：逐路取 hop(带采集时间戳) → 各 AEC 行（far 按外部时钟
        网格配对直达 + AEC）→ 回环输入行（网格逐 hop 出队进混音）→ 等权混音。

        AEC far/mic 按**外部时钟**配对：mic 侧带采集时间戳（PortAudio adc→
        QPC 映射 / QPC），far 侧按时间戳入 48k 网格历史；far 参考 = 该 mic
        时刻 − far_delay 的历史段，谁先到/晚到只影响有没有样本，不影响对齐。
        far 历史不足时该行直通 mic（不丢人声、不进模型）。
        """
        ent = self._bridge.read_each_ts(HOP_LENGTH) \
            if self._bridge is not None else None
        hops: list = []
        mts: list = []
        if ent:
            for e in ent:
                if e is None:
                    hops.append(None)
                    mts.append(None)
                else:
                    ts0, hop = e
                    hops.append(hop)
                    mts.append(ts0)
        n_mic = len(hops)   # 本轮固定分母（混音归一化用，见末尾）
        for live in self._aec_live:
            idx = live["mic_index"]
            hop = hops[idx] if 0 <= idx < len(hops) else None
            if hop is None:
                continue
            cap = live["capture"]
            navail = cap.available()
            if navail > 0:
                got = cap.read_ts(navail)
                if got is not None:
                    live["row"].push_far_ts(got[0], got[1])
            out_hop = live["row"].process_mic(hop, mts[idx] or 0.0)
            hops[idx] = out_hop.tolist()
            # AEC VU：每 hop 计算 mic/far/out 峰值（UI 线程直接读 float，无需加锁）
            mic_pk = max(abs(x) for x in hop) if hop else 0.0
            far_pk = max(abs(x) for x in live["row"].last_far)
            out_pk = max(abs(x) for x in out_hop)
            vu = self._aec_vu.get(live["mic"])
            if vu is not None:
                vu["mic"] = mic_pk
                vu["far"] = far_pk
                vu["out"] = out_pk
        for lb in self._loopback_live:
            cap = lb["capture"]
            navail = cap.available()
            if navail > 0:
                got = cap.read_ts(navail)
                if got is not None:
                    if lb["rs"] is not None:
                        seq = lb["rs"].process(list(got[1]), lb["ratio"])
                    else:
                        seq = got[1]
                    # 回环行只需连续流（不做跨时钟配对）：按时间戳入网格会因
                    # 取整产生周期性时基抖动（轻微嘀嗒毛刺），故按连续流推入
                    lb["hist"].push_contig(seq)
            # 预充 3 个 hop 再开始消费：回环生产者与消费者都是 ~1 hop/10ms，
            # 不预充则水位恒在 0 附近，任何包到达抖动都会让 pop 返回 None，
            # 该 hop 便插入静音——听感即持续「卡卡卡」。预充把这层抖动吃掉。
            if not lb["primed"]:
                hist = lb["hist"]
                eg, sg = hist.end_grid(), hist.start_grid()
                if eg is None or sg is None or (eg - sg) < HOP_LENGTH * 3:
                    continue
                lb["primed"] = True
            hop_lb = lb["hist"].pop_hop(HOP_LENGTH)
            if hop_lb is not None:
                hops.append(hop_lb)
        chunks = [h for h in hops if h is not None]
        if not chunks:
            return None
        # 归一化分母取「本轮配好的输入路数」（mic 路 + 回环行数），不随
        # 某路本 hop 是否有数据而变：否则回环行时有时无会让整体增益在
        # 1 与 1/N 之间逐 hop 跳变，听感即「丝丝拉拉」的拉链噪声。
        k = 1.0 / max(1, n_mic + len(self._loopback_live))
        if len(chunks) == 1:
            return [v * k for v in chunks[0]]
        acc = [0.0] * len(chunks[0])
        for c in chunks:
            for i, v in enumerate(c):
                acc[i] += v
        return [v * k for v in acc]

    def _bridge_loop(self, network: bool) -> None:
        """统一处理循环：read(hop) → process → sinks.write。

        本地模式走 _read_mix（逐路取 hop → AEC 行独立处理 →
        回环输入行拉齐 → 等权混音，无 AEC/回环时退化为 plain 混音）；
        网络模式 read 自 _network_reader。处理/可视化/录音钩子路径共用，
        平台差异只在后端插件。每 ~2s 检查后端健康，流死即退出
        （上层走会话重启路径）。
        """
        d = self._diag
        last_viz = 0.0
        t_last_health = time.time()
        t_aec_diag_end = time.time() + 60.0   # AEC 诊断只在启动后前 60s 打印
        while not self._stop_event.is_set():
            if network:
                data = self._network_reader()
            else:
                data = self._read_mix()
            if not data:
                time.sleep(0.002)
                continue

            t0 = time.perf_counter()
            chunk = data if len(data) == HOP_LENGTH else data[-HOP_LENGTH:]

            if not network and self._aec_live:
                out = self._process_frame(chunk)
                # ── AEC 诊断：每 100 帧 (1s) 一行，仅启动后前 60s ──
                self._aec_diag_count += 1
                if self._aec_diag_count >= 100 and \
                        time.time() <= t_aec_diag_end:
                    self._aec_diag_count = 0
                    for live in self._aec_live:
                        row = live["row"]
                        sd = row.diag()
                        far = row.last_far
                        far_rms = (sum(x * x for x in far) / len(far)) ** 0.5 \
                            if len(far) else 0.0
                        out_rms = (sum(x * x for x in chunk) / len(chunk)) ** 0.5
                        _module_log(
                            "[AEC] mic=%s far(%s)=%s | far_hist=%d drop=%d "
                            "resync=%d | far=%.4f out=%.4f cache=%.1f"
                            % (live["mic"], live["far_kind"],
                               live["far_device"], sd["len"], sd["drops"],
                               sd["resync"], far_rms, out_rms,
                               sd["cache_norm"]))
            elif network:
                out = self.processor.process_pipeline(chunk)
            else:
                out = self._process_frame(chunk)
            dt = time.perf_counter() - t0
            d["fc"] += 1
            d["t_sum"] += dt
            d["t_max"] = max(d["t_max"], dt)

            if out:
                if not network:
                    # 录音捕获：降噪后的音频（TSE 前）
                    if self._recording_hook is not None:
                        try:
                            self._recording_hook(list(out))
                        except Exception:
                            pass
                    # TSE 录音钩子兜底（挂了 recording_hook 时不重复喂）
                    if self._tse_hook is not None \
                            and self._recording_hook is None:
                        try:
                            pre_tse = self.processor.get_tse_recording_audio()
                            if pre_tse:
                                self._tse_hook(list(pre_tse))
                        except Exception:
                            pass
                    if self._viz_enabled:
                        try:
                            self._spectrum_in.write(
                                self.processor.process_eq_only(list(chunk)))
                        except Exception:
                            self._spectrum_in.write(list(chunk))
                        self._spectrum_out.write(list(out))
                    self._vu_peak = max(abs(x) for x in out)
                else:
                    # 网络可视化：管线抽头（process_pipeline 内启用），50ms 节流
                    now_v = time.time()
                    if self._viz_enabled and now_v - last_viz > 0.05:
                        viz_in = self.processor.get_and_clear_viz_input()
                        if viz_in:
                            self._spectrum_in.write(viz_in)
                        viz_out = self.processor.get_and_clear_viz_output()
                        if viz_out:
                            self._vu_peak = max(abs(x) for x in viz_out)
                            self._spectrum_out.write(viz_out)
                        last_viz = now_v

                self._deliver(out)

            now = time.time()
            if now - d["t0"] >= 60.0:
                self._loop_diag()
            if now - t_last_health > 2.0:
                t_last_health = now
                if self._bridge is not None and not self._bridge.active():
                    _module_log("[音频] 传输流已停止（设备断开/拔出）")
                    break

    def _loop_diag(self) -> None:
        """60s 一行健康汇总：处理耗时 / sink 水位 / 欠载·垫零·丢最旧。

        正常值：fps≈100、平均处理 <10ms、欠载/垫/丢 = 0（稳态速率差
        由 sink 伺服消化，不产生任何补零或丢弃）。
        """
        d = self._diag
        now = time.time()
        win = max(1e-6, now - d["t0"])
        fps = d["fc"] / win
        avg_ms = d["t_sum"] / max(1, d["fc"]) * 1000.0
        max_ms = d["t_max"] * 1000.0
        tot_pads = tot_drops = tot_urs = 0
        min_level = -1
        for s in self._sinks:
            g = s.diag()
            tot_pads += g["pads"]
            tot_drops += g["drops"]
            tot_urs += g["underruns"]
            if min_level < 0 or g["level"] < min_level:
                min_level = g["level"]
        _module_log(
            "[诊断] 处理循环健康（60s）: fps=%.1f/100 平均=%.2fms 最大=%.2fms "
            "sink最小水位=%d 欠载=%d 垫零=%d 丢最旧=%d (正常: 欠载/垫/丢=0)"
            % (fps, avg_ms, max_ms, min_level,
               tot_urs - d["s_urs"], tot_pads - d["s_pads"],
               tot_drops - d["s_drops"]))
        d.update(fc=0, t_sum=0.0, t_max=0.0, t0=now,
                 s_pads=tot_pads, s_drops=tot_drops, s_urs=tot_urs)

    def stop(self) -> None:
        """优雅地停止音频线程。"""
        self._stop_event.set()
        if self.is_alive():
            self.join(timeout=2.0)
        self._cleanup()

    def _cleanup(self) -> None:
        """释放音频资源（AEC/回环行采集 + 后端流 + PyAudio 实例）。"""
        self._stop_aec_rows()
        self._stop_loopback_rows()
        if self._bridge is not None:
            try:
                self._bridge.close()
            except Exception as e:
                _module_log(f"[音频] 后端关闭异常: {e}")
            self._bridge = None
        self._sinks = []
        if self._p is not None:
            try:
                self._p.terminate()
            except Exception as e:
                _module_log(f"[音频] _cleanup() PortAudio 终止异常: {e}")
            self._p = None

    def set_tse_audio_hook(self, hook: Optional[Callable[[List[float]], None]]) -> None:
        """设置 TSE 音频钩子回调函数"""
        self._tse_hook = hook

    def set_recording_hook(self, hook: Optional[Callable[[List[float]], None]]) -> None:
        """设置录音捕获钩子（捕获降噪后、TSE 前的音频）"""
        self._recording_hook = hook

    def set_tse_enabled(self, enabled: bool) -> None:
        """动态启用/禁用 TSE 处理（委托给 引擎 AudioProcessor）"""
        if hasattr(self.processor, 'set_tse_enabled'):
            self.processor.set_tse_enabled(enabled)

    def set_recording_enabled(self, enabled: bool) -> None:
        """启用/禁用录音模式（委托给 引擎 AudioProcessor）"""
        if hasattr(self.processor, 'set_recording_enabled'):
            self.processor.set_recording_enabled(enabled)

    def is_recording_enabled(self) -> bool:
        if hasattr(self.processor, 'is_recording_enabled'):
            return self.processor.is_recording_enabled()
        return False

    def is_tse_reference_loaded(self) -> bool:
        """检查 TSE 参考音频是否已加载"""
        if hasattr(self.processor, 'is_tse_reference_loaded'):
            return self.processor.is_tse_reference_loaded()
        return False


# ═══════════════════════════════════════════════════════════════
#  TSE 参考音频工具（录音器 / WAV 转换）
# ═══════════════════════════════════════════════════════════════

TSE_SAMPLE_RATE = 48000           # TSE 模型要求 48kHz
HOOK_SAMPLE_RATE = 48000
HOOK_HOP_LENGTH = 480

RECORD_DURATION = 10.0            # 参考录音总时长（秒）

CFG_REF_WAV_PATH = "tse_reference_wav_path"   # 参考音频 WAV 路径 config 键


def _samples_to_wav_bytes(audio: List[float], sr: int = TSE_SAMPLE_RATE) -> bytes:
    """float 样本列表 → 单声道 16bit WAV bytes。"""
    buf = io.BytesIO()
    with wave.open(buf, 'wb') as wf:
        wf.setnchannels(1)
        wf.setsampwidth(2)
        wf.setframerate(sr)
        clamped = [max(-1.0, min(1.0, s)) for s in audio]
        ints = [int(s * 32767) for s in clamped]
        wf.writeframes(struct.pack(f'{len(ints)}h', *ints))
    return buf.getvalue()


class _Recorder:
    """参考音频录音器：处理流钩子喂入，10 秒后取回。"""

    __slots__ = ('_buf', '_lock', '_active', '_start_time')

    def __init__(self):
        self._buf: List[float] = []
        self._lock = threading.Lock()
        self._active = False
        self._start_time = 0.0

    def feed(self, samples: List[float]):
        if not self._active:
            return
        with self._lock:
            self._buf.extend(samples)
            cap = int(RECORD_DURATION * HOOK_SAMPLE_RATE) + HOOK_HOP_LENGTH * 2
            if len(self._buf) > cap:
                self._buf = self._buf[-cap:]

    def start(self):
        with self._lock:
            self._buf.clear()
            self._active = True
            self._start_time = time.time()

    def wait_and_get(self) -> Optional[List[float]]:
        if not self._active:
            return None
        deadline = self._start_time + RECORD_DURATION + 0.5
        while time.time() < deadline:
            if time.time() - self._start_time >= RECORD_DURATION:
                break
            time.sleep(0.05)
        with self._lock:
            self._active = False
            need = int(RECORD_DURATION * HOOK_SAMPLE_RATE)
            if len(self._buf) >= need:
                audio = self._buf[-need:]
                self._buf.clear()
                return audio
            if len(self._buf) > 0:
                audio = self._buf[:]
                self._buf.clear()
                return audio + [0.0] * (need - len(audio))
            return None


_recorder = _Recorder()


def get_tse_recorder() -> _Recorder:
    """返回 TSE 参考录音器单例。"""
    return _recorder


def register_tse_audio_hook(thread, log: Callable):
    """把线程的处理后音频钩子接到参考录音器（TSE 录音用）。"""
    if thread is None:
        return
    try:
        thread.set_tse_audio_hook(lambda s: _recorder.feed(list(s)))
    except AttributeError:
        log("[TSE] 钩子注册失败")


def load_tse_reference(processor, wav_path: str) -> bool:
    """加载 TSE 参考音频到处理器，成功返回 True。

    09c 契约: 注册取自然 10s（不足由引擎 fix_ref 平铺），不再做 WSOLA 压缩
    （10s→2s 是 tse15/2s 时代的遗留，对 10s 全帧 key 契约是错的）。
    enr_tok 结果缓存为 <wav>_enrtok.npz，键 = 录音 mtime/size + ref_encoder
    mtime/size —— 录音未变且模型版本未变时启动直接载入缓存（跳过
    STFT+ref_encoder 处理）；录音变了或换模型版本自动失效重算。
    """
    if not wav_path or not os.path.exists(wav_path):
        _module_log(f"[TSE] 参考 WAV 未找到: {wav_path!r}")
        return False

    try:
        from wav_io import read_wav
        audio, sr = read_wav(wav_path)
        if sr != TSE_SAMPLE_RATE:
            r = Resampler()
            audio = r.process(audio, float(TSE_SAMPLE_RATE) / sr, True)
    except Exception as e:
        _module_log(f"[TSE] 参考 WAV 加载失败: {e}")
        return False

    try:
        if hasattr(processor, 'set_tse_reference'):
            processor.set_tse_reference(audio, ref_key=wav_path)
        return True
    except Exception as e:
        _module_log(f"[TSE] 设置参考失败: {e}")
        return False


def create_audio_processor(pre_gain_db: float = 0.0):
    """创建音频处理器实例（pvengine 纯 Python 组件化引擎）。

    模型路径由插件内部按 model_config 常量解析，无需外部传入。
    """
    if not ENGINE_AVAILABLE:
        raise RuntimeError("pvengine not available")
    return AudioProcessor(pre_gain_db)


def start_audio_stream(input_id: Optional[int], output_id: int,
                       processor: object, hop_length: Optional[int] = None,
                       network_source = None,
                       api_type: int = 13,
                       ready_msg: str = "",
                       extra_output_ids=None,
                       pw_ports: Tuple[List[str], List[str]] = ([], []),
                       aec_rows=None,
                       aec_inputs=None,
                       loopbacks=None) -> AudioThread:
    """启动音频流并返回线程实例。

    参数:
        input_id: 输入设备 ID（网络输入模式下为 None）。
        output_id: 输出设备 ID。
        processor: pvengine.AudioProcessor 实例（处理器，直接使用）。
        hop_length: 处理 hop 长度（默认 480，10ms @48kHz）。
        network_source: 网络输入模式下的 RemoteAudioSource（可选）。
        ready_msg: 音频流就绪后由 AudioThread 记录的日志消息。
        extra_output_ids: 额外输出设备 ID 列表（仅 Windows PortAudio 路径，
            多输出扇出同一路降噪音频）。
        pw_ports: Linux PipeWire 模式的 (输入节点列表, 输出节点列表)；
            多输入自动混音、多输出扇出同一路降噪音频。
        aec_rows: 行级 AEC 配置（SessionPlan.aec_rows 原样）；
            须在线程 start 前就位，随流装配。
        aec_inputs: 建桥输入顺序（与 read_each 对齐，供 AEC 行定位 mic）。
        loopbacks: 回环输入行（SessionPlan.loopbacks 原样），随流装配。
    """
    if hop_length is None:
        hop_length = HOP_LENGTH
    thread = AudioThread(input_id, output_id, processor.process, hop_length,
                         processor, network_source=network_source,
                         api_type=api_type, ready_msg=ready_msg,
                         extra_output_ids=extra_output_ids)
    if any(pw_ports[0]) or any(pw_ports[1]):
        thread.set_pw_ports(pw_ports[0], pw_ports[1])
    thread.set_aec_rows(aec_rows, aec_inputs if aec_inputs is not None
                        else (list(pw_ports[0]) if pw_ports else []))
    thread.set_loopback_rows(loopbacks)
    thread.start()
    return thread


# ═══════════════════════════════════════════════════════════════
#  音频设备枚举 & Core Audio 工具（原 audio_device.py；平台感知）
# ═══════════════════════════════════════════════════════════════

# 平台感知的设备 API。数值在别的平台由 device_api 回退到该平台默认 host API。
from pvplatform.audio import device_api as _device_api


def default_api_type() -> int:
    """当前平台默认的 PortAudio host API 类型。"""
    return _device_api.platform_default_api_type()


def _get_host_api_indices(p: Any, api_type: int) -> List[int]:
    """获取指定 API 类型的 host API 索引列表（平台感知，按名字匹配）。"""
    return _device_api.get_host_api_indices(p, api_type)


def get_device_names(api_type: int = None) -> Tuple[List[str], List[str]]:
    """获取设备名称列表（已去重）。api_type 为 None 时用平台默认。

    Linux：原生 PipeWire 节点枚举（api_type==API_PIPEWIRE）。
    Windows/macOS：只枚举所选 host API（`_get_host_api_indices` 分级匹配，
    如 WASAPI / MME）下的设备，避免混入其它 host API 的重复/无关端点，
    设备按方向区分。
    """
    if api_type is None:
        api_type = default_api_type()
    if IS_LINUX:
        return _pw_sources(), _pw_dests()
    p = pyaudio.PyAudio()
    try:
        host_api_indices = _get_host_api_indices(p, api_type)
        input_names: List[str] = []
        output_names: List[str] = []
        for i in range(p.get_device_count()):
            try:
                dev = p.get_device_info_by_index(i)
            except Exception:
                continue
            if dev['hostApi'] not in host_api_indices:
                continue
            name = _device_api.fix_device_name(dev['name']).strip()
            if dev['maxInputChannels'] > 0 and name not in input_names:
                input_names.append(name)
            if dev['maxOutputChannels'] > 0 and name not in output_names:
                output_names.append(name)
        return input_names, output_names
    finally:
        p.terminate()


def get_device_id(device_name: str, is_input: bool, api_type: int = None) -> Optional[int]:
    """按设备名获取设备索引（与主设备选择同一名字模糊匹配实现）。

    名字匹配与 AEC 校准共用 `device_api.best_name_match`（归一化精确 →
    前缀 → 相似度模糊），方向过滤避免返回同名输出端点；配置存的旧名/
    跨来源名（PortAudio vs WASAPI）找不到时回退第一个可用设备。
    Linux 走原生 PipeWire（node.name 直接使用，不需要 PortAudio 索引），
    返回 None。
    """
    if api_type is None:
        api_type = default_api_type()
    # Linux：输入/输出都是 PipeWire 节点名，直接使用，无 PortAudio 索引
    if IS_LINUX:
        return None
    try:
        input_names, output_names = get_device_names(api_type=api_type)
    except Exception:
        input_names, output_names = [], []
    target_names = input_names if is_input else output_names

    matched_name = _device_api.best_name_match(device_name, target_names)
    if matched_name is None and target_names:
        # 兼容：配置里存的设备名被过滤/已不存在时，回退到第一个可用设备
        matched_name = target_names[0]
    if not matched_name:
        raise ValueError(f"Device not found")

    p = pyaudio.PyAudio()
    try:
        host_api_indices = _get_host_api_indices(p, api_type)
        for i in range(p.get_device_count()):
            try:
                dev = p.get_device_info_by_index(i)
            except Exception:
                continue
            if dev['hostApi'] not in host_api_indices:
                continue
            if _device_api.fix_device_name(dev['name']).strip() == matched_name:
                if is_input and dev.get('maxInputChannels', 0) <= 0:
                    continue
                if not is_input and dev.get('maxOutputChannels', 0) <= 0:
                    continue
                return i

        raise ValueError(f"Device '{matched_name}' ID not found")
    finally:
        p.terminate()