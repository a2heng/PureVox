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

"""纯 Tkinter 版入口：python run_tk.py（单实例锁 + 配置装载）"""

import sys
import datetime

# Fix console encoding for Windows
import os
os.system('chcp 65001 >nul')
if sys.stdout is not None:
    sys.stdout.reconfigure(encoding='utf-8')

def _early_log(msg, tag="SYS"):
    try:
        from user_paths import get_log_path
        now = datetime.datetime.now()
        ts = now.strftime('%Y-%m-%d %H:%M:%S')
        with open(get_log_path(), 'a', encoding='utf-8') as f:
            f.write(f'[{ts}] [{tag:>4s}] {msg}\n')
    except Exception:
        pass


def main():
    _early_log("PureVox(tk) 启动中...")
    from pvplatform.system import acquire_single_instance
    if not acquire_single_instance("PureVox"):
        _early_log("检测到重复启动，本次启动已终止")
        try:
            import tkinter as tk
            from tkinter import messagebox
            r = tk.Tk()
            r.withdraw()
            messagebox.showinfo("PureVox", "PureVox 已在运行，请勿重复启动。")
        except Exception:
            print("程序已在运行")
        sys.exit(1)

    config = None
    try:
        from user_paths import CONFIG_PATH, ensure_dirs
        from config_manager import ConfigManager
        ensure_dirs()
        config = ConfigManager(CONFIG_PATH)
    except Exception:
        pass

    from uitk.main_window import MainWindowTk
    MainWindowTk(config=config).run()


if __name__ == "__main__":
    main()
