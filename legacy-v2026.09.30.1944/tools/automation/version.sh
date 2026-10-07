#!/bin/bash
# PureVox - build version/stamp resolver (single implementation path).
# Copyright (C) 2024-2026 a2heng <752848283@qq.com>
# SPDX-License-Identifier: GPL-3.0-or-later
#
# Usage: eval "$(bash tools/automation/version.sh [--lite])"
#   tag run  v2026.09.03.1423      -> VERSION=2026.09.03.1423 STAMP=2026-09-03-1423
#   tag run  lite-v2026.09.03.1703 -> (with --lite) same stripping of the lite-v prefix
#   local/manual run               -> current UTC time in the same shapes
#
# VERSION = dotted build version (deb control / rpm spec / _build_version source)
# STAMP   = filename timestamp (yyyy-MM-dd-HHmm), derived from VERSION on tag
#           runs so concurrent jobs cannot drift apart.
set -euo pipefail

LITE=0
[ "${1:-}" = "--lite" ] && LITE=1

ref="${GITHUB_REF_NAME:-}"
if [ "$LITE" = "1" ]; then
    case "$ref" in
        lite-v*) ref="${ref#lite-v}" ;;
        lite*)   ref="${ref#lite}"; ref="${ref#-}"; ref="${ref#_}" ;;
        v*)      ref="${ref#v}" ;;
    esac
else
    case "$ref" in
        v*) ref="${ref#v}" ;;
        *)  ref="" ;;
    esac
fi

if [ -n "$ref" ]; then
    VERSION="$ref"
else
    VERSION="$(date -u +%Y.%m.%d.%H%M)"
fi
STAMP="${VERSION//./-}"

printf 'VERSION=%s\nSTAMP=%s\n' "$VERSION" "$STAMP"
