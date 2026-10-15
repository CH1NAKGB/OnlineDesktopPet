Unicode true
!ifdef NSIS_MAX_STRLEN
  !undef NSIS_MAX_STRLEN
!endif
!define NSIS_MAX_STRLEN 8192
!include "MUI2.nsh"
!include "LogicLib.nsh"
!include "x64.nsh"

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
!define MUI_DIRECTORYPAGE_TEXT_TOP "安装前请退出正在运行的小鸡。默认安装到当前用户目录，无需管理员权限。升级和卸载都会保留聊天记录及个人配置。"
!insertmacro MUI_PAGE_DIRECTORY
!insertmacro MUI_PAGE_INSTFILES
!define MUI_FINISHPAGE_RUN "$INSTDIR\pet-client.exe"
!define MUI_FINISHPAGE_RUN_TEXT "启动 Chick Link"
!insertmacro MUI_PAGE_FINISH
!insertmacro MUI_UNPAGE_CONFIRM
!insertmacro MUI_UNPAGE_INSTFILES
!insertmacro MUI_LANGUAGE "SimpChinese"

Function .onInit
  ${IfNot} ${RunningX64}
    MessageBox MB_ICONSTOP "此安装程序需要 64 位 Windows。"
    Abort
  ${EndIf}
  SetShellVarContext current
FunctionEnd

Section "Chick Link" SEC_MAIN
  SetShellVarContext current
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
  ; Never overwrite an established identity, even when reinstalling.
  IfFileExists "$APPDATA\ChickLink\connection.json" private_done
  SetOutPath "$APPDATA\ChickLink"
  File /oname=relay-public.pem "${PRIVATE_CONFIG}/relay-public.pem"
  File /oname=connection.json "${PRIVATE_CONFIG}/chicklink-connection.json"
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
