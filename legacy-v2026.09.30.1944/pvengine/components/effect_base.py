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

"""插件基类——核心处理插件（增益/AGC/门/EQ/压缩器/AI 三件套）的统一契约。

每个插件 = 一个 Effect 子类：
- PARAMS 声明参数模式（key → (标签, 最小, 最大, 默认, 步进)），UI 据此自动生成控件；
- process(frame, ctx) -> frame 为唯一处理入口（float32 一维帧）；
- 实现必须满足实时预算（整帧向量化，禁止逐样本 Python 循环）。
"""

from abc import ABC, abstractmethod
import numpy as np


class Effect(ABC):
    """处理插件基类。子类需设置 NAME（注册键）与 LABEL（显示名）。"""

    NAME: str = "base"
    LABEL: str = "未命名"
    # {key: (label, vmin, vmax, vdefault, step)}
    PARAMS: dict = {}

    def __init__(self, params: dict | None = None):
        self.params = {k: spec[3] for k, spec in self.PARAMS.items()}  # 默认值
        if params:
            self.set_params(params)

    def set_params(self, params: dict):
        for k, v in (params or {}).items():
            if k in self.PARAMS:
                lo, hi = self.PARAMS[k][1], self.PARAMS[k][2]
                try:
                    self.params[k] = float(min(max(float(v), lo), hi))
                except (TypeError, ValueError):
                    pass
        self.on_params_changed()

    def on_params_changed(self):
        """参数变更钩子（如重建滤波器系数），默认无操作。"""

    @abstractmethod
    def process(self, frame: np.ndarray, ctx) -> np.ndarray:
        ...

    def reset(self):
        """清空流式状态（缓冲/包络/滤波器 zi）。"""
