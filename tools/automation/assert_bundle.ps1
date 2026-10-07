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

# 发布产物断言（对应旧版「Assert packaged bundle layout」+「bundled smoke」）：
#   1. MSI / NSIS 产物存在且体积合理（内置模型 ~68MB，缺模型只有 ~10MB）
#   2. NSIS 静默安装到临时目录，断言 exe + 模型落位（验证真实安装布局）
#   3. -Smoke：启动安装产物，轮询 /debug 就绪后杀死（不留进程，自收尾）
# 用法：pwsh tools/automation/assert_bundle.ps1 [-Smoke]
param([switch]$Smoke)
$ErrorActionPreference = 'Stop'
$root = Split-Path -Parent (Split-Path -Parent $PSScriptRoot)
$bundle = Join-Path $root 'src-tauri\target\release\bundle'
$check = Join-Path ([IO.Path]::GetTempPath()) 'purevox_bundle_check'
$minMB = 40   # 产物体积下限（MB）：无模型 ~10MB，带模型 >60MB

# ---- 1) 产物存在 + 体积 ----
$nsis = Get-ChildItem (Join-Path $bundle 'nsis') -Filter '*.exe' -File -ErrorAction Stop |
  Where-Object { $_.Name -notmatch '\.log$' } | Select-Object -First 1
$msi = Get-ChildItem (Join-Path $bundle 'msi') -Filter '*.msi' -File -ErrorAction Stop | Select-Object -First 1
foreach ($a in @($nsis, $msi)) {
  $mb = [math]::Round($a.Length / 1MB, 1)
  if ($a.Length -lt ($minMB * 1MB)) {
    throw "产物过小：$($a.Name) 仅 $mb MB（下限 ${minMB}MB，疑似模型未随包）"
  }
  Write-Host ("产物 {0}  {1} MB" -f $a.Name, $mb)
}

# ---- 2) NSIS 静默安装 + 布局断言 ----
if (Test-Path $check) { Remove-Item $check -Recurse -Force }
Write-Host "静默安装到 $check"
Start-Process -FilePath $nsis.FullName -ArgumentList @('/S', "/D=$check") -Wait
$exe = Join-Path $check 'PureVox.exe'
if (-not (Test-Path $exe)) { throw "安装后缺少 PureVox.exe（$check）" }
# 资源落位可能是 <安装目录>/models 或 <安装目录>/resources/models（model_path 两者都认）
$candidates = @((Join-Path $check 'models'), (Join-Path $check 'resources\models'))
$found = @{}
foreach ($d in $candidates) {
  foreach ($f in (Get-ChildItem $d -Filter '*.onnx' -File -ErrorAction SilentlyContinue)) { $found[$f.Name] = $true }
}
$expected = @(Get-ChildItem (Join-Path $root 'models') -Filter '*.onnx' -File).Count
if ($found.Count -lt $expected) {
  throw "安装包模型不全：$($found.Count)/$expected（落位检查：$($candidates -join '；')）"
}
Write-Host "布局 OK：PureVox.exe + 模型 $($found.Count)/$expected"

# ---- 3) 启动冒烟（可选）：/debug 就绪即通过，结束时必杀进程 ----
if ($Smoke) {
  $proc = $null
  try {
    $proc = Start-Process -FilePath $exe -PassThru
    $ready = $false
    for ($i = 0; $i -lt 60 -and -not $ready; $i++) {
      Start-Sleep -Milliseconds 500
      try {
        $null = Invoke-RestMethod 'http://127.0.0.1:47821/debug' -TimeoutSec 2
        $ready = $true
      } catch { }
    }
    if (-not $ready) { throw '安装产物启动后 /debug 60s 内未就绪' }
    Write-Host '冒烟 OK：/debug 就绪'
  } finally {
    if ($proc) { Stop-Process -Id $proc.Id -Force -ErrorAction SilentlyContinue }
    Get-Process purevox -ErrorAction SilentlyContinue | Stop-Process -Force -ErrorAction SilentlyContinue
    Start-Sleep -Milliseconds 500
    Remove-Item $check -Recurse -Force -ErrorAction SilentlyContinue
  }
} else {
  Remove-Item $check -Recurse -Force -ErrorAction SilentlyContinue
}
Write-Host '产物断言全部通过' -ForegroundColor Green
