#!/bin/bash
# PureVox - release-notes generator (single implementation path).
# Copyright (C) 2024-2026 a2heng <752848283@qq.com>
# SPDX-License-Identifier: GPL-3.0-or-later
#
# Usage: bash tools/automation/release_notes.sh <tag> <outfile> <title-line...>
# Writes "<title-line>", a blank line, "**提交记录**", then the commit log
# from the previous tag to <tag> (or the last 15 commits for the first tag).
# Used by release.yml (main) and release-lite.yml (Lite).
#
# Tag family is matched by prefix: a main release (v*) must not be shadowed by
# a Lite tag (lite-v*) that happens to sit on a closer commit, and vice versa.
# Without --match the nearest tag of EITHER family wins and the commit log of
# one release gets truncated to a single commit.
set -euo pipefail
TAG="$1"; OUT="$2"; shift 2
case "$TAG" in
    lite-*) TAG_MATCH='lite-v*' ;;
    *)      TAG_MATCH='v*' ;;
esac
prev="$(git describe --tags --abbrev=0 --match "$TAG_MATCH" "${TAG}^" 2>/dev/null || true)"
{
    echo "$*"
    echo ""
    echo "**提交记录**"
    if [ -n "$prev" ]; then
        git log --pretty="format:- %h %s" "$prev..$TAG"
    else
        git log --pretty="format:- %h %s" -15
    fi
    echo ""
} > "$OUT"
