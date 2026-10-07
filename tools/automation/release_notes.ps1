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

# 生成 release 描述（对应旧版 tools/automation/release_notes.sh）：
# 取「上一个 v* tag → 本 tag」的提交标题列表；首个 tag 则取全部历史。
# 注意 AGENTS.md §2.6：CI 失败、从未生成 release 的 tag 必须删除，
# 否则会成为下个 tag 的「上一个 tag」，截断 release 提交记录。
param(
  [Parameter(Mandatory)] [string]$Tag,
  [Parameter(Mandatory)] [string]$OutFile
)
$ErrorActionPreference = 'Stop'
Push-Location (Split-Path -Parent (Split-Path -Parent $PSScriptRoot))
try {
  $prev = git describe --tags --abbrev=0 --match 'v*' "$Tag^" 2>$null
  if ($LASTEXITCODE -ne 0 -or -not $prev) { $prev = $null }
  $range = if ($prev) { "$prev..$Tag" } else { $Tag }
  $subjects = @(git log $range --pretty=format:'- %s')
  if ($LASTEXITCODE -ne 0) { throw "git log 失败（range=$range）" }
  $body = @()
  $body += "# PureVox $Tag"
  $body += ''
  if ($prev) { $body += "自 $prev 以来的变更：" } else { $body += '首个 tag：' }
  $body += ''
  $body += $subjects
  $body += ''
  $body += '## 产物（Windows x64）'
  $body += '- `PureVox_x64-setup.exe`（NSIS，静默参数 `/S`）'
  $body += '- `PureVox_x64_en-US.msi`（MSI）'
  $body += '- 两者均内置现役 ONNX 模型，模型授权见 `MODEL-LICENSE.md`'
  Set-Content -Path $OutFile -Value $body -Encoding utf8
  Write-Host "release notes -> $OutFile（$($subjects.Count) 条提交）"
} finally { Pop-Location }
