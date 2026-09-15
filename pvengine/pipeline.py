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

"""Pipeline——Stage 链式执行器。

按序调用各组件 process(frame, ctx)；enabled=False 的组件直接旁路。
reset()/release() 广播到全链。
"""

import numpy as np

from pvengine.context import FrameContext
from pvengine.stages.base import Stage


class Pipeline:
    def __init__(self, stages: list[Stage]):
        self.stages = [s for s in stages if s is not None]

    def get(self, stage_type) -> Stage | None:
        """按类型取组件（同类型多个时返回第一个）。"""
        for s in self.stages:
            if isinstance(s, stage_type):
                return s
        return None

    def process(self, frame: np.ndarray, ctx: FrameContext) -> np.ndarray:
        for s in self.stages:
            if s.accepts(ctx):
                frame = s.process(frame.astype(np.float32, copy=False), ctx)
        return frame

    def reset(self):
        for s in self.stages:
            s.reset()

    def release(self):
        for s in self.stages:
            s.release()
