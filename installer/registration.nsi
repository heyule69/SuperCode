Unicode true
ManifestDPIAware true
ManifestDPIAwareness PerMonitorV2
RequestExecutionLevel user
SilentInstall silent
SetCompressor /SOLID lzma
!include LogicLib.nsh
!include FileFunc.nsh
!define MUI_ICON "${ICON_SOURCE}"
!define MUI_UNICON "${ICON_SOURCE}"
!include MUI2.nsh
!define PRODUCT "SuperCode"
!ifndef KEY
  !define KEY "Software\Microsoft\Windows\CurrentVersion\Uninstall\SuperCode"
!endif
!ifndef DESKTOP_LINK
  !define DESKTOP_LINK "$DESKTOP\SuperCode.lnk"
!endif
!ifndef PROGRAMS_LINK
  !define PROGRAMS_LINK "$SMPROGRAMS\SuperCode.lnk"
!endif
Name "SuperCode"
OutFile "${OUTPUT}"
Icon "${ICON_SOURCE}"
UninstallIcon "${ICON_SOURCE}"
InstallDir ""
VIProductVersion "${VERSION}.0"
VIAddVersionKey "ProductName" "SuperCode"
VIAddVersionKey "FileDescription" "SuperCode installation registration"
VIAddVersionKey "FileVersion" "${VERSION}"
VIAddVersionKey "LegalCopyright" "SuperCode"
Var NoRegister
Var PreviousPath
!insertmacro MUI_UNPAGE_CONFIRM
!insertmacro MUI_UNPAGE_INSTFILES
!insertmacro MUI_LANGUAGE "SimpChinese"
!insertmacro MUI_LANGUAGE "English"
Function .onInit
  SetShellVarContext current
  SetRegView 64
  ; The native parent passes a validated path as an environment value. NSIS /D
  ; has unusual quoting rules for spaces, so it is never used for this worker.
  ReadEnvStr $INSTDIR "SUPERCODE_INSTALL_TARGET"
  ${If} $INSTDIR == ""
    SetErrorLevel 7
    Quit
  ${EndIf}
  ${IfNot} ${FileExists} "$INSTDIR\supercode.exe"
    SetErrorLevel 7
    Quit
  ${EndIf}
  ${GetOptions} $CMDLINE "/NOREG" $NoRegister
  ${IfNot} ${Errors}
    StrCpy $NoRegister 1
  ${EndIf}
FunctionEnd
Section
  SetOutPath "$INSTDIR"
  ClearErrors
  WriteUninstaller "$INSTDIR\uninstall.exe"
  ${If} ${Errors}
    SetErrorLevel 2
    Quit
  ${EndIf}
  ${If} $NoRegister == 1
    SetErrorLevel 0
    Quit
  ${EndIf}
  ReadRegStr $PreviousPath HKCU "${KEY}" "InstallLocation"
  ${If} $PreviousPath == ""
    ReadRegStr $PreviousPath HKCU "${KEY}" ""
  ${EndIf}
  StrCpy $0 $PreviousPath 1
  ${If} $0 == '$\"'
    StrCpy $PreviousPath $PreviousPath -1 1
  ${EndIf}
  ${If} $PreviousPath != ""
  ${AndIf} $PreviousPath != $INSTDIR
    ; Changing a registered location requires an explicit uninstall first.
    SetErrorLevel 3
    Quit
  ${EndIf}
  ClearErrors
  WriteRegStr HKCU "${KEY}" "" "$INSTDIR"
  WriteRegStr HKCU "${KEY}" "InstallLocation" "$INSTDIR"
  WriteRegStr HKCU "${KEY}" "DisplayName" "SuperCode"
  WriteRegStr HKCU "${KEY}" "DisplayVersion" "${VERSION}"
  WriteRegStr HKCU "${KEY}" "Publisher" "SuperCode"
  WriteRegStr HKCU "${KEY}" "DisplayIcon" "$INSTDIR\${ICON_NAME}"
  WriteRegStr HKCU "${KEY}" "UninstallString" '$\"$INSTDIR\uninstall.exe$\"'
  WriteRegStr HKCU "${KEY}" "QuietUninstallString" '$\"$INSTDIR\uninstall.exe$\" /S'
  WriteRegDWORD HKCU "${KEY}" "NoModify" 1
  WriteRegDWORD HKCU "${KEY}" "NoRepair" 1
  WriteRegDWORD HKCU "${KEY}" "EstimatedSize" ${SIZE_KB}
  CreateShortcut "${PROGRAMS_LINK}" "$INSTDIR\supercode.exe" "" "$INSTDIR\${ICON_NAME}" 0
  CreateShortcut "${DESKTOP_LINK}" "$INSTDIR\supercode.exe" "" "$INSTDIR\${ICON_NAME}" 0
  ${If} ${Errors}
    SetErrorLevel 4
    Quit
  ${EndIf}
  System::Call 'shell32::SHChangeNotify(i 0x08000000, i 0, p 0, p 0)'
  SetErrorLevel 0
SectionEnd
Function un.onInit
  SetShellVarContext current
  SetRegView 64
FunctionEnd
Section "Uninstall"
  ; Delete only installed application files. Chat data and backups stay intact.
  ClearErrors
  Delete "$INSTDIR\supercode.exe"
  ${If} ${Errors}
    MessageBox MB_OK|MB_ICONEXCLAMATION "请先从系统托盘退出 SuperCode，再重试卸载。" /SD IDOK
    SetErrorLevel 2
    Abort
  ${EndIf}
  Delete "$INSTDIR\${ICON_NAME}"
  Delete "$INSTDIR\installation.json"
  ReadRegStr $0 HKCU "${KEY}" "InstallLocation"
  ${If} $0 == ""
    ReadRegStr $0 HKCU "${KEY}" ""
  ${EndIf}
  StrCpy $1 $0 1
  ${If} $1 == '$\"'
    StrCpy $0 $0 -1 1
  ${EndIf}
  ${If} $0 == $INSTDIR
    Delete "${DESKTOP_LINK}"
    Delete "${PROGRAMS_LINK}"
    DeleteRegKey HKCU "${KEY}"
    System::Call 'shell32::SHChangeNotify(i 0x08000000, i 0, p 0, p 0)'
  ${EndIf}
  Delete "$INSTDIR\uninstall.exe"
  RMDir "$INSTDIR"
  SetErrorLevel 0
SectionEnd
