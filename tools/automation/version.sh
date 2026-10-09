#!/usr/bin/env bash
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
#
# 构建版本解析（**版本号 = 日期**；与 version.ps1 同一契约）：
#   tag 触发  GITHUB_REF_NAME=v2026.10.08.1430
#   本机/手动                                  → 当前 UTC 日期时间
#   VERSION = 应用/包版本（semver 三段）：`yy.M.<日×1440+时分>`
#             如 v2026.10.08.1430 → `26.10.14509`。三段都有硬上限（MSI 的
#             ProductVersion：major≤255、minor≤255、build≤65535），日期不能直译：
#               major = 年 - 2000 → 26
#               minor = 月         → 1..12
#               build = 日×1440 + (时×60+分) → 1..47059，全序且每分钟唯一
#             tag 本身仍是完整时间戳 `v2026.10.08.1430`，可读性不受影响。
#   STAMP   = VERSION 的 `.`→`-`（文件名用）。
# 同时生成 Tauri 版本覆盖配置 `src-tauri/.build-version.json`（不提交，见该目录 .gitignore），
# 构建时用 `cargo tauri build --config .build-version.json`；界面显示版本走
# `PUREVOX_BUILD_VERSION` 环境变量（见 `src/version.rs`）。
# 用法：本机 `eval "$(bash tools/automation/version.sh)"`；
#       CI（Linux）`bash tools/automation/version.sh >> "$GITHUB_ENV"`。
set -euo pipefail

ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/../.." && pwd)"
ref="${GITHUB_REF_NAME:-}"
tag=""
case "$ref" in
  v*) tag="${ref#v}" ;;
esac
if [ -n "$tag" ]; then
  IFS='.' read -r y mo d hm <<<"$tag"
else
  IFS='.' read -r y mo d hm <<<"$(date -u +%Y.%m.%d.%H%M)"
fi
# 10# 去掉前导零（bash 把 09 当八进制）：月 09→9、日 09→9、时分 0629→629。
yy=$((10#${y} % 100))
mo_n=$((10#${mo}))
d_n=$((10#${d}))
hm_n=$((10#${hm}))
# build = 日×1440 + 时分：把「几号几点几分」压成一段，仍按时间全序、每分钟唯一，
# 且上限 47059 < 65535（MSI ProductVersion 的 build 上限）。
ver="${yy}.${mo_n}.$((d_n * 1440 + hm_n))"
stamp="${ver//./-}"

printf '{"version":"%s"}\n' "$ver" >"$ROOT/src-tauri/.build-version.json"
printf 'VERSION=%s\n' "$ver"
printf 'STAMP=%s\n' "$stamp"
printf 'PUREVOX_BUILD_VERSION=%s\n' "$ver"
