#!/bin/bash
# PureVox - syntax gate (single implementation path).
# Copyright (C) 2024-2026 a2heng <752848283@qq.com>
# SPDX-License-Identifier: GPL-3.0-or-later
#
# Usage: bash tools/automation/compileall.sh <python>   (run from the repo root)
# Compiles every path in tools/automation/compileall_srcs.txt.
set -euo pipefail
PY="${1:-python3}"
# shellcheck disable=SC2207
SRCS=($(grep -v '^\s*#' tools/automation/compileall_srcs.txt | grep -v '^\s*$'))
"$PY" -m compileall -q "${SRCS[@]}"
echo "compileall OK"
