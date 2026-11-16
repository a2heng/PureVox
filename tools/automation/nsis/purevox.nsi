Unicode true
!include "MUI2.nsh"
!include "LogicLib.nsh"
!include "FileFunc.nsh"

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
;
; 更新器传入的运行时参数（与 Tauri 的 NSIS 模板同名）：
;   /S       静默安装（NSIS 内建）
;   /R       安装完成后启动 $INSTDIR\purevox.exe
;   /NS      不创建开始菜单 / 桌面快捷方式
;   /UPDATE  升级模式（当前与普通安装同一套文件布局，保留参数以兼容）
;
; 没有 /R 时**不**自动启动：手工双击安装包的用户走 MUI_FINISHPAGE_RUN。

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

Var RunApp
Var NoShortcut

Function .onInit
  ; 更新器的命令行长这样：
  ;   setup.exe /S /UPDATE /NS /R
  ; GetOptions 只认「/R」这种裸开关；找不到时置错误标志。
  StrCpy $RunApp "0"
  StrCpy $NoShortcut "0"
  ${GetParameters} $R0
  ClearErrors
  ${GetOptions} $R0 "/R" $R1
  IfErrors +2
    StrCpy $RunApp "1"
  ClearErrors
  ${GetOptions} $R0 "/NS" $R1
  IfErrors +2
    StrCpy $NoShortcut "1"
FunctionEnd

; 更新器带 /R：静默安装结束（没有 MUI 完成页）也要把新版本拉起来。
; 手工双击安装包不带 /R，交给 MUI_FINISHPAGE_RUN 的勾选框，不在这里启动。
Function .onInstSuccess
  ${If} $RunApp == "1"
    Exec '"$INSTDIR\purevox.exe"'
  ${EndIf}
FunctionEnd

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

  ${If} $NoShortcut != "1"
    CreateShortCut "$SMPROGRAMS\PureVox.lnk" "$INSTDIR\purevox.exe"
    CreateShortCut "$DESKTOP\PureVox.lnk" "$INSTDIR\purevox.exe"
  ${EndIf}
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
