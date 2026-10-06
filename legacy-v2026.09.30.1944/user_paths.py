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
用户数据目录管理 — ~/.purevox/
"""

import os
import datetime

USER_DIR = os.path.join(os.path.expanduser("~"), ".purevox")
CONFIG_PATH = os.path.join(USER_DIR, "config.json")
LOG_DIR = os.path.join(USER_DIR, "logs")
WAV_PATH = os.path.join(USER_DIR, "tse_reference.wav")
# 媒体库：导入时归一为 48k/mono WAV 各自入库（音效板 / 音乐播放器分开）
SOUNDPAD_DIR = os.path.join(USER_DIR, "soundpad")
MUSIC_DIR = os.path.join(USER_DIR, "music")


def ensure_dirs():
    """确保用户目录结构存在。"""
    os.makedirs(USER_DIR, exist_ok=True)
    os.makedirs(LOG_DIR, exist_ok=True)


def get_log_path() -> str:
    """获取当日日志文件路径。"""
    ensure_dirs()
    today = datetime.datetime.now().strftime("%Y%m%d")
    return os.path.join(LOG_DIR, f"purevox_{today}.log")
