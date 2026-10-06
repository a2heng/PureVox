# PureVox — AI 麦克风降噪工具
# Copyright (C) 2024-2026 a2heng <752848283@qq.com>
# SPDX-License-Identifier: GPL-3.0-or-later

"""uitk 包：纯标准库 Tkinter UI（桌面端唯一实现）。

分层：
  theme.py       颜色令牌（墨黑主题 + 系统 accent），唯一取色处
  metrics.py     尺寸表 / 分辨率挡位 / HIDPI / 像素字体注册与选择
  widgets.py     基础组件（FlatButton/DarkCombo/DarkCheck/ScrollFrame）
  viz.py         可视化（VU 段表/频谱直方图/运行指示灯，纯 Canvas 自绘）
  tray.py        Windows 托盘图标（ctypes Shell_NotifyIcon，零依赖）
  engine.py      引擎控制器（链→SessionPlan→音频流启停 + 设备枚举）
  main_window.py 主窗口（工具条 + 节点面板 + plugin_chain 双向绑定）

约定：变量宽度 = pack(fill=X, expand)；变量颜色 = 一律经 theme 取色；
分辨率缩放 = sizes 表唯一来源，换挡 update + apply_sizes。
"""
