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

"""测试套件统一入口（CI 与本地共用）：python tests/run_all.py

依次执行 tests/ 下全部测试文件（各自以 __main__ 语义运行），任一失败
即整体失败。新增测试文件后在此登记即可被所有工作流覆盖。
"""

import os
import runpy
import sys

_HERE = os.path.dirname(os.path.abspath(__file__))

#: 执行顺序 = 依赖从轻到重（纯函数 → 合成 → 传输/设备降级）
TEST_FILES = [
    "test_session_plan.py",     # L3 会话计划（纯函数）
    "test_playback_sink.py",    # 播放正确性（合成时钟，无硬件）
    "test_agc.py",              # AGC 弹道/门限（合成 DSP，无硬件）
    "test_spectrum.py",         # Mel 频谱 64 段/20Hz-16kHz（合成 DSP，无硬件）
    "test_aec_rows.py",         # 行级 AEC：HopQueue 严格配对/AecRow（桩会话，无硬件）
    "test_aec_calib.py",        # AEC far 延迟校准：探测音+延迟估计（合成，无硬件）
    "test_transport.py",        # 传输层导入与优雅降级
    "test_input_resample.py",   # Windows 输入自适应：hop/下混/重采样（合成，无硬件）
    "test_devices.py",          # 设备面：枚举/虚拟麦克风/配置键
]


def main() -> int:
    failed = []
    for name in TEST_FILES:
        path = os.path.join(_HERE, name)
        print(f"\n===== {name} =====")
        try:
            runpy.run_path(path, run_name="__main__")
        except SystemExit as e:
            if e.code not in (None, 0):
                failed.append(name)
        except Exception as e:
            import traceback
            traceback.print_exc()
            failed.append(name)
    if failed:
        print(f"\n失败: {failed}")
        return 1
    print("\n测试套件全部通过")
    return 0


if __name__ == "__main__":
    sys.exit(main())
