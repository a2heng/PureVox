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

"""传输层导入与优雅降级冒烟（无音频硬件可跑，CI 容器/产物内嵌 python 通用）：
python tests/test_transport.py

验证（任何环境恒真）：
- pvplatform.audio 各模块可导入（_libpulse / pwpipe_client / pa_backend /
  media_session / backends），无音频环境不崩溃、按 False 优雅降级；
- PwBridge 未连接时 read/read_far/active 全部安全返回，open 空列表直接
  拒绝（不发起任何连接）；
- backends.probe_backends() / select_backend() 在无后端环境返回空结果；
- MediaSession.start() 无输出设备时返回 False 且 error 有文案（有设备时
  返回 True，随后 stop() 释放）。
"""

import os
import sys

sys.path.insert(0, os.path.dirname(os.path.dirname(os.path.abspath(__file__))))

from pvplatform.audio import _libpulse
from pvplatform.audio.backends import probe_backends, select_backend


def test_libpulse_graceful():
    """libpulse 缺席必须优雅降级（导入不炸、探测返回 False）。"""
    ok = _libpulse.libpulse_available()
    assert isinstance(ok, bool)
    print(f"  libpulse available = {ok}  (容器/无音频环境 False 为正常)")


def test_pwbridge_unconnected_safe():
    from pvplatform.audio.pwpipe_client import PwBridge, pw_available
    assert isinstance(pw_available(), bool)
    bridge = PwBridge()
    # 空列表 = 拒绝打开（在任何连接发生之前），错误文案可读
    assert bridge.open([], []) is False
    assert bridge.last_error()
    assert bridge.active() is False
    assert bridge.read(480) is None
    assert bridge.read_each(480) is None
    assert bridge.read_far_h(0, 480) is None
    assert bridge.far_available(0) == 0
    # far 自建会话：无音频环境返回 <0；有 PipeWire 时设备名未知会回退默认
    # 源，可能 >=0——两者都算合法（不得崩溃），返回后按句柄安全关闭。
    h = bridge.open_far("nosuch", monitor=True)
    assert isinstance(h, int)
    if h >= 0:
        bridge.close_far(h)
    bridge.close_far(999)        # 无效句柄关闭必须无异常
    bridge.close()               # 未连接状态关闭必须无异常
    bridge.close()               # 幂等
    print("  PwBridge 未连接安全 + open 空列表拒绝  OK")


def test_pabridge_import():
    from pvplatform.audio.pa_backend import PaBridge
    bridge = PaBridge()
    assert bridge.active() is False
    print("  PaBridge 可导入（pyaudio 懒加载，Linux 导入不受影响）  OK")


def test_backends_registry():
    probe = probe_backends()
    assert isinstance(probe, list)
    for spec, ok in probe:
        assert spec.name and isinstance(ok, bool)
    sel = select_backend(frozenset())
    assert sel is None or hasattr(sel, "name")
    print(f"  backends probe = {[(s.name, ok) for s, ok in probe]}  select = "
          f"{sel.name if sel else None}  OK")


class _SinkStub:
    def write(self, frame):
        pass

    def pull(self, n):
        return [0.0] * n


def test_media_session_no_device():
    from pvplatform.audio.media_session import MediaSession
    session = MediaSession(lambda n: [0.0] * n, [], lambda: _SinkStub())
    started = session.start()
    assert isinstance(started, bool)
    if started:
        session.stop()           # 有音频设备的开发机：启动成功后立即释放
        print("  MediaSession 启动成功（有输出设备）→ stop 释放  OK")
    else:
        assert session.error     # 无设备环境必须给出错误文案而非崩溃
        print("  MediaSession 无设备优雅降级（error 有文案）  OK")


if __name__ == "__main__":
    print("传输层冒烟:")
    test_libpulse_graceful()
    test_pwbridge_unconnected_safe()
    test_pabridge_import()
    test_backends_registry()
    test_media_session_no_device()
    print("全部通过")
