# PureVox window screenshot without stealing focus (Windows PowerShell 5.1).
# Usage:
#   powershell -NoProfile -ExecutionPolicy Bypass -File shot-window.ps1 -ProcessId 1234 -Out C:\Temp\ui.png
#
# Why PrintWindow(PW_RENDERFULLCONTENT=2):
#   SetForegroundWindow is refused for background processes on Windows, so
#   CopyFromScreen captures whatever window the user has in front. PrintWindow
#   asks the window to render itself into a bitmap and works while covered.
#   The main WebView2 window renders fine; a SECOND WebView2 window is white
#   (known wry/Tauri alpha limitation) -- do not use this for that.
# Keep this file ASCII-only: PowerShell 5.1 decodes BOM-less files as ANSI.

param(
  [Parameter(Mandatory = $true)][int]$ProcessId,
  [Parameter(Mandatory = $true)][string]$Out
)

Add-Type -AssemblyName System.Drawing
Add-Type -TypeDefinition @"
using System;
using System.Runtime.InteropServices;
public class PvShot {
  [DllImport("user32.dll")] public static extern bool SetProcessDpiAwarenessContext(IntPtr v);
  [DllImport("user32.dll")] public static extern bool GetWindowRect(IntPtr h, out RECT r);
  [DllImport("user32.dll")] public static extern bool PrintWindow(IntPtr h, IntPtr dc, uint flags);
  [StructLayout(LayoutKind.Sequential)] public struct RECT { public int L, T, R, B; }
}
"@

[PvShot]::SetProcessDpiAwarenessContext([IntPtr]::new(-4)) | Out-Null

$proc = Get-Process -Id $ProcessId -ErrorAction Stop
$h = $proc.MainWindowHandle
if ($h -eq [IntPtr]::Zero) {
  Write-Error "process $ProcessId has no main window"
  exit 1
}

$r = New-Object PvShot+RECT
[PvShot]::GetWindowRect($h, [ref]$r) | Out-Null
$w = $r.R - $r.L
$hgt = $r.B - $r.T
if ($w -le 0 -or $hgt -le 0) {
  Write-Error "bad window rect $w x $hgt"
  exit 1
}

$bmp = New-Object System.Drawing.Bitmap $w, $hgt
$g = [System.Drawing.Graphics]::FromImage($bmp)
$dc = $g.GetHdc()
$ok = [PvShot]::PrintWindow($h, $dc, 2)
$g.ReleaseHdc($dc)
$g.Dispose()
$bmp.Save($Out, [System.Drawing.Imaging.ImageFormat]::Png)
$bmp.Dispose()
Write-Output "$ok $w x $hgt -> $Out"
