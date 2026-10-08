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
# 跨发行版「装得上、起得来」验证（环境模拟）：在**目标发行版容器**里真实安装我们的
# 安装包，断言依赖解析、文件落位、动态库链接、可执行文件存在。
#   - deb  → ubuntu:24.04（apt）
#   - rpm  → fedora:latest（dnf）
#   - AppImage → 本机 `--appimage-extract` 解包断言（无需容器）
# 容器运行时用 docker（GitHub runner 自带）；本机也可用 `DOCKER=podman` 覆盖。
# 用法：bash tools/automation/test_packages.sh
set -euo pipefail

ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/../.." && pwd)"
BUNDLE="$ROOT/src-tauri/target/release/bundle"
DOCKER="${DOCKER:-docker}"
EXPECTED="$(find "$ROOT/models" -name '*.onnx' -type f | wc -l)"

say() { printf '%s\n' "$*"; }

if ! command -v "$DOCKER" >/dev/null 2>&1; then
  say "找不到容器运行时 '$DOCKER'（用 DOCKER=podman 可覆盖）；跨发行版验证需要它"
  exit 1
fi

# 容器内断言脚本：可执行文件存在 + 模型齐全 + 动态库无缺失。
read -r -d '' CHECK <<'EOS' || true
set -euo pipefail
bin="$(command -v purevox || true)"
[ -n "$bin" ] || bin=/usr/bin/purevox
test -x "$bin" || { echo "缺少可执行文件 $bin"; exit 1; }
n=$(find /usr -name '*.onnx' -type f | wc -l)
[ "$n" -ge "$EXPECTED" ] || { echo "模型不全：$n/$EXPECTED"; exit 1; }
missing=$(ldd "$bin" 2>/dev/null | grep 'not found' || true)
[ -z "$missing" ] || { echo "动态库缺失："; echo "$missing"; exit 1; }
echo "OK: $bin, models=$n, ldd 无缺失"
EOS

# 容器共用宿主网络（CI runner 直连；本机配合代理可用）。设了代理环境变量就透传进去，
# 否则容器内 apt/dnf 可能连不上官方源。
RUN_ARGS=(--rm --network=host)
if [ -n "${https_proxy:-}${HTTPS_PROXY:-}${http_proxy:-}${HTTP_PROXY:-}" ]; then
  RUN_ARGS+=(-e http_proxy -e https_proxy -e HTTP_PROXY -e HTTPS_PROXY -e no_proxy -e NO_PROXY)
fi

# ---- deb → ubuntu:24.04 ----
deb="$(find "$BUNDLE/deb" -name '*.deb' -type f 2>/dev/null | head -1)"
if [ -n "$deb" ]; then
  say "== deb → ubuntu:24.04 =="
  "$DOCKER" run "${RUN_ARGS[@]}" -e EXPECTED="$EXPECTED" -e CHECK="$CHECK" \
    -v "$BUNDLE/deb:/pkg:ro" ubuntu:24.04 bash -c \
    'apt-get update -qq &&
     apt-get install -y -qq --no-install-recommends /pkg/*.deb >/dev/null &&
     bash -c "$CHECK"'
else
  say "跳过 deb（无产物）"
fi

# ---- rpm → fedora ----
rpm="$(find "$BUNDLE/rpm" -name '*.rpm' -type f 2>/dev/null | head -1)"
if [ -n "$rpm" ]; then
  say "== rpm → fedora:latest =="
  "$DOCKER" run "${RUN_ARGS[@]}" -e EXPECTED="$EXPECTED" -e CHECK="$CHECK" \
    -v "$BUNDLE/rpm:/pkg:ro" fedora:latest bash -c \
    'dnf install -y --setopt=install_weak_deps=False /pkg/*.rpm >/dev/null &&
     bash -c "$CHECK"'
else
  say "跳过 rpm（无产物）"
fi

# ---- AppImage → 本机解包断言（免 FUSE）----
appimg="$(find "$BUNDLE/appimage" -name '*.AppImage' -type f 2>/dev/null | head -1)"
if [ -n "$appimg" ]; then
  say "== AppImage → 本机解包 =="
  tmp="$(mktemp -d)"
  trap 'rm -rf "$tmp"' EXIT
  ( cd "$tmp" && "$appimg" --appimage-extract >/dev/null )
  test -x "$tmp/squashfs-root/AppRun" || { say "AppImage 缺少 AppRun"; exit 1; }
  n=$(find "$tmp/squashfs-root" -name '*.onnx' -type f | wc -l)
  [ "$n" -ge "$EXPECTED" ] || { say "AppImage 模型不全：$n/$EXPECTED"; exit 1; }
  say "OK: AppRun + models=$n"
else
  say "跳过 AppImage（无产物）"
fi

say "跨发行版验证全部通过"
