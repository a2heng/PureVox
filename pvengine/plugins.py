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

"""节点注册表——一切用户可见音频组件的唯一规范来源。

规范见 DESIGN.md §2：NodeSpec 描述每个节点（名称/显示名/类别/UI 形态/参数模式），
四类 kind：
    input  采集源，处理链之前，可多实例（混音）
    output 播放汇，处理链之后，可多实例（扇出）
    fx     引擎 Stage 处理级，按用户排列串接
    viz    可视化旁路（tap），只读

发现入口只有两个：all_specs() 与 get_spec(name)。
UI 渲染与会话计划（session_plan.py）禁止自建类型清单。

engine_cache：AudioProcessor 持有的 dict，AI 插件共享模型 Stage，
链重建不重复加载模型。
"""

from dataclasses import dataclass, field

from pvengine.components.core_plugins import (
    GainPlugin, GatePlugin,
    Eq10Plugin, Eq31Plugin, Eq61Plugin,
    CompressorPlugin,
    DenoiserPlugin,
    TsePlugin,
)
from pvengine.components.agc import AgcPlugin
from pvengine.components.soundpad import SoundPadPlugin
from pvengine.components.music_player import MusicPlayerPlugin
from pvengine.components.desktop_audio import DesktopAudioPlugin


@dataclass(frozen=True)
class NodeSpec:
    """节点描述符（DESIGN.md §2.1）。"""
    name: str          # 全局唯一稳定 id
    label: str         # 中文显示名
    kind: str          # input | output | fx | viz
    tier: str = "toggle"   # UI 三级形态：toggle | inline | expand
    params: dict = field(default_factory=dict)  # 滑杆模式 {key: (label,lo,hi,default,step)}


# ── 插件目录（信号流惯例顺序）──
CATALOG: list[type] = [
    GainPlugin,
    DenoiserPlugin,
    TsePlugin,
    GatePlugin,
    AgcPlugin,
    Eq10Plugin,
    Eq31Plugin,
    Eq61Plugin,
    CompressorPlugin,
    SoundPadPlugin,
    MusicPlayerPlugin,
    DesktopAudioPlugin,
]

PLUGIN_TYPES: dict[str, type] = {cls.NAME: cls for cls in CATALOG}

# 特殊 UI 钩子：这些类型在行内渲染额外控件（由 ui 层判断类型实现）
SPECIAL_ROWS = {"eq10", "eq31", "eq61", "tse", "soundpad",
                "music_player", "desktop_audio"}

# 媒体源节点：设备外输入（会话计划据此放行「无麦克风输入」的纯媒体会话）
MEDIA_NODE_TYPES = frozenset({"soundpad", "music_player", "desktop_audio"})

# ── UI 层级元数据 ──
# toggle  = 仅开/关（无参数）
# inline  = 行内参数滑杆（默认，按 PARAMS 自动生成）
# expand  = 行内控制 + 「展开」按钮弹出独立 UI 对话框
UI_TIERS = {
    "denoiser": "toggle",
    "eq10": "expand",          # 展开：EQ 曲线编辑器（10 段）
    "eq31": "expand",          # 展开：EQ 曲线编辑器（31 段）
    "eq61": "expand",          # 展开：EQ 曲线编辑器（61 段）
    "tse": "expand",           # 展开：参考音频录制对话框
    "soundpad": "inline",      # 行内：音效垫子按钮组（+添加 / 全部停止）
    "music_player": "inline",  # 行内：曲目选择与进度 seek
    "desktop_audio": "inline",  # 行内：loopback 捕获说明（音量滑杆）
}

# 展开对话框标题（ui 层据此路由到对应编辑器）
EXPAND_TITLES = {
    "eq10": "均衡器",
    "eq31": "均衡器",
    "eq61": "均衡器",
    "tse": "参考音频",
}

# ── 系统节点显式注册（input/output/viz；fx 由插件类派生）──
# echo_cancel 是输入种节点（一行一路 AEC：mic 设备 + mic 增益 + far 源），
# 行级状态与采集由 AudioThread 按 SessionPlan.aec_rows 装配，不进 fx 链。
# loopback 是回环输入种节点（一路扬声器播出直采为输入，代码名 loopback）；
# AEC 行 far=扬声器与它继承同一套回环采集机制（两者的组合即完整 AEC 行）。
_SYSTEM_SPECS = [
    NodeSpec("audio_input", "录音输入", "input"),
    NodeSpec("remote_mic", "网络输入", "input"),
    NodeSpec("loopback", "播放输入", "input"),
    NodeSpec("echo_cancel", "AEC 输入", "input"),
    NodeSpec("audio_output", "音频输出", "output"),
    NodeSpec("vu_meter", "VU 电平表", "viz"),
    NodeSpec("spectrum", "频谱图", "viz"),
]

_SPEC_CACHE: list[NodeSpec] | None = None


def all_specs() -> list[NodeSpec]:
    """全部节点描述（系统节点在前，fx 在后）；首次调用构建并缓存。"""
    global _SPEC_CACHE
    if _SPEC_CACHE is None:
        specs = list(_SYSTEM_SPECS)
        for cls in CATALOG:
            specs.append(NodeSpec(
                name=cls.NAME,
                label=cls.LABEL,
                kind="fx",
                tier=ui_tier(cls.NAME),
                params=dict(cls.PARAMS),
            ))
        _SPEC_CACHE = specs
    return list(_SPEC_CACHE)


def get_spec(name: str) -> NodeSpec | None:
    for s in all_specs():
        if s.name == name:
            return s
    return None


def ui_tier(ptype: str) -> str:
    """节点 UI 层级：未显式声明时，有滑杆参数=inline，无参数=toggle。"""
    if ptype in UI_TIERS:
        return UI_TIERS[ptype]
    cls = PLUGIN_TYPES.get(ptype)
    return "inline" if (cls and cls.PARAMS) else "toggle"

# 全新配置的默认链：输入 → 降噪 → 输出 + 可视化
DEFAULT_CHAIN = [
    {"type": "audio_input", "enabled": True, "params": {"device": ""}},
    {"type": "denoiser", "enabled": True, "params": {}},
    {"type": "audio_output", "enabled": True, "params": {"device": ""}},
    {"type": "vu_meter", "enabled": True, "params": {}},
    {"type": "spectrum", "enabled": True, "params": {}},
]


def create_plugin(ptype: str, params: dict | None = None,
                  stage_cache: dict | None = None):
    """按注册名实例化 fx 插件；未知/系统类型返回 None（向前兼容旧配置）。

    stage_cache：AudioProcessor 持有的 AI Stage 缓存，链重建不重复加载模型。
    """
    cls = PLUGIN_TYPES.get(ptype)
    if cls is None:
        return None
    try:
        return cls(params, stage_cache=stage_cache)
    except TypeError:
        return cls(params)
