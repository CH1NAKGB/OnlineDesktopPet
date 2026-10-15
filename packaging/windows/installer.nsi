Unicode true
!ifdef NSIS_MAX_STRLEN
  !undef NSIS_MAX_STRLEN
!endif
!define NSIS_MAX_STRLEN 8192
!include "MUI2.nsh"
!include "LogicLib.nsh"
!include "x64.nsh"
!include "WordFunc.nsh"

!ifndef ROOT
  !error "ROOT must be supplied by build-installer.sh"
!endif
!ifndef OUTPUT
  !error "OUTPUT must be supplied by build-installer.sh"
!endif
!ifndef VERSION
  !error "VERSION must be supplied by build-installer.sh"
!endif
!ifndef VI_VERSION
  !error "VI_VERSION must be supplied by build-installer.sh"
!endif
!define UNINSTALL_KEY "Software\Microsoft\Windows\CurrentVersion\Uninstall\ChickLink"
!define RUN_KEY "Software\Microsoft\Windows\CurrentVersion\Run"
Name "Chick Link ${VERSION}"
OutFile "${OUTPUT}"
InstallDir "$LOCALAPPDATA\Programs\ChickLink"
InstallDirRegKey HKCU "${UNINSTALL_KEY}" "InstallLocation"
RequestExecutionLevel user
SetCompressor /SOLID lzma
SetDatablockOptimize on
AllowSkipFiles off
ShowInstDetails show
ShowUninstDetails show
VIProductVersion "${VI_VERSION}"
VIAddVersionKey "ProductName" "Chick Link"
VIAddVersionKey "FileDescription" "Chick Link per-user installer"
VIAddVersionKey "FileVersion" "${VI_VERSION}"
VIAddVersionKey "LegalCopyright" "Chick Link contributors"

!define MUI_ABORTWARNING
!define MUI_DIRECTORYPAGE_TEXT_TOP "默认安装到当前用户目录，无需管理员权限。升级和卸载都会保留聊天记录及个人配置。"
!define MUI_PAGE_CUSTOMFUNCTION_PRE dir_page_pre
!insertmacro MUI_PAGE_DIRECTORY
!insertmacro MUI_PAGE_INSTFILES
!define MUI_FINISHPAGE_RUN "$INSTDIR\pet-client.exe"
!define MUI_FINISHPAGE_RUN_TEXT "启动 Chick Link"
!insertmacro MUI_PAGE_FINISH
!insertmacro MUI_UNPAGE_CONFIRM
!insertmacro MUI_UNPAGE_INSTFILES
!insertmacro MUI_LANGUAGE "SimpChinese"

Var IsUpgrade
Var WasRunning
Var OldVersion
Var ExistingDir
Var WaitCount
Var FileHandle
!ifdef PRIVATE_CONFIG
Var KeyMatches
!endif

Function dir_page_pre
  ${If} $IsUpgrade == 1
    Abort
  ${EndIf}
FunctionEnd

Function .onInit
  ${IfNot} ${RunningX64}
    MessageBox MB_ICONSTOP "此安装程序需要 64 位 Windows。"
    Abort
  ${EndIf}
  SetShellVarContext current
  StrCpy $IsUpgrade 0
  StrCpy $WasRunning 0
  ReadRegStr $ExistingDir HKCU "${UNINSTALL_KEY}" "InstallLocation"
  ${If} $ExistingDir != ""
    StrCpy $IsUpgrade 1
    StrCpy $INSTDIR $ExistingDir
    ReadRegStr $OldVersion HKCU "${UNINSTALL_KEY}" "DisplayVersion"
    ${If} $OldVersion != ""
      ${VersionCompare} "$OldVersion" "${VERSION}" $R0
      ${If} $R0 == 1
        MessageBox MB_YESNO|MB_ICONEXCLAMATION "当前已安装 $OldVersion，继续安装 ${VERSION} 属于降级，可能导致新版数据无法读取。确定继续？" IDYES +2
        Abort
      ${EndIf}
    ${EndIf}
  ${EndIf}
FunctionEnd

Function CheckExeUnlocked
  IfFileExists "$INSTDIR\pet-client.exe" 0 unlocked
  ClearErrors
  FileOpen $FileHandle "$INSTDIR\pet-client.exe" a
  IfErrors locked
  FileClose $FileHandle
unlocked:
  Push 1
  Return
locked:
  Push 0
FunctionEnd

Section "Chick Link" SEC_MAIN
  SetShellVarContext current
  IfFileExists "$INSTDIR\pet-client.exe" 0 executable_ready
  nsExec::Exec 'taskkill /IM pet-client.exe'
  Pop $R0
  ${If} $R0 == 0
    StrCpy $WasRunning 1
  ${EndIf}
  StrCpy $WaitCount 0
wait_for_exit:
  Call CheckExeUnlocked
  Pop $R0
  ${If} $R0 == 1
    Goto executable_ready
  ${EndIf}
  Sleep 500
  IntOp $WaitCount $WaitCount + 1
  ${If} $WaitCount < 10
    Goto wait_for_exit
  ${EndIf}
  nsExec::Exec 'taskkill /F /IM pet-client.exe'
  Pop $R0
  Sleep 1000
  Call CheckExeUnlocked
  Pop $R0
  ${If} $R0 != 1
    MessageBox MB_ICONSTOP "小鸡程序仍在运行，无法安全升级。请手动退出后重试。"
    Abort
  ${EndIf}
executable_ready:
  SetOutPath "$INSTDIR"
  SetOverwrite on
  ; A locked running executable must be retried or cancelled, never skipped.
  File /oname=pet-client.exe "${ROOT}/dist/ChickLink-windows-x64-v${VERSION}.exe"
  File "${ROOT}/packaging/WINDOWS-INSTALL.md"
  SetOutPath "$INSTDIR\font-licenses"
  File "${ROOT}/pet-client/assets/fonts/LICENSE.txt"
  File "${ROOT}/pet-client/assets/fonts/AUTHORS.txt"
  File "${ROOT}/pet-client/assets/fonts/README.md"
!ifdef PRIVATE_CONFIG
  ; Identity is never replaced, including when an old client is upgraded.
  IfFileExists "$APPDATA\ChickLink\connection.json" connection_done
  SetOutPath "$APPDATA\ChickLink"
  File /oname=connection.json "${PRIVATE_CONFIG}/chicklink-connection.json"
connection_done:
  IfFileExists "$APPDATA\ChickLink\relay-public.pem" check_superseded_key
  SetOutPath "$APPDATA\ChickLink"
  File /oname=relay-public.pem "${PRIVATE_CONFIG}/relay-public.pem"
  Goto private_done
check_superseded_key:
  StrCpy $KeyMatches 0
  InitPluginsDir
!ifdef SUPERSEDED_KEYS_INCLUDE
  !include "${SUPERSEDED_KEYS_INCLUDE}"
!endif
  ${If} $KeyMatches == 1
    ClearErrors
    Rename "$APPDATA\ChickLink\relay-public.pem" "$APPDATA\ChickLink\relay-public.pem.superseded-${VERSION}"
    IfErrors key_backup_failed
    SetOutPath "$APPDATA\ChickLink"
    File /oname=relay-public.pem "${PRIVATE_CONFIG}/relay-public.pem"
    Goto private_done
key_backup_failed:
    MessageBox MB_ICONSTOP "旧公钥备份失败；没有覆盖原文件。请检查目录权限后重试。"
    Abort
  ${EndIf}
private_done:
!endif
  SetOutPath "$INSTDIR"
  WriteUninstaller "$INSTDIR\Uninstall.exe"
  CreateDirectory "$SMPROGRAMS\Chick Link"
  CreateShortcut "$SMPROGRAMS\Chick Link\Chick Link.lnk" "$INSTDIR\pet-client.exe"
  CreateShortcut "$SMPROGRAMS\Chick Link\卸载 Chick Link.lnk" "$INSTDIR\Uninstall.exe"
  CreateShortcut "$DESKTOP\Chick Link.lnk" "$INSTDIR\pet-client.exe"
  WriteRegStr HKCU "${UNINSTALL_KEY}" "DisplayName" "Chick Link"
  WriteRegStr HKCU "${UNINSTALL_KEY}" "DisplayVersion" "${VERSION}"
  WriteRegStr HKCU "${UNINSTALL_KEY}" "InstallLocation" "$INSTDIR"
  WriteRegStr HKCU "${UNINSTALL_KEY}" "DisplayIcon" "$INSTDIR\pet-client.exe"
  WriteRegStr HKCU "${UNINSTALL_KEY}" "UninstallString" '"$INSTDIR\Uninstall.exe"'
  WriteRegDWORD HKCU "${UNINSTALL_KEY}" "NoModify" 1
  WriteRegDWORD HKCU "${UNINSTALL_KEY}" "NoRepair" 1
  ReadRegStr $0 HKCU "${RUN_KEY}" "ChickLink"
  ${If} $0 != ""
    WriteRegStr HKCU "${RUN_KEY}" "ChickLink" '"$INSTDIR\pet-client.exe"'
  ${EndIf}
  ${If} $WasRunning == 1
    DetailPrint "升级完成，原配置已保留。"
  ${EndIf}
SectionEnd

Section "Uninstall"
  SetShellVarContext current
  ClearErrors
  Delete "$INSTDIR\pet-client.exe"
  ${If} ${Errors}
    MessageBox MB_ICONSTOP "请先退出小鸡，再重新运行卸载。"
    Abort
  ${EndIf}
  ReadRegStr $0 HKCU "${RUN_KEY}" "ChickLink"
  ${If} $0 == '"$INSTDIR\pet-client.exe"'
    DeleteRegValue HKCU "${RUN_KEY}" "ChickLink"
  ${EndIf}
  Delete "$DESKTOP\Chick Link.lnk"
  Delete "$SMPROGRAMS\Chick Link\Chick Link.lnk"
  Delete "$SMPROGRAMS\Chick Link\卸载 Chick Link.lnk"
  RMDir "$SMPROGRAMS\Chick Link"
  Delete "$INSTDIR\WINDOWS-INSTALL.md"
  Delete "$INSTDIR\font-licenses\LICENSE.txt"
  Delete "$INSTDIR\font-licenses\AUTHORS.txt"
  Delete "$INSTDIR\font-licenses\README.md"
  RMDir "$INSTDIR\font-licenses"
  Delete "$INSTDIR\Uninstall.exe"
  RMDir "$INSTDIR"
  DeleteRegKey HKCU "${UNINSTALL_KEY}"
  ; APPDATA and any portable connection files are intentionally retained.
SectionEnd
