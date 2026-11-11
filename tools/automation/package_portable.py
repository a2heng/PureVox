#!/usr/bin/env python3
# PureVox — AI 麦克风降噪工具
# Copyright (C) 2024-2026 a2heng <752848283@qq.com>
#
# PureVox is licensed under the GNU General Public License v3.0 or
# later (GPL-3.0-or-later).  See LICENSE for details.
#
# SPDX-License-Identifier: GPL-3.0-or-later
"""桌面便携测试包（Windows）：`cargo build --release` + 目录 + zip。

**为什么先有便携包**：安装包流水线（NSIS / AppImage / deb）还没随 GPUI 重建恢复，
但测试版需要能直接双击运行。便携包把 exe、`opus.dll`、`models/` 放在同一目录，
不写注册表、不装服务，删目录即卸载。

用法：
  python tools/automation/package_portable.py            # 构建 + 打包
  python tools/automation/package_portable.py --no-build # 复用已有 release 产物

产物：`dist/PureVox-<版本>-windows-x64-portable.zip`
"""

from __future__ import annotations

import argparse
import os
import shutil
import subprocess
import sys
import zipfile
from pathlib import Path

sys.path.insert(0, str(Path(__file__).resolve().parent))
import _utf8  # noqa: E402

_utf8.force()

ROOT = Path(__file__).resolve().parents[2]
TAURI = ROOT / "src-tauri"
DIST = ROOT / "dist"


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


def main() -> int:
    ap = argparse.ArgumentParser(description="桌面便携测试包")
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
    (stage / "运行说明.txt").write_text(
        "PureVox 便携测试版\r\n"
        "1. 双击 purevox.exe\r\n"
        "2. 模型在 models/ 目录，opus.dll 是网络音频依赖，两者必须和 exe 同级\r\n"
        "3. 本目录可整体移动 / 删除；不写注册表\r\n",
        encoding="utf-8",
        newline="\r\n",
    )

    zip_path = DIST / f"{name}-portable.zip"
    zip_path.unlink(missing_ok=True)
    with zipfile.ZipFile(zip_path, "w", compression=zipfile.ZIP_DEFLATED) as z:
        for p in sorted(stage.rglob("*")):
            if p.is_file():
                z.write(p, p.relative_to(DIST))
    size_mb = zip_path.stat().st_size / (1024 * 1024)
    print(f"portable -> {zip_path}（{size_mb:.1f} MB）")
    print(f"目录      -> {stage}")
    return 0


if __name__ == "__main__":
    sys.exit(main())
