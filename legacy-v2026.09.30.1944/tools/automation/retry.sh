#!/bin/bash
# PureVox - network retry helper (single implementation path).
# Copyright (C) 2024-2026 a2heng <752848283@qq.com>
# SPDX-License-Identifier: GPL-3.0-or-later
#
# Usage: bash tools/automation/retry.sh <attempts> <command> [args...]
# Sleeps 10s * attempt between tries. Used for every CI download step
# (dnf/apt, sdkmanager, wget/curl tarballs) so a single mirror hiccup
# no longer fails the whole job.
set -u

n="$1"; shift
i=1
while true; do
    if "$@"; then
        exit 0
    fi
    if [ "$i" -ge "$n" ]; then
        echo "retry.sh: giving up after $i attempts: $*" >&2
        exit 1
    fi
    echo "retry.sh: attempt $i/$n failed, sleeping $((i * 10))s: $*" >&2
    sleep $((i * 10))
    i=$((i + 1))
done
