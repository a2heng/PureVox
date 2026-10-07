# PureVox - build version/stamp resolver, PowerShell twin of version.sh.
# Copyright (C) 2024-2026 a2heng <752848283@qq.com>
# SPDX-License-Identifier: GPL-3.0-or-later
# Single implementation path for all .ps1 version logic (build_win.ps1,
# build_lite_*.ps1, ci.yml / lite.yml timestamp steps).
# Pure ASCII: Windows PowerShell 5.1 safe (no non-ASCII, no BOM).
# Usage: . tools/automation/version.ps1 [-Lite]; then use $VERSION (dotted) / $STAMP.
param([switch]$Lite)
$ref = $env:GITHUB_REF_NAME
if ($Lite) {
    if ($ref -like 'lite-v*') { $ref = $ref.Substring(6) }
    elseif ($ref -like 'lite*') { $ref = $ref.Substring(4) -replace '^[-_]*', '' }
    elseif ($ref -like 'v*') { $ref = $ref.Substring(1) }
    else { $ref = '' }
} else {
    if ($ref -like 'v*') { $ref = $ref.Substring(1) } else { $ref = '' }
}
if ($ref) { $VERSION = $ref } else { $VERSION = (Get-Date).ToUniversalTime().ToString('yyyy.MM.dd.HHmm') }
$STAMP = $VERSION.Replace('.', '-')
