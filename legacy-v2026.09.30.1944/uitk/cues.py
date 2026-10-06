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

"""启动/停止提示音播放（系统默认输出设备）。

波形由 pvengine.cues 合成（纯 numpy）；本模块只负责把 WAV 字节送到系统
默认输出设备，非阻塞（播放丢到短命后台线程），播放失败一律静默——
提示音是尽力而为的 UI 反馈，绝不影响启停主流程。

后端单一实现路径：Windows = 标准库 winsound（默认 waveOut 设备）；
Linux = pw-play / paplay / aplay（PipeWire/Pulse/ALSA 依序降级）；
macOS = afplay。全都没有时静默。
"""

import os
import shutil
import subprocess
import sys
import tempfile
import threading


def play(pid: str, kind: str) -> None:
    """播放提示音（pid 为空 = 关闭，直接返回；kind = "start"/"stop"）。"""
    if not pid:
        return
    threading.Thread(target=_play_sync, args=(str(pid), str(kind)),
                     daemon=True).start()


def _play_sync(pid: str, kind: str) -> None:
    data = _wav(pid, kind)
    if not data:
        return
    try:
        if sys.platform.startswith("win"):
            _play_winsound(data)
        elif sys.platform.startswith("darwin"):
            _play_file(data, ["afplay"])
        else:
            _play_file(data, ["pw-play", "paplay", "aplay"])
    except Exception:
        pass


# 停止时引擎刚释放输出设备，win 驱动可能短暂拒播——短退避重试，别让提示音被吞掉
_WIN_RETRY_DELAYS = (0.0, 0.08, 0.16, 0.32)


def _play_winsound(data: bytes) -> None:
    import time
    import winsound
    for delay in _WIN_RETRY_DELAYS:
        if delay:
            time.sleep(delay)
        try:
            winsound.PlaySound(data, winsound.SND_MEMORY)
            return
        except RuntimeError:
            continue


def _wav(pid: str, kind: str):
    try:
        from pvengine.cues import wav_bytes
        return wav_bytes(pid, kind)
    except Exception:
        return None


def _play_file(data: bytes, candidates) -> None:
    exe = None
    for name in candidates:
        exe = shutil.which(name)
        if exe:
            break
    if not exe:
        return
    fd, path = tempfile.mkstemp(suffix=".wav", prefix="purevox_cue_")
    try:
        with os.fdopen(fd, "wb") as f:
            f.write(data)
        subprocess.run([exe, path], stdout=subprocess.DEVNULL,
                       stderr=subprocess.DEVNULL, timeout=5,
                       check=False)
    finally:
        try:
            os.remove(path)
        except OSError:
            pass
