#!/bin/bash
# PureVox - pinned pip install with platform fallbacks (single implementation path).
# Copyright (C) 2024-2026 a2heng <752848283@qq.com>
# SPDX-License-Identifier: GPL-3.0-or-later
#
# Usage: bash tools/automation/pip_install.sh <python> <requirements> [extra-pkg...]
# Order: plain install -> PEP668 --break-system-packages -> --user.
# No "|| true": a failed dependency install must fail the step, otherwise the
# smoke test fails later with a misleading error.
set -euo pipefail
PY="$1"; REQ="$2"; shift 2
"$PY" -m pip install -q -r "$REQ" "$@" \
    || "$PY" -m pip install -q --break-system-packages -r "$REQ" "$@" \
    || "$PY" -m pip install -q --user -r "$REQ" "$@"
