#!/usr/bin/env python3
# PureVox — AI 麦克风降噪工具
# Copyright (C) 2024-2026 a2heng <752848283@qq.com>
#
# PureVox is licensed under the GNU General Public License v3.0 or
# later (GPL-3.0-or-later).  See LICENSE for details.
#
# SPDX-License-Identifier: GPL-3.0-or-later
"""Windows 安装包（NSIS）：`cargo build --release` + 文件布局 + `makensis`。

与旧 Tauri 产物的**同一套形状**（按用户安装到 `%LOCALAPPDATA%\\PureVox`、
开始菜单/桌面快捷方式、卸载项、`PureVox_<版本>_x64-setup.exe`），
只是不再依赖 Tauri / WebView —— 应用是 GPUI，安装器只需要 exe + 资源。

用法：
  python tools/automation/package_windows.py            # 构建 + 打包
  python tools/automation/package_windows.py --no-build # 复用已有 release 产物

产物：`dist/PureVox_<版本>_x64-setup.exe`
"""

from __future__ import annotations

import argparse
import os
import shutil
import subprocess
import sys
from pathlib import Path

sys.path.insert(0, str(Path(__file__).resolve().parent))
import _utf8  # noqa: E402

_utf8.force()

ROOT = Path(__file__).resolve().parents[2]
TAURI = ROOT / "src-tauri"
DIST = ROOT / "dist"
NSI = ROOT / "tools" / "automation" / "nsis" / "purevox.nsi"


def version() -> str:
    env = dict(os.environ)
    env.setdefault("PV_PLATFORM", "desktop")
    out = subprocess.run(
        [sys.executable, str(ROOT / "tools" / "automation" / "version.py")],
        cwd=str(ROOT), env=env, capture_output=True, text=True,
        encoding="utf-8", errors="replace",
    )
    if out.returncode != 0:
        raise SystemExit(f"version.py 失败：{out.stderr.strip()}")
    for line in out.stdout.splitlines():
        if line.startswith("VERSION="):
            return line.split("=", 1)[1].strip()
    raise SystemExit("version.py 没有输出 VERSION")


def makensis() -> str:
    found = shutil.which("makensis")
    if found:
        return found
    for p in (
        r"C:\Program Files (x86)\NSIS\makensis.exe",
        r"C:\Program Files\NSIS\makensis.exe",
    ):
        if Path(p).is_file():
            return p
    raise SystemExit("找不到 makensis：装 NSIS（choco install nsis）后重试")


def main() -> int:
    ap = argparse.ArgumentParser(description="Windows NSIS 安装包")
    ap.add_argument("--no-build", action="store_true", help="不跑 cargo build，直接用已有产物")
    args = ap.parse_args()

    if not args.no_build:
        print("== cargo build --release ==", flush=True)
        subprocess.run(
            ["cargo", "build", "--release", "--manifest-path", str(TAURI / "Cargo.toml")],
            check=True,
        )

    exe = TAURI / "target" / "release" / "purevox.exe"
    if not exe.is_file():
        raise SystemExit(f"找不到 {exe}（先跑 cargo build --release）")
    opus = TAURI / "opus.dll"
    models = ROOT / "models"
    if not opus.is_file():
        raise SystemExit(f"找不到 {opus}（网络 Opus 运行时依赖）")
    if not models.is_dir():
        raise SystemExit(f"找不到模型目录 {models}")

    ver = version()
    name = f"PureVox-{ver}-windows-x64"
    stage = DIST / name
    shutil.rmtree(stage, ignore_errors=True)
    stage.mkdir(parents=True)

    shutil.copy2(exe, stage / "purevox.exe")
    shutil.copy2(opus, stage / "opus.dll")
    shutil.copytree(models, stage / "models")
    for f in ("LICENSE", "MODEL-LICENSE.md", "README.md"):
        src = ROOT / f
        if src.is_file():
            shutil.copy2(src, stage / f)

    out = DIST / f"PureVox_{ver}_x64-setup.exe"
    out.unlink(missing_ok=True)
    r = subprocess.run(
        [
            makensis(),
            "/INPUTCHARSET",
            "UTF8",
            f"/DVERSION={ver}",
            f"/DPRODUCTVER={ver}.0",
            f"/DSTAGE={stage}",
            f"/DOUTFILE={out}",
            str(NSI),
        ],
        cwd=str(ROOT),
    )
    if r.returncode != 0:
        raise SystemExit(f"makensis 失败（exit={r.returncode}）")
    if not out.is_file() or out.stat().st_size == 0:
        raise SystemExit(f"安装包没有生成：{out}")
    print(f"installer -> {out}（{out.stat().st_size / (1024 * 1024):.1f} MB）")
    return 0


if __name__ == "__main__":
    sys.exit(main())
