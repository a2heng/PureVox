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
"""构建版本解析（**版本号 = 年.月.日**；跨平台唯一实现）。

**两条发布通道**（`RELEASE-CHANNELS.md`）：
  桌面 tag `v2026.10.08.1430`；安卓 tag `android-v2026.10.08.1430`。
同一份脚本两边都认：`PLATFORM` 告诉 CI 该发到哪个通道，`TAG` 输出完整 tag。

产出（全部写到 stdout，CI 重定向进 $GITHUB_ENV / 本机 eval）：

  PLATFORM = desktop | android
  TAG     = v2026.10.08.1430 / android-v2026.10.08.1430   完整时间戳（**比较更新的依据**）
  VERSION = 26.10.8            应用/包版本 = 年.月.日（semver 三段，Tauri 硬要求）
  STAMP   = 2026-10-08-1430    完整时间戳的文件名形式（产物重命名用，含时分故唯一）
  ACODE   = 261008143          Android versionCode = yyMMddHHmm ÷ 10（单调递增，Int32 内）
  PUREVOX_BUILD_VERSION / PUREVOX_BUILD_STAMP → 编译进程序（见 src/version.rs）

为什么 VERSION 只有三段：Tauri 要 semver 三段；历史上还要出 MSI，MSI ProductVersion 限
major≤255 / minor≤255 / build≤65535，`2026.10.08` 直译会报 major > 255。故 major=年-2000、
minor=月、build=日（≤31，远低于上限）。**现在 Windows 只发 NSIS（exe）**，但映射保持不变
（显示版本 / 产物名 / 更新说明都按它走）。**同日多次发布靠 TAG 的时分区分**，更新检查
比较 TAG（见 src/update/），因此显示版本重复不影响更新判定。

Android 的 ACODE 取 tag 的 `yyMMddHHmm` **去掉最低一位**（10 分钟粒度）：versionCode 是
Int32，十位数的值装不进去，溢出只会让安装包悄悄变成 versionCode=1（应用内更新永远报
「开发构建」）。详见 main() 里的注释与 `_VERSION_CODE_MAX`。**两侧必须同算法**：
应用侧是 `android/.../update/UpdateChecker.kt` 的 `stampToCode`。

同时生成 Tauri 版本覆盖配置 `src-tauri/.build-version.json`（不提交，见该目录
.gitignore），构建时用 `cargo tauri build --config .build-version.json`。

tag 触发时 GITHUB_REF_NAME=v2026.10.08.1430；否则取当前 **UTC**（不是本地时间：
CI 的 runner 与发版机器时区各不相同，只有 UTC 能让「同一天」的含义一致）。

用法：
  本机    eval "$(python3 tools/automation/version.py)"
  CI      python3 tools/automation/version.py >> "$GITHUB_ENV"

只依赖标准库。取代了曾经的 version.ps1 / version.sh 两份实现 —— 那两份从不同时被 CI
跑到，漂移了没人知道，表现是「Linux 打出的包版本比 Windows 的旧」。见 tests.yml 的
`Gate · version`：那份门禁对同一批 tag 断言输出固定。
"""

from __future__ import annotations

import json
import os
import sys
from datetime import datetime, timezone
from pathlib import Path

ROOT = Path(__file__).resolve().parents[2]

# Android versionCode 的上限（Play 的硬限值，也确保 Int32 装得下）。ACODE 必须落在这以内，
# 否则 Gradle 侧会溢出 —— 那种错误不会自己浮出来，只会让 APK 永远报「开发构建」。
_VERSION_CODE_MAX = 2_100_000_000


def derive(ref: str, default_platform: str = "desktop") -> tuple[str, str, int, int, int, int]:
    """从 tag（或「现在」）推出 (platform, 年, 月, 日, 时分)。

    `android-v...` = 安卓通道；`v...` = 桌面通道。手动触发没有 tag 时用
    `PV_PLATFORM`（默认 desktop）决定通道。
    """
    platform = default_platform
    rest = ref
    if ref.startswith("android-v"):
        platform = "android"
        rest = ref[len("android-"):]  # 留下 "v2026...."，交给下面统一解析
    if rest.startswith("v"):
        parts = rest[1:].split(".")
        if len(parts) != 4:
            raise SystemExit(
                f"tag 格式应为 v<yyyy.MM.dd.HHmm> 或 android-v<yyyy.MM.dd.HHmm>，收到：{ref}"
            )
        y, mo, d, hm = parts
        mo, d, hm = int(mo), int(d), int(hm)
        # 范围校验与应用侧 `UpdateChecker.stampToCode` 一致：脏 tag（如 `2399`）不许算出
        # 一个「看着合法」的 versionCode —— 两侧各判各的就会漂移。
        if not 1 <= mo <= 12 or not 1 <= d <= 31 or not 0 <= hm <= 2359 or hm % 100 > 59:
            raise SystemExit(f"tag 的月/日/时分越界：{ref}")
        return platform, y, mo, d, hm
    now = datetime.now(timezone.utc)
    return platform, now.strftime("%Y"), now.month, now.day, now.hour * 100 + now.minute


def main() -> int:
    ref = os.environ.get("GITHUB_REF_NAME", "")
    platform, y, mo, d, hm = derive(ref, os.environ.get("PV_PLATFORM", "desktop"))
    yy = int(y) % 100

    # tag 里的月/日/时分可能是 `08`、`09`，拼回去时补零（文件名与 versionCode 都要定长）
    mo2, d2, hm4 = f"{mo:02d}", f"{d:02d}", f"{hm:04d}"
    prefix = "android-v" if platform == "android" else "v"
    tag = f"{prefix}{y}.{mo2}.{d2}.{hm4}"
    ver = f"{yy}.{mo}.{d}"                      # 显示版本：整数，不补零（26.9.9）
    stamp = f"{y}-{mo2}-{d2}-{hm4}"             # 文件名形式
    acode = (
        int(yy) * 10**7 + mo * 10**5 + d * 10**3 + hm // 10
    )  # Android versionCode，见下；定长补零靠乘 10 的幂（与应用侧逐字同构）
    # **÷10 不能省**：versionCode 是 Int32（上限 2147483647），十位数的 `yyMMddHHmm`
    # （2610172359 ≈ 2.6e9）装不进去 —— 传过去会**静默溢出**：Gradle 侧 `toIntOrNull()`
    # 返回 null、回退成 1，CI 全绿却发出一个「永远报开发构建」的 APK（2026-10-17 实测踩到：
    # versionName=26.10.17 / versionCode=1）。去掉最低一位即 10 分钟粒度：顺序仍单调，
    # 上限 991231235 永不溢出；撞号要两个 tag 落在同一 10 分钟内，而一次发版 CI 就要 20 分钟。
    # 拼法是**乘 10 的幂**而不是 `String.format`：应用侧 Kotlin 的 `%d` 按默认 locale 出数字，
    # 阿拉伯语等 locale 会给非 ASCII 数字（那条链同样只有一条实现，两侧必须一模一样）。
    if not 0 < acode <= _VERSION_CODE_MAX:
        raise SystemExit(f"ACODE 越界（{acode}，tag {tag}）：versionCode 是 Int32")

    # Tauri 的版本覆盖配置（不提交；构建时 --config 传进去）
    cfg = ROOT / "src-tauri" / ".build-version.json"
    cfg.write_text(json.dumps({"version": ver}) + "\n", encoding="utf-8", newline="\n")

    for line in (
        f"PLATFORM={platform}",
        f"TAG={tag}",
        f"VERSION={ver}",
        f"STAMP={stamp}",
        f"ACODE={acode}",
        f"PUREVOX_BUILD_VERSION={ver}",
        f"PUREVOX_BUILD_STAMP={y}.{mo2}.{d2}.{hm4}",
    ):
        print(line)
    return 0


if __name__ == "__main__":
    sys.exit(main())