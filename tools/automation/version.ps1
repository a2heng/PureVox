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
# 构建版本解析（Windows / pwsh 用；与 version.sh 同一契约，见其文件头）：
#   tag v<yyyy.MM.dd.HHmm> → VERSION = `yyyy.MMdd.HHmm`（semver，Tauri 要求）；
#   否则当前 UTC 日期时间。同时写 `src-tauri/.build-version.json`（Tauri `--config`，不提交）。
# 用法：CI `./tools/automation/version.ps1 | Out-File -FilePath $env:GITHUB_ENV -Append -Encoding utf8`
$ErrorActionPreference = 'Stop'
$root = Split-Path -Parent (Split-Path -Parent $PSScriptRoot)
$ref = $env:GITHUB_REF_NAME
if ($ref -and $ref.StartsWith('v')) {
  # v2026.10.08.1430 → 2026.1008.1430
  $p = $ref.Substring(1).Split('.')
  $ver = "$($p[0]).$($p[1])$($p[2]).$($p[3])"
} else {
  $ver = (Get-Date).ToUniversalTime().ToString('yyyy.MMdd.HHmm')
}
$stamp = $ver -replace '\.', '-'

$cfg = Join-Path $root 'src-tauri/.build-version.json'
$json = '{"version":"' + $ver + '"}' + "`n"
[IO.File]::WriteAllText($cfg, $json, (New-Object Text.UTF8Encoding $false))

"VERSION=$ver"
"STAMP=$stamp"
"PUREVOX_BUILD_VERSION=$ver"
