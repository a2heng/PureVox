# PureVox - Local CI mirror (full pipeline on this machine)
# Mirrors every .github/workflows/ci.yml job so the whole CI can be verified
# locally without pushing a tag:
#   - windows job : runs natively in PowerShell (PyInstaller bundle)
#   - linux job   : runs inside WSL (Ubuntu-24.04) via verify-local.sh
#                   (smoke + deb/appimage); repo visible at /mnt/d/...,
#                   artifacts copied back into the repo dist/.
#   - android job : runs natively with the Windows SDK/NDK (gradlew.bat)
# Usage:
#   powershell -ExecutionPolicy Bypass -File verify-local.ps1              # all stages
#   powershell -ExecutionPolicy Bypass -File verify-local.ps1 windows
#   powershell -ExecutionPolicy Bypass -File verify-local.ps1 linux        # WSL required
#   powershell -ExecutionPolicy Bypass -File verify-local.ps1 android
# Linux stage requirements (one-time setup):
#           wsl -d Ubuntu-24.04 bash ./verify-local.sh sysdeps
param(
    [Parameter(Position = 0)]
    [ValidateSet("all", "linux", "windows", "android")]
    [string]$Stage = "all"
)

$ErrorActionPreference = "Stop"
Set-Location -LiteralPath $PSScriptRoot

function Invoke-Windows {
    Write-Host "==> [windows] install Python deps"
    python -m pip install -q -r requirements-win.txt
    if ($LASTEXITCODE -ne 0) { throw "pip install failed" }

    Write-Host "==> [windows] compileall (syntax check)"
    python -m compileall -q pvengine audio_processor.py config_manager.py model_config.py about_content.py run_tk.py uitk pvplatform server 2>$null
    if ($LASTEXITCODE -ne 0) { throw "compileall failed" }
    Write-Host "compileall OK"

    Write-Host "==> [windows] PyInstaller bundle"
    pwsh -NoProfile -ExecutionPolicy Bypass -File build_win.ps1
    if ($LASTEXITCODE -ne 0) { throw "build_win.ps1 failed" }

    Write-Host "==> [windows] engine smoke test (tools/automation/smoke.py)"
    python tools/automation/smoke.py
    if ($LASTEXITCODE -ne 0) { throw "engine smoke failed" }

    Write-Host "==> [windows] done: dist/PureVox/"
}

$WslDistro = "Ubuntu-24.04"
$SshArgs = @("-p", "2222", "-o", "BatchMode=yes", "-o", "ConnectTimeout=4", "dev@127.0.0.1")
function Invoke-Linux {
    # Full Linux pipeline inside the local WSL distro (same Ubuntu family as the
    # CI ubuntu container). verify-local.sh auto-relocates the build off /mnt/* (9p)
    # into a native ext4 dir because drvfs cannot store symlinks.
    Write-Host "==> [linux] run verify-local.sh all in WSL ($WslDistro)"
    wsl.exe -d $WslDistro -e bash -lc "cd /mnt/d/code/projects/purevox && bash ./verify-local.sh all"
    if ($LASTEXITCODE -ne 0) { throw "WSL verify-local.sh failed" }
    Write-Host "==> [linux] done: dist/ artifacts copied back into the repo"
}

function Invoke-Android {
    Write-Host "==> [android] check SDK env"
    if (-not $env:ANDROID_HOME) {
        $env:ANDROID_HOME = "$env:LOCALAPPDATA\Android\Sdk"
    }
    if (-not (Test-Path "$env:ANDROID_HOME\cmdline-tools")) {
        Write-Warning "ANDROID_HOME not found at $env:ANDROID_HOME - set it manually before running."
    }
    $env:ANDROID_SDK_ROOT = $env:ANDROID_HOME

    Write-Host "==> [android] ensure opus-src"
    if (-not (Test-Path "android\opus-src\CMakeLists.txt")) {
        New-Item -ItemType Directory -Force -Path "android\opus-src" | Out-Null
        Invoke-WebRequest -Uri "https://github.com/xiph/opus/archive/refs/tags/v1.5.2.zip" -OutFile "$env:TEMP\opus.zip"
        Expand-Archive -Path "$env:TEMP\opus.zip" -DestinationPath $env:TEMP -Force
        Copy-Item -Recurse -Force "$env:TEMP\opus-1.5.2\*" "android\opus-src\"
    }

    Write-Host "==> [android] gradlew assembleDebug"
    Push-Location android
    try {
        .\gradlew.bat assembleDebug --no-daemon
        if ($LASTEXITCODE -ne 0) { throw "gradlew failed" }
    } finally {
        Pop-Location
    }
    $stamp = Get-Date -Format 'yyyy-MM-dd-HHmm'
    Move-Item "android\app\build\outputs\apk\debug\app-debug.apk" "PureVox-Android-arm64-$stamp-debug.apk" -Force
    Write-Host "==> [android] done: PureVox-Android-arm64-$stamp-debug.apk"
}

switch ($Stage) {
    "linux"   { Invoke-Linux }
    "windows" { Invoke-Windows }
    "android" { Invoke-Android }
    "all"     {
        Invoke-Linux
        Invoke-Windows
        Invoke-Android
    }
}
Write-Host "==> verify-local[$Stage] ALL DONE"
