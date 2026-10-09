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
#   tag v<yyyy.MM.dd.HHmm> 或当前 UTC → VERSION = `yy.M.<日×1440+时分>`（semver 三段）。
#   三段都有硬上限（MSI ProductVersion：major≤255、minor≤255、build≤65535），日期不能直译：
#     major = 年 - 2000 → 26；minor = 月 → 1..12；build = 日×1440 + 时分 → 1..47059，
#     按时间全序且每分钟唯一。tag 本身仍是完整时间戳 v2026.10.08.1430。
#   同时写 `src-tauri/.build-version.json`（Tauri `--config`，不提交）。
# 用法：CI `./tools/automation/version.ps1 | Out-File -FilePath $env:GITHUB_ENV -Append -Encoding utf8`
$ErrorActionPreference = 'Stop'
$root = Split-Path -Parent (Split-Path -Parent $PSScriptRoot)
$ref = $env:GITHUB_REF_NAME
if ($ref -and $ref.StartsWith('v')) {
  $p = $ref.Substring(1).Split('.')   # v2026.10.08.1430 → y=2026 mo=10 d=08 hm=1430
  $y = $p[0]; $mo = $p[1]; $d = $p[2]; $hm = $p[3]
} else {
  $t = (Get-Date).ToUniversalTime()
  $y = $t.ToString('yyyy'); $mo = $t.ToString('MM'); $d = $t.ToString('dd'); $hm = $t.ToString('HHmm')
}
# 三段映射（[int] 去掉前导零：'09'→9、'0629'→629）：
#   major = 年 - 2000、minor = 月、build = 日×1440 + 时分（上限 47059 < 65535）
$ver = "$([int]$y % 100).$([int]$mo).$([int]$d * 1440 + [int]$hm)"
$stamp = $ver -replace '\.', '-'

$cfg = Join-Path $root 'src-tauri/.build-version.json'
$json = '{"version":"' + $ver + '"}' + "`n"
[IO.File]::WriteAllText($cfg, $json, (New-Object Text.UTF8Encoding $false))

"VERSION=$ver"
"STAMP=$stamp"
"PUREVOX_BUILD_VERSION=$ver"
