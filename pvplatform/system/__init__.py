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
系统服务平台抽象层。

把 Windows 专有的系统集成（注册表自启动、防火墙、全局热键、提示音、
标题栏主题、声音面板、提权、单实例、电源事件）收敛为统一接口，
Linux / macOS 提供各自实现，上层 UI 无需平台分支。

接口（按模块级函数暴露，win 与 linux 后端签名一致）：

    单实例:
        acquire_single_instance(lock_name) -> bool
            返回 True 表示获得锁；False 表示已有实例在运行。
            释放锁：进程退出自动释放（flock / Mutex Handle 保持打开即可）。

    自启动:
        is_autostart() -> bool
        enable_autostart(logger) -> bool
        disable_autostart(logger) -> bool

    防火墙:
        add_firewall_rule(logger)  (Linux 通常无需开放入站，空实现)

    提示音:
        beep(freq_hz: int, duration_ms: int)

    系统声音设置面板:
        open_sound_panel(logger)

    虚拟声卡控制面板（Windows VB-CABLE）:
        open_virtual_cable_panel(logger)

    主题标题栏:
        set_titlebar_theme(win_id: int, dark: bool)

    提权运行命令（Windows UAC；Linux 用 pkexec，可选）:
        run_as_admin(cmd, logger) -> bool

    虚拟麦克风（Linux；Windows 用 VB-CABLE 驱动）:
        virtual_mic_ready() -> bool
        ensure_virtual_mic(logger) -> bool
        remove_virtual_mic(logger) -> None
        通过 PipeWire/PulseAudio null-sink（purevox_out + .monitor）实现，
        等价 VB-CABLE 的 CABLE Input（播放）+ CABLE Output（录音源）。

    电源事件:
        is_windows_power_event 相关常量与解析（仅 Windows 有意义）
"""

from .. import IS_WINDOWS, IS_LINUX, IS_MACOS

# VB-CABLE 端点逻辑键常量（平台感知）：
# Windows 从 _win 导出；非 Windows 给占位值（音量条不显示，但 UI 引用不报错）。
# 这些字符串是后端 _STABLE_RULES 的键，代表"哪个端点"的逻辑标识，
# 不是设备 FriendlyName（可被用户重命名）。所有调用方必须引用此常量。
if IS_WINDOWS:
    from ._win import CABLE_OUTPUT_KEY, CABLE_INPUT_KEY
else:
    CABLE_OUTPUT_KEY = "CABLE Output"
    CABLE_INPUT_KEY = "CABLE Input"


def acquire_single_instance(lock_name: str) -> bool:
    """跨平台单实例锁。Windows 用命名 Mutex，POSIX 用 flock 锁文件。"""
    if IS_WINDOWS:
        from ._win import acquire_single_instance_win
        return acquire_single_instance_win(lock_name)
    from ._posix import acquire_single_instance_posix
    return acquire_single_instance_posix(lock_name)


def is_autostart() -> bool:
    if IS_WINDOWS:
        from ._win import is_autostart_win
        return is_autostart_win()
    from ._posix import is_autostart_posix
    return is_autostart_posix()


def enable_autostart(logger) -> bool:
    if IS_WINDOWS:
        from ._win import enable_autostart_win
        return enable_autostart_win(logger)
    from ._posix import enable_autostart_posix
    return enable_autostart_posix(logger)


def disable_autostart(logger) -> bool:
    if IS_WINDOWS:
        from ._win import disable_autostart_win
        return disable_autostart_win(logger)
    from ._posix import disable_autostart_posix
    return disable_autostart_posix(logger)


def add_firewall_rule(logger):
    """开放入站防火墙规则。Linux 一般无需（局域网内网），为空实现。"""
    if IS_WINDOWS:
        from ._win import add_firewall_rule_win
        add_firewall_rule_win(logger)


def beep(freq_hz: int, duration_ms: int):
    """播放提示音（非阻塞）。"""
    if IS_WINDOWS:
        from ._win import beep_win
        beep_win(freq_hz, duration_ms)
    else:
        from ._posix import beep_posix
        beep_posix(freq_hz, duration_ms)


def open_sound_panel(logger):
    if IS_WINDOWS:
        from ._win import open_sound_panel_win
        open_sound_panel_win(logger)
    else:
        from ._posix import open_sound_panel_posix
        open_sound_panel_posix(logger)


def open_virtual_cable_panel(logger):
    """VB-CABLE 控制面板。仅 Windows 有意义。"""
    if IS_WINDOWS:
        from ._win import open_virtual_cable_panel_win
        open_virtual_cable_panel_win(logger)
    else:
        logger.sys("虚拟声卡仅 Windows 需要（Linux 直接选真实输出设备）")


def set_titlebar_theme(win_id: int, dark: bool):
    """设置系统标题栏深色/浅色。仅 Windows 有效，其它平台空实现。"""
    if IS_WINDOWS:
        from ._win import set_titlebar_theme_win
        set_titlebar_theme_win(win_id, dark)


def run_as_admin(cmd: str, logger) -> bool:
    """以管理员权限执行命令。Windows 用 UAC；Linux 用 pkexec。"""
    if IS_WINDOWS:
        from ._win import run_as_admin_win
        return run_as_admin_win(cmd, logger)
    from ._posix import run_as_admin_posix
    return run_as_admin_posix(cmd, logger)


def virtual_mic_ready() -> bool:
    """虚拟麦克风（null-sink + monitor）是否已创建。Linux 专用。"""
    if IS_WINDOWS:
        return False
    from ._posix import virtual_mic_ready as _ready
    return _ready()


def ensure_virtual_mic(logger) -> bool:
    """确保虚拟麦克风存在（已存在则跳过）。Linux 专用。"""
    if IS_WINDOWS:
        return False
    from ._posix import ensure_virtual_mic as _ensure
    return _ensure(logger)


def remove_virtual_mic(logger) -> None:
    """卸载虚拟麦克风（幂等）。Linux 专用。"""
    if IS_WINDOWS:
        return
    from ._posix import remove_virtual_mic as _remove
    _remove(logger)


def get_endpoint_volume_pct(name_sub: str):
    """读 Windows endpoint 主音量百分比(0.0~1.0)。非 Windows 返回 None。

    name_sub 用设备友好名子串匹配（如 "CABLE Output"）。用于 UI 实时回显
    系统端点音量；非 Windows 无对应实现，返回 None 令 UI 隐藏音量条。
    """
    if IS_WINDOWS:
        from ._win import get_endpoint_volume_pct_win
        return get_endpoint_volume_pct_win(name_sub)
    return None


def set_endpoint_volume_pct(name_sub: str, pct: float) -> bool:
    """写 Windows endpoint 主音量百分比(0.0~1.0)。非 Windows 返回 False。"""
    if IS_WINDOWS:
        from ._win import set_endpoint_volume_pct_win
        return set_endpoint_volume_pct_win(name_sub, pct)
    return False


def invalidate_endpoint_volume_cache(name_sub=None) -> None:
    """失效端点音量缓存（设备拔插/重命名后调用）。非 Windows 空实现。"""
    if IS_WINDOWS:
        from ._win import invalidate_endpoint_volume_cache_win
        invalidate_endpoint_volume_cache_win(name_sub)


def get_vb_cable_names() -> set:
    """返回所有 VB-CABLE 设备的 FriendlyName 集合。非 Windows 返回空集。

    UI 层用此集合判断选中设备是否 VB-CABLE，替代硬编码 'CABLE' 子串
    匹配（用户重命名设备后仍生效）。
    """
    if IS_WINDOWS:
        from ._win import get_vb_cable_names_win
        return get_vb_cable_names_win()
    return set()


def get_cable_output_name():
    """返回 CABLE Output 录音端点的实际 FriendlyName。非 Windows 返回 None。

    UI 标签用此动态显示实际设备名（抗用户重命名），替代硬编码文本。
    """
    if IS_WINDOWS:
        from ._win import get_cable_output_name_win
        return get_cable_output_name_win()
    return None


def get_endpoint_mute(name_sub: str):
    """读 Windows endpoint 静音状态。True=静音,False=未静音,非 Windows/失败返回 None。"""
    if IS_WINDOWS:
        from ._win import get_endpoint_mute_win
        return get_endpoint_mute_win(name_sub)
    return None


def set_endpoint_mute(name_sub: str, mute: bool) -> bool:
    """写 Windows endpoint 静音状态。返回是否成功。非 Windows 返回 False。"""
    if IS_WINDOWS:
        from ._win import set_endpoint_mute_win
        return set_endpoint_mute_win(name_sub, mute)
    return False
