# PureVox - bundled Python 3.12 bootstrap script (Windows)
# Downloads the full prebuilt Python 3.12 from NuGet (includes dev headers/libs for
# PyInstaller bundling) into packages\python312w\, independent of any system Python.
# Idempotent: skips download if already present; only installs pip deps.
#
# Usage: powershell -ExecutionPolicy Bypass -File bootstrap_python312.ps1
$ErrorActionPreference = "Stop"
[Net.ServicePointManager]::SecurityProtocol = [Net.SecurityProtocolType]::Tls12

$root = Split-Path -Parent $MyInvocation.MyCommand.Path
$dest = Join-Path $root "packages"
$pyDir = Join-Path $dest "python312w"
$srcPy = Join-Path $pyDir "tools\python.exe"
$py = Join-Path $pyDir "python.exe"
$pyVer = "3.12.11"   # Python 3.12 NuGet package

# 1. Download and unpack full Python 3.12 (NuGet, no admin needed; the real root is
#    tools\, flattened to pyDir\ after unpacking)
if (-not (Test-Path $py)) {
    Write-Host "==> Downloading Python $pyVer (NuGet)..."
    $nupkg = Join-Path $dest "python.$pyVer.nupkg.tmp"
    Invoke-WebRequest -Uri "https://api.nuget.org/v3-flatcontainer/python/$pyVer/python.$pyVer.nupkg" -OutFile $nupkg
    $tmp = Join-Path $dest "python-$pyVer.tmp"
    if (Test-Path $tmp) { Remove-Item $tmp -Recurse -Force }
    Expand-Archive -Path $nupkg -DestinationPath $tmp -Force
    Remove-Item $nupkg
    # Flatten: move tmp\tools\* up to pyDir\*
    if (Test-Path $pyDir) { Remove-Item $pyDir -Recurse -Force }
    New-Item -ItemType Directory -Force -Path $pyDir | Out-Null
    Move-Item -Path (Join-Path $tmp "tools\*") -Destination $pyDir -Force
    Remove-Item $tmp -Recurse -Force
}

# 2. Bootstrap pip (the NuGet package bundles pip/ensurepip)
& $py -m ensurepip --upgrade
if ($LASTEXITCODE -ne 0) { throw "ensurepip failed" }
& $py -m pip install --upgrade pip setuptools wheel
& $py -m pip install -r (Join-Path $root "requirements-win.txt")

Write-Host "PureVox bundled Python 3.12 ready: $py"
Write-Host "Use build_win.ps1 to package (it will pick up packages\python312w\python.exe)"
