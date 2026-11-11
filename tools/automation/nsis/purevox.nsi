Unicode true
!include "MUI2.nsh"

; PureVox — AI 麦克风降噪工具
; Copyright (C) 2024-2026 a2heng <752848283@qq.com>
; SPDX-License-Identifier: GPL-3.0-or-later
;
; 桌面安装包（Tauri NSIS 同款布局）：按用户安装到 %LOCALAPPDATA%\PureVox，
; 不请求管理员权限；带开始菜单 / 桌面快捷方式和卸载项。
;
; 参数（package_windows.py 传入）：
;   /DVERSION=26.11.11      显示版本
;   /DPRODUCTVER=26.11.11.0 VIProductVersion 要求的四段版本
;   /DSTAGE=<目录>          已备好的 exe + opus.dll + models/
;   /DOUTFILE=<setup.exe>   输出路径

!ifndef VERSION
  !define VERSION "0.0.0"
!endif
!ifndef PRODUCTVER
  !define PRODUCTVER "0.0.0.0"
!endif
!ifndef STAGE
  !error "STAGE 未定义：请用 package_windows.py 调用"
!endif
!ifndef OUTFILE
  !define OUTFILE "PureVox_${VERSION}_x64-setup.exe"
!endif

Name "PureVox ${VERSION}"
OutFile "${OUTFILE}"
InstallDir "$LOCALAPPDATA\PureVox"
InstallDirRegKey HKCU "Software\PureVox" "InstallDir"
RequestExecutionLevel user
SetCompressor /SOLID lzma

VIProductVersion "${PRODUCTVER}"
VIAddVersionKey "ProductName" "PureVox"
VIAddVersionKey "FileDescription" "PureVox 安装程序"
VIAddVersionKey "FileVersion" "${VERSION}"
VIAddVersionKey "ProductVersion" "${VERSION}"
VIAddVersionKey "LegalCopyright" "GPL-3.0-or-later"

!define MUI_ABORTWARNING
!define MUI_FINISHPAGE_RUN "$INSTDIR\purevox.exe"
!define MUI_FINISHPAGE_RUN_TEXT "运行 PureVox"
!insertmacro MUI_PAGE_WELCOME
!insertmacro MUI_PAGE_DIRECTORY
!insertmacro MUI_PAGE_INSTFILES
!insertmacro MUI_PAGE_FINISH

!insertmacro MUI_UNPAGE_CONFIRM
!insertmacro MUI_UNPAGE_INSTFILES

!insertmacro MUI_LANGUAGE "SimpChinese"
!insertmacro MUI_LANGUAGE "English"

Section "PureVox" SecMain
  SetOutPath "$INSTDIR"
  ; 升级时先关掉正在跑的旧版本（没在跑就返回非零，忽略即可）
  nsExec::Exec 'taskkill /IM purevox.exe /F'

  ; exe + opus.dll + models/（递归）
  File /r "${STAGE}\*.*"

  CreateShortCut "$SMPROGRAMS\PureVox.lnk" "$INSTDIR\purevox.exe"
  CreateShortCut "$DESKTOP\PureVox.lnk" "$INSTDIR\purevox.exe"
  WriteUninstaller "$INSTDIR\uninstall.exe"

  WriteRegStr HKCU "Software\PureVox" "InstallDir" "$INSTDIR"
  WriteRegStr HKCU "Software\Microsoft\Windows\CurrentVersion\Uninstall\PureVox" "DisplayName" "PureVox"
  WriteRegStr HKCU "Software\Microsoft\Windows\CurrentVersion\Uninstall\PureVox" "DisplayVersion" "${VERSION}"
  WriteRegStr HKCU "Software\Microsoft\Windows\CurrentVersion\Uninstall\PureVox" "Publisher" "a2heng"
  WriteRegStr HKCU "Software\Microsoft\Windows\CurrentVersion\Uninstall\PureVox" "DisplayIcon" "$INSTDIR\purevox.exe"
  WriteRegStr HKCU "Software\Microsoft\Windows\CurrentVersion\Uninstall\PureVox" "UninstallString" '"$INSTDIR\uninstall.exe"'
  WriteRegDWORD HKCU "Software\Microsoft\Windows\CurrentVersion\Uninstall\PureVox" "NoModify" 1
  WriteRegDWORD HKCU "Software\Microsoft\Windows\CurrentVersion\Uninstall\PureVox" "NoRepair" 1
SectionEnd

Section "Uninstall"
  nsExec::Exec 'taskkill /IM purevox.exe /F'
  Delete "$SMPROGRAMS\PureVox.lnk"
  Delete "$DESKTOP\PureVox.lnk"
  DeleteRegKey HKCU "Software\Microsoft\Windows\CurrentVersion\Uninstall\PureVox"
  DeleteRegKey HKCU "Software\PureVox"
  RMDir /r "$INSTDIR"
SectionEnd
