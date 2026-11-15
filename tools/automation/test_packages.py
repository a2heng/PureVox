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
"""跨发行版「装得上、起得来」验证：**一个文件、跨平台**。

在**目标发行版容器**里真实安装我们的安装包，断言依赖解析、文件落位、动态库与
**符号版本**都能解析：

  - deb       → ubuntu:24.04（apt）—— 这是**打包基线**（release.yml 的 linux job
                跑在它上面），所以「能不能在 24.04 上跑」正是本产物最该被断言的事
  - rpm       → fedora:latest（dnf）
  - AppImage  → 本机 `--appimage-extract` 解包断言（免 FUSE，不需要容器）

**同一个文件兼任两种角色**：宿主上跑是「驱动容器的那一方」，容器里跑
（`--in-container`）是「做断言的那一方」。所以不需要另开一个检查脚本，容器里只需
`python3 /pv/test_packages.py --in-container` —— 跨平台靠 `sys.platform` 与参数分支，
不靠文件（与 assert_bundle.py 同一取向：脚本/测试尽可能一个 py 跨平台）。

容器运行时默认 `docker`（GitHub runner 自带）；本机可用 `DOCKER=podman` 覆盖。

用法：
  python tools/automation/test_packages.py               # 宿主：驱动容器
  PV_IN_CONTAINER=1 PV_EXPECTED=7 python3 test_packages.py --in-container
"""

from __future__ import annotations

import argparse
import os
import shutil
import subprocess
import sys
import tempfile
from pathlib import Path

# 中文输出必须先切 UTF-8（Windows CI runner 默认 cp1252），见 _utf8.py 的说明。
sys.path.insert(0, str(Path(__file__).resolve().parent))
import _utf8  # noqa: E402

_utf8.force()

# **模块级绝对不能算仓库路径**：容器里本脚本挂在 `/pv/test_packages.py`，
# `parents` 只有 `[/pv, /]` 两层，`parents[2]` 直接 IndexError —— 而模块级语句在
# `--in-container` 模式下也会执行，于是三项断言一项都跑不到（实测 exit 1）。
# 所以路径一律现算：宿主模式下算仓库根，容器模式下压根不需要。
def repo_root() -> Path:
    return Path(__file__).resolve().parents[2]


def bundle_dir() -> Path:
    return repo_root() / "" / "target" / "release" / "bundle"


def say(*parts: str) -> None:
    print(*parts, flush=True)


# ───────────────────── 容器内：做断言的那一方 ─────────────────────


def find_binary() -> Path:
    """定位装出来的可执行文件（deb/rpm 都装到 PATH 上的 /usr/bin/purevox）。"""
    found = shutil.which("purevox")
    if found:
        return Path(found)
    for cand in (Path("/usr/bin/purevox"), Path("/usr/local/bin/purevox")):
        if cand.is_file():
            return cand
    raise SystemExit("缺少可执行文件 purevox（既不在 PATH 上也不在 /usr/bin）")


def check_models(expected: int) -> None:
    n = sum(1 for _ in Path("/usr").rglob("*.onnx"))
    if n < expected:
        raise SystemExit(f"模型不全：{n}/{expected}")


def glibc_version() -> str:
    proc = subprocess.run(["ldd", "--version"], capture_output=True, text=True)
    out = (proc.stdout or proc.stderr or "").splitlines()
    return out[0].strip() if out else "?"


def check_dynamic(bin_: Path) -> None:
    """断言动态库**与符号版本**都能解析。

    **不能用 `ldd`**：ldd 只看「库文件在不在」，不看「符号版本够不够」，两边都会漏。
    实测过的坑：在 glibc 2.43 上编的包要求 `log10f@GLIBC_2.43`，装到 2.39（24.04）
    的系统上 `ldd` 输出一行 `not found` 都没有，程序一启动就
    `symbol lookup error: undefined symbol: log10f, version GLIBC_2.43`。
    这里直接用动态链接器（ldd 的底层）做版本解析，库缺失与版本缺失**都会**报出来。

    只匹配 `not found`：宽泛地 grep `error` 会把 `libgpg-error.so.0` 这类正常行误判。
    """
    ldso = next(
        (p for p in (Path("/lib64/ld-linux-x86-64.so.2"), Path("/lib/ld-linux-x86-64.so.2"))
         if p.exists()),
        None,
    )
    if ldso is None:
        raise SystemExit("找不到动态链接器（/lib64/ld-linux-x86-64.so.2）")
    proc = subprocess.run([str(ldso), "--list", str(bin_)], capture_output=True, text=True)
    bad = [
        line
        for line in ((proc.stdout or "") + (proc.stderr or "")).splitlines()
        if "not found" in line
    ]
    if bad:
        raise SystemExit(
            "动态库缺失 / 符号版本不够（glibc 太老）:\n  " + "\n  ".join(bad[:20])
        )


def run_in_container_checks() -> int:
    """容器内入口：三项断言。"""
    expected = int(os.environ.get("PV_EXPECTED", "0"))
    bin_ = find_binary()
    check_models(expected)
    check_dynamic(bin_)
    say(f"  OK: {bin_} · 模型 {expected} 个齐 · 动态库与符号版本均可解析")
    say(f"  目标环境 glibc: {glibc_version()}")
    return 0


# ───────────────────── 宿主：驱动容器的那一方 ─────────────────────


def run_in_container(
    runtime: str,
    image: str,
    pkg_dir: Path,
    install_cmd: str,
    expected: int,
    provision: str,
    probe: str = "",
) -> None:
    """把本脚本挂进容器，装完包后用 `python3 <本脚本> --in-container` 做断言。

    `provision` 是**给目标发行版装一个 python3** 的命令：容器内断言要做的是本脚本那份
    实现（宿主与容器同一份，不会各自漂移），而 `ubuntu:24.04` / `fedora:latest` 的基础
    镜像里**都没有 python3**（实测 `bash: python3: command not found`）。

    **必须是完整的 `python3`，不能用 `python3-minimal`** —— 后者的标准库是裁剪过的，
    本脚本要的 `shutil` 直接 `ModuleNotFoundError: No module named 'shutil'`（实测）。

    `probe` 是**装之前**在容器里跑的诊断命令，输出直接进日志。它的用途：rpm 的依赖
    是 `rpmbuild` 从 `DT_NEEDED` 自动生成的，那套机制到底生成了没有、生成了什么，
    不看 `rpm -qp --requires` 就只能猜 —— 而「猜错了」的后果正是这轮挂掉的 `soup3`
    （而且它在第一个错名就停下，后面的错根本不会暴露）。
    """
    args = [runtime, "run", "--rm", "--network=host"]
    # 容器共用宿主网络（CI runner 直连；本机配合代理可用）。设了代理环境变量就透传进去，
    # 否则容器内 apt/dnf 可能连不上官方源。
    if any(os.environ.get(v) for v in ("http_proxy", "https_proxy", "HTTP_PROXY", "HTTPS_PROXY")):
        args += ["-e", "http_proxy", "-e", "https_proxy", "-e", "HTTP_PROXY",
                 "-e", "HTTPS_PROXY", "-e", "no_proxy", "-e", "NO_PROXY"]
    chain = " && ".join(
        c for c in (probe, install_cmd, provision,
                    "python3 /pv/test_packages.py --in-container") if c)
    args += [
        "-e", f"PV_EXPECTED={expected}",
        "-v", f"{pkg_dir}:/pkg:ro",
        "-v", f"{Path(__file__).resolve().parent}:/pv:ro",
        image, "bash", "-c", chain,
    ]
    say(f"  $ {runtime} run … {image}")
    rc = subprocess.run(args).returncode
    if rc != 0:
        raise SystemExit(f"容器内验证失败（{image}，exit={rc}）")


def check_appimage(appimg: Path, expected: int) -> None:
    with tempfile.TemporaryDirectory(prefix="purevox_appimg_") as td:
        # AppImage 需要在目标目录里解包（它会写 squashfs-root 到 CWD）
        proc = subprocess.run(
            [str(appimg), "--appimage-extract"], cwd=td, capture_output=True, text=True
        )
        if proc.returncode != 0:
            raise SystemExit(f"--appimage-extract 失败：{proc.stderr.strip()}")
        root = Path(td) / "squashfs-root"
        if not (root / "AppRun").exists():
            raise SystemExit("AppImage 缺少 AppRun")
        n = count_onnx(root)
        if n < expected:
            raise SystemExit(f"AppImage 模型不全：{n}/{expected}")
        say(f"  OK: AppRun + 模型 {n}")


def count_onnx(root: Path) -> int:
    return len(list(root.rglob("*.onnx")))


def newest(dir_path: Path, pattern: str) -> Path | None:
    if not dir_path.is_dir():
        return None
    hits = [p for p in dir_path.glob(pattern) if p.is_file()]
    return max(hits, key=lambda p: p.stat().st_mtime) if hits else None


def main() -> int:
    ap = argparse.ArgumentParser(description="跨发行版安装验证")
    ap.add_argument(
        "--in-container",
        action="store_true",
        help="容器内入口（由宿主调用，勿手工跑）：做三项断言后退出",
    )
    args = ap.parse_args()
    if args.in_container:
        return run_in_container_checks()

    runtime = os.environ.get("DOCKER", "docker")
    if not shutil.which(runtime):
        say(f"找不到容器运行时 '{runtime}'（用 DOCKER=podman 可覆盖）；跨发行版验证需要它")
        return 1

    root = repo_root()
    bundle = bundle_dir()
    expected = count_onnx(root / "models")
    say(f"模型数 {expected}")

    # **每一档都是独立的**：一档挂了其余照样跑，最后统一汇总。
    # 之前是 `run_in_container` 直接 raise SystemExit，于是 deb 一失败 rpm / AppImage
    # 根本不会执行 —— 一轮下来只知道 deb 挂了，rpm 能不能装是未知的，
    # 「rpm 用独立的 Fedora 容器验」也就失去了意义。
    failed: list[str] = []

    def attempt(label: str, fn) -> None:
        try:
            fn()
            say(f"  ✓ {label}")
        except SystemExit as e:
            say(f"  ✗ {label}：{e}")
            failed.append(label)

    deb = newest(bundle / "deb", "*.deb")
    if deb:
        # 24.04 是**打包基线**（release.yml 的 linux job 跑在它上面），所以「能不能在
        # 24.04 上跑」正是本产物最该被断言的事 —— 不是额外加的兼容性抽查。
        say("== deb → ubuntu:24.04（= 打包基线，必须能跑）==")
        attempt(
            "deb @ ubuntu:24.04",
            lambda: run_in_container(
                runtime, "ubuntu:24.04", bundle / "deb",
                "apt-get update -qq && "
                "apt-get install -y -qq --no-install-recommends /pkg/*.deb >/dev/null",
                expected,
                "apt-get install -y -qq --no-install-recommends python3 >/dev/null",
            ),
        )
        # **向前兼容**：glibc 的版本化符号单方向兼容 —— 新 glibc 必定保留全部旧版本标签，
        # 所以「老基线编的包 → 新发行版能跑」理论上成立。理论不值钱，在真 runner 上跑一遍。
        # 这一条挡住的是「哪天有人把打包基线又调回最新一档」：那时 24.04 那条会红，
        # 但这里会绿，正好互相印证「问题确实出在基线而不是别处」。
        say("== deb → ubuntu:26.04（向前兼容，必须能跑）==")
        attempt(
            "deb @ ubuntu:26.04",
            lambda: run_in_container(
                runtime, "ubuntu:26.04", bundle / "deb",
                "apt-get update -qq && "
                "apt-get install -y -qq --no-install-recommends /pkg/*.deb >/dev/null",
                expected,
                "apt-get install -y -qq --no-install-recommends python3 >/dev/null",
            ),
        )
    else:
        say("跳过 deb（无产物）")

    rpm = newest(bundle / "rpm", "*.rpm")
    if rpm:
        # rpm 在**它自己的 Fedora 容器**里验，与 deb 那两档互不影响：Ubuntu 的容器装不了
        # rpm，Fedora 的容器也装不了 deb —— 包格式与发行版一一对应，不能互相顶替。
        # 另外 rpm 是在 Ubuntu 上用 rpmbuild 打的，Fedora 上能不能跑受 glibc 与库布局双重
        # 影响，这条断言的价值是「拦住明显装不上」，不是「证明 Fedora 完全兼容」。
        say("== rpm → fedora:latest（独立容器）==")
        attempt(
            "rpm @ fedora:latest",
            lambda: run_in_container(
                runtime, "fedora:latest", bundle / "rpm",
                "dnf install -y --setopt=install_weak_deps=False /pkg/*.rpm >/dev/null",
                expected,
                "dnf install -y python3 >/dev/null",
                # 装之前先把自动生成的 Requires 打进日志：tauri.conf.json 里
                # `rpm.depends` 是空的（`bundle` 门禁就断言它必须空），依赖全靠
                # rpmbuild 从 DT_NEEDED 生成。这里把生成结果原样打出来，才知道
                # 自动依赖到底工作没有 —— 不打出来就只能靠猜。
                "rpm -qp --requires /pkg/*.rpm | sed 's/^/  [rpm Requires] /'",
            ),
        )
    else:
        say("跳过 rpm（无产物）")

    appimg = newest(bundle / "appimage", "*.AppImage")
    if appimg:
        say("== AppImage → 本机解包 ==")
        attempt("AppImage 解包", lambda: check_appimage(appimg, expected))
    else:
        say("跳过 AppImage（无产物）")

    say()
    if failed:
        say(f"跨发行版验证失败：{len(failed)} 档 —— " + "；".join(failed))
        return 1
    say("跨发行版验证全部通过")
    return 0


if __name__ == "__main__":
    sys.exit(main())