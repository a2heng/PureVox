# PureVox Lite — Windows 托盘图标（ctypes Shell_NotifyIcon，零第三方依赖）
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

"""Lite 托盘图标（仅 Windows）：与主线 uitk/tray.py 同一原语。

健壮性三条原则（无看门狗、无定时器、无延迟重试）：
1. 创建即校验——窗口与 NIM_ADD 在托盘线程完成后经 Event 同步回传布尔结果，
   构造函数返回前真相已知（异常路径同样置位，不死锁）；
2. 事件驱动保活——监听系统广播 TaskbarCreated（explorer 重启即收到），
   收到立刻重新 NIM_ADD，图标自愈；
3. 策略跟随状态——alive 只反映最近一次添加的真实结果，
   调用方据此决定「关窗=隐藏」还是「关窗=退出」，僵尸进程结构上不可能。

菜单为动态构建：右键弹出时调用 build_menu() 现算勾选态（等价 pystray
checked=lambda 的实时语义），项规格：
  None                              分隔线
  {"label","cb"}                    普通项
  {"label","cb","checked"}          可勾选项（当前态打勾）
  {"label","cb","default"}          默认加粗项（左键动作）
  {"label","sub":[...]}             子菜单
回调均在托盘线程执行；涉及 Tk 的操作由调用方自行 after(0) 投递。
"""

import ctypes
import threading
from ctypes import wintypes

WM_APP_TRAY = 0x8000 + 100
WM_CLOSE = 0x0010

MF_STRING = 0x00000000
MF_SEPARATOR = 0x00000800
MF_POPUP = 0x00000010
MF_CHECKED = 0x00000008
TPM_RIGHTBUTTON = 0x0002
TPM_RETURNCMD = 0x0100


def load_tray_hicon(ico_path):
    """从 ico 资产加载 HICON：取系统任务栏实际像素宽（SM_CXSMICON，随
    DPI 变化）的帧——资产内含 16/20/24/32/48/64/128/256 全帧梯，
    命中即 1:1 渲染，无任何重采样。"""
    user32 = ctypes.windll.user32
    sm = user32.GetSystemMetrics(49) or 16     # SM_CXSMICON
    IMAGE_ICON, LR_LOADFROMFILE = 1, 0x10
    return user32.LoadImageW(None, ico_path, IMAGE_ICON, sm, sm,
                             LR_LOADFROMFILE)


class NOTIFYICONDATAW(ctypes.Structure):
    class _union(ctypes.Union):
        _fields_ = [("uTimeout", ctypes.c_uint),
                    ("uVersion", ctypes.c_uint)]
    _fields_ = [("cbSize", ctypes.c_uint),
                ("hWnd", wintypes.HWND),
                ("uID", ctypes.c_uint),
                ("uFlags", ctypes.c_uint),
                ("uCallbackMessage", ctypes.c_uint),
                ("hIcon", wintypes.HICON),
                ("szTip", ctypes.c_wchar * 128),
                ("dwState", ctypes.c_uint),
                ("dwStateMask", ctypes.c_uint),
                ("szInfo", ctypes.c_wchar * 256),
                ("uVersionOrTimeout", _union),
                ("szInfoTitle", ctypes.c_wchar * 64),
                ("dwInfoFlags", ctypes.c_uint)]


class LiteTray:
    def __init__(self, ico_path, cls_name, tip,
                 on_show, on_quit, build_menu):
        self.on_show = on_show
        self.on_quit = on_quit
        self.build_menu = build_menu
        self._tip = tip[:127]
        self._cls = cls_name
        self._hicon = load_tray_hicon(ico_path)
        if not self._hicon:
            raise OSError(f"tray icon load failed: {ico_path}")
        self._nid = None
        self._hwnd = None
        # alive = 最近一次 NIM_ADD 的真实结果；创建结果在构造返回前同步可得
        self.alive = False
        self._ready = threading.Event()
        t = threading.Thread(target=self._run, daemon=True)
        t.start()
        self._ready.wait()

    def stop(self):
        """结束消息循环并移除图标（进程退出前调用）。"""
        self.alive = False
        try:
            if self._nid is not None:
                ctypes.windll.shell32.Shell_NotifyIconW(
                    0x02, ctypes.byref(self._nid))   # NIM_DELETE
            if self._hwnd is not None:
                ctypes.windll.user32.PostMessageW(self._hwnd, WM_CLOSE, 0, 0)
        except Exception:
            pass

    def _run(self):
        try:
            self._run_inner()
        except Exception:
            pass
        finally:
            # 异常路径兜底置位，保证构造函数不死锁；
            # 正常路径的置位在 NIM_ADD 完成处（早于消息循环）
            self._ready.set()

    def _run_inner(self):
        user32 = ctypes.windll.user32
        shell32 = ctypes.windll.shell32
        kernel32 = ctypes.windll.kernel32
        from ctypes import wintypes

        user32.DefWindowProcW.restype = ctypes.c_longlong
        user32.DefWindowProcW.argtypes = [
            wintypes.HWND, ctypes.c_uint,
            ctypes.c_size_t, ctypes.c_longlong]

        WNDPROC = ctypes.WINFUNCTYPE(
            ctypes.c_longlong, wintypes.HWND, ctypes.c_uint,
            wintypes.WPARAM, wintypes.LPARAM)

        class WNDCLASSW(ctypes.Structure):
            _fields_ = [("style", ctypes.c_uint),
                        ("lpfnWndProc", WNDPROC),
                        ("cbClsExtra", ctypes.c_int),
                        ("cbWndExtra", ctypes.c_int),
                        ("hInstance", wintypes.HINSTANCE),
                        ("hIcon", wintypes.HICON),
                        ("hCursor", ctypes.c_void_p),
                        ("hbrBackground", wintypes.HBRUSH),
                        ("lpszMenuName", wintypes.LPCWSTR),
                        ("lpszClassName", wintypes.LPCWSTR)]

        # 系统注册广播：explorer（任务栏）重启时所有托盘图标被清空，
        # 广播此消息通知重加；这是事件驱动自愈的唯一通路，无定时器
        RegisterWindowMessageW = user32.RegisterWindowMessageW
        RegisterWindowMessageW.restype = ctypes.c_uint
        RegisterWindowMessageW.argtypes = [wintypes.LPCWSTR]
        wm_taskbar = RegisterWindowMessageW("TaskbarCreated")

        nid = NOTIFYICONDATAW()

        # 事件识别兼容两代 Shell 约定：经典版 lParam 本体即鼠标事件；
        # NOTIFYICON_VERSION_4 起 lParam 低 16 位才是事件（高位为坐标），
        # 另有键盘唤起的 WM_CONTEXTMENU / NIN_SELECT。掩码取低字全覆盖。
        WM_CONTEXTMENU = 0x007B
        NIN_SELECT = 0x0400

        def _event(lp):
            return lp & 0xFFFF

        def wnd_proc(hwnd, msg, wp, lp):
            if msg == WM_APP_TRAY:
                ev = _event(lp)
                if ev in (0x0202, NIN_SELECT) and self.on_show:   # 左键抬起/选中
                    self.on_show()
                    return 0
                if ev in (0x0205, WM_CONTEXTMENU):                # 右键抬起/上下文
                    self._popup_menu(hwnd, user32)
                    return 0
                return 0
            if msg == wm_taskbar:
                # explorer 重启广播：立刻重加图标（事件驱动自愈）
                self.alive = bool(shell32.Shell_NotifyIconW(0x00, ctypes.byref(nid)))
                return 0
            if msg == WM_CLOSE:
                user32.DestroyWindow(hwnd)
                return 0
            return user32.DefWindowProcW(hwnd, msg, wp, lp)

        proc = WNDPROC(wnd_proc)
        # 显式签名：句柄是 64 位，缺省按 c_int 处理会截断/溢出
        kernel32.GetModuleHandleW.restype = wintypes.HINSTANCE
        kernel32.GetModuleHandleW.argtypes = [wintypes.LPCWSTR]
        hinst = kernel32.GetModuleHandleW(None)
        cls = WNDCLASSW()
        cls.lpfnWndProc = proc
        cls.lpszClassName = self._cls
        cls.hInstance = hinst
        user32.RegisterClassW(ctypes.byref(cls))
        user32.CreateWindowExW.restype = wintypes.HWND
        user32.CreateWindowExW.argtypes = [
            wintypes.DWORD, wintypes.LPCWSTR, wintypes.LPCWSTR, wintypes.DWORD,
            ctypes.c_int, ctypes.c_int, ctypes.c_int, ctypes.c_int,
            wintypes.HWND, wintypes.HMENU, wintypes.HINSTANCE, wintypes.LPVOID]
        hwnd = user32.CreateWindowExW(0, self._cls, self._cls,
                                      0, 0, 0, 0, 0, None, None, hinst, None)
        self._hwnd = hwnd
        nid.cbSize = ctypes.sizeof(nid)
        nid.hWnd = hwnd
        nid.uID = 1
        nid.uFlags = 0x1 | 0x2 | 0x4   # MESSAGE | ICON | TIP
        nid.uCallbackMessage = WM_APP_TRAY
        nid.hIcon = self._hicon
        nid.szTip = self._tip
        self._nid = nid
        # 创建即校验：结果直接写回 self.alive，构造函数经 Event 同步读取；
        # 不重试不等待——失败即向上层如实报告（上层据此走无托盘路径）
        self.alive = bool(shell32.Shell_NotifyIconW(0x00, ctypes.byref(nid)))  # NIM_ADD
        # 设置完成：真相已定，放行构造函数；本线程转入消息循环长期驻留
        self._ready.set()
        msg = wintypes.MSG()
        while user32.GetMessageW(ctypes.byref(msg), None, 0, 0) > 0:
            user32.TranslateMessage(ctypes.byref(msg))
            user32.DispatchMessageW(ctypes.byref(msg))

    def _popup_menu(self, hwnd, user32):
        """右键弹出：勾选态此刻现算（跟随当前配置/缩放），选中项立即派发。"""
        cmd_map = {}
        next_id = [4001]

        def add_spec(hmenu, spec):
            if spec is None:
                user32.AppendMenuW(hmenu, MF_SEPARATOR, 0, None)
                return
            sub_specs = spec.get("sub")
            if sub_specs is not None:
                hsub = user32.CreatePopupMenu()
                try:
                    for s in sub_specs:
                        add_spec(hsub, s)
                    user32.AppendMenuW(hmenu, MF_STRING | MF_POPUP,
                                       hsub, spec["label"])
                except Exception:
                    user32.DestroyMenu(hsub)
                return
            mid = next_id[0]
            next_id[0] += 1
            cmd_map[mid] = spec["cb"]
            flags = MF_STRING | (MF_CHECKED if spec.get("checked") else 0)
            user32.AppendMenuW(hmenu, flags, mid, spec["label"])
            if spec.get("default"):
                # 加粗默认项（等价 pystray default=True 的视觉语义）
                user32.SetMenuDefaultItem(hmenu, mid, 0)

        menu = user32.CreatePopupMenu()
        try:
            for spec in self.build_menu():
                add_spec(menu, spec)
            pt = ctypes.wintypes.POINT()
            user32.GetCursorPos(ctypes.byref(pt))
            user32.SetForegroundWindow(hwnd)
            pick = user32.TrackPopupMenu(menu, TPM_RIGHTBUTTON | TPM_RETURNCMD,
                                         pt.x, pt.y, 0, hwnd, None)
            user32.PostMessageW(hwnd, 0x0000, 0, 0)     # WM_NULL 收尾（KB135788）
            if pick:
                cb = cmd_map.get(pick)
                if cb:
                    cb()
        finally:
            user32.DestroyMenu(menu)


def make_tray(ico_path, cls_name, tip, on_show, on_quit, build_menu):
    """入口：Windows 且图标加载成功且 NIM_ADD 成功才返回 LiteTray，否则 None。"""
    import os
    import sys
    if not sys.platform.startswith("win"):
        return None
    if not os.path.isfile(ico_path):
        return None
    try:
        obj = LiteTray(ico_path, cls_name, tip, on_show, on_quit, build_menu)
    except Exception:
        return None
    return obj if obj.alive else None
