#!/usr/bin/env python3
# PureVox — AI 麦克风降噪工具
# Copyright (C) 2024-2026 a2heng <752848283@qq.com>
#
# PureVox is licensed under the GNU General Public License v3.0 or
# later (GPL-3.0-or-later).  See LICENSE for details.
#
# SPDX-License-Identifier: GPL-3.0-or-later
"""Linux 打包：release 二进制 + models/ + 文档 → 便携 tar.gz（有 dpkg-deb 时再出 deb）。

用法（在 Linux 上跑；Windows 只用于读脚本/门禁）：
  python3 tools/automation/package_linux.py
  python3 tools/automation/package_linux.py --no-build
  python3 tools/automation/package_linux.py --no-deb

产物：
  dist/PureVox_<VERSION>_x64.tar.gz
  dist/purevox_<VERSION>_amd64.deb   （可选）
"""

from __future__ import annotations

import argparse
import os
import shutil
import subprocess
import sys
import tarfile
import tempfile
from pathlib import Path

sys.path.insert(0, str(Path(__file__).resolve().parent))
import _utf8  # noqa: E402

_utf8.force()

ROOT = Path(__file__).resolve().parents[2]
SRC = ROOT / "src-tauri"
DIST = ROOT / "dist"
MODELS = ROOT / "models"
ICON = SRC / "icons" / "128x128.png"
BIN = SRC / "target" / "release" / "purevox"


def version() -> str:
    out = subprocess.check_output([sys.executable, str(ROOT / "tools" / "automation" / "version.py")])
    for line in out.decode("utf-8", "replace").splitlines():
        if line.startswith("VERSION="):
            return line.split("=", 1)[1].strip()
    raise SystemExit("version.py 没有输出 VERSION")


def build() -> None:
    subprocess.run(
        ["cargo", "build", "--release", "--manifest-path", str(SRC / "Cargo.toml")],
        cwd=str(ROOT),
        check=True,
    )


def stage(dest: Path) -> Path:
    """把运行所需文件塞进 dest/（目录布局同时给 tar.gz 和 deb 用）。"""
    if dest.exists():
        shutil.rmtree(dest)
    dest.mkdir(parents=True)
    shutil.copy2(BIN, dest / "purevox")
    (dest / "purevox").chmod(0o755)
    if not MODELS.is_dir():
        raise SystemExit(f"缺少模型目录：{MODELS}")
    shutil.copytree(MODELS, dest / "models")
    for name in ("LICENSE", "LICENSE-THIRD-PARTY.txt", "MODEL-LICENSE.md", "README.md"):
        src = ROOT / name
        if src.is_file():
            shutil.copy2(src, dest / name)
    if ICON.is_file():
        shutil.copy2(ICON, dest / "purevox.png")
    (dest / "purevox.desktop").write_text(
        """[Desktop Entry]
Name=PureVox
Comment=AI microphone noise reduction and mixer
Exec=purevox
Icon=purevox
Terminal=false
Type=Application
Categories=AudioVideo;Audio;
""",
        encoding="utf-8",
    )
    return dest


def make_tarball(ver: str, staged: Path) -> Path:
    DIST.mkdir(parents=True, exist_ok=True)
    out = DIST / f"PureVox_{ver}_x64.tar.gz"
    with tarfile.open(out, "w:gz") as tar:
        tar.add(staged, arcname="PureVox")
    return out


def make_deb(ver: str, staged: Path) -> Path | None:
    dpkg = shutil.which("dpkg-deb")
    if not dpkg:
        print("  跳过 deb：找不到 dpkg-deb（Debian/Ubuntu 上装 dpkg）")
        return None
    pkg = Path(tempfile.mkdtemp(prefix="purevox-deb-"))
    try:
        root = pkg / "root"
        (root / "usr" / "bin").mkdir(parents=True)
        (root / "usr" / "lib" / "purevox").mkdir(parents=True)
        (root / "usr" / "share" / "applications").mkdir(parents=True)
        (root / "usr" / "share" / "icons" / "hicolor" / "128x128" / "apps").mkdir(parents=True)
        shutil.copy2(staged / "purevox", root / "usr" / "bin" / "purevox.bin")
        (root / "usr" / "bin" / "purevox.bin").chmod(0o755)
        shutil.copytree(staged / "models", root / "usr" / "lib" / "purevox" / "models")
        if (staged / "purevox.png").is_file():
            shutil.copy2(
                staged / "purevox.png",
                root / "usr" / "share" / "icons" / "hicolor" / "128x128" / "apps" / "purevox.png",
            )
        shutil.copy2(staged / "purevox.desktop", root / "usr" / "share" / "applications" / "purevox.desktop")
        # 运行时按 exe 同级找 models/；deb 的二进制在 /usr/bin，模型在 /usr/lib/purevox，
        # 用一层 sh 包装把 PUREVOX_MODEL_DIR 对上（不把模型塞进 /usr/bin）。
        wrapper = root / "usr" / "bin" / "purevox"
        wrapper.write_text(
            "#!/bin/sh\n"
            "export PUREVOX_MODEL_DIR=/usr/lib/purevox/models\n"
            'exec /usr/bin/purevox.bin "$@"\n',
            encoding="utf-8",
        )
        wrapper.chmod(0o755)

        control = root / "DEBIAN" / "control"
        control.parent.mkdir()
        control.write_text(
            "Package: purevox\n"
            f"Version: {ver}\n"
            "Section: sound\n"
            "Priority: optional\n"
            "Architecture: amd64\n"
            "Maintainer: a2heng <752848283@qq.com>\n"
            "Depends: libasound2 | libasound2t64, libfontconfig1, libxkbcommon0, libwayland-client0, libvulkan1, libssl3 | libssl3t64, libopus0\n"
            "Description: PureVox AI microphone noise reduction and mixer\n"
            " Real-time AI denoise / TSE / AEC, multi-device node mixer and network audio.\n",
            encoding="utf-8",
        )
        DIST.mkdir(parents=True, exist_ok=True)
        out = DIST / f"purevox_{ver}_amd64.deb"
        subprocess.run([dpkg, "--build", "--root-owner-group", str(root), str(out)], check=True)
        return out
    finally:
        shutil.rmtree(pkg, ignore_errors=True)


def main() -> int:
    ap = argparse.ArgumentParser(description="PureVox Linux 打包")
    ap.add_argument("--no-build", action="store_true", help="不跑 cargo build --release")
    ap.add_argument("--no-deb", action="store_true", help="只出 tar.gz，不出 deb")
    args = ap.parse_args()
    if sys.platform != "linux":
        raise SystemExit("package_linux.py 只能在 Linux 上跑（当前不是 Linux）")
    ver = version()
    if not args.no_build:
        build()
    if not BIN.is_file():
        raise SystemExit(f"找不到 release 二进制：{BIN}（先跑 cargo build --release）")
    pkg = Path(tempfile.mkdtemp(prefix="purevox-stage-"))
    try:
        stage(pkg)
        tar = make_tarball(ver, pkg)
        print(f"tar.gz: {tar}")
        if not args.no_deb:
            deb = make_deb(ver, pkg)
            if deb:
                print(f"deb:    {deb}")
    finally:
        shutil.rmtree(pkg, ignore_errors=True)
    return 0


if __name__ == "__main__":
    sys.exit(main())
