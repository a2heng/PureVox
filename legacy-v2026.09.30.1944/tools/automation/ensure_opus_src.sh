#!/bin/bash
# PureVox - idempotent Opus source fetch for the Android JNI build.
# Copyright (C) 2024-2026 a2heng <752848283@qq.com>
# SPDX-License-Identifier: GPL-3.0-or-later
#
# Single implementation path for: ci.yml (android job), cache.yml
# (opus-android), verify-local.sh. Version comes from tools/automation/versions.env.
# Retries the download so a truncated zip no longer fails the job; a cached
# tree with CMakeLists.txt is reused untouched.
set -euo pipefail
cd "$(dirname "$0")/../.."

# shellcheck disable=SC1091
source tools/automation/versions.env

DEST="android/opus-src"
if [ -f "$DEST/CMakeLists.txt" ]; then
    echo "opus-src $OPUS_VER cached, skip download"
    exit 0
fi
mkdir -p "$DEST"
TMP_ZIP="$(mktemp --suffix=-opus.zip)"
bash tools/automation/retry.sh 3 wget -q -O "$TMP_ZIP" \
    "https://github.com/xiph/opus/archive/refs/tags/v${OPUS_VER}.zip"
rm -rf "$DEST"
mkdir -p /tmp/opus-extract-$$
unzip -q "$TMP_ZIP" -d /tmp/opus-extract-$$
mv /tmp/opus-extract-$$/opus-"$OPUS_VER" "$DEST"
rm -rf /tmp/opus-extract-$$ "$TMP_ZIP"
test -f "$DEST/CMakeLists.txt"
echo "opus-src $OPUS_VER ready"
