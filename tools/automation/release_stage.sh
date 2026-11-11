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
# release.yml 的「创建 release」这一步的**唯一实现**。
#
# ## 为什么是独立文件，而不是写在 workflow 的 run: 里
#
# 这段脚本此前**从来没有成功执行过** —— release job 只在打 tag 时才跑，而第一次真打 tag
# 就暴露出：收集完文件名后 `cd ..` 回了上一层，于是那些相对路径全部失效，`gh` 报
# `no matches found for './PureVox-…apk'`。教训是「内联的脚本等于没被测过的脚本」。
#
# 落到文件里之后，`tools/automation/run_release_stage.py` 能用假产物树与假 `gh` 在本地
# 把它跑通 —— **测的和发布的是同一份代码**，不会因为 YAML 缩进变化而各跑各的。
#
# ## 用法
#
#   bash tools/automation/release_stage.sh
#
# 需要环境变量（release.yml 的对应步骤负责设）：
#   TAG / VERSION / REPO / WORKSPACE（= $GITHUB_WORKSPACE）
#   ARTIFACTS（下载产物目录，默认 ./artifacts）
# 输入：$ARTIFACTS/<artifact 名>/<原始文件名>（download-artifact 的目录结构）
# 输出：$ARTIFACTS/out/PureVox-<版本>-<平台>.<扩展名>，然后交给 gh release create。

set -euo pipefail

ARTIFACTS="${ARTIFACTS:-artifacts}"
REPO="${REPO:-a2heng/PureVox}"
TAG="${TAG:?TAG 必须由调用方设置（github.ref_name）}"
VERSION="${VERSION:?VERSION 必须由调用方设置（version.py 推导）}"
PLATFORM="${PLATFORM:-desktop}"
NOTES="${NOTES:-notes.md}"

# 下面要 cd 两层进 out/，先把「仓库根」记成绝对路径 —— notes 文件在那里，
# 事后再用相对路径拼是拼不出来的（少算一层就变成 artifacts/notes.md）。
WORKSPACE_ABS="$(cd "$(dirname "${BASH_SOURCE[0]}")/../.." && pwd)"
case "$NOTES" in
  /*) notes="$NOTES" ;;
  *)  notes="$WORKSPACE_ABS/$NOTES" ;;
esac

# ── 统一重命名成「PureVox-<版本>-<平台>.<扩展名>」 ──────────────────────────
# 更新检查（桌面 src/update/ 与安卓 App）按**文件名后缀**认领本平台产物，不看版本号
# （版本号只有年月日，同日会重复），故文件名必须稳定。
# 命名规则另有 update/mod.rs 的 `ci_asset_names_match_the_tag_version` 断言
# （它 include_str! 读 release.yml，改了会红），release_notes.py 的产物清单逐字跟着这里走。
shopt -s nullglob
cd "$ARTIFACTS"
mkdir -p out

missing=""
stage() { # <稳定名> <匹配通配…>；缺产物时记下来并返回 1
  local out="$1"; shift
  local f
  for f in "$@"; do
    [ -e "$f" ] || continue
    cp -f "$f" "$out" && return 0
  done
  missing="$missing $out"
  return 1
}
if [ "$PLATFORM" = "android" ]; then
  # 安卓通道**只**发 APK（`RELEASE-CHANNELS.md`：与桌面 release 完全分开）
  stage "out/PureVox-${VERSION}-android-arm64.apk" */*.apk || true
  want=1
  title="PureVox Android $VERSION"
elif [ "$PLATFORM" = "portable" ]; then
  # 桌面**便携测试通道**（安装包流水线恢复前的唯一桌面产物）
  stage "out/PureVox-${VERSION}-windows-x64-portable.zip" */*portable.zip || true
  want=1
  title="PureVox $VERSION（便携测试版）"
else
  stage "out/PureVox-${VERSION}-windows-x64-setup.exe" */*-setup.exe || true
  stage "out/PureVox-${VERSION}-linux-x64.AppImage"    */*.AppImage  || true
  stage "out/PureVox-${VERSION}-linux-x64.deb"         */*.deb       || true
  stage "out/PureVox-${VERSION}-linux-x64.rpm"         */*.rpm       || true
  want=4
  title="PureVox $VERSION"
fi

# **缺任何一项都让 job 失败**（原来只 echo 一行警告就继续）。tag 已经推出去了，这时候发一个
# 缺安装包的 release 比不发更糟：用户点进去才发现，事后又得删 tag 重来一遍
# （AGENTS §2.6：CI 失败的 tag 必须删，否则会截断下个 release 的提交记录）。
if [ -n "$missing" ]; then
  echo "::error::缺少产物：$missing"
  exit 1
fi

# ── 挂上去 ────────────────────────────────────────────────────────────────
cd out
files=$(find . -type f | sort | tr '\n' ' ')
echo "release assets: $files"

# **绝不要 `cd ..` 回到 artifacts/**：$files 是相对 out/ 的 `./xxx`，回上一层后那些路径
# 全部失效，gh 自己的 glob 找不到就报 `no matches found for ...`。`--repo` 与
# `--notes-file` 给的都是绝对值，本来就不需要换目录。
#
# 挂出去之前逐个断言文件真的在：gh 报「找不到文件」时说的是 `no matches found`，
# 哪个附件缺了要人自己反推，这里提前说清楚。
n=0
for f in $files; do
  if [ ! -f "$f" ]; then
    echo "::error::附件不存在：$f（CWD=$PWD，files=$files）"
    exit 1
  fi
  n=$((n + 1))
done
if [ "$n" -ne "$want" ]; then
  echo "::error::附件数应为 $want，实际 $n（files=$files）"
  exit 1
fi

# `notes` 已经在顶部算成绝对路径了 —— **不要再用 `notes="$NOTES"` 把它覆盖回相对路径**：
# 此刻 CWD 已经是 artifacts/out/，相对路径必然指不到文件。
# （这个 bug 就是被 tools/automation/run_release_stage.py 的本地台子当场抓出来的。）
[ -f "$notes" ] || { echo "::error::找不到 notes 文件：$notes"; exit 1; }

gh release create "$TAG" \
  --repo "$REPO" \
  --title "$title" \
  --notes-file "$notes" \
  $files