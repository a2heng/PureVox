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
r"""用假产物树 + 假 `gh` 在本地跑一遍 `release_stage.sh`，两个场景都断言退出码。

## 为什么必须有这个

发版那段逻辑（按扩展名 stage → 统一改名 → 交给 `gh release create`）**从未成功执行过**：
`release` job 只在打 tag 时运行，而第一次真打 tag（v2026.10.15.0439）就暴露出
「收集完文件名后 `cd ..` 回上一层 → 相对路径全部失效 → `gh` 报
`no matches found for './PureVox-…apk'`」。tag 已经推出去了，只能删掉重来。

教训：**内联在 workflow 里的脚本等于没被测过的脚本**。所以实现搬到了
`tools/automation/release_stage.sh`，这里测的就是发布的同一份文件 —— 不是从 YAML 里
抠出来的片段（那种抠法还会随缩进变化而静默测错东西）。

## 依赖

**只用标准库**（门禁在 ubuntu runner 上跑，那里没有 PyYAML）。

需要 `bash`，而且**必须显式指定，不能靠 PATH 找** —— 本机 `bash` 解析到的是
`C:\Windows\System32\bash.exe`，那是 **WSL 启动器**：它起的是 WSL 发行版，Windows 的
环境块整个被丢弃（子进程里 `HOME=/root`、`PATH=/usr/local/sbin:…`，`TAG` / `VERSION`
之类一个都读不到，文件路径也换了一套视图）。症状特别隐蔽：脚本「跑起来了」，但所有
环境变量都是空。本机正确的 bash 是 `C:\Program Files\Git\bin\bash.exe`
（AGENTS §2.8 记着本机 shell 就是它）。

## 场景

| mode      | 输入                     | 期望                        |
| --------- | ------------------------ | --------------------------- |
| `ok`      | 五种产物齐全             | 退出 0，`gh` 收到五个路径   |
| `missing` | 少一个 deb               | 退出 1，报「缺少产物」      |
| `notes`   | 五种齐全但没有 notes 文件 | 退出 1，指名道姓            |

用法：`python tools/automation/run_release_stage.py`（不带参数跑全部）
"""

from __future__ import annotations

import ast
import os
import pathlib
import shutil
import subprocess
import sys

# 中文输出必须先切 UTF-8（Windows 控制台默认 cp936），见 _utf8.py 的说明。
sys.path.insert(0, str(pathlib.Path(__file__).resolve().parent))
import _utf8  # noqa: E402

_utf8.force()

ROOT = pathlib.Path(__file__).resolve().parents[2]
STAGE_SH = "tools/automation/release_stage.sh"
WORK = ROOT / ".relstage_test"

VERSION = "26.10.15"
TAG = "v2026.10.15.0439"

# 每个 artifact 目录里放什么 —— 文件名要与 release_stage.sh 的 `stage` 通配一一对应。
# 目录名刻意带上版本/时间戳，与 CI 里 upload-artifact 的实际命名保持一致。
LAYOUT = {
    f"PureVox-Windows-x64-{VERSION}-nsis": [f"PureVox_{VERSION}_x64-setup.exe"],
    f"PureVox-Linux-x64-{VERSION}-deb": [f"purevox_{VERSION}_amd64.deb"],
    f"PureVox-Linux-x64-{VERSION}-rpm": [f"purevox-{VERSION}-1.x86_64.rpm"],
    f"PureVox-Linux-x64-{VERSION}-appimage": [f"purevox_{VERSION}_amd64.AppImage"],
    f"PureVox-Android-arm64-2026-10-15-0439-apk": ["PureVox-Android-arm64-2026-10-15-0439.apk"],
}

# 假 gh：把参数打出来，并逐个检查附件是否真的存在 —— 真 gh 找不到文件时报的
# 「no matches found」太含糊，测的时候要能看出是哪一步的问题。
# **额外写一个哨兵文件**：PATH 没生效时 bash 会去调真 gh，那是真的会去建 release 的
# （本机就踩过：assets 全是 64 字节的假文件被挂上去）。哨兵一缺就知道假 gh 没被用上。
FAKE_GH = """#!/bin/bash
: > "$(dirname "$0")/gh_was_used"
echo "[gh] $*"
# 只检查 `./` 开头的参数 —— 那些才是资产路径；`release` / `create` 这种子命令词、
# `--repo` 之类的开关都不是。别用 `-*` 判断，`release` 不以 - 开头会被当成附件。
for a in "$@"; do
  case "$a" in ./*) ;; *) continue ;; esac
  if [ ! -f "$a" ]; then echo "[gh] FATAL no matches found for '$a'"; exit 1; fi
  echo "[gh] asset ok: $a"
done
exit 0
"""


def find_bash() -> str:
    """找 bash —— **显式找，别信 PATH**。

    本机 `bash` 命中的是 `C:\\Windows\\System32\\bash.exe`（WSL 启动器）：它会起 WSL
    发行版，把 Windows 环境块整个丢掉，`HOME` / `PATH` 换成 WSL 里的，传入的 `TAG` 等
    一个都读不到 —— 症状是脚本「跑通了」但所有环境变量为空，极难看出是选错了 shell。
    """
    if sys.platform != "win32":
        return "bash"
    for cand in (r"C:\Program Files\Git\bin\bash.exe",
                 r"C:\Program Files\Git\usr\bin\bash.exe"):
        if os.path.exists(cand):
            return cand
    found = shutil.which("bash")
    if found and "system32" not in found.lower():
        return found
    raise SystemExit(
        "找不到 Git Bash。本机 `bash` 命中的是 WSL 启动器（C:\\Windows\\System32\\bash.exe），"
        "会丢掉整个环境块；请装 Git for Windows 或改本文件的候选路径。")


def build(drop: str | None = None, notes: bool = True) -> None:
    shutil.rmtree(WORK, ignore_errors=True)
    for art, files in LAYOUT.items():
        if art == drop:
            continue
        d = WORK / "artifacts" / art
        d.mkdir(parents=True)
        for f in files:
            (d / f).write_bytes(b"x" * 64)
    bindir = WORK / "bin"
    bindir.mkdir(parents=True)
    gh = bindir / "gh"
    gh.write_text(FAKE_GH, encoding="utf-8", newline="\n")
    gh.chmod(0o755)
    if notes:
        (WORK / "notes.md").write_text("# PureVox\n\n假 notes\n", encoding="utf-8")


def run_case(label: str, expect_rc: int, expect_gh: bool = False, **kw) -> bool:
    build(**kw)
    bindir = WORK / "bin"
    env = dict(os.environ)
    # **绝对路径**：MSYS 会把 Windows 的 PATH 转成 POSIX 形式，相对路径那段会被丢掉
    # （本机踩过：假 gh 没被用上，结果真 gh 建了个 release，附件全是 64 字节假文件）。
    env["PATH"] = f"{bindir}{os.pathsep}{env['PATH']}"
    env["ARTIFACTS"] = ".relstage_test/artifacts"
    env["REPO"] = "a2heng/PureVox"
    env["TAG"] = TAG
    env["VERSION"] = VERSION
    env["NOTES"] = ".relstage_test/notes.md"
    proc = subprocess.run([find_bash(), STAGE_SH], cwd=ROOT, env=env,
                          capture_output=True, text=True, encoding="utf-8",
                          errors="replace")
    out = ((proc.stdout or "") + (proc.stderr or "")).strip().splitlines()
    print(f"  ── {label} ──")
    for line in out:
        print(f"     {line}")
    ok = proc.returncode == expect_rc
    # 走到 gh 那一步的场景，必须证明**用的是假 gh**。没走到（缺产物/缺 notes）则哨兵本就不该有。
    sentinel = bindir / "gh_was_used"
    if expect_gh and not sentinel.exists():
        ok = False
        out.append("!! 假 gh 没被用上 —— 这个场景没有真的被验证（PATH 覆盖失效？）")
        print("     !! 假 gh 没被用上 —— 这个场景没有真的被验证")
    print(f"     ⇒ 退出码 {proc.returncode}（期望 {expect_rc}）"
          f"{'✓' if ok else '✗'}")
    if not ok:
        for line in out:
            print(f"       {line}")
    return ok


def assert_bundle_constants() -> bool:
    """`assert_bundle.py` 的常量存在性（**静态**检查，不需要产物树）。

    来历（2026-10-26）：改体积下限时把注释写在常量那一行上，替换完常量本身没了 ——
    本机没产物树所以跑不到那段，直到 CI 打完整包才炸 `NameError: MIN_MB`。
    这类「只在有产物时才走到」的脚本，最容易在门禁里静默烂掉，所以在这里静态看一眼。
    """
    path = ROOT / "tools" / "automation" / "assert_bundle.py"
    tree = ast.parse(path.read_text(encoding="utf-8"))
    names = {
        t.id
        for node in tree.body
        if isinstance(node, ast.Assign)
        for t in node.targets
        if isinstance(t, ast.Name)
    }
    missing = [n for n in ("MIN_MB", "DEBUG_URL", "BUNDLE") if n not in names]
    if missing:
        print(f"  assert_bundle.py 缺少常量：{missing}")
        return False
    return True


def main() -> int:
    print("== run_release_stage：用假产物树 + 假 gh 跑 release_stage.sh ==")
    ok_const = assert_bundle_constants()
    print(f"  assert_bundle.py 常量：{'OK' if ok_const else '缺'}")
    results = [
        ok_const,
        run_case("五种产物齐全（必须走到 gh，且用的是假 gh）", 0, expect_gh=True),
        run_case("缺 deb", 1, drop=f"PureVox-Linux-x64-{VERSION}-deb"),
        run_case("没有 notes 文件", 1, notes=False),
    ]
    shutil.rmtree(WORK, ignore_errors=True)
    passed = sum(results)
    print(f"  {passed}/{len(results)} 个场景通过")
    return 0 if passed == len(results) else 1


if __name__ == "__main__":
    raise SystemExit(main())