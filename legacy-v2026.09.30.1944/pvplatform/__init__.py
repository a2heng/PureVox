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
PureVox 平台抽象层。

按 sys.platform 分发音频后端（AEC 扬声器回采、设备枚举）与系统服务
（单实例、自启动、防火墙、全局热键、系统声音设置等），使同一套上层
代码（DSP、UI、网络服务器）可运行于 Windows / Linux / macOS。
"""

import sys

#: 当前运行平台标记：win32 / linux / darwin
PLATFORM = sys.platform

IS_WINDOWS = PLATFORM.startswith("win")
IS_LINUX = PLATFORM.startswith("linux")
IS_MACOS = PLATFORM.startswith("darwin")
