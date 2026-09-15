# PureVox Lite Net Only — 音频输出流（仅 WASAPI）
# Copyright (C) 2024-2026 a2heng <752848283@qq.com>
# SPDX-License-Identifier: GPL-3.0-or-later
#
# 零复用：不 import audio_processor / pvplatform
# 网络解码写入 PlaybackBuffer，本模块回调按设备时钟取帧播放；输出设备只列 WASAPI

import numpy as np

try:
    import pyaudio
except ImportError:
    pyaudio = None

SAMPLE_RATE = 48000
HOP = 480          # 引擎 hop 10ms @48kHz（与网络帧一致）
FORMAT = pyaudio.paFloat32 if pyaudio else None
CHANNELS = 1

def db_to_linear(db):
    return 10.0 ** (db / 20.0)

def fix_device_name(name):
    """修复 PortAudio 在中文 Windows 返回的乱码设备名（语义对齐主线）。"""
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
    """设备名归一化（比较用）：转小写 + 丢弃单字符 token。"""
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
    """在候选设备名里选最佳匹配（归一化精确 → 前缀 → 相似度 ≥0.6，对齐主线）。"""
    if not name or not candidates:
        return None
    cands = [c for c in candidates if c]
    if not cands:
        return None
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


def list_output_devices():
    """仅 WASAPI 输出设备，返回 [(纯设备名, 索引, 属性行), ...]（对齐主线）。"""
    if pyaudio is None:
        return []
    pa = pyaudio.PyAudio()
    outs = []
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
                continue            # 输出只要 WASAPI
            if info.get("maxOutputChannels", 0) <= 0:
                continue
            name = fix_device_name(info.get("name", "")).strip()
            if not name:
                continue
            ch = int(info.get("maxOutputChannels", 0))
            sr = int(info.get("defaultSampleRate", 0) or 0)
            props = f"WASAPI · {ch}ch · {sr}Hz"
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

    return dedup(outs)

def check_output_48k(idx):
    """WASAPI 共享模式锁 MixFormat，非 48k 设备直接拒绝并提示"""
    if pyaudio is None:
        return False, "PyAudio 未安装"
    pa = pyaudio.PyAudio()
    try:
        pa.is_format_supported(
            rate=SAMPLE_RATE,
            input_device=None,
            input_channels=None,
            input_format=None,
            output_device=idx,
            output_channels=CHANNELS,
            output_format=FORMAT,
        )
        return True, ""
    except Exception as e:
        return False, str(e)
    finally:
        pa.terminate()

class LiteNetStream:
    """输出流：回调从 PlaybackBuffer 按 frame_count 取样本，后增益后播放。

    与主线同构：网络解码线程按到达节奏 write()，设备回调按真实时钟 pull(n)；
    速率差/调度抖动/欠载由 PlaybackBuffer 消化（不丢样本、不周期性垫零）。
    """

    def __init__(self, out_idx, sink, post_db=0.0):
        self.out_idx = out_idx
        self.sink = sink
        self.post_gain = db_to_linear(post_db)
        self._pa = None
        self._stream = None
        self._running = False

    def set_post_gain(self, post_db):
        self.post_gain = db_to_linear(post_db)

    def _callback(self, _in_data, frame_count, _time_info, _status):
        try:
            data = np.asarray(self.sink.pull(frame_count), dtype=np.float32)
            data *= self.post_gain
            np.clip(data, -1.0, 1.0, out=data)
            return (data.tobytes(), pyaudio.paContinue)
        except Exception:
            return (np.zeros(frame_count, dtype=np.float32).tobytes(), pyaudio.paContinue)

    def start(self):
        if pyaudio is None:
            raise RuntimeError("PyAudio 未安装")
        if self._running:
            return
        ok, msg = check_output_48k(self.out_idx)
        if not ok:
            raise RuntimeError(f"输出设备不支持 48kHz: {msg}")
        self._pa = pyaudio.PyAudio()
        self._stream = self._pa.open(
            rate=SAMPLE_RATE,
            channels=CHANNELS,
            format=FORMAT,
            input=False,
            output=True,
            output_device_index=self.out_idx,
            frames_per_buffer=HOP,
            stream_callback=self._callback,
        )
        self._stream.start_stream()
        self._running = True

    def stop(self):
        self._running = False
        try:
            if self._stream:
                self._stream.stop_stream()
                self._stream.close()
        except Exception:
            pass
        try:
            if self._pa:
                self._pa.terminate()
        except Exception:
            pass
        self._stream = None
        self._pa = None
