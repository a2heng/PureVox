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
"""代码质量门禁的**唯一实现**：CI 各 job 与本机开发跑同一份脚本。

  python tools/automation/check.py                  # 全部门禁
  python tools/automation/check.py -Gate fmt        # 单项（CI 按 step 拆开跑，日志好定位）

门禁项：fmt（rustfmt 校验）/ clippy（-D warnings 零容忍）/ build / test /
version（版本号推导）/ bundle（NSIS 安装包 + 发版脚本）。

`ui` 是历史遗留的 WebView 门禁：GPUI 界面是 Rust，没有 `jsconfig.json` 时会明确跳过，
保留它只为让旧调试面板的 JS 仍可在需要时被单独检查。

为什么是 Python：门禁要在 Windows / Linux / macOS 上跑，而 PowerShell 只有前两者
装得像样、还得单独装（CI runner 自带，本机要自己搞）。Python 三边都是系统自带，
冷启动 ~20 ms（PowerShell ~330 ms），只用标准库、不需要 pip install。

只依赖标准库。改了门禁项要同步 `.github/workflows/tests.yml`（CI 按 step 拆开跑）。
"""

from __future__ import annotations

import argparse
import os
import shutil
import subprocess
import sys
from pathlib import Path

# 必须在任何 print 之前把 stdout 改成 UTF-8（Windows CI runner 的默认是 cp1252，
# 打中文会 UnicodeEncodeError —— 门禁跑完了却被编码错误判成失败）。
# 同目录导入：脚本以 `python tools/automation/check.py` 调用时，脚本所在目录在 sys.path 上。
sys.path.insert(0, str(Path(__file__).resolve().parent))
import _utf8  # noqa: E402  （刻意放在 path 注入之后）

_utf8.force()

ROOT = Path(__file__).resolve().parents[2]
SRC = ROOT / "src-tauri"
UI = SRC / "ui"

GATES = ("fmt", "clippy", "build", "test", "ui", "version", "bundle")

# 版本号推导的固定样本。09 月 / 09 日 / 凌晨时分是**前导零陷阱**：bash 的 `$((10#09))`
# 不加 10# 会当八进制，`09` 直接语法错误 —— 现在只有 Python 一份实现，但样本仍留着，
# 哪天有人「优化」成字符串拼接就会当场露馅。
#
# ACODE 一律是 **9 位数**（`yyMMddHHmm` 去掉最低一位）：十位数的值超过 versionCode 的
# Int32 上限，会被 Gradle 侧静默丢成 1（见 version.py / UpdateChecker.stampToCode）。
# 这几行同时钉住「÷10 与补零」两件事。
VERSION_CASES = (
    # tag,                    VERSION, STAMP,            ACODE,     PUREVOX_BUILD_STAMP,   PLATFORM
    ("v2026.10.14.1530", "26.10.14", "2026-10-14-1530", "261014153", "2026.10.14.1530", "desktop"),
    ("v2026.09.09.0901", "26.9.9",   "2026-09-09-0901", "260909090", "2026.09.09.0901", "desktop"),
    ("v2026.01.02.0000", "26.1.2",   "2026-01-02-0000", "260102000", "2026.01.02.0000", "desktop"),
    # 同一天第二次发版：显示版本相同，靠 tag 的时分区分（AGENTS §2.6 / version.rs）
    ("v2026.10.14.2309", "26.10.14", "2026-10-14-2309", "261014230", "2026.10.14.2309", "desktop"),
    # 安卓独立通道：tag 带 `android-` 前缀，输出 TAG 必须原样保留前缀（更新检查按它筛）
    ("android-v2026.11.12.1015", "26.11.12", "2026-11-12-1015", "261112101", "2026.11.12.1015", "android"),
)


class GateError(Exception):
    """门禁未通过。"""


class ToolMissing(GateError):
    """工具不在 PATH 上（与「跑了但没过」分开报，别混成一句 command not found）。"""


def _which(name: str, hint: str) -> str:
    found = shutil.which(name)
    if not found:
        raise ToolMissing(f"找不到 {name}（{hint}）")
    return found


def _env() -> dict[str, str]:
    """把 cargo 所在目录加进 PATH。

    Linux/macOS 上 cargo 装在 ~/.cargo/bin，多数情况已在 PATH；但 GUI 启动的终端、
    某些 CI step 会漏。Windows 上 PATH 用 `;` 分隔、其他平台用 `:`，不能写死。
    """
    env = dict(os.environ)
    cargo_bin = str(Path.home() / ".cargo" / "bin")
    sep = ";" if os.name == "nt" else ":"
    if cargo_bin not in env.get("PATH", "").split(sep):
        env["PATH"] = cargo_bin + sep + env.get("PATH", "")
    return env


def run(cmd: list[str], cwd: Path, *, env: dict[str, str] | None = None) -> str:
    """跑一条命令，失败即抛；返回 stdout。"""
    print(f"  $ {' '.join(str(c) for c in cmd)}   (cwd={cwd.name})", flush=True)
    proc = subprocess.run(
        [str(c) for c in cmd], cwd=str(cwd), env=env or _env(),
        capture_output=True, text=True, encoding="utf-8", errors="replace",
    )
    out = (proc.stdout or "") + (proc.stderr or "")
    if proc.returncode != 0:
        print(out, flush=True)
        raise GateError(f"命令失败（exit={proc.returncode}）：{' '.join(str(c) for c in cmd)}")
    return out


# --------------------------------------------------------------------------
# 各道门禁
# --------------------------------------------------------------------------
def gate_fmt() -> None:
    run(["cargo", "fmt", "--check"], SRC)


def gate_clippy() -> None:
    # **先碰一下 crate 根，逼它重新编译一遍。**
    # cargo 会把上一次 clippy 的产物判成 Fresh 直接复用，而 Fresh 时**一个警告都不会再打印** ——
    # 于是「门禁绿」静默退化成「上次编译时是绿的」。本地缓存几乎总能命中，CI 干净树才暴露真相：
    # 2026-10-18 实测，同一份代码本地报绿、CI 报 `manual_is_multiple_of`，白跑一轮发版。
    # 只碰 crate 根：依赖仍是 Fresh（它们的 lint 本来也不报），代价就是本包重编一次。
    (SRC / "src" / "main.rs").touch()
    run(["cargo", "clippy", "--all-targets", "--", "-D", "warnings"], SRC)


def gate_build() -> None:
    run(["cargo", "build"], SRC)


def gate_test() -> None:
    out = run(["cargo", "test"], SRC)
    for line in out.splitlines():
        if line.startswith("test result:"):
            print(f"  {line.strip()}")


def gate_ui() -> None:
    # 旧 WebView 前端已随 GPUI 重建删除，只剩调试面板的 JS（debug.js / i18n.js …）。
    # 没有 `jsconfig.json` 就没有可类型检查的工程 —— **明确跳过并说明**，不要静默通过。
    if not (UI / "jsconfig.json").is_file():
        print("  跳过：src-tauri/ui/jsconfig.json 不存在（GPUI 界面是 Rust；调试面板 JS 无类型配置）")
        return
    # Windows 上 tsc 常常只以 .cmd/.ps1 形式存在（npm 全局包装脚本），shutil.which 只认
    # 可执行扩展名。这里显式把三种形态都试一遍，找不到再报「工具缺失」而不是
    # FileNotFoundError 的裸栈。
    run([_which("tsc", "npm install -g typescript@<TS_VER>"), "-p", "jsconfig.json", "--noEmit"], UI)
    # i18n_lint 用 node 跑（它就是 node 生态的脚本，不必为了门禁改写成 Python）
    run([_which("node", "装 Node.js"), ROOT / "tools" / "automation" / "i18n_lint.js"], ROOT)


def gate_version() -> None:
    """版本号推导：喂固定 tag，断言输出**逐字**符合预期。

    版本推导错了的表现是包名 / 产物文件名里的日期不对 —— 要等用户装上、或去 release
    页面找安装包时才发现。所以在这钉死，不依赖联网。

    版本只由 `version.py` 输出到环境（`VERSION` / `STAMP` / `TAG`），不再写 Tauri 的
    `.build-version.json` —— GPUI 构建直接用 `PUREVOX_BUILD_*` 环境变量。门禁喂的是
    5 个**假** tag，不能污染本机任何构建配置。
    """
    script = ROOT / "tools" / "automation" / "version.py"
    for tag, version, stamp, acode, bstamp, platform in VERSION_CASES:
        env = _env()
        env["GITHUB_REF_NAME"] = tag
        out = run([sys.executable, script], ROOT, env=env)
        kv = dict(line.split("=", 1) for line in out.splitlines() if "=" in line.strip())
        want = {
            "PLATFORM": platform,
            "TAG": tag,
            "VERSION": version,
            "STAMP": stamp,
            "ACODE": acode,
            "PUREVOX_BUILD_VERSION": version,
            "PUREVOX_BUILD_STAMP": bstamp,
        }
        for k, v in want.items():
            got = kv.get(k)
            if got != v:
                raise GateError(f"tag {tag}：{k} = {got!r}，期望 {v!r}")
        print(f"  {tag} -> VERSION={version}  STAMP={stamp}  ACODE={acode}")


def gate_bundle() -> None:
    """打包 / 发版自检。

    当前桌面产物是 NSIS 安装包（`package_windows.py` + `nsis/purevox.nsi`）。
    这道门禁只做两件不会误报的事：
    1. 安装包脚本存在、能解析，且真的把 `opus.dll` / `models/` / `makensis` 写进去；
    2. 发版那一步的真实实现（`release_stage.sh`）用假产物树 + 假 `gh` 跑通。
    """
    import ast

    installer = ROOT / "tools" / "automation" / "package_windows.py"
    nsi = ROOT / "tools" / "automation" / "nsis" / "purevox.nsi"
    if not installer.is_file():
        raise GateError(f"缺少安装包脚本：{installer}")
    if not nsi.is_file():
        raise GateError(f"缺少 NSIS 脚本：{nsi}")
    text = installer.read_text(encoding="utf-8")
    ast.parse(text)  # 语法错误当场暴露（这类脚本平时只在发版时跑）
    for token, why in (
        ("opus.dll", "网络音频运行时依赖，缺了网络功能全废"),
        ("models", "AI 模型目录，缺了降噪/TSE/AEC 全部不可用"),
        ("makensis", "安装包必须由 NSIS 生成（Tauri 同款布局）"),
    ):
        if token not in text:
            raise GateError(f"package_windows.py 没有处理 {token}（{why}）")
    print("  installer: exe + opus.dll + models/ → NSIS setup.exe")

    # Linux 打包脚本只做语法/关键路径自检（Windows 上不跑 cargo/tar）。
    linux = ROOT / "tools" / "automation" / "package_linux.py"
    if linux.is_file():
        text = linux.read_text(encoding="utf-8")
        ast.parse(text)
        for token, why in (
            ("tar.gz", "便携包必须产出 tar.gz"),
            ("dpkg-deb", "有 dpkg-deb 时要能出 deb"),
            ("models", "AI 模型目录必须随包"),
        ):
            if token not in text:
                raise GateError(f"package_linux.py 没有处理 {token}（{why}）")
        print("  linux: release + models/ → tar.gz / deb")

    # 发版那一步是独立文件：用假产物树 + 假 gh 在本地跑通（桌面 / 安卓 / 缺产物 / 缺 notes）。
    harness = ROOT / "tools" / "automation" / "run_release_stage.py"
    proc = subprocess.run(
        [sys.executable, str(harness)], cwd=str(ROOT),
        capture_output=True, text=True, encoding="utf-8", errors="replace",
    )
    if proc.returncode != 0:
        detail = ((proc.stdout or "") + (proc.stderr or "")).rstrip()
        raise GateError(
            "发版脚本台子没过（release_stage.sh 的 stage/改名/gh 调用）："
            + chr(10) + detail
        )
    for line in (proc.stdout or "").splitlines():
        if "个场景通过" in line:
            print(f"  发版脚本: {line.strip()}")


GATE_FUNCS = {
    "fmt": gate_fmt,
    "clippy": gate_clippy,
    "build": gate_build,
    "test": gate_test,
    "ui": gate_ui,
    "version": gate_version,
    "bundle": gate_bundle,
}


def main() -> int:
    ap = argparse.ArgumentParser(description="PureVox 代码质量门禁")
    ap.add_argument(
        "-Gate", action="append", default=None, choices=("all", *GATES),
        help="跑哪几道门禁，可重复（-Gate fmt -Gate clippy）；不带 = 全部",
    )
    args = ap.parse_args()
    # **可重复**：`-Gate` 原本是单值参数，串着写 `-Gate fmt -Gate clippy -Gate test` 只会跑最后一道 ——
    # 「门禁全绿」于是静默退化成「只跑了 version / bundle」（2026-10-18 实测踩到，白跑一轮发版）。
    # 另外把**实际要跑的清单**先打出来：日志被截断时也能一眼看出跑了几道。
    want = args.Gate or ["all"]
    names = list(GATES) if "all" in want else [g for g in GATES if g in set(want)]
    print(f"门禁：{' '.join(names)}", flush=True)

    failed: list[str] = []
    for name in names:
        print(f"==> {name}", flush=True)
        try:
            GATE_FUNCS[name]()
        except GateError as e:
            print(f"!! 门禁 {name} 未通过：{e}", file=sys.stderr, flush=True)
            failed.append(name)

    if failed:
        print(f"\n未通过：{', '.join(failed)}", file=sys.stderr)
        return 1
    print(f"\n门禁全部通过（{', '.join(names)}）")
    return 0


if __name__ == "__main__":
    sys.exit(main())