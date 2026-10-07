#!/bin/bash
# PureVox - packaged-artifact smoke test (single implementation path).
# Copyright (C) 2024-2026 a2heng <752848283@qq.com>
# SPDX-License-Identifier: GPL-3.0-or-later
#
# Usage: bash tools/automation/bundled_smoke.sh <deb|rpm|appimage> <artifact-file>
# Unpacks the artifact, asserts the embedded python312 + models are present,
# then runs the engine smoke and the full tests/run_all.py with the PACKAGED
# interpreter (catches missing deps / broken embedded runtimes, not just
# source-tree issues). Replaces the three near-identical blocks in ci.yml.
set -euo pipefail
KIND="$1"; FILE="$2"
FILE="$(readlink -f "$FILE")"
SMOKE_SRC="$(pwd)/tools/automation/smoke.py"

case "$KIND" in
    deb)
        root="$(mktemp -d)"
        dpkg-deb -x "$FILE" "$root"
        PV="$root/opt/purevox"
        PY="$PV/python312/bin/python3.12"
        ;;
    rpm)
        root="$(mktemp -d)"
        rpm2cpio "$FILE" | cpio -idmD "$root"
        PV="$root/opt/purevox"
        PY="$PV/python312/bin/python3.12"
        ;;
    appimage)
        root="$(mktemp -d)"
        (cd "$root" && "$FILE" --appimage-extract >/dev/null)
        PV="$root/squashfs-root/usr/lib/purevox"
        PY="$root/squashfs-root/usr/python312/bin/python3"
        ;;
    *)
        echo "usage: bundled_smoke.sh <deb|rpm|appimage> <file>" >&2
        exit 2
        ;;
esac

test -x "$PY"
test "$(ls "$PV/models" | grep -c '\.onnx$')" -ge 4
cp -r tests "$PV/tests"
cp "$SMOKE_SRC" "$PV/smoke_check.py"
cd "$PV"
if [ "$KIND" = "appimage" ]; then
    export PYTHONHOME="$root/squashfs-root/usr/python312"
    export LD_LIBRARY_PATH="$PV${LD_LIBRARY_PATH:+:$LD_LIBRARY_PATH}"
else
    export PYTHONHOME="$PV/python312"
fi
"$PY" smoke_check.py
"$PY" tests/run_all.py
