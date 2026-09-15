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

"""关于页元数据与文档加载器（无任何 GUI 依赖）。

大文本全部是真正的 markdown 文件，按页存放、直接手改：
    about/changelog.md   更新日志——发版/改动的唯一记录位置
    about/windows.md     Windows 使用手册
    about/linux.md       Linux 使用手册
本模块只保留小型元数据（应用信息/链接/第三方库清单）与介绍页、
许可证页文本；uitk.dialogs 渲染关于对话框的五个标签页。
写入手册/日志时保持中文、无 emoji；只记用户可感知的技术变更（见 AGENTS.md）。
"""

import os
import sys

APP_NAME = 'PureVox'
APP_DESC = 'AI 麦克风降噪工具'
APP_AUTHOR = 'a2heng'
BUILD_DATE = ''   # 打包脚本写入 yyyy-MM-dd-HHmm；源码态留空显示「开发版」

URLS = {
    'GitHub': 'https://a2heng.github.io/',
    '哔哩哔哩': 'https://space.bilibili.com/10850943',
    '声音测试工具': 'https://a2heng.github.io/mic-test.html',
}

# 第三方库清单（关于页「许可证」标签渲染）
LIBS = [
    {'name': 'numpy', 'ver': '2.x', 'license': 'BSD-3',
     'url': 'https://numpy.org/', 'desc': '帧级 DSP 数值计算'},
    {'name': 'scipy', 'ver': '1.18.x', 'license': 'BSD-3',
     'url': 'https://scipy.org/', 'desc': 'EQ 双二阶层联（lfilter）'},
    {'name': 'onnxruntime', 'ver': '1.29.x', 'license': 'MIT',
     'url': 'https://onnxruntime.ai/', 'desc': 'AI 模型推理'},
    {'name': 'PyAudio', 'ver': '0.2.x', 'license': 'MIT',
     'url': 'https://people.csail.mit.edu/hubert/pyaudio/',
     'desc': '设备枚举 / 虚拟声卡检测（Windows/macOS）'},
    {'name': 'zeroconf', 'ver': '0.150.x', 'license': 'LGPL-2.1',
     'url': 'https://github.com/python-zeroconf/python-zeroconf',
     'desc': 'mDNS 服务发现'},
    {'name': 'aiohttp', 'ver': '3.14.x', 'license': 'Apache-2.0',
     'url': 'https://docs.aiohttp.org/', 'desc': '远程麦克风 HTTPS/WSS 服务'},
    {'name': 'cryptography', 'ver': '50.x', 'license': 'Apache-2.0 / BSD-3',
     'url': 'https://cryptography.io/', 'desc': '自签名 TLS 证书'},
    {'name': 'opuslib', 'ver': '3.0.x', 'license': 'BSD-3',
     'url': 'https://github.com/STACi32/opuslib-python',
     'desc': 'Opus 编解码绑定（ctypes 系统 libopus）'},
    {'name': 'miniaudio', 'ver': '1.71', 'license': 'MIT',
     'url': 'https://github.com/serge-rgb/miniaudio',
     'desc': '媒体解码/播放（自包含 C）'},
]

_INTRO_TEXT = """\
# PureVox

AI 麦克风降噪工具

版本: {BUILD_DATE} · 作者: a2heng · Copyright (C) 2024-2026 a2heng · GPL-3.0-or-later

[GitHub](https://a2heng.github.io/) · [哔哩哔哩](https://space.bilibili.com/10850943) · [声音测试工具](https://a2heng.github.io/mic-test.html)

## 它是什么

PureVox 是一款 AI 麦克风降噪工具，实时消除键盘声、鼠标声、电流声、
风扇声、风声等背景噪音，只保留纯净人声；也支持目标说话人提取与回声消除。

## 功能特性

- 实时 AI 降噪：48kHz 模型推理，本地低延迟链路
- 目标说话人提取（TSE）：只保留指定人的声音
- 回声消除（AEC）：消除扬声器外放的回声
- 均衡器：10/31/61 段三种规格 EQ + 高切低切 + 预设，可视化拖拽
- 远程麦克风：手机 App / 浏览器经局域网推流到电脑降噪
- 虚拟声卡：Windows 用 VB-CABLE；Linux 用原生 PipeWire 虚拟麦克风
- 音效板：音效单次播放，每个可自定义全局快捷键与音量
- 便捷功能：可变托盘图标、可自定义全局快捷键、启停提示音、开机自启、
  系统声音面板直达、VU 电平表

## 支持平台

Windows（含 VB-CABLE 虚拟声卡）与 Linux（原生 PipeWire），
详细操作见「Windows 使用」「Linux 使用」标签页。
"""

_LICENSE_TEXT = """\
## 许可条款

本软件采用 **GPL-3.0 开源许可证**。

1) GNU General Public License v3.0 (GPL-3.0) —— 开源许可。您有权自由使用、
修改与分发，但修改版须保持 GPL-3.0 并公开源码。
[GPL-3.0 全文](https://www.gnu.org/licenses/gpl-3.0.html)

2) **内置 AI 模型 —— 单独授权**。模型不随 GPL 授权，禁止用于其他项目，
仅可在 PureVox 内经作者授权使用（见 MODEL-LICENSE.md）。

## 第三方库

{LIBS_MD}

VB-CABLE 为 VB-Audio 专有驱动，用户在 Windows 端自行安装，不随本软件分发。
"""

_LICENSE_TEXT = _LICENSE_TEXT.replace('{LIBS_MD}', '\n'.join(
    '- [%s %s](%s) — %s · %s'
    % (l['name'], l['ver'], l['url'], l['license'], l['desc'])
    for l in LIBS))


def load_doc(name: str) -> str:
    """读取 about/<name>.md（PyInstaller 冻结资源 → 应用根 → 源码目录）。"""
    bases = []
    meipass = getattr(sys, "_MEIPASS", None)
    if meipass:
        bases.append(os.path.join(meipass, "about"))
        bases.append(meipass)
    here = os.path.dirname(os.path.abspath(__file__))
    bases.append(os.path.join(here, "about"))
    for base in bases:
        path = os.path.join(base, name + ".md")
        if os.path.isfile(path):
            try:
                with open(path, encoding="utf-8") as f:
                    return f.read()
            except Exception:
                pass
    return "（缺失文档 about/%s.md）" % name
