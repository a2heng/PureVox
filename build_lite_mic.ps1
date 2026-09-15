# PureVox Lite - local/CI build script (pure python, no gcc)
# Single source of truth for the working PyInstaller recipe.
# Usage: powershell -ExecutionPolicy Bypass -File build_lite_mic.ps1
$ErrorActionPreference = "Stop"

# --- Version stamp: single source tools/automation/version.ps1 -Lite
# (tag lite-v<yyyy.MM.dd.HHmm> -> ver; fallback to local UTC time) ---
. (Join-Path (Split-Path -Parent $MyInvocation.MyCommand.Path) "tools\automation\version.ps1") -Lite
$ver = $VERSION
Write-Host "Lite version: $ver"

# --- Python resolution: embedded python312w > python on PATH ---
$scriptDir = Split-Path -Parent $MyInvocation.MyCommand.Path
$embeddedPy = Join-Path $scriptDir "packages\python312w\python.exe"
if (Test-Path $embeddedPy) { $PY = @($embeddedPy) }
elseif (Get-Command python -ErrorAction SilentlyContinue) { $PY = @("python") }
elseif (Get-Command py -ErrorAction SilentlyContinue) { $PY = @("py", "-3") }
else { throw "no python found" }
& $PY --version

# --- Deps (requirements-win.txt: Windows full deps) ---
& $PY -m pip install -q -r requirements-win.txt
if ($LASTEXITCODE -ne 0) { throw "pip install failed" }

# --- Syntax check + smoke import ---
& $PY -m compileall -q lite_mic
if ($LASTEXITCODE -ne 0) { throw "compileall failed" }
& $PY -c "import sys; sys.path.insert(0, 'lite_mic'); import lite_mic.config, lite_mic.audio, lite_mic.engine, lite_mic.ui; print('import OK')"
if ($LASTEXITCODE -ne 0) { throw "smoke import failed" }

# --- Model file name from model_config (single source of truth, no hardcode) ---
$modelRel = (& $PY -c "import model_config; print(model_config.DENOISE_MODEL)").Trim()
if ($LASTEXITCODE -ne 0 -or -not $modelRel) { throw "model_config read failed" }
$modelWin = $modelRel.Replace('/', '\')
if (-not (Test-Path $modelWin)) { throw "model missing: $modelWin" }

# --- Icon: committed asset (dev/build share the same file) ---
if (-not (Test-Path "assets\icons\lite_tray.ico")) { throw "assets\icons\lite_tray.ico missing" }

# --- Version stamp module (window title) ---
Set-Content _build_version.py "BUILD_DATE = `"$ver`"" -Encoding UTF8

# --- PyInstaller onedir ---
# onedir: no per-launch extraction to %TEMP%\_MEI* (onefile = +121MB temp disk & slow start)
& $PY -m PyInstaller --noconfirm --name PureVoxLite `
    --windowed `
    --icon assets\icons\lite_tray.ico `
    --collect-all onnxruntime `
    --collect-all numpy `
    --hidden-import=pyaudio `
    --exclude-module torch `
    --exclude-module torchvision `
    --exclude-module torchaudio `
    --exclude-module numba `
    --exclude-module pytest `
    --add-data "$modelWin;models" `
    --add-data "assets\fonts\*.ttf;assets/fonts" `
    --add-data "assets\icons\lite_tray.ico;assets\icons" `
    --add-data "assets\icons\lite_tray.png;assets\icons" `
    --add-data "_build_version.py;." `
    lite_mic/main.py
if ($LASTEXITCODE -ne 0) { throw "PyInstaller failed" }

if (-not (Test-Path "dist\PureVoxLite\PureVoxLite.exe")) { throw "PureVoxLite.exe not built" }
$sz = [math]::Round((Get-ChildItem dist\PureVoxLite -Recurse -File | Measure-Object Length -Sum).Sum/1MB, 1)
Write-Host "==> Done: dist\PureVoxLite\  (${sz} MB)"

