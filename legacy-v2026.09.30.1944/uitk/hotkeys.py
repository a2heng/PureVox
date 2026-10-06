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

"""全局热键（仅 Windows）：Win32 RegisterHotKey，事件驱动零轮询。

键位一律用规范字符串表达，修饰键顺序固定 `Ctrl+Alt+Shift+Win`，如
`Ctrl+Alt+1` / `Alt+.` / `F8`；**空串 = 不监听**（可置空即解绑）。
本模块是键位字符串的唯一契约：
- `keysym_to_spec`：录制弹窗把 Tk 键事件转成规范串（纯字母/数字必须带
  至少一个修饰键，F1–F24 可单键）；
- `normalize_spec`：校验/规范化；
- `GlobalHotkeys.set_bindings([(action, spec), ...])`：注册（同步返回，
  `last_failed` 给出被系统/他程序占用而未生效的键位）。

独占一条消息线程 + message-only 窗口；`SendMessage` 重注册保证调用返回时
结果已知；`on_trigger(action)` 在热键线程回调（UI 侧自行投递主线程）。
非 Windows 平台 set_bindings 为空操作。
"""

import ctypes
import threading
from ctypes import wintypes

WM_APP_REBIND = 0x8000 + 201
WM_HOTKEY = 0x0312
WM_CLOSE = 0x0010
MOD_ALT = 0x0001
MOD_CONTROL = 0x0002
MOD_SHIFT = 0x0004
MOD_WIN = 0x0008
MOD_NOREPEAT = 0x4000
ID_BASE = 0xB000
HWND_MESSAGE = -3

_MOD_ORDER = (("Ctrl", MOD_CONTROL), ("Alt", MOD_ALT),
              ("Shift", MOD_SHIFT), ("Win", MOD_WIN))
_MOD_NAMES = {name.lower(): bit for name, bit in _MOD_ORDER}
_MOD_NAMES["control"] = MOD_CONTROL
_MOD_NAMES["super"] = MOD_WIN
_MOD_NAMES["meta"] = MOD_WIN

# 单独按下修饰键（录制时应继续等待真正的主键）
MODIFIER_KEYSYMS = {
    "control_l": "Ctrl", "control_r": "Ctrl",
    "alt_l": "Alt", "alt_r": "Alt",
    "shift_l": "Shift", "shift_r": "Shift",
    "super_l": "Win", "super_r": "Win",
    "win_l": "Win", "win_r": "Win",
    "meta_l": "Win", "meta_r": "Win",
}


def _build_tokens():
    t = {}
    for i in range(26):
        c = chr(ord("a") + i)
        t[c] = (0x41 + i, c.upper())
    for i in range(10):
        t[str(i)] = (0x30 + i, str(i))
        t["num%d" % i] = (0x60 + i, "Num%d" % i)
    for i in range(1, 25):
        t["f%d" % i] = (0x70 + i - 1, "F%d" % i)
    t.update({
        "backspace": (0x08, "Backspace"), "tab": (0x09, "Tab"),
        "enter": (0x0D, "Enter"), "esc": (0x1B, "Esc"),
        "space": (0x20, "Space"), "pageup": (0x21, "PageUp"),
        "pagedown": (0x22, "PageDown"), "end": (0x23, "End"),
        "home": (0x24, "Home"), "left": (0x25, "Left"),
        "up": (0x26, "Up"), "right": (0x27, "Right"),
        "down": (0x28, "Down"), "insert": (0x2D, "Insert"),
        "delete": (0x2E, "Delete"),
        "`": (0xC0, "`"), "-": (0xBD, "-"), "=": (0xBB, "="),
        "[": (0xDB, "["), "]": (0xDD, "]"), "\\": (0xDC, "\\"),
        ";": (0xBA, ";"), "'": (0xDE, "'"), ",": (0xBC, ","),
        ".": (0xBE, "."), "/": (0xBF, "/"),
    })
    return t


_TOKENS = _build_tokens()

# Tk keysym → 规范 token（含 Shift 变体；Shift 由录制器单独跟踪）
_KEYSYM_TOKEN = {
    "period": ".", "greater": ".",
    "comma": ",", "less": ",",
    "slash": "/", "question": "/",
    "semicolon": ";", "colon": ";",
    "apostrophe": "'", "quotedbl": "'",
    "bracketleft": "[", "braceleft": "[",
    "bracketright": "]", "braceright": "]",
    "backslash": "\\", "bar": "\\",
    "grave": "`", "asciitilde": "`",
    "minus": "-", "underscore": "-",
    "equal": "=", "plus": "=",
    "exclam": "1", "at": "2", "numbersign": "3", "dollar": "4",
    "percent": "5", "asciicircum": "6", "ampersand": "7",
    "asterisk": "8", "parenleft": "9", "parenright": "0",
    "space": "space", "return": "enter", "kp_enter": "enter",
    "backspace": "backspace", "tab": "tab", "escape": "esc",
    "delete": "delete", "insert": "insert",
    "home": "home", "end": "end",
    "prior": "pageup", "next": "pagedown",
    "left": "left", "up": "up", "right": "right", "down": "down",
}
for _i in range(10):
    _KEYSYM_TOKEN["kp_%d" % _i] = "num%d" % _i


def _is_function(token: str) -> bool:
    return (token.startswith("f") and token[1:].isdigit()
            and 1 <= int(token[1:]) <= 24)


def format_spec(mods, token: str) -> str:
    """（修饰键名集合, token）→ 规范串。

    mods 为 {"Ctrl","Alt","Shift","Win"} 子集；顺序固定，便于比较与展示。
    """
    ent = _TOKENS.get(str(token).lower())
    display = ent[1] if ent else str(token)
    parts = [name for name, _bit in _MOD_ORDER if name in mods]
    parts.append(display)
    return "+".join(parts)


def keysym_to_spec(keysym: str, mods) -> str:
    """录制：Tk 键事件 → 规范串；不可用/缺修饰键返回 ""。"""
    low = str(keysym).lower()
    if low in MODIFIER_KEYSYMS:
        return ""
    token = _KEYSYM_TOKEN.get(low)
    if token is None and low in _TOKENS:
        token = low          # 字母 / 数字 / F1–F24 等 keysym 本身即 token
    if token is None:
        s = str(keysym)
        if len(s) == 1 and s.isprintable():
            token = s.lower()
    if token is None:
        return ""
    token = token.lower()
    if token not in _TOKENS:
        return ""
    if not mods and not _is_function(token):
        return ""
    return format_spec(mods, token)


def parse_spec(spec: str):
    """规范串 → (mods_bitmask, vk)；空串/非法/无修饰非功能键返回 None。"""
    if not spec:
        return None
    bits = 0
    token = None
    for raw in str(spec).split("+"):
        p = raw.strip()
        if not p:
            continue
        low = p.lower()
        if low in _MOD_NAMES:
            bits |= _MOD_NAMES[low]
        else:
            token = low
    if token is None:
        return None
    ent = _TOKENS.get(token)
    if ent is None:
        return None
    if not bits and not _is_function(token):
        return None
    return bits, ent[0]


def normalize_spec(spec: str) -> str:
    """校验并归一化；非法返回 ""（空串本身合法，表示不监听）。"""
    if not spec:
        return ""
    if parse_spec(spec) is None:
        return ""
    parts = []
    bits = 0
    token = None
    for raw in str(spec).split("+"):
        p = raw.strip()
        if not p:
            continue
        low = p.lower()
        if low in _MOD_NAMES:
            bits |= _MOD_NAMES[low]
        else:
            token = low
    parts = [name for name, bit in _MOD_ORDER if bits & bit]
    parts.append(_TOKENS[token][1])
    return "+".join(parts)


class GlobalHotkeys:
    """热键宿主。on_trigger(action) 在热键线程回调。"""

    def __init__(self, on_trigger):
        self._on_trigger = on_trigger
        self._lock = threading.Lock()
        self._bindings = []       # [(action, spec)]
        self._hwnd = None
        self._tid = None
        self._proc = None
        self._ready = threading.Event()
        self.last_failed = []     # 最近一次注册失败的 spec 列表
        threading.Thread(target=self._run, daemon=True).start()
        self._ready.wait()

    def set_bindings(self, bindings):
        """[(action, spec), ...]；空 spec 自动跳过。同步返回注册结果。"""
        with self._lock:
            self._bindings = [(a, str(s or "")) for a, s in (bindings or [])]
        hwnd = self._hwnd
        if hwnd:
            ctypes.windll.user32.SendMessageW(hwnd, WM_APP_REBIND, 0, 0)

    def stop(self):
        hwnd = self._hwnd
        if hwnd:
            ctypes.windll.user32.PostMessageW(hwnd, WM_CLOSE, 0, 0)

    # ── 热键线程 ──
    def _run(self):
        if not _is_win():
            self._ready.set()
            return
        user32 = ctypes.windll.user32
        kernel32 = ctypes.windll.kernel32
        self._tid = int(kernel32.GetCurrentThreadId())
        # 显式签名：WPARAM/LPARAM 是指针宽整数，缺省按 int32 处理会溢出
        user32.DefWindowProcW.restype = ctypes.c_longlong
        user32.DefWindowProcW.argtypes = [
            wintypes.HWND, ctypes.c_uint,
            ctypes.c_size_t, ctypes.c_longlong]
        registered = []
        id_map = {}

        def rebind():
            for hid in registered:
                user32.UnregisterHotKey(None, hid)
            registered.clear()
            id_map.clear()
            with self._lock:
                bindings = list(self._bindings)
            failed = []
            seen = set()
            for i, (action, spec) in enumerate(bindings):
                parsed = parse_spec(spec)
                if parsed is None:
                    continue
                bits, vk = parsed
                if (bits, vk) in seen:
                    continue
                hid = ID_BASE + i
                # hWnd=NULL：WM_HOTKEY 投递到本线程消息队列（非窗口），
                # 由消息循环直接取，不经 DispatchMessage/wnd_proc
                if user32.RegisterHotKey(
                        None, hid, bits | MOD_NOREPEAT, vk):
                    registered.append(hid)
                    id_map[hid] = action
                    seen.add((bits, vk))
                else:
                    failed.append(spec)
            self.last_failed = failed

        def wnd_proc(hwnd, msg, wp, lp):
            # 注意：这里收不到 WM_HOTKEY（NULL hWnd 注册的是线程消息），
            # 热键在下面的 GetMessage 循环里直接处理
            if msg == WM_APP_REBIND:
                rebind()
                return 0
            if msg == WM_CLOSE:
                for hid in registered:
                    try:
                        user32.UnregisterHotKey(None, hid)
                    except Exception:
                        pass
                registered.clear()
                user32.DestroyWindow(hwnd)
                return 0
            return user32.DefWindowProcW(hwnd, msg, wp, lp)

        WNDPROC = ctypes.WINFUNCTYPE(
            ctypes.c_longlong, wintypes.HWND, ctypes.c_uint,
            wintypes.WPARAM, wintypes.LPARAM)
        proc = WNDPROC(wnd_proc)
        self._proc = proc

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

        kernel32.GetModuleHandleW.restype = wintypes.HINSTANCE
        kernel32.GetModuleHandleW.argtypes = [wintypes.LPCWSTR]
        hinst = kernel32.GetModuleHandleW(None)
        cls = WNDCLASSW()
        cls.lpfnWndProc = proc
        cls.lpszClassName = "PureVoxHotkeys"
        cls.hInstance = hinst
        user32.RegisterClassW(ctypes.byref(cls))
        user32.CreateWindowExW.restype = wintypes.HWND
        user32.CreateWindowExW.argtypes = [
            wintypes.DWORD, wintypes.LPCWSTR, wintypes.LPCWSTR, wintypes.DWORD,
            ctypes.c_int, ctypes.c_int, ctypes.c_int, ctypes.c_int,
            wintypes.HWND, wintypes.HMENU, wintypes.HINSTANCE, wintypes.LPVOID]
        # message-only 窗口：parent = HWND_MESSAGE
        hwnd = user32.CreateWindowExW(
            0, "PureVoxHotkeys", "PureVoxHotkeys", 0,
            0, 0, 0, 0, wintypes.HWND(HWND_MESSAGE), None, hinst, None)
        self._hwnd = hwnd
        rebind()
        self._ready.set()
        msg = wintypes.MSG()
        while user32.GetMessageW(ctypes.byref(msg), None, 0, 0) > 0:
            # WM_HOTKEY 是线程消息（RegisterHotKey 传 NULL hWnd），
            # DispatchMessage 不会送到窗口过程——必须在此直接处理
            if msg.message == WM_HOTKEY:
                action = id_map.get(int(msg.wParam))
                if action is not None:
                    try:
                        self._on_trigger(action)
                    except Exception:
                        pass
                continue
            user32.TranslateMessage(ctypes.byref(msg))
            user32.DispatchMessageW(ctypes.byref(msg))


def _is_win() -> bool:
    import sys
    return sys.platform.startswith("win")
