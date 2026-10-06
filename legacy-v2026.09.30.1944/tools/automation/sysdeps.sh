#!/bin/bash
# PureVox - CI system-dependency installer (single implementation path).
# Copyright (C) 2024-2026 a2heng <752848283@qq.com>
# SPDX-License-Identifier: GPL-3.0-or-later
#
# Usage: SYSDEPS="<pkgs>" bash tools/automation/sysdeps.sh
# Installs before checkout (containers have no git). apt and dnf both go
# through tools/automation/retry.sh: a single mirror hiccup (e.g. Fedora's
# openh264 repo) must not fail the whole job.
set -euo pipefail
: "${SYSDEPS:?SYSDEPS is empty}"

if command -v apt-get >/dev/null; then
    bash tools/automation/retry.sh 3 apt-get update -qq
    DEBIAN_FRONTEND=noninteractive bash tools/automation/retry.sh 3 \
        apt-get install -y -qq $SYSDEPS
else
    bash tools/automation/retry.sh 3 dnf install -y --quiet \
        --setopt=retries=5 --setopt=timeout=60 $SYSDEPS
fi
