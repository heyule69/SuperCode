Unicode true
ManifestDPIAware true
ManifestDPIAwareness PerMonitorV2
RequestExecutionLevel user
SilentInstall silent
SetCompressor /SOLID lzma
!include LogicLib.nsh
!include x64.nsh
!include FileFunc.nsh
Name "SuperCode"
OutFile "${OUTPUT}"
Icon "${ICON_SOURCE}"
VIProductVersion "${VERSION}.0"
VIAddVersionKey "ProductName" "SuperCode"
VIAddVersionKey "FileDescription" "SuperCode Installer"
VIAddVersionKey "FileVersion" "${VERSION}"
VIAddVersionKey "LegalCopyright" "SuperCode"
Var VerifyPath
Var ExitStatus
Var UpdateRequest
Var VerifyUpdate
Function .onInit
  ${GetOptions} $CMDLINE "/VERIFY=" $VerifyPath
  ${GetOptions} $CMDLINE "/UPDATE_REQUEST=" $UpdateRequest
  ${GetOptions} $CMDLINE "/VERIFY_UPDATE=" $VerifyUpdate
FunctionEnd
Section
  ${IfNot} ${RunningX64}
    MessageBox MB_OK|MB_ICONEXCLAMATION "此安装包需要 64 位 Windows。"
    SetErrorLevel 7
    Quit
  ${EndIf}
  InitPluginsDir
  SetOutPath "$PLUGINSDIR"
  File /oname=installer.exe "${GUI_SOURCE}"
  ${If} $VerifyUpdate != ""
    ExecWait '$\"$PLUGINSDIR\installer.exe$\" --verify-update $\"$VerifyUpdate$\"' $ExitStatus
    SetErrorLevel $ExitStatus
    Quit
  ${EndIf}
  ${If} $VerifyPath != ""
    ExecWait '$\"$PLUGINSDIR\installer.exe$\" --verify-install $\"$VerifyPath$\"' $ExitStatus
    SetErrorLevel $ExitStatus
    Quit
  ${EndIf}
  ; The UI shares the system WebView2 runtime; only missing machines bootstrap it.
  SetRegView 32
  ReadRegStr $0 HKLM "SOFTWARE\Microsoft\EdgeUpdate\Clients\{F3017226-FE2A-4295-8BDF-00C3A9A7E4C5}" "pv"
  ${If} $0 == ""
    ReadRegStr $0 HKCU "SOFTWARE\Microsoft\EdgeUpdate\Clients\{F3017226-FE2A-4295-8BDF-00C3A9A7E4C5}" "pv"
  ${EndIf}
  ${If} $0 == ""
  ${OrIf} $0 == "0.0.0.0"
    NSISdl::download /TIMEOUT=30000 "https://go.microsoft.com/fwlink/p/?LinkId=2124703" "$PLUGINSDIR\WebView2Setup.exe"
    Pop $0
    ${If} $0 != "success"
      MessageBox MB_OK|MB_ICONEXCLAMATION "无法准备安装界面。请检查网络后重新打开安装器。"
      SetErrorLevel 5
      Quit
    ${EndIf}
    ExecWait '$\"$PLUGINSDIR\WebView2Setup.exe$\" /silent /install' $0
    ${If} $0 != 0
      MessageBox MB_OK|MB_ICONEXCLAMATION "WebView2 安装失败，请重试。"
      SetErrorLevel 6
      Quit
    ${EndIf}
  ${EndIf}
  ${If} $UpdateRequest != ""
    ExecWait '$\"$PLUGINSDIR\installer.exe$\" --update-request $\"$UpdateRequest$\"' $ExitStatus
  ${Else}
    ExecWait '$\"$PLUGINSDIR\installer.exe$\"' $ExitStatus
  ${EndIf}
  SetErrorLevel $ExitStatus
SectionEnd
