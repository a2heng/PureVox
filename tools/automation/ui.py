#!/usr/bin/env python3
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
"""Linux 界面自动化驱动（AT-SPI）——Windows 的 UI Automation 在 Linux 的对应物。

按**可访问名**查找 / 点击 WebView 里的控件（不需要像素坐标），用于无人点击时验证界面交互
（AGENTS.md「调试优先」：交互验证不许以「点不了」为由跳过）。

用法（在桌面会话里跑：DISPLAY / DBUS_SESSION_BUS_ADDRESS / XDG_RUNTIME_DIR 已设）：
    python3 tools/automation/ui.py list                # 列出 PureVox 窗口里所有命名控件
    python3 tools/automation/ui.py click "校准"        # 点击第一个名为「校准」的控件
    python3 tools/automation/ui.py click "启动"
    python3 tools/automation/ui.py sel "EDIFIER R20, USB Audio"   # 在打开的下拉里选一项
依赖：python3-pyatspi（apt install python3-pyatspi）。窗口标题含 `PureVox` 即可。
"""

import sys
import time

try:
  import pyatspi  # type: ignore
except ImportError:
  sys.exit("缺少 python3-pyatspi（sudo apt install python3-pyatspi）")

APP_HINT = "purevox"


def _frames():
  desk = pyatspi.Registry.getDesktop(0)
  for app in desk:
    if app and APP_HINT in (app.name or "").lower():
      for c in app:
        yield c


def _walk(n):
  yield n
  try:
    for c in n:
      yield from _walk(c)
  except Exception:
    return


def _name(n):
  try:
    return n.name or ""
  except Exception:
    return ""


def find(text, exact=True):
  out = []
  for f in _frames():
    for n in _walk(f):
      nm = _name(n)
      if (nm == text) if exact else (text in nm):
        out.append(n)
  return out


def do_click(n):
  try:
    a = n.queryAction()
  except Exception as e:
    return f"（该控件不支持动作：{e}）"
  for i in range(a.nActions):
    if a.getName(i) in ("click", "press", "activate", "jump"):
      a.doAction(i)
      return f"action={a.getName(i)}"
  a.doAction(0)
  return "action=默认"


def cmd_list():
  for f in _frames():
    print(f"窗口：{_name(f)}")
    for n in _walk(f):
      nm = _name(n)
      if nm.strip():
        try:
          role = n.getRoleName()
        except Exception:
          role = "?"
        print(f"  [{role}] {nm!r}")


def cmd_click(text):
  nodes = find(text)
  if not nodes:
    print(f"找不到控件：{text!r}")
    near = find(text, exact=False)
    if near:
      print("近似匹配：")
      for n in near[:20]:
        print(f"  [{n.getRoleName()}] {_name(n)!r}")
    return 1
  n = nodes[0]
  print(f"点击 [{n.getRoleName()}] {text!r} {do_click(n)}")
  return 0


def main(argv):
  if len(argv) < 2:
    print(__doc__)
    return 2
  op = argv[1]
  if op == "list":
    cmd_list()
    return 0
  if op in ("click", "sel"):
    if len(argv) < 3:
      print("用法：ui.py click <控件名>")
      return 2
    # AT-SPI 的下拉选项在展开后才出现；点击前若目标正是某个 <option>，直接点它
    return cmd_click(argv[2])
  print(__doc__)
  return 2


if __name__ == "__main__":
  sys.exit(main(sys.argv))
