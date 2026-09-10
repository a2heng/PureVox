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
    except Exception:
        pass


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


# ── CABLE Output 端点音量读写(pycaw Core Audio 包装) ──
# 项目硬约束"无自编译二进制";pycaw + comtypes + psutil 均为纯 Python。
# 枚举设备 + 读写 IAudioEndpointVolume master volume scalar。AudioDevice
# 按 name_sub 缓存,UI 33ms tick 直接读缓存的 EndpointVolume(亚毫秒,
# 不阻塞 UI)。设备拔插/重命名后 invalidate 清空缓存。非 Windows 或
# pycaw 缺失时优雅降级(返回 None/False)。
try:
    from pycaw.pycaw import AudioUtilities
    _PYCAW_OK = True
except Exception:
    _PYCAW_OK = False

_DEV_CACHE: dict = {}   # name_sub -> AudioDevice
_DEV_CACHE_LOCK = threading.Lock()   # 保护 _DEV_CACHE 并发读写


def _cache_pop(name_sub):
    """线程安全地从缓存移除一项（COM 调用失败时失效缓存）。"""
    with _DEV_CACHE_LOCK:
        _cache_pop(name_sub)


def _cache_clear():
    """线程安全地清空缓存（设备拔插/重命名时失效全部）。"""
    with _DEV_CACHE_LOCK:
        _DEV_CACHE.clear()

# 逻辑键常量：这些字符串是后端 _STABLE_RULES 的键，代表"哪个端点"的逻辑
# 标识，不是设备 FriendlyName（可被用户重命名）。所有调用方必须引用此常量，
# 禁止裸写 "CABLE Output"/"CABLE Input"——后端按驱动描述+ID前缀稳定匹配，
# 与 FriendlyName 无关。
CABLE_OUTPUT_KEY = "CABLE Output"   # 录音端点（其他软件 AGC 调的就是它）
CABLE_INPUT_KEY = "CABLE Input"     # 播放端点

# 稳定设备定位规则（用户重命名 FriendlyName 后仍生效）：
# VB-CABLE 驱动描述 'VB-Audio Virtual Cable' 不可由用户修改（INF 写入），
# 结合设备 ID 前缀区分输入/输出端点：
#   {0.0.0.00000000} = eRender（播放端点）= CABLE Input
#   {0.0.1.00000000} = eCapture（录音端点）= CABLE Output
_STABLE_RULES = {
    CABLE_OUTPUT_KEY: ("VB-Audio Virtual Cable", "0.0.1"),
    CABLE_INPUT_KEY: ("VB-Audio Virtual Cable", "0.0.0"),
}

_COM_LOCAL = threading.local()


def _ensure_com():
    """确保当前线程已初始化 STA COM 套间（pycaw/comtypes 要求）。

    COM 套间是线程级的：Tkinter 主线程会自动初始化，但后台线程
    （如 refresh_devices._work）未初始化，调用 pycaw 会报
    OSError '尚未调用 CoInitialize'。每线程只初始化一次，幂等。
    用 comtypes.CoInitialize 走 STA（与 pycaw 内部一致）。
    """
    if getattr(_COM_LOCAL, "inited", False):
        return
    try:
        from comtypes import CoInitialize
        CoInitialize()
    except Exception:
        try:
            # 回退：直接调 ole32。COINIT_APARTMENTTHREADED = 2（STA），
            # 与 pycaw/comtypes 内部一致——不能用 0（MTA），否则 pycaw
            # 枚举设备会失败。
            ctypes.windll.ole32.CoInitializeEx(None, 2)
        except OSError:
            pass  # 已初始化则忽略
    _COM_LOCAL.inited = True


def _find_dev(name_sub: str):
    """查找 AudioDevice,结果缓存。失败返回 None。

    优先用稳定的驱动描述 + 设备 ID 前缀匹配（抗用户重命名）；
    对未配置稳定规则的 name_sub 回退到 FriendlyName 子串匹配。
    仅匹配 Active 状态的端点，跳过禁用/断开的设备。

    线程安全：枚举在锁外执行（耗时），仅缓存读写加锁（double-check），
    避免长时间持锁阻塞其他线程（主线程 _viz_tick 与后台预热并发）。
    """
    if not _PYCAW_OK:
        return None
    _ensure_com()
    # 快速路径：无锁读
    dev = _DEV_CACHE.get(name_sub)
    if dev is not None:
        return dev
    # 慢路径：枚举（锁外，避免长时间持锁）
    found = None
    try:
        stable = _STABLE_RULES.get(name_sub)
        for d in AudioUtilities.GetAllDevices():
            # 仅匹配 Active 端点（跳过禁用/断开）
            try:
                if d.state.value != 1:   # AudioDeviceState.Active.value == 1
                    continue
            except Exception:
                continue
            if stable is not None:
                # 稳定规则：驱动描述 + ID 前缀
                driver_kw, id_kw = stable
                dev_id = d.id or ""
                if id_kw not in dev_id:
                    continue
                has_driver = False
                try:
                    for v in d.properties.values():
                        if isinstance(v, str) and driver_kw in v:
                            has_driver = True
                            break
                except Exception:
                    pass
                if not has_driver:
                    continue
            else:
                # 回退：FriendlyName 子串匹配
                if name_sub not in (d.FriendlyName or ""):
                    continue
            found = d
            break
    except Exception:
        pass
    # 持锁写入（double-check：可能已被其他线程填充）
    if found is not None:
        with _DEV_CACHE_LOCK:
            if name_sub not in _DEV_CACHE:
                _DEV_CACHE[name_sub] = found
        return found
    return None


def get_cable_output_name_win():
    """返回 CABLE Output 录音端点的实际 FriendlyName（抗用户重命名）。

    按稳定规则（驱动描述 + ID前缀 0.0.1）定位录音端点，返回其
    FriendlyName 供 UI 标签动态显示。失败返回 None。
    """
    dev = _find_dev(CABLE_OUTPUT_KEY)
    if dev is None:
        return None
    return dev.FriendlyName


def get_vb_cable_names_win() -> set:
    """返回所有 VB-CABLE 设备的 FriendlyName 集合（抗用户重命名）。

    遍历 pycaw 设备，检查属性值含 'VB-Audio Virtual Cable' 的设备，
    收集其 FriendlyName。UI 层用此集合判断选中设备是否 VB-CABLE，
    替代硬编码 'CABLE' 子串匹配（重命名后仍生效）。
    """
    if not _PYCAW_OK:
        return set()
    _ensure_com()
    names = set()
    try:
        for d in AudioUtilities.GetAllDevices():
            try:
                if d.state.value != 1:   # 仅 Active
                    continue
            except Exception:
                continue
            is_vb = False
            try:
                for v in d.properties.values():
                    if isinstance(v, str) and "VB-Audio Virtual Cable" in v:
                        is_vb = True
                        break
            except Exception:
                pass
            if is_vb and d.FriendlyName:
                names.add(d.FriendlyName)
    except Exception:
        pass
    return names


def get_endpoint_volume_pct_win(name_sub: str):
    """读 endpoint master volume scalar(0.0~1.0)。失败返回 None。"""
    dev = _find_dev(name_sub)
    if dev is None:
        return None
    try:
        return max(0.0, min(1.0, float(
            dev.EndpointVolume.GetMasterVolumeLevelScalar())))
    except Exception:
        _cache_pop(name_sub)
        return None


def set_endpoint_volume_pct_win(name_sub: str, pct: float) -> bool:
    """写 endpoint master volume scalar(0.0~1.0)。返回是否成功。"""
    dev = _find_dev(name_sub)
    if dev is None:
        return False
    try:
        v = max(0.0, min(1.0, float(pct)))
        dev.EndpointVolume.SetMasterVolumeLevelScalar(v, None)
        return True
    except Exception:
        _cache_pop(name_sub)
        return False


def invalidate_endpoint_volume_cache_win(name_sub=None) -> None:
    """失效缓存(设备拔插/重命名后调用)。name_sub=None 清全部。"""
    if name_sub is None:
        _cache_clear()
    else:
        _cache_pop(name_sub)


def get_endpoint_mute_win(name_sub: str):
    """读端点静音状态。True=静音,False=未静音,None=失败。"""
    dev = _find_dev(name_sub)
    if dev is None:
        return None
    try:
        return bool(dev.EndpointVolume.GetMute())
    except Exception:
        _cache_pop(name_sub)
        return None


def set_endpoint_mute_win(name_sub: str, mute: bool) -> bool:
    """写端点静音状态。返回是否成功。"""
    dev = _find_dev(name_sub)
    if dev is None:
        return False
    try:
        dev.EndpointVolume.SetMute(1 if mute else 0, None)
        return True
    except Exception:
        _cache_pop(name_sub)
        return False