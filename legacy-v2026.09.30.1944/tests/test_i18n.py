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

"""i18n 字符串表测试（纯函数，无音频副作用）：
python tests/test_i18n.py

- zh 恒等：T(x) == x；
- en 查表：每条 en 值确实翻译了（非恒等回退）；
- 占位符奇偶校验：en 值的 {name} 集合与 msgid 一致（防漏参/多参）。
"""

import os
import re
import sys

sys.path.insert(0, os.path.dirname(os.path.dirname(os.path.abspath(__file__))))

import i18n
from i18n import T, set_language, get_language


_PLACEHOLDER_RE = re.compile(r"\{([a-zA-Z_][a-zA-Z0-9_]*)\}")


def test_zh_identity():
    set_language("zh")
    assert get_language() == "zh"
    samples = ["启动音频处理", "输入：未启动",
               "端口 {port} · 客户端 {clients} 个", "不存在的一条"]
    for s in samples:
        assert T(s) == s, s
    print("  zh 恒等：T(x) == x  OK")


def test_unknown_language_falls_back():
    set_language("xx")
    assert get_language() == "zh", "未知 code 应回退 zh"
    assert T("启动音频处理") == "启动音频处理"
    set_language("zh")
    print("  未知语言回退 zh  OK")


def test_en_translated():
    set_language("en")
    table = i18n._TABLES["en"]
    assert table, "en 表不能为空"
    missing = [k for k, v in table.items() if v == k]
    assert not missing, f"en 表存在未翻译条目（恒等回退）: {missing[:10]}"
    # 抽查核心条目
    assert T("启动音频处理") == "Start Audio Processing"
    assert T("（默认）") == "(Default)"
    assert T("端口 {port} · 客户端 {clients} 个").format(
        port=59123, clients=1) == "Port 59123 · 1 client(s)"
    set_language("zh")
    print(f"  en 查表全部翻译（{len(table)} 条）  OK")


def test_placeholder_parity():
    for lang, table in i18n._TABLES.items():
        bad = []
        for msgid, val in table.items():
            src = sorted(set(_PLACEHOLDER_RE.findall(msgid)))
            dst = sorted(set(_PLACEHOLDER_RE.findall(val)))
            if src != dst:
                bad.append((msgid, src, dst))
        assert not bad, f"{lang} 表占位符不一致: {bad[:5]}"
    set_language("zh")
    print("  占位符奇偶校验（msg↔译文集合一致）  OK")


def test_missing_msgid_falls_back():
    set_language("en")
    assert T("绝不存在的条目") == "绝不存在的条目", "缺失条目回退中文原文"
    set_language("zh")
    print("  en 缺失条目回退中文  OK")


if __name__ == "__main__":
    test_zh_identity()
    test_unknown_language_falls_back()
    test_en_translated()
    test_placeholder_parity()
    test_missing_msgid_falls_back()
    print("test_i18n 全部通过")
