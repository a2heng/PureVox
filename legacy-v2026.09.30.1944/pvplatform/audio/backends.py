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

"""传输后端注册表（DESIGN.md §5）。

每个平台音频 API 是一个可插拔后端：元数据（BackendSpec）+ 可用性探测
（probe）。数据面契约与 PwBridge 同形——open/read/read_each/close/
active/last_error/open_far/close_far/read_far_h/far_available。

选择规则：平台匹配 → probe() 可用 → 能力覆盖计划需求，取优先级最高者。
禁止在传输代码里散布 if IS_LINUX 平台分支；平台差异只存在于后端实现内部。
"""

import platform as _platform
from dataclasses import dataclass, field


def _os_name() -> str:
    return {"Linux": "linux", "Windows": "windows", "Darwin": "macos"}.get(
        _platform.system(), "unknown")


@dataclass(frozen=True)
class BackendSpec:
    """传输后端描述符。"""
    name: str                     # 稳定 id："pipewire" / "wasapi" / ...
    label: str                    # 中文显示名
    platforms: tuple              # 支持的操作系统
    capabilities: frozenset = field(default_factory=frozenset)
    priority: int = 100           # 同平台多后端时越小越优先

    def supports_platform(self, osname: str) -> bool:
        return osname in self.platforms

    def covers(self, required: frozenset) -> bool:
        return required <= self.capabilities


def _probe_pipewire() -> bool:
    try:
        from pvplatform.audio.pwpipe_client import pw_available
        return bool(pw_available())
    except Exception:
        return False


def _probe_portaudio() -> bool:
    if _os_name() == "linux":
        return False          # Linux 强制原生 PipeWire，不落 PortAudio
    try:
        import pyaudio        # noqa: F401
        return True
    except Exception:
        return False


BACKENDS = [
    BackendSpec("pipewire", "PipeWire（pipewire-pulse）", ("linux",),
                frozenset({"multi_input", "multi_output", "loopback_far"}),
                priority=10),
    BackendSpec("wasapi", "WASAPI", ("windows",),
                frozenset({"multi_output", "loopback_far"}), priority=20),
    BackendSpec("mme", "MME", ("windows",),
                frozenset({"multi_output"}), priority=30),
]


def probe_backends():
    """全部候选后端及其探测结果：[(spec, available)]。"""
    probes = {"pipewire": _probe_pipewire}
    out = []
    osname = _os_name()
    for spec in BACKENDS:
        if not spec.supports_platform(osname):
            continue
        probe = probes.get(spec.name, _probe_portaudio)
        try:
            ok = bool(probe())
        except Exception:
            ok = False
        out.append((spec, ok))
    return out


def select_backend(required_caps=frozenset()):
    """按 平台 → 探测可用 → 能力覆盖 → 优先级 选出唯一后端；无则 None。"""
    candidates = [(s, ok) for s, ok in probe_backends()
                  if ok and s.covers(frozenset(required_caps))]
    if not candidates:
        return None
    candidates.sort(key=lambda pair: pair[0].priority)
    return candidates[0][0]
