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

"""二维码 → Tk PhotoImage（零 PIL 依赖）。

qrcode 矩阵 → 内存 PPM P6 → Tk 8.6+ 原生 PhotoImage；整数倍 NEAREST 放大
保证像素对齐（不模糊、不裁切）。主线网络输入节点与 Lite Net 页共用同一份
渲染实现（唯一实现路径）。
"""


def make_qr_photo(master, data, target_px=128):
    """生成 data 的二维码 PhotoImage（归属 master），失败返回 None。

    target_px 为期望边长，实际取「模块数 × 整数倍」向下取整，严格正方形。
    """
    if not data:
        return None
    try:
        import tkinter as tk
        import qrcode
        qr = qrcode.QRCode(border=1)
        qr.add_data(data)
        qr.make(fit=True)
        matrix = qr.modules
        natural = len(matrix)          # 含 border
        scale = max(2, int(round(int(target_px) / natural)))
        w = h = natural * scale
        black = b"\x00\x00\x00"
        white = b"\xff\xff\xff"
        pixels = b"".join(
            black if matrix[ry // scale][rx // scale] else white
            for ry in range(h) for rx in range(w))
        header = f"P6\n{w} {h}\n255\n".encode()
        return tk.PhotoImage(master=master, data=header + pixels)
    except Exception:
        return None


def qr_unavailable_reason() -> str:
    """二维码不可用时的简短原因（可用时返回空串），供 UI 提示排查。"""
    try:
        import qrcode  # noqa: F401
    except Exception:
        return "缺 qrcode 库"
    return "数据无效"
