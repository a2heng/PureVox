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
"""发布产物断言：**一个文件、跨平台**（Windows / Linux）。

对应旧版「Assert packaged bundle layout」+「bundled smoke」：

  1. 产物存在且体积合理（内置模型 ~68MB，缺模型只有 ~10MB）
  2. **真实安装/解包**到临时目录，断言可执行文件 + 全部模型落位
  3. `--smoke`：启动安装产物，轮询 `/debug` 就绪后杀死（不留进程，自收尾）

**为什么平台差异不拆文件**：AGENTS §4 的 `<功能>_<平台>.rs` 是给 Rust 的约定（那边确实
有大段互斥实现）。Python 侧刻意反过来 —— **脚本/测试尽可能一个 py 跨平台**：这里的
平台差异只有「产物格式、安装方式、启动前缀、收尾命令」四处，各自三五行，拆成三个
文件只会让「冒烟到底做了哪几步」要跨文件拼起来才看得全。跨平台靠 `sys.platform`
分支，不靠文件。

用法：
  python tools/automation/assert_bundle.py --smoke
"""

from __future__ import annotations

import argparse
import shutil
import subprocess
import sys
import tempfile
import time
import urllib.error
import urllib.request
from pathlib import Path

# 中文输出必须先切 UTF-8（Windows CI runner 默认 cp1252），见 _utf8.py 的说明。
sys.path.insert(0, str(Path(__file__).resolve().parent))
import _utf8  # noqa: E402

_utf8.force()

# 产物体积下限（MB）：漏打模型时安装包会缩到 ~10MB。
# 2026-10-26 从 40 调到 20：模型瘦身（删 202609a/202609c，-23 MB）后 Windows 安装包
# 只有 36.8 MB，旧下限把**正确的包**判成「疑似模型未随包」。这条只是「有没有模型」的
# 粗筛，真正齐全与否由 `check_layout` 数 .onnx 个数；模型再瘦就跟着调这个数。
MIN_MB = 10
DEBUG_URL = "http://127.0.0.1:47821/debug"
SMOKE_POLL = 60      # 轮询次数
SMOKE_INTERVAL = 0.5  # 秒

ROOT = Path(__file__).resolve().parents[2]
BUNDLE = ROOT / "src-tauri" / "target" / "release" / "bundle"

IS_WIN = sys.platform == "win32"
IS_LINUX = sys.platform.startswith("linux")


def require_supported_platform() -> None:
    """其它平台明确报「不可用」，不静默跳过（AGENTS：不静默）。"""
    if not (IS_WIN or IS_LINUX):
        raise SystemExit(f"assert_bundle 暂不支持 {sys.platform}（只有 Windows / Linux）")


# ───────────────────────── 公共部分 ─────────────────────────


def count_models(root: Path) -> int:
    return len(list(root.rglob("*.onnx")))


def newest(dir_path: Path, pattern: str) -> Path | None:
    """取最新的一个产物。

    **必须挑最新的，不能按名字排序取第一个**：`target/release/bundle/` 会**累积**历次
    构建的产物（开发机上常有 4 个 setup.exe，全是不同日期的）。按名字排序取
    第一个会拿到几个月前那个旧的，断言它的体积/布局 —— 测的其实是上次的包。
    """
    if not dir_path.is_dir():
        return None
    hits = [p for p in dir_path.glob(pattern) if p.is_file()]
    return max(hits, key=lambda p: p.stat().st_mtime) if hits else None


def check_size(path: Path) -> None:
    """产物体积下限：漏打模型时安装包会缩到 ~10MB，肉眼很难发现（模型清单见 `check_layout`）。"""
    mb = path.stat().st_size / (1024 * 1024)
    if mb < MIN_MB:
        raise SystemExit(
            f"产物过小：{path.name} 仅 {mb:.1f} MB（下限 {MIN_MB}MB，疑似模型未随包）"
        )
    print(f"产物 {path.name}  {mb:.1f} MB")


def check_layout(exe: Path, root: Path, expected: int, candidates: list[str]) -> None:
    """断言可执行文件存在 + 模型齐全。模型落位路径各家不同，调用方列候选。"""
    if not exe.exists():
        raise SystemExit(f"安装后缺少可执行文件 {exe}（落位检查：{'；'.join(candidates)}）")
    found = count_models(root)
    if found < expected:
        raise SystemExit(
            f"安装包模型不全：{found}/{expected}（落位检查：{'；'.join(candidates)}）"
        )
    print(f"布局 OK：{exe.name} + 模型 {found}/{expected}")


def debug_ready() -> bool:
    try:
        with urllib.request.urlopen(DEBUG_URL, timeout=2) as r:
            return r.status == 200
    except (urllib.error.URLError, OSError):
        return False


# ───────────────────── 平台差异：产物与安装 ─────────────────────


def find_artifacts(bundle: Path) -> list[Path]:
    """列出必须做体积下限检查的主产物（`check_size` 用）。"""
    if IS_WIN:
        # **Windows 只有 NSIS 安装包一个产物**（`tauri.windows.conf.json` 把 targets
        # 收成 ["nsis"]，不再出 MSI）。这里少一种就当场红，避免打出残缺的发布。
        exe = newest(bundle / "nsis", "*.exe")
        if exe is None:
            raise SystemExit(f"找不到 nsis/*.exe 产物（{bundle}）")
        return [exe]
    # Linux：deb 是主产物（要解包断言布局）
    deb = newest(bundle / "deb", "*.deb")
    if deb is None:
        raise SystemExit(f"找不到 deb 产物（{bundle / 'deb'}/*.deb）")
    return [deb]


def install(bundle: Path, tmp: Path) -> tuple[Path, Path, list[str]]:
    """真实安装 / 解包，返回 (可执行文件, 扫描根, 落位候选说明)。"""
    if IS_WIN:
        nsis = find_artifacts(bundle)[0]
        print(f"静默安装到 {tmp}")
        # /S 静默；/D 必须在最后且不能带引号（NSIS 的老规矩，带引号会把路径当字面量）
        subprocess.run([str(nsis), "/S", f"/D={tmp}"], check=True)
        # 资源落位可能是 <安装目录>/models 或 <安装目录>/resources/models
        # （model_path 两者都认），所以模型检查扫整个 tmp。
        return tmp / "PureVox.exe", tmp, [
            "<安装目录>/models",
            "<安装目录>/resources/models",
        ]

    # Linux：解包 deb（不真装 —— 真装要 root，而且会污染 runner）
    deb = find_artifacts(bundle)[0]
    dpkg = shutil.which("dpkg-deb")
    if not dpkg:
        raise SystemExit("找不到 dpkg-deb（apt 包 dpkg），无法解包 deb")
    subprocess.run([dpkg, "-x", str(deb), str(tmp)], check=True)
    # Tauri 的 deb 把可执行文件装到 /usr/bin/purevox
    cands = sorted(p for p in tmp.rglob("purevox") if p.is_file() and p.parent.name == "bin")
    if not cands:
        raise SystemExit("deb 内缺少可执行文件（*/bin/purevox）")
    # 模型随 deb 落在 usr/lib/<productName>
    return cands[0], tmp, ["usr/lib/<productName>/*.onnx"]


def check_extra_artifacts(bundle: Path) -> None:
    """主产物之外必须存在的那些。"""
    if IS_WIN:
        return  # Windows 只有 NSIS 一个安装包，已在 find_artifacts 里查过

    if newest(bundle / "rpm", "*.rpm") is None:
        raise SystemExit(f"缺少 rpm 产物（{bundle / 'rpm'}/*.rpm）")
    # rpm 也做体积下限检查 —— 漏打模型同样会让它缩到 ~10MB，只查存在性会漏
    check_size(newest(bundle / "rpm", "*.rpm"))  # type: ignore[arg-type]
    if newest(bundle / "appimage", "*.AppImage") is not None:
        print("AppImage OK")
    else:
        print("警告：无 AppImage（best-effort，不判失败）")


# ─────────────────────── 平台差异：冒烟 ───────────────────────


def smoke_prefix() -> list[str]:
    if IS_LINUX and shutil.which("xvfb-run"):
        return ["xvfb-run", "-a"]  # CI runner 没有 X server
    return []


def kill_tree(proc: subprocess.Popen) -> None:
    """杀掉我们拉起的进程**及其子进程**（Tauri 会另起 WebView2 / WebKit 子进程）。

    **绝不用 `pkill -f purevox` 扫全盘**：冒烟跑的就是 `purevox`，本机开发时开发者
    自己开着同一个程序，扫名字会把它一起杀掉。而且 Windows 上根本没有 `pkill`
    （Git for Windows 的 MSYS2 不带 procps），`subprocess.run` 会抛
    `FileNotFoundError`，而它挂在 `smoke()` 的 finally 上 —— **成功也会炸**。
    """
    if proc.poll() is not None:
        return
    if IS_LINUX:
        import os
        import signal

        try:  # start_new_session=True 让它自成一组，整组杀最干净
            os.killpg(os.getpgid(proc.pid), signal.SIGTERM)
        except (ProcessLookupError, PermissionError, AttributeError):
            proc.terminate()
    else:
        # /T = 连子进程一起杀；用 /PID 精确限定在我们自己拉起的那棵树上
        subprocess.run(
            ["taskkill", "/F", "/T", "/PID", str(proc.pid)], capture_output=True
        )
    try:
        proc.wait(timeout=5)
    except subprocess.TimeoutExpired:
        proc.kill()
        try:
            proc.wait(timeout=5)
        except subprocess.TimeoutExpired:
            print(f"警告：pid {proc.pid} 杀不掉，CI 上会占住 /debug 端口")


def smoke(exe: Path, log: Path) -> None:
    """启动安装产物 → 轮询 /debug 就绪 → **必杀**。

    只判 `/debug` 起来了，不做功能验证 —— 那是 Rust 单测的事，这里只证明「装出来的
    这个二进制能启动、调试端口能开」。
    """
    cmd = smoke_prefix() + [str(exe)]
    log.parent.mkdir(parents=True, exist_ok=True)
    with log.open("wb") as fh:
        proc = subprocess.Popen(
            cmd, stdout=fh, stderr=subprocess.STDOUT, start_new_session=True
        )
    try:
        for _ in range(SMOKE_POLL):
            if debug_ready():
                print("冒烟 OK：/debug 就绪")
                return
            time.sleep(SMOKE_INTERVAL)
        raise SystemExit(
            f"冒烟失败：启动后 /debug {SMOKE_POLL * SMOKE_INTERVAL:.0f}s 内未就绪（日志 {log}）"
        )
    finally:
        kill_tree(proc)


def main() -> int:
    ap = argparse.ArgumentParser(description="发布产物断言")
    ap.add_argument("--smoke", action="store_true", help="额外做启动冒烟（装完跑一次 /debug）")
    args = ap.parse_args()

    require_supported_platform()
    expected = count_models(ROOT / "models")
    tmp = Path(tempfile.mkdtemp(prefix="purevox_bundle_check_"))
    try:
        for artifact in find_artifacts(BUNDLE):
            check_size(artifact)
        exe, root, candidates = install(BUNDLE, tmp)
        check_layout(exe, root, expected, candidates)
        check_extra_artifacts(BUNDLE)
        if args.smoke:
            smoke(exe, tmp / "smoke.log")
    finally:
        # 不管成没成，临时目录都清掉（AGENTS：测试自收尾）
        shutil.rmtree(tmp, ignore_errors=True)
    print("产物断言全部通过")
    return 0


if __name__ == "__main__":
    sys.exit(main())