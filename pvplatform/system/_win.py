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
Windows 系统服务实现（win32 / reg / COM）。

仅在 Windows 被 platform.system 模块按需延迟导入；本文件不应对 Linux
造成 import 即崩溃（其 win32 依赖在函数内部导入）。
"""

import ctypes
import os
import sys
import threading


def acquire_single_instance_win(lock_name: str) -> bool:
    """Windows 命名 Mutex 单实例锁。返回 True 表示成功获得锁。

    锁句柄存于模块级全局，进程存活期间保持打开，退出时系统自动释放。
    """
    try:
        import win32event
        import win32api
        import winerror
    except ImportError:
        raise
    global _SINGLE_INSTANCE_MUTEX
    _SINGLE_INSTANCE_MUTEX = win32event.CreateMutex(None, True, lock_name)
    if win32api.GetLastError() == winerror.ERROR_ALREADY_EXISTS:
        return False
    return True


_SINGLE_INSTANCE_MUTEX = None


def is_autostart_win() -> bool:
    """检查注册表 Run 键是否含 PureVox。"""
    try:
        import winreg
        k = winreg.OpenKey(
            winreg.HKEY_LOCAL_MACHINE,
            r"SOFTWARE\Microsoft\Windows\CurrentVersion\Run",
            0, winreg.KEY_READ)
        try:
            winreg.QueryValueEx(k, "PureVox")
            winreg.CloseKey(k)
            return True
        except FileNotFoundError:
            winreg.CloseKey(k)
            return False
    except Exception:
        return False


def run_as_admin_win(cmd: str, logger) -> bool:
    """通过 UAC 以管理员权限运行注册表命令（reg.exe）。"""
    try:
        return ctypes.windll.shell32.ShellExecuteW(None, "runas", "reg.exe", cmd, None, 1) > 32
    except Exception as e:
        logger.err(f"管理员权限: {e}")
        return False


def enable_autostart_win(logger) -> bool:
    try:
        exe = os.path.abspath(sys.executable if getattr(sys, 'frozen', False) else __file__)
        return run_as_admin_win(
            f'add "HKLM\\SOFTWARE\\Microsoft\\Windows\\CurrentVersion\\Run" '
            f'/v "PureVox" /t REG_SZ /d "\\"{exe}\\"" /f', logger)
    except Exception as e:
        logger.err(f"添加注册表: {e}")
        return False


def disable_autostart_win(logger) -> bool:
    try:
        return run_as_admin_win(
            'delete "HKLM\\SOFTWARE\\Microsoft\\Windows\\CurrentVersion\\Run" '
            '/v "PureVox" /f', logger)
    except Exception as e:
        logger.err(f"删除注册表: {e}")
        return False


def add_firewall_rule_win(logger):
    """使用 win32com 添加进站防火墙规则。"""
    try:
        import win32com.client
        exe = os.path.abspath(sys.executable if getattr(sys, 'frozen', False) else __file__)
        fw = win32com.client.Dispatch("HNetCfg.FwPolicy2")
        rule = win32com.client.Dispatch("HNetCfg.FwRule")
        rule.Name = "PureVox AI Mic Denoise"
        rule.Description = "PureVox AI Mic Denoise"
        rule.Direction = 1
        rule.Action = 1
        rule.Program = exe
        rule.Enabled = True
        rule.Profiles = 0x7FFFFFFF
        fw.Rules.Add(rule)
        logger.sys("防火墙规则: 已添加")
    except Exception as e:
        logger.err(f"防火墙规则添加失败（需管理员权限，可能影响手机推流）: {e}")


def beep_win(freq_hz: int, duration_ms: int):
    threading.Thread(
        target=lambda: ctypes.windll.kernel32.Beep(int(freq_hz), int(duration_ms)),
        daemon=True).start()


def open_sound_panel_win(logger):
    """打开声音控制面板（mmsys.cpl）。

    用 ShellExecuteW 而非 subprocess：GUI 无控制台进程下最可靠，
    不闪 cmd 黑框、不被会话上下文吞掉。"""
    try:
        import ctypes
        rc = ctypes.windll.shell32.ShellExecuteW(
            None, "open", "control.exe", "mmsys.cpl", None, 1)
        if rc > 32:
            logger.msg("已打开声音控制面板")
        else:
            logger.err(f"打开声音控制面板失败: ShellExecuteW rc={rc}")
    except Exception as e:
        logger.err(f"打开失败: {e}")


def open_virtual_cable_panel_win(logger):
    """打开 VB-CABLE 控制面板（需管理员权限，走 UAC 提权）。"""
    candidates = (
        r"C:\Program Files\VB\CABLE\VBCABLE_ControlPanel.exe",
        r"C:\Program Files (x86)\VB\CABLE\VBCABLE_ControlPanel.exe",
    )
    exe = next((p for p in candidates if os.path.exists(p)), None)
    if exe is None:
        logger.warn("未找到 VB-CABLE 控制面板——请先安装 VB-CABLE 驱动")
        return
    try:
        rc = ctypes.windll.shell32.ShellExecuteW(None, "runas", exe,
                                                 None, None, 1)
        if rc > 32:
            logger.msg("已打开 VB-CABLE 控制面板")
        else:
            logger.warn(f"VB-CABLE 控制面板打开失败: rc={rc}"
                        "（UAC 取消或驱动异常）")
    except Exception as e:
        logger.err(f"打开失败: {e}")


def set_titlebar_theme_win(win_id: int, dark: bool):
    """通过 DWM API 设置 Windows 标题栏深色/浅色（Win10 1809+ / Win11）。"""
    try:
        DWMWA_USE_IMMERSIVE_DARK_MODE = 20
        ctypes.windll.dwmapi.DwmSetWindowAttribute(
            win_id, DWMWA_USE_IMMERSIVE_DARK_MODE,
            ctypes.byref(ctypes.c_int(1 if dark else 0)),
            ctypes.sizeof(ctypes.c_int))
    except Exception:
        pass