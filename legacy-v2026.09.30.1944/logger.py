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
日志模块 - 格式化对齐的日志输出

格式：
    [2026-05-31 19:30:45.123] [MSG ] 消息内容
    [2026-05-31 19:30:45.124] [ERR ] 错误信息
    [2026-05-31 19:30:45.125] [WARN] 警告信息
    [2026-05-31 19:30:45.126] [ SYS] 系统相关消息

方括号宽度：3-4个字母，右对齐
时间戳：年-月-日 时:分:秒.毫秒，固定宽度
"""

import datetime
from typing import Callable, Optional


# 日志标签定义（3-4字母，右对齐）
TAG_MSG  = "MSG"
TAG_ERR  = "ERR"
TAG_WARN = "WARN"
TAG_SYS  = "SYS"


class Logger:
    """格式化日志器"""

    def __init__(self, callback: Optional[Callable[[str], None]] = None):
        self._callback = callback
        self._buffer = []
        self._flush_scheduled = False
        self._log_file = None
        self._init_file_log()

    def _init_file_log(self):
        """初始化文件日志输出。"""
        try:
            from user_paths import get_log_path
            log_path = get_log_path()
            self._log_file = open(log_path, 'a', encoding='utf-8')
        except Exception:
            self._log_file = None

    def _format_timestamp(self) -> str:
        """格式化时间戳：年-月-日 时:分:秒.毫秒"""
        now = datetime.datetime.now()
        return now.strftime('%Y-%m-%d %H:%M:%S') + f'.{now.microsecond // 1000:03d}'

    def _format_tag(self, tag: str) -> str:
        """格式化标签：固定4字符宽度，右对齐"""
        return f'{tag:>4s}'

    def _log(self, tag: str, message: str) -> None:
        """内部日志方法"""
        timestamp = self._format_timestamp()
        tag_str = self._format_tag(tag)
        line = f'[{timestamp}] [{tag_str}] {message}'

        # 输出到控制台
        print(line)

        # 写入文件
        if self._log_file:
            try:
                self._log_file.write(line + '\n')
                self._log_file.flush()
            except Exception:
                pass

        # 存入缓冲区
        self._buffer.append(line)

        # 触发回调
        if self._callback:
            self._callback(line)

    def msg(self, message: str) -> None:
        """普通消息"""
        self._log(TAG_MSG, message)

    def err(self, message: str) -> None:
        """错误消息"""
        self._log(TAG_ERR, message)

    def warn(self, message: str) -> None:
        """警告消息"""
        self._log(TAG_WARN, message)

    def sys(self, message: str) -> None:
        """系统相关"""
        self._log(TAG_SYS, message)

