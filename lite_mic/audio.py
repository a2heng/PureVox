# PureVox Lite Denoise Only — 音频流
# Copyright (C) 2024-2026 a2heng <752848283@qq.com>
# SPDX-License-Identifier: GPL-3.0-or-later
#
# 零复用：不 import audio_processor / pvplatform
# 48kHz 强制检测 + 前后增益 + 纯 Python 引擎

import threading
import numpy as np

from playback import PlaybackBuffer

try:
    import pyaudio
except ImportError:
    pyaudio = None

SAMPLE_RATE = 48000
HOP = 480          # 10ms @48kHz (202609 模型契约)
FORMAT = pyaudio.paFloat32 if pyaudio else None
CHANNELS = 1

def db_to_linear(db):
    return 10.0 ** (db / 20.0)

def fix_device_name(name):
    """修复 PortAudio 在中文 Windows 返回的乱码设备名。

    PortAudio 返回 UTF-8 字节、PyAudio 按 GBK 误读时得到乱码；按 GBK 编回
    再按 UTF-8 解即可还原（合法文本不受影响）。语义对齐主线 device_api。
    """
    if not name:
        return name
    try:
        fixed = name.encode("gbk").decode("utf-8")
    except (UnicodeEncodeError, UnicodeDecodeError):
        return name
    if fixed == name or "\ufffd" in fixed:
        return name
    return fixed


def _normalize_name(name):
    """设备名归一化（比较用）：转小写 + 丢弃单字符 token（去 (R) 等噪音）。"""
    import re
    if not name:
        return ""
    toks = [t for t in re.findall(r"\w+", name.lower()) if len(t) >= 2]
    return " ".join(toks)


def _name_similarity(a, b):
    """两个设备名的相似度（0~1）：序列比 + token 重叠 + 前缀包含。"""
    na, nb = _normalize_name(a), _normalize_name(b)
    if not na or not nb:
        return 0.0
    if na == nb:
        return 1.0
    import difflib
    ratio = difflib.SequenceMatcher(None, na, nb).ratio()
    ta, tb = set(na.split()), set(nb.split())
    overlap = 2.0 * len(ta & tb) / (len(ta) + len(tb))
    prefix = 0.9 if (na.startswith(nb) or nb.startswith(na)) else 0.0
    return max(ratio, overlap * 0.85, prefix)


def best_name_match(name, candidates):
    """在候选设备名里选最佳匹配（归一化精确 → 前缀 → 相似度 ≥0.6，对齐主线）。

    兼容旧 Lite 配置里带 "[WASAPI] " 前缀的显示名：先剥方括号前缀再比对。
    """
    if not name or not candidates:
        return None
    cands = [c for c in candidates if c]
    if not cands:
        return None
    if name.startswith("["):
        i = name.find("]")
        if i > 0:
            name = name[i + 1:].strip()
    na = _normalize_name(name)
    if not na:
        return cands[0]
    for c in cands:
        if _normalize_name(c) == na:
            return c
    for c in cands:
        if _normalize_name(c).startswith(na):
            return c
    best, best_s = None, 0.6
    for c in cands:
        s = _name_similarity(name, c)
        if s > best_s:
            best, best_s = c, s
    return best


def list_devices():
    """列举 WASAPI 输入/输出设备（对齐主线：只枚举单一 host API）。

    返回 [(纯设备名, PortAudio 索引, 属性行), ...]；设备名不含 API 前缀
    （Lite 只有 WASAPI，无需区分），重名加 "#n" 便于下拉唯一键。
    """
    if pyaudio is None:
        return [], []
    pa = pyaudio.PyAudio()
    ins, outs = [], []
    try:
        for i in range(pa.get_device_count()):
            try:
                info = pa.get_device_info_by_index(i)
            except Exception:
                continue
            try:
                api_info = pa.get_host_api_info_by_index(info.get("hostApi", 0))
                api_name = (api_info.get("name", "") or "").lower()
            except Exception:
                api_name = ""
            if "wasapi" not in api_name:
                continue        # 只要 WASAPI
            name = fix_device_name(info.get("name", "")).strip()
            if not name:
                continue
            # 属性行：通道 / 默认采样率 / 延迟
            ch = int(info.get("maxInputChannels", 0) or info.get("maxOutputChannels", 0) or 0)
            sr = int(info.get("defaultSampleRate", 0) or 0)
            try:
                lat = float(info.get("defaultLowInputLatency",
                                    info.get("defaultLowOutputLatency", 0)) * 1000)
                lat_s = f"{lat:.1f}ms"
            except Exception:
                lat_s = ""
            props = f"WASAPI · {ch}ch · {sr}Hz" + (f" · {lat_s}" if lat_s else "")
            if info.get("maxInputChannels", 0) > 0:
                ins.append((name, i, props))
            if info.get("maxOutputChannels", 0) > 0:
                outs.append((name, i, props))
    finally:
        pa.terminate()

    def dedup(lst):
        seen, out = {}, []
        for nm, idx, props in lst:
            cnt = seen.get(nm, 0)
            seen[nm] = cnt + 1
            out.append((nm, idx, props) if cnt == 0
                       else (f"{nm} #{cnt + 1}", idx, props))
        return out

    return dedup(ins), dedup(outs)

def try_open_48k(device_index, is_input):
    if pyaudio is None:
        return False, "PyAudio 未安装"
    pa = pyaudio.PyAudio()
    try:
        if is_input:
            ok = pa.is_format_supported(
                rate=SAMPLE_RATE,
                input_device=device_index,
                input_channels=CHANNELS,
                input_format=FORMAT,
                output_device=None,
                output_channels=None,
                output_format=None,
            )
        else:
            ok = pa.is_format_supported(
                rate=SAMPLE_RATE,
                input_device=None,
                input_channels=None,
                input_format=None,
                output_device=device_index,
                output_channels=CHANNELS,
                output_format=FORMAT,
            )
        pa.terminate()
        return True, ""
    except Exception as e:
        pa.terminate()
        return False, str(e)

class _InputRing:
    """输入回调 → 处理线程的单生产者/单消费者 FIFO（内部按 float32 字节存）。

    容量 = 200ms；超限丢最旧（回调线程被处理线程拖慢时不让缓冲无限增长）。
    """

    def __init__(self, capacity=SAMPLE_RATE // 5):
        self._buf = bytearray()
        self._cap = int(capacity) * 4
        self._cv = threading.Condition()

    def write(self, samples):
        # samples: np.float32 1-D
        with self._cv:
            self._buf += samples.tobytes()
            over = len(self._buf) - self._cap
            if over > 0:
                del self._buf[:over]
            self._cv.notify()

    def read(self, n, stop):
        """阻塞取 n 个样本；stop 置位且仍不足时返回 None。"""
        need = int(n) * 4
        with self._cv:
            while len(self._buf) < need and not stop.is_set():
                self._cv.wait(0.05)
            if len(self._buf) < need:
                return None
            raw = bytes(self._buf[:need])
            del self._buf[:need]
        return np.frombuffer(raw, dtype=np.float32).copy()

    def clear(self):
        with self._cv:
            self._buf.clear()

    def wake(self):
        with self._cv:
            self._cv.notify_all()


class LiteAudioStream:
    """麦克风 → 降噪 → 输出（与主线同构的时钟模型）。

    设备时钟是唯一主时钟：输入/输出拆成两条独立的 PortAudio 流，回调只搬运
    （输入回前增益进环 / 输出从 PlaybackBuffer 按 frame_count 取帧），推理在
    专用处理线程按 hop 推进（read → process → 写 sink）。ONNX 绝不在设备
    回调里跑；帧长抖动与跨设备速率差由 PlaybackBuffer 消化。
    """

    def __init__(self, in_idx, out_idx, engine, pre_db=0.0, post_db=0.0):
        self.in_idx = in_idx
        self.out_idx = out_idx
        self.engine = engine
        self.pre_gain = db_to_linear(pre_db)
        self.post_gain = db_to_linear(post_db)
        self._lock = threading.Lock()
        self._pa = None
        self._in_stream = None
        self._out_stream = None
        self._worker = None
        self._stop = threading.Event()
        self._running = False
        self._in_ring = _InputRing()
        self._sink = PlaybackBuffer(hop=HOP)

    def set_gains(self, pre_db, post_db):
        with self._lock:
            self.pre_gain = db_to_linear(pre_db)
            self.post_gain = db_to_linear(post_db)

    # ── 设备回调（只搬运，不做推理）──

    def _input_callback(self, in_data, frame_count, time_info, status):
        try:
            with self._lock:
                pre = self.pre_gain
            chunk = np.frombuffer(in_data, dtype=np.float32)
            self._in_ring.write(chunk * pre)
        except Exception:
            pass
        return (None, pyaudio.paContinue)

    def _output_callback(self, in_data, frame_count, time_info, status):
        try:
            data = np.asarray(self._sink.pull(frame_count), dtype=np.float32)
            return (data.tobytes(), pyaudio.paContinue)
        except Exception:
            return (np.zeros(frame_count, dtype=np.float32).tobytes(),
                    pyaudio.paContinue)

    # ── 处理线程（read hop → 引擎 → 后增益/限幅 → 写 sink）──

    def _worker_loop(self):
        while not self._stop.is_set():
            hop_in = self._in_ring.read(HOP, self._stop)
            if hop_in is None:
                continue
            try:
                with self._lock:
                    post = self.post_gain
                out = self.engine.process(hop_in) * post
                np.clip(out, -1.0, 1.0, out=out)
                self._sink.write(out)
            except Exception:
                pass

    def start(self):
        if pyaudio is None:
            raise RuntimeError("PyAudio 未安装")
        if self._running:
            return
        # 48k check
        ok, msg = try_open_48k(self.in_idx, True)
        if not ok:
            raise RuntimeError(f"输入设备不支持 48kHz: {msg}")
        ok, msg = try_open_48k(self.out_idx, False)
        if not ok:
            raise RuntimeError(f"输出设备不支持 48kHz: {msg}")
        self.engine.reset()
        self._stop.clear()
        self._in_ring.clear()
        self._sink.reset()
        self._pa = pyaudio.PyAudio()
        self._in_stream = self._pa.open(
            rate=SAMPLE_RATE, channels=CHANNELS, format=FORMAT, input=True,
            input_device_index=self.in_idx, frames_per_buffer=HOP,
            stream_callback=self._input_callback)
        self._out_stream = self._pa.open(
            rate=SAMPLE_RATE, channels=CHANNELS, format=FORMAT, output=True,
            output_device_index=self.out_idx, frames_per_buffer=HOP,
            stream_callback=self._output_callback)
        self._in_stream.start_stream()
        self._out_stream.start_stream()
        self._worker = threading.Thread(target=self._worker_loop, daemon=True)
        self._worker.start()
        self._running = True

    def stop(self):
        self._running = False
        self._stop.set()
        self._in_ring.wake()
        for s in (self._in_stream, self._out_stream):
            try:
                if s:
                    s.stop_stream()
                    s.close()
            except Exception:
                pass
        if self._worker is not None:
            self._worker.join(timeout=1.0)
            self._worker = None
        try:
            if self._pa:
                self._pa.terminate()
        except Exception:
            pass
        self._in_stream = None
        self._out_stream = None
        self._pa = None
