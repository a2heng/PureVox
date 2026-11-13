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
  dist/purevox_<VERSION>_amd64.deb   （有 dpkg-deb 时）
  dist/PureVox_<VERSION>_x64.AppImage（传了 --appimagetool 时，CI 用这条）
"""

from __future__ import annotations

import argparse
import glob
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
# 与旧 Tauri 版一致：deb / AppImage 用 256 px 图标（桌面环境更常见）
ICON = SRC / "icons" / "256x256.png"
if not ICON.is_file():
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


APPSTREAM = """<?xml version="1.0" encoding="UTF-8"?>
<component type="desktop-application">
  <id>com.purevox.PureVox</id>
  <name>PureVox</name>
  <summary>Real-time AI microphone noise reduction</summary>
  <developer id="com.purevox.PureVox">
    <name>a2heng</name>
  </developer>
  <metadata_license>CC0-1.0</metadata_license>
  <project_license>GPL-3.0-or-later</project_license>
  <description>
    <p>Real-time AI denoise, target speech extraction and echo cancellation,
    with a node graph mixer and network audio.</p>
  </description>
  <launchable type="desktop-id">purevox.desktop</launchable>
  <url type="homepage">https://github.com/a2heng/PureVox</url>
  <provides>
    <binary>purevox</binary>
  </provides>
  <categories>
    <category>AudioVideo</category>
  </categories>
  <content_rating type="oars-1.1"/>
</component>
"""

DESKTOP_ENTRY = """[Desktop Entry]
Type=Application
Name=PureVox
Name[zh_CN]=PureVox
GenericName=AI Mic Noise Reduction
GenericName[zh_CN]=AI 麦克风降噪
Comment=Real-time AI microphone noise reduction
Comment[zh_CN]=实时 AI 麦克风降噪
Exec=purevox
Icon=purevox
Terminal=false
Categories=AudioVideo;Audio;Utility;
Keywords=mic;noise;denoise;audio;PureVox;
StartupNotify=false
"""


def bundle_opus(dest: Path) -> None:
    """把系统 libopus 带进 AppImage（旧 Tauri 版同款做法）。

    网络音频在 Linux 上 `dlopen("libopus.so.1")`；宿主机没装 libopus0 时不该
    静默丢掉网络音频。AppRun 会把 `usr/lib/purevox` 加进 LD_LIBRARY_PATH。
    """
    dest.mkdir(parents=True, exist_ok=True)
    copied = 0
    for pattern in (
        "/usr/lib/x86_64-linux-gnu/libopus.so*",
        "/usr/lib64/libopus.so*",
        "/usr/lib/libopus.so*",
    ):
        for so in glob.glob(pattern):
            shutil.copy2(so, dest / Path(so).name)
            copied += 1
    if copied == 0:
        print("  警告：没找到 libopus.so；AppImage 里的网络音频将依赖宿主机 libopus0")


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
    (dest / "purevox.desktop").write_text(DESKTOP_ENTRY, encoding="utf-8")
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
        (root / "usr" / "share" / "metainfo").mkdir(parents=True)
        (root / "usr" / "share" / "icons" / "hicolor" / "256x256" / "apps").mkdir(parents=True)
        shutil.copy2(staged / "purevox", root / "usr" / "bin" / "purevox.bin")
        (root / "usr" / "bin" / "purevox.bin").chmod(0o755)
        shutil.copytree(staged / "models", root / "usr" / "lib" / "purevox" / "models")
        if (staged / "purevox.png").is_file():
            shutil.copy2(
                staged / "purevox.png",
                root / "usr" / "share" / "icons" / "hicolor" / "256x256" / "apps" / "purevox.png",
            )
        shutil.copy2(staged / "purevox.desktop", root / "usr" / "share" / "applications" / "purevox.desktop")
        (root / "usr" / "share" / "metainfo" / "com.purevox.PureVox.metainfo.xml").write_text(
            APPSTREAM, encoding="utf-8"
        )
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


def make_appimage(ver: str, staged: Path, tool: Path | None) -> Path | None:
    """hand-made AppDir + appimagetool：便携单文件，不依赖安装包管理器。

    `--appimage-extract-and-run` 让 appimagetool 在没 FUSE 的 CI 容器里也能跑。
    """
    if tool is None or not tool.is_file():
        print("  跳过 AppImage：没给 --appimagetool（CI 下载后传进来）")
        return None
    appdir = Path(tempfile.mkdtemp(prefix="purevox-appdir-")) / "PureVox.AppDir"
    try:
        (appdir / "usr" / "bin").mkdir(parents=True)
        (appdir / "usr" / "share" / "applications").mkdir(parents=True)
        (appdir / "usr" / "share" / "metainfo").mkdir(parents=True)
        (appdir / "usr" / "share" / "icons" / "hicolor" / "256x256" / "apps").mkdir(
            parents=True
        )
        shutil.copy2(staged / "purevox", appdir / "usr" / "bin" / "purevox")
        (appdir / "usr" / "bin" / "purevox").chmod(0o755)
        shutil.copytree(staged / "models", appdir / "usr" / "bin" / "models")
        shutil.copy2(staged / "purevox.png", appdir / "purevox.png")
        shutil.copy2(
            staged / "purevox.png",
            appdir / "usr" / "share" / "icons" / "hicolor" / "256x256" / "apps" / "purevox.png",
        )
        shutil.copy2(staged / "purevox.desktop", appdir / "purevox.desktop")
        shutil.copy2(
            staged / "purevox.desktop",
            appdir / "usr" / "share" / "applications" / "purevox.desktop",
        )
        (
            appdir / "usr" / "share" / "metainfo" / "com.purevox.PureVox.metainfo.xml"
        ).write_text(APPSTREAM, encoding="utf-8")
        apprun = appdir / "AppRun"
        apprun.write_text(
            "#!/bin/sh\n"
            'HERE="$(dirname "$(readlink -f "$0")")"\n'
            'exec "$HERE/usr/bin/purevox" "$@"\n',
            encoding="utf-8",
        )
        apprun.chmod(0o755)
        DIST.mkdir(parents=True, exist_ok=True)
        out = DIST / f"PureVox_{ver}_x64.AppImage"
        env = dict(os.environ, ARCH="x86_64")
        subprocess.run(
            # appimagetool 只找 `<desktop名>.appdata.xml`，而 AppStream 规范要求
            # 文件名跟随反向域名 ID（我们是 com.purevox.PureVox.metainfo.xml）。
            # 这个检查在 appimagetool 里是出了名的误报；用 --no-appstream 关掉它的
            # 旧检查，元数据仍然原样打进 AppDir，发行版 / 软件中心照样能读。
            [
                str(tool),
                "--appimage-extract-and-run",
                "--no-appstream",
                str(appdir),
                str(out),
            ],
            check=True,
            env=env,
        )
        out.chmod(0o755)
        return out
    finally:
        shutil.rmtree(appdir.parent, ignore_errors=True)


def main() -> int:
    ap = argparse.ArgumentParser(description="PureVox Linux 打包")
    ap.add_argument("--no-build", action="store_true", help="不跑 cargo build --release")
    ap.add_argument("--no-deb", action="store_true", help="只出 tar.gz，不出 deb")
    ap.add_argument(
        "--appimagetool",
        type=Path,
        default=None,
        help="appimagetool 可执行文件（CI 下载后传入；不给就跳过 AppImage）",
    )
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
        appimage = make_appimage(ver, pkg, args.appimagetool)
        if appimage:
            print(f"AppImage: {appimage}")
    finally:
        shutil.rmtree(pkg, ignore_errors=True)
    return 0


if __name__ == "__main__":
    sys.exit(main())
