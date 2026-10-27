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
"""重建 Android 端的 libopus 静态库 + libpvopus.so（预编译产物**入库**，平时打包用不到本脚本）。

为什么要「产物入库、构建不编译」：.so 直接放 `app/src/main/jniLibs/arm64-v8a/`，
Gradle 构建就不需要 NDK —— CI 也就不用装 NDK、不用多一步编译。和电脑端的
`server/opus.dll`（预编译入库、运行时加载）是同一条路子。

只在两种时候需要跑本脚本：
  1. 升级 libopus 版本（第一个位置参数）
  2. 改了 `cpp/pv_opus.cpp` 或 `cpp/CMakeLists.txt`

产物：
  `third_party/opus/include/*.h`        ← libopus 公开头（JNI 只需要这些）
  `third_party/opus/arm64-v8a/libopus.a`
  `jniLibs/arm64-v8a/libpvopus.so`

用法：
  python tools/automation/build_opus_android.py [版本] [--keep-tmp]

前置：Android NDK（含 clang/cmake）、cmake、ninja、curl、tar。

跨平台单份实现（取代原先的 .ps1 + .sh 两份）。
"""

from __future__ import annotations

import argparse
import os
import platform
import shutil
import subprocess
import sys
import tempfile
from pathlib import Path

# 中文输出必须先切 UTF-8（Windows CI runner 默认 cp1252），见 _utf8.py 的说明。
sys.path.insert(0, str(Path(__file__).resolve().parent))
import _utf8  # noqa: E402

_utf8.force()

IS_WIN = os.name == "nt"
DEFAULT_OPUS_VERSION = "1.5.2"


def run(cmd: list[str], cwd: Path | None = None, env: dict[str, str] | None = None) -> None:
    printable = " ".join(str(c) for c in cmd)
    print(f"  $ {printable}", flush=True)
    proc = subprocess.run([str(c) for c in cmd], cwd=str(cwd) if cwd else None, env=env)
    if proc.returncode != 0:
        raise SystemExit(f"命令失败（exit={proc.returncode}）：{printable}")


def find_ndk() -> Path:
    """定位 NDK。优先级：显式环境变量 > SDK 下版本号最大的那个。

    版本比较用 sort -V（27.0.12077973 > 26.3.11579264，字典序会把 26 当成更大）。
    """
    for var in ("ANDROID_NDK_HOME", "ANDROID_NDK_ROOT"):
        val = os.environ.get(var)
        if val and Path(val).is_dir():
            return Path(val)

    sdk_roots = [
        os.environ.get("ANDROID_HOME"),
        os.environ.get("ANDROID_SDK_ROOT"),
        Path.home() / "AppData" / "Local" / "Android" / "Sdk" if IS_WIN else None,
        Path.home() / "Library" / "Android" / "sdk",
        Path.home() / "Android" / "Sdk",
    ]
    for root in sdk_roots:
        if not root:
            continue
        ndk_dir = Path(root) / "ndk"
        if not ndk_dir.is_dir():
            continue
        versions = sorted((p for p in ndk_dir.iterdir() if p.is_dir()), reverse=True)
        if versions:
            return versions[0]
    raise SystemExit(
        "找不到 NDK。设 ANDROID_NDK_HOME，或用 sdkmanager \"ndk;27.0.12077973\" 装一个。"
    )


def find_nm(ndk: Path) -> Path:
    """llvm-nm 只在 NDK 里，不在 PATH 上。"""
    hit = next(ndk.glob("toolchains/llvm/prebuilt/*/bin/llvm-nm*"), None)
    if hit is None:
        raise SystemExit("NDK 里找不到 llvm-nm。")
    return hit


def max_load_align(path: Path) -> int:
    """读 ELF64 程序头，返回所有 PT_LOAD 的 p_align 最大值（纯标准库，不需要 NDK）。

    16 KB 页：`0x4000`。4 KB 页：`0x1000`（Android 15+ 的设备上会装不上）。
    """
    raw = path.read_bytes()
    if raw[:4] != b"ELF" or raw[4] != 2:  # ELF64
        raise SystemExit(f"{path} 不是 ELF64")
    e_phoff = int.from_bytes(raw[0x20:0x28], "little")
    e_phentsize = int.from_bytes(raw[0x36:0x38], "little")
    e_phnum = int.from_bytes(raw[0x38:0x3A], "little")
    best = 0
    for i in range(e_phnum):
        off = e_phoff + i * e_phentsize
        p_type = int.from_bytes(raw[off:off + 4], "little")
        if p_type != 1:  # PT_LOAD
            continue
        align = int.from_bytes(raw[off + 0x30:off + 0x38], "little")
        best = max(best, align)
    return best


def main() -> int:
    ap = argparse.ArgumentParser(description="重建 Android 的 libopus.a 与 libpvopus.so")
    ap.add_argument("version", nargs="?", default=DEFAULT_OPUS_VERSION,
                    help=f"libopus 版本（默认 {DEFAULT_OPUS_VERSION}；改它等于换库，"
                         "**必须同步** LICENSE-THIRD-PARTY.txt）")
    ap.add_argument("--keep-tmp", action="store_true",
                    help="保留下载与编译的临时目录（排查编译问题时用）")
    args = ap.parse_args()

    opus_version = args.version
    repo = Path(__file__).resolve().parents[2]
    cpp_dir = repo / "android" / "app" / "src" / "main" / "cpp"
    opus_dir = cpp_dir / "third_party" / "opus"
    jni_dir = repo / "android" / "app" / "src" / "main" / "jniLibs" / "arm64-v8a"

    ndk = find_ndk()
    toolchain = ndk / "build" / "cmake" / "android.toolchain.cmake"
    if not toolchain.is_file():
        raise SystemExit(f"NDK 里没有 {toolchain}（{ndk} 不是完整 NDK？）")

    cmake = shutil.which("cmake")
    if not cmake:
        raise SystemExit("找不到 cmake。")
    if not shutil.which("ninja"):
        raise SystemExit("找不到 ninja。")
    # 注意：SDK 自带的 cmake 带的是 Windows 版 ninja，Linux 上必须用系统的 ——
    # CMake 只按当前平台找 ninja。
    nm_bin = find_nm(ndk)

    print(f"平台    : {platform.system()} {platform.machine()}")
    print(f"NDK     : {ndk}")
    print(f"cmake   : {run_and_capture([cmake, '--version']).splitlines()[0]}")
    print(f"libopus : {opus_version}")

    tmp = Path(tempfile.mkdtemp(prefix=f"pv_opus_{opus_version}_"))
    try:
        # ---- 1. 取源码 ----
        print(f"\n[1/4] 下载 libopus {opus_version}", flush=True)
        src_dir = tmp / f"opus-{opus_version}"
        if not src_dir.is_dir():
            url = (f"https://github.com/xiph/opus/releases/download/v{opus_version}"
                   f"/opus-{opus_version}.tar.gz")
            tar = tmp / "opus.tar.gz"
            curl = shutil.which("curl")
            if curl:
                run([curl, "-fL", "--retry", "3", "-o", tar, url])
            else:
                run(["wget", "-O", tar, url])
            # **用标准库解包**，不要 shell 出去调 tar：
            # Git for Windows 的 GNU tar 把 `C:\…` 当远程主机（"Cannot connect to C"），
            # 加 `--force-local` 又和「选项簇必须在第一个参数」的规则打架；Windows 自带的
            # bsdtar 不认那个开关。tarfile 三个平台一个行为，没有这些坑。
            import tarfile

            with tarfile.open(tar, "r:gz") as tf:
                tf.extractall(tmp, filter="data")
        # 发版包里没有 .git，libopus 的 CMake 会退回到 package_version 文件（已验证可用）
        if not src_dir.is_dir():
            raise SystemExit(f"源码解压后没找到 {src_dir}")

        # ---- 2. 编 libopus.a ----
        print("\n[2/4] 编译 libopus.a（arm64-v8a, API 24）", flush=True)
        opus_build = tmp / "build-opus"
        run([cmake, "-G", "Ninja",
             f"-DCMAKE_TOOLCHAIN_FILE={toolchain}",
             "-DANDROID_ABI=arm64-v8a",
             "-DANDROID_PLATFORM=android-24",
             "-DCMAKE_BUILD_TYPE=Release",
             "-DBUILD_SHARED_LIBS=OFF",
             "-DOPUS_BUILD_PROGRAMS=OFF",
             "-DOPUS_BUILD_TESTING=OFF",
             "-DOPUS_INSTALL_PKG_CONFIG_MODULE=OFF",
             "-S", src_dir, "-B", opus_build])
        run([cmake, "--build", opus_build])

        # ---- 3. 编 libpvopus.so（JNI 转发层）----
        print("\n[3/4] 编译 libpvopus.so", flush=True)
        (opus_dir / "arm64-v8a").mkdir(parents=True, exist_ok=True)
        (opus_dir / "include").mkdir(parents=True, exist_ok=True)
        shutil.copy2(opus_build / "libopus.a", opus_dir / "arm64-v8a" / "libopus.a")
        for header in (src_dir / "include").glob("*.h"):
            shutil.copy2(header, opus_dir / "include" / header.name)
        # libopus 的许可证文件名各版本不一（COPYING / LICENSE），挨个找
        for pattern in ("COPYING*", "LICENSE*"):
            found = next(iter(sorted(src_dir.glob(pattern))), None)
            if found and found.is_file():
                shutil.copy2(found, opus_dir / "LICENSE-opus")
                break

        jni_build = tmp / "build-jni"
        run([cmake, "-G", "Ninja",
             f"-DCMAKE_TOOLCHAIN_FILE={toolchain}",
             "-DANDROID_ABI=arm64-v8a",
             "-DANDROID_PLATFORM=android-24",
             # Android 15+ 要求原生库按 16 KB 页对齐（NDK 的这个开关会带上
             # -Wl,-z,max-page-size=16384；CMakeLists 里也显式写了，双保险）
             "-DANDROID_SUPPORT_FLEXIBLE_PAGE_SIZES=ON",
             "-DCMAKE_BUILD_TYPE=Release",
             "-S", cpp_dir, "-B", jni_build])
        run([cmake, "--build", jni_build])

        # ---- 4. 落位 + 校验 ----
        print("\n[4/4] 落位并校验", flush=True)
        jni_dir.mkdir(parents=True, exist_ok=True)
        so = jni_dir / "libpvopus.so"
        shutil.copy2(jni_build / "libpvopus.so", so)

        # **16 KB 页对齐**：读 ELF 的 PT_LOAD，p_align 必须 >= 0x4000。
        # 这一条错了不会在编译期报，只会在 16 KB 页的机器上装不上（或起不来），所以当门禁看。
        align = max_load_align(so)
        if align < 0x4000:
            raise SystemExit(
                f"{so.name} 的 PT_LOAD 对齐只有 0x{align:x}（要求 >= 0x4000 = 16 KB）。"
                "检查 CMakeLists 的 -Wl,-z,max-page-size=16384。"
            )
        print(f"  16 KB 对齐：p_align=0x{align:x} ✓")

        # 必须只导出 JNI 入口：libopus 的 C 符号漏出去会污染 .so 的符号表（更大，
        # 还可能与 app 里的同名符号撞车），所以这一条要当门禁看，不是「看一眼」。
        symbols = run_and_capture([nm_bin, "-D", "--defined-only", str(so)]).split()
        names = [s for s in symbols if s.startswith("Java_") or s.startswith("opus_")]
        stray = [s for s in names if not s.startswith("Java_")]
        jni_count = sum(1 for s in names if s.startswith("Java_"))
        if stray:
            raise SystemExit(f"导出了非 JNI 符号（应为 0）：{', '.join(stray)}")
        if jni_count < 8:
            raise SystemExit(f"只导出 {jni_count} 个 JNI 符号，疑似 visibility 配置丢了")

        kb = so.stat().st_size // 1024
        print(f"\n完成：libpvopus.so {kb} KB，{jni_count} 个 JNI 符号，0 个泄漏。")
        print("打包：cd android && ./gradlew assembleDebug")
    finally:
        if args.keep_tmp:
            print(f"临时目录保留在 {tmp}")
        else:
            shutil.rmtree(tmp, ignore_errors=True)
    return 0


def run_and_capture(cmd: list[str]) -> str:
    proc = subprocess.run([str(c) for c in cmd], capture_output=True, text=True,
                          encoding="utf-8", errors="replace")
    return proc.stdout or ""


if __name__ == "__main__":
    sys.exit(main())