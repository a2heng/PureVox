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
#   VERSION = 应用/包版本（semver）：`yyyy.MMdd.HHmm`（如 2026.1008.1430）。
#             Tauri 的 version 必须是 semver（`yyyy.MM.dd.HHmm` 四段会被拒），故把
#             「月.日」并成一段；日期的年/月/日/时/分信息一点不丢。
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
  # v2026.10.08.1430 → 2026.1008.1430
  IFS='.' read -r y mo d hm <<<"$tag"
else
  IFS='.' read -r y mo d hm <<<"$(date -u +%Y.%m.%d.%H%M)"
fi
# semver 不允许前导零（`2026.1009.0005` 会被 Tauri 拒），所以「月日」「时分」按十进制数写：
# 月日 1009 / 0109 → 1009 / 109；时分 0005 / 1430 → 5 / 1430（仍是「每天每分」唯一）。
ver="${y}.$((10#${mo}${d})).$((10#${hm}))"
stamp="${ver//./-}"

printf '{"version":"%s"}\n' "$ver" >"$ROOT/src-tauri/.build-version.json"
printf 'VERSION=%s\n' "$ver"
printf 'STAMP=%s\n' "$stamp"
printf 'PUREVOX_BUILD_VERSION=%s\n' "$ver"
