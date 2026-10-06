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

"""音频文件格式工具——媒体类插件（音效板/音乐播放器）共用。

**入库唯一实现** `import_media`：任何来源文件一律归一为 48kHz 单声道
16bit WAV，存入调用方给定的库目录（~/.purevox/soundpad 或 /music）。
- 首选 miniaudio 解码（wav/mp3/flac/ogg-vorbis；自包含 C，跨平台 wheel）；
- miniaudio 不支持的容器/编码（m4a/mp4/aac/wma/opus…）用 PyAV 兜底解码；
- 文件名 `<原名>-<sha1 前 8 位>.wav`：重复导入同一文件幂等复用、不同内容
  同名不互相覆盖。

**运行时解码唯一实现**：miniaudio——音效板整段 `decode_to_mono_48k` 进内存，
音乐播放器 `miniaudio.stream_file` 流式。用户库里存的已是 48k/mono WAV，
运行时不再需要任何转码/回退。

本模块零状态、零相互依赖，仅函数。
"""

import hashlib
import os
import wave

import numpy as np

_TARGET_SR = 48000
_INVALID = set('<>:"/\\|?*')
_MAX_STEM = 60


def decode_to_mono_48k(path):
    """miniaudio 解码整文件 → float32 单声道 48k；不支持/失败返回 None。

    经内存解码（decode）：decode_file 的 char* 路径在 Windows 上按 ANSI
    fopen，中文/非 ASCII 文件名必挂；先自读字节则全平台一致。
    """
    try:
        import miniaudio
        with open(path, "rb") as f:
            data = f.read()
        dec = miniaudio.decode(
            data, output_format=miniaudio.SampleFormat.FLOAT32,
            nchannels=1, sample_rate=_TARGET_SR)
    except Exception:
        return None
    x = np.asarray(dec.samples, dtype=np.float32)
    if x.ndim > 1:
        x = x.reshape(-1)
    return np.ascontiguousarray(x, dtype=np.float32)


def import_media(path, dest_dir, stem=None) -> str:
    """把任意音频文件归一为 48k/mono/16bit WAV 存入 dest_dir，返回库内路径。

    已存在同一内容（同 hash）直接复用；解码失败抛 RuntimeError。
    """
    os.makedirs(dest_dir, exist_ok=True)
    base = _safe_stem(
        stem if stem is not None
        else os.path.splitext(os.path.basename(path))[0])
    out = os.path.join(dest_dir, f"{base}-{_digest(path)}.wav")
    if os.path.exists(out):
        return out
    samples = decode_to_mono_48k(path)
    if samples is None:
        samples = _decode_pyav(path)
    if samples is None or not len(samples):
        raise RuntimeError("无法解码该音频文件")
    tmp = out + ".part"
    try:
        _write_wav_s16(tmp, samples)
        os.replace(tmp, out)
    finally:
        if os.path.exists(tmp):
            try:
                os.remove(tmp)
            except OSError:
                pass
    return out


def _decode_pyav(path):
    """PyAV 兜底解码首个音频流 → float32 单声道 48k；失败返回 None。"""
    try:
        import av
    except Exception:
        return None
    chunks = []
    try:
        with av.open(path) as c:
            streams = [s for s in c.streams if s.type == "audio"]
            if not streams:
                return None
            res = av.AudioResampler(format="s16", layout="mono",
                                    rate=_TARGET_SR)
            for frame in c.decode(streams[0]):
                for o in (res.resample(frame) or []):
                    chunks.append(o.to_ndarray().reshape(-1))
    except Exception:
        return None
    if not chunks:
        return None
    return np.concatenate(chunks).astype(np.float32) / np.float32(32768.0)


def _write_wav_s16(path, samples):
    pcm = np.clip(np.asarray(samples, dtype=np.float32) * 32767.0,
                  -32768.0, 32767.0).astype("<i2")
    with wave.open(path, "wb") as w:
        w.setnchannels(1)
        w.setsampwidth(2)
        w.setframerate(_TARGET_SR)
        w.writeframes(pcm.tobytes())


def _safe_stem(name) -> str:
    s = "".join(("_" if ch in _INVALID or ord(ch) < 32 else ch)
                for ch in str(name)).strip().rstrip(".")
    s = s[:_MAX_STEM].strip()
    return s or "audio"


def _digest(path) -> str:
    h = hashlib.sha1()
    with open(path, "rb") as f:
        while True:
            b = f.read(1 << 16)
            if not b:
                break
            h.update(b)
    return h.hexdigest()[:8]
