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
"""生成 release 描述（对应旧版 tools/automation/release_notes.sh）。

取「上一个 `v*` tag → 本 tag」的提交标题列表；首个 tag 则取全部历史。

注意 AGENTS.md §2.6：CI 失败、从未生成 release 的 tag **必须删除**，否则它会成为下个
tag 的「上一个 tag」，把 release 的提交记录截断。

用法：
  python tools/automation/release_notes.py --tag v2026.10.08.1430 --out notes.md
"""

from __future__ import annotations

import argparse
import subprocess
import sys
from pathlib import Path

# 中文输出必须先切 UTF-8（Windows CI runner 默认 cp1252），见 _utf8.py 的说明。
sys.path.insert(0, str(Path(__file__).resolve().parent))
import _utf8  # noqa: E402

_utf8.force()

ROOT = Path(__file__).resolve().parents[2]

# 产物清单**逐字**跟着 release_stage.sh 的 `stage` 重命名规则走 —— 用户是照着这段去找
# 安装包的，写错就是「点进 release 找不到我说的那个文件」。改那边改这里。
# 命名规则见 update/mod.rs 的 `ci_asset_names_match_the_tag_version` 测试（它直接读
# release_stage.sh，改了会红）。
PRODUCT_LINES = (
    "- **Windows**：`PureVox-<版本>-windows-x64-setup.exe`",
    "- **Linux**：`PureVox-<版本>-linux-x64.AppImage` / `.deb` / `.rpm`",
    "- **Android**：`PureVox-<版本>-android-arm64.apk`",
    "",
    "桌面端内置 ONNX 模型；授权见 `MODEL-LICENSE.md`。",
)


def git(*args: str, check: bool = True) -> str:
    proc = subprocess.run(
        ["git", *args], cwd=str(ROOT), capture_output=True, text=True,
        encoding="utf-8", errors="replace",
    )
    if check and proc.returncode != 0:
        raise SystemExit(f"git {' '.join(args)} 失败：{proc.stderr.strip()}")
    return proc.stdout


def previous_tag(tag: str) -> str | None:
    """`git describe --tags --abbrev=0 --match 'v*' <tag>^` —— 没有更早的 tag 时返回 None。"""
    try:
        out = git("describe", "--tags", "--abbrev=0", "--match", "v*", f"{tag}^")
    except SystemExit:
        return None
    out = out.strip()
    return out or None


def main() -> int:
    ap = argparse.ArgumentParser(description="生成 GitHub release 描述")
    ap.add_argument("--tag", required=True, help="本次发布的 tag，如 v2026.10.08.1430")
    ap.add_argument("--out", required=True, help="输出文件（Markdown）")
    args = ap.parse_args()

    prev = previous_tag(args.tag)
    rng = f"{prev}..{args.tag}" if prev else args.tag
    subjects = [s for s in git("log", rng, "--pretty=format:- %s").splitlines() if s.strip()]

    lines = [f"# PureVox {args.tag}", ""]
    lines.append(f"自 {prev} 以来的变更：" if prev else "首个 tag：")
    lines.append("")
    lines.extend(subjects)
    lines.append("")
    lines.append("## 产物")
    lines.append("")
    lines.extend(PRODUCT_LINES)

    out = Path(args.out)
    out.parent.mkdir(parents=True, exist_ok=True)
    out.write_text("\n".join(lines) + "\n", encoding="utf-8", newline="\n")
    print(f"release notes -> {out}（{len(subjects)} 条提交）")
    return 0


if __name__ == "__main__":
    sys.exit(main())