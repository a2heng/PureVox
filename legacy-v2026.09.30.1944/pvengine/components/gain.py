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

"""前置增益组件。"""

import numpy as np

from pvengine.context import FrameContext
from pvengine.stages.base import Stage


class GainStage(Stage):
    """链首增益（固定 pre-gain）。"""

    name = "gain"

    def __init__(self, pre_gain_db: float = 0.0):
        super().__init__()
        self.pre_gain = 10.0 ** (pre_gain_db / 20.0)

    def set_pre_gain_db(self, db: float):
        self.pre_gain = 10.0 ** (db / 20.0)

    def process(self, frame, ctx: FrameContext):
        return frame * np.float32(self.pre_gain)
