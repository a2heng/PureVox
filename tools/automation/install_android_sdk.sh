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
# Android SDK 安装（CI 与本机同一份；sdkmanager 由调用方保证在 PATH 上）。
#   sdkmanager 的默认包列表里有已下架的 `tools`，必须显式给平台与 build-tools；
#   android-actions/setup-android 只为它自己的 `packages` 装包，平台/branch 由本脚本负责。
# 版本号来自 tools/automation/versions.env（唯一来源），不在这里硬编码。
# 注意：本工程**不需要 NDK / CMake**（Opus 走系统 MediaCodec，无 JNI），勿再加。
set -euo pipefail

ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/../.." && pwd)"
# shellcheck source=/dev/null
. "$ROOT/tools/automation/versions.env"

: "${ANDROID_PLATFORM:?versions.env 缺少 ANDROID_PLATFORM}"
: "${ANDROID_BUILD_TOOLS:?versions.env 缺少 ANDROID_BUILD_TOOLS}"

yes 2>/dev/null | sdkmanager --licenses >/dev/null || true
sdkmanager "platforms;$ANDROID_PLATFORM" "build-tools;$ANDROID_BUILD_TOOLS"
echo "Android SDK ready: platforms;$ANDROID_PLATFORM + build-tools;$ANDROID_BUILD_TOOLS"
