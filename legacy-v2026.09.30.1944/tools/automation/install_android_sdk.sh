#!/bin/bash
# PureVox - Android SDK/NDK installer (single implementation path).
# Copyright (C) 2024-2026 a2heng <752848283@qq.com>
# SPDX-License-Identifier: GPL-3.0-or-later
#
# Usage: bash tools/automation/install_android_sdk.sh
# Accepts licenses, installs platform 34 + NDK r27 + build-tools, retrying
# each sdkmanager call. A truncated NDK zip leaves a corrupt partial tree
# that sdkmanager will NOT repair on retry, so the NDK dir is wiped before
# every attempt. ("yes" SIGPIPE noise is silenced: it is not a failure.)
set -euo pipefail

PLATFORM="platforms;android-34"
NDK_PKG="ndk;27.0.12077973"
NDK_DIR="$ANDROID_HOME/ndk/27.0.12077973"
BUILD_TOOLS="build-tools;34.0.0"

yes 2>/dev/null | sdkmanager --licenses >/dev/null || true

bash tools/automation/retry.sh 3 sdkmanager "$PLATFORM" "$BUILD_TOOLS"

i=1
while true; do
    rm -rf "$NDK_DIR"
    if sdkmanager "$NDK_PKG"; then
        break
    fi
    if [ "$i" -ge 3 ]; then
        echo "install_android_sdk.sh: NDK install failed 3 times, giving up" >&2
        exit 1
    fi
    echo "install_android_sdk.sh: NDK attempt $i/3 failed, retrying..." >&2
    sleep $((i * 10))
    i=$((i + 1))
done
test -d "$NDK_DIR"
echo "Android SDK ready: $PLATFORM + $NDK_PKG"
