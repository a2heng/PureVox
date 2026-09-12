# PureVox Lite Net - local/CI build script (pure python, no gcc)
# Same recipe as build_lite_mic.ps1; extra: websockets/av/cryptography/zeroconf/qrcode + html page.
# Usage: powershell -ExecutionPolicy Bypass -File build_lite_net.ps1
$ErrorActionPreference = "Stop"

# --- Version stamp: single source tools/automation/version.ps1 -Lite
# (tag lite-v<yyyy.MM.dd.HHmm> -> ver; fallback to local UTC time) ---
. (Join-Path (Split-Path -Parent $MyInvocation.MyCommand.Path) "tools\automation\version.ps1") -Lite
$ver = $VERSION
Write-Host "Lite Net version: $ver"

# --- Python resolution: embedded python312w > python on PATH ---
$scriptDir = Split-Path -Parent $MyInvocation.MyCommand.Path
$embeddedPy = Join-Path $scriptDir "packages\python312w\python.exe"
if (Test-Path $embeddedPy) { $PY = @($embeddedPy) }
elseif (Get-Command python -ErrorAction SilentlyContinue) { $PY = @("python") }
elseif (Get-Command py -ErrorAction SilentlyContinue) { $PY = @("py", "-3") }
else { throw "no python found" }
& $PY --version

# --- Deps (requirements-win.txt: Windows 全量依赖) ---
& $PY -m pip install -q -r requirements-win.txt
if ($LASTEXITCODE -ne 0) { throw "pip install failed" }

# --- Syntax check + smoke import ---
& $PY -m compileall -q lite_net
if ($LASTEXITCODE -ne 0) { throw "compileall failed" }
& $PY -c "import lite_net.config, lite_net.audio, lite_net.engine, lite_net.net, lite_net.ui; print('import OK')"
if ($LASTEXITCODE -ne 0) { throw "smoke import failed" }

# --- Icon: committed asset (dev/build share the same file) ---
if (-not (Test-Path "assets\icons\lite_tray.ico")) { throw "assets\icons\lite_tray.ico missing" }

# --- Version stamp module (window title) ---
Set-Content _build_version.py "BUILD_DATE = `"$ver`"" -Encoding UTF8

# --- PyInstaller onedir ---
# assets/fonts -> _internal/assets/fonts (ui.py resolves ../assets/fonts from __file__,
#                 same relative walk as in the source tree; matches build_win.ps1);
# html         -> _internal/html  (browser client served from the WSS port)
& $PY -m PyInstaller --noconfirm --name PureVoxNetLite `
    --windowed `
    --icon assets\icons\lite_tray.ico `
    --collect-all onnxruntime `
    --collect-all numpy `
    --collect-all av `
    --hidden-import=pyaudio `
    --hidden-import=websockets `
    --hidden-import=zeroconf `
    --exclude-module torch `
    --exclude-module torchvision `
    --exclude-module torchaudio `
    --exclude-module numba `
    --exclude-module pytest `
    --add-data "models\purevox_denoise_202609_ep0000.onnx;models" `
    --add-data "assets\fonts\*.ttf;assets/fonts" `
    --add-data "assets\icons\lite_tray.ico;assets\icons" `
    --add-data "assets\icons\lite_tray.png;assets\icons" `
    --add-data "html;html" `
    --add-data "_build_version.py;." `
    lite_net/main.py
if ($LASTEXITCODE -ne 0) { throw "PyInstaller failed" }

if (-not (Test-Path "dist\PureVoxNetLite\PureVoxNetLite.exe")) { throw "PureVoxNetLite.exe not built" }
$sz = [math]::Round((Get-ChildItem dist\PureVoxNetLite -Recurse -File | Measure-Object Length -Sum).Sum/1MB, 1)
Write-Host "==> Done: dist\PureVoxNetLite\  (${sz} MB)"

