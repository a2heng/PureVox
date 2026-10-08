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
# Linux 发布产物断言（对应 Windows 的 assert_bundle.ps1）：
#   1. deb 存在且体积合理（内置模型 ~68MB，缺模型只有 ~10MB）
#   2. 解包 deb，断言可执行文件与全部模型落位（验证真实安装布局）
#   3. rpm / AppImage 存在性（AppImage 为 best-effort，仅告警）
#   4. --smoke：解包后的可执行文件在 xvfb 下启动，轮询 /debug 就绪后杀死（自收尾）
# 用法：bash tools/automation/assert_bundle_linux.sh [--smoke]
set -euo pipefail

ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/../.." && pwd)"
BUNDLE="$ROOT/src-tauri/target/release/bundle"
SMOKE=0
[ "${1:-}" = "--smoke" ] && SMOKE=1
MIN_MB=40

say() { printf '%s\n' "$*"; }

# ---- 1) deb 存在 + 体积 ----
deb="$(find "$BUNDLE/deb" -name '*.deb' -type f 2>/dev/null | head -1)"
if [ -z "$deb" ]; then
  say "找不到 deb 产物（$BUNDLE/deb）"
  exit 1
fi
mb=$(( $(stat -c%s "$deb") / 1024 / 1024 ))
if [ "$mb" -lt "$MIN_MB" ]; then
  say "deb 过小：$(basename "$deb") 仅 ${mb}MB（下限 ${MIN_MB}MB，疑似模型未随包）"
  exit 1
fi
say "产物 $(basename "$deb")  ${mb} MB"

# ---- 2) 解包 + 布局断言 ----
tmp="$(mktemp -d)"
cleanup() { rm -rf "$tmp"; }
trap cleanup EXIT
dpkg-deb -x "$deb" "$tmp"

bin="$(find "$tmp" -type f -name purevox -path '*/bin/*' | head -1)"
if [ -z "$bin" ]; then
  say "deb 内缺少可执行文件 purevox"
  exit 1
fi

expected="$(find "$ROOT/models" -name '*.onnx' -type f | wc -l)"
found="$(find "$tmp" -name '*.onnx' -type f | wc -l)"
if [ "$found" -lt "$expected" ]; then
  say "模型不全：$found/$expected（解包目录 $tmp）"
  exit 1
fi
say "布局 OK：$(basename "$bin") + 模型 $found/$expected"

# ---- 3) rpm / AppImage 存在性 ----
if ! find "$BUNDLE/rpm" -name '*.rpm' -type f | grep -q .; then
  say "缺少 rpm 产物（$BUNDLE/rpm）"
  exit 1
fi
if find "$BUNDLE/appimage" -name '*.AppImage' -type f | grep -q .; then
  say "AppImage OK"
else
  say "警告：无 AppImage（best-effort，不判失败）"
fi

# ---- 4) 启动冒烟（可选）：/debug 就绪即通过，结束时必杀进程 ----
if [ "$SMOKE" -eq 1 ]; then
  runner=()
  if command -v xvfb-run >/dev/null 2>&1; then
    runner=(xvfb-run -a)
  fi
  # Tauri 资源目录（模型）随 deb 落在 usr/lib/<productName>；开发态 model_path 会自己找。
  modeldir="$(dirname "$(find "$tmp" -name '*.onnx' -type f | head -1)")"
  "${runner[@]}" "$bin" >/tmp/opencode/purevox-smoke.log 2>&1 &
  pid=$!
  ok=0
  for _ in $(seq 1 60); do
    sleep 0.5
    if curl -fsS -m 2 http://127.0.0.1:47821/debug >/dev/null 2>&1; then
      ok=1
      break
    fi
  done
  kill "$pid" 2>/dev/null || true
  wait "$pid" 2>/dev/null || true
  if [ "$ok" -ne 1 ]; then
    say "冒烟失败：启动后 /debug 60s 内未就绪（模型目录 $modeldir）"
    exit 1
  fi
  say "冒烟 OK：/debug 就绪"
fi

say "产物断言全部通过"
