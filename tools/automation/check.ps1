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

# 代码质量门禁的唯一实现路径：CI 各 job 与本机开发跑同一份脚本。
#   pwsh tools/automation/check.ps1                 # 全部门禁
#   pwsh tools/automation/check.ps1 -Gate fmt       # 单项（CI 按 step 拆开跑，日志好定位）
# 门禁项：fmt（rustfmt 校验）/ clippy（-D warnings 零容忍）/ build / test / ui（tsc + i18n_lint）。
param(
  [ValidateSet('fmt', 'clippy', 'build', 'test', 'ui', 'all')]
  [string]$Gate = 'all'
)
$ErrorActionPreference = 'Stop'
$root = Split-Path -Parent (Split-Path -Parent $PSScriptRoot)
$tauri = Join-Path $root 'src-tauri'
# 跨平台：Windows 与 Linux/macOS 的 cargo 都在 $HOME/.cargo/bin（PowerShell 的 $HOME
# 在 Windows 上等于 USERPROFILE）。用 PathSeparator 拼，不能写死 ';'。
$env:Path = "$(Join-Path $HOME '.cargo/bin')$([IO.Path]::PathSeparator)$env:Path"

function Invoke-Gate([string]$name, [scriptblock]$body) {
  Write-Host "==> $name" -ForegroundColor Cyan
  & $body
  if ($LASTEXITCODE -ne 0 -and $null -ne $LASTEXITCODE) {
    throw "门禁 $name 未通过（exit=$LASTEXITCODE）"
  }
}

$gates = switch ($Gate) {
  'all' { @('fmt', 'clippy', 'build', 'test', 'ui') }
  default { @($Gate) }
}

foreach ($g in $gates) {
  switch ($g) {
    'fmt' {
      Invoke-Gate 'cargo fmt --check' {
        Push-Location $tauri
        try { cargo fmt --check } finally { Pop-Location }
      }
    }
    'clippy' {
      Invoke-Gate 'cargo clippy -D warnings' {
        Push-Location $tauri
        try { cargo clippy --all-targets -- -D warnings } finally { Pop-Location }
      }
    }
    'build' {
      Invoke-Gate 'cargo build' {
        Push-Location $tauri
        try { cargo build } finally { Pop-Location }
      }
    }
    'test' {
      Invoke-Gate 'cargo test' {
        Push-Location $tauri
        try { cargo test } finally { Pop-Location }
      }
    }
    'ui' {
      Invoke-Gate 'tsc (ui/jsconfig.json)' {
        Push-Location (Join-Path $tauri 'ui')
        try { tsc -p jsconfig.json --noEmit } finally { Pop-Location }
      }
      Invoke-Gate 'i18n_lint (ui 字符串门禁)' {
        node (Join-Path $root 'tools/automation/i18n_lint.js')
      }
    }
  }
}
Write-Host "门禁全部通过（$Gate）" -ForegroundColor Green
