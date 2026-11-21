; Velox 0.2.0 — per-user NSIS installer, OFFLINE variant (WebView2 Evergreen standalone bundled).
; Recreated 2026-11-21 (and restored after rollback #3 same day).
; Primary (online bootstrapper) variant: scripts/installer-online-template.nsi.
Unicode true
ManifestDPIAware true
RequestExecutionLevel user

!define PRODUCT_NAME "Velox"
!define PRODUCT_VERSION "0.2.0"
!define IDENTIFIER "io.velox.downloadmanager"
!define UNINST_KEY "Software\Microsoft\Windows\CurrentVersion\Uninstall\${IDENTIFIER}"
!define WEBVIEW2_KEY_L "SOFTWARE\Microsoft\EdgeUpdate\Clients\{F3017226-FE2A-4295-8BDF-00C3A9A7E4C5}"
!define WEBVIEW2_KEY_W "SOFTWARE\WOW6432Node\Microsoft\EdgeUpdate\Clients\{F3017226-FE2A-4295-8BDF-00C3A9A7E4C5}"

!include "MUI2.nsh"

Name "${PRODUCT_NAME} ${PRODUCT_VERSION}"
OutFile "/tmp/installer/Velox-Setup-x64-offline.exe"
InstallDir "$LOCALAPPDATA\Programs\Velox"
InstallDirRegKey HKCU "${UNINST_KEY}" "InstallDir"
VIProductVersion "0.2.0.0"
VIAddVersionKey ProductName "${PRODUCT_NAME}"
VIAddVersionKey FileVersion "0.2.0.0"
VIAddVersionKey ProductVersion "0.2.0.0"
VIAddVersionKey CompanyName "Velox Project"
VIAddVersionKey FileDescription "Velox installer"
VIAddVersionKey LegalCopyright "MIT License"

!define MUI_ICON "/home/z/my-project/download-engine/app/src-tauri/icons/icon.ico"
!define MUI_UNICON "/home/z/my-project/download-engine/app/src-tauri/icons/icon.ico"
!define MUI_ABORTWARNING
!define MUI_FINISHPAGE_RUN "$INSTDIR\Velox.exe"
!define MUI_FINISHPAGE_RUN_TEXT "Launch Velox"

!insertmacro MUI_PAGE_WELCOME
!insertmacro MUI_PAGE_DIRECTORY
!insertmacro MUI_PAGE_INSTFILES
!insertmacro MUI_PAGE_FINISH
!insertmacro MUI_UNPAGE_CONFIRM
!insertmacro MUI_UNPAGE_INSTFILES
!insertmacro MUI_LANGUAGE "English"

!macro CLOSEVELOX
  DetailPrint "Checking for a running Velox instance..."
  nsExec::ExecToLog 'taskkill /IM Velox.exe /F'
  Pop $0
  Sleep 500
!macroend

Section "-Bootstrap files" SecFiles
  SetOutPath "$INSTDIR"
  !insertmacro CLOSEVELOX
  File "/oname=Velox.exe" "/tmp/installer/Velox.exe"
  File "/oname=WebView2Setup.exe" "/tmp/installer/MicrosoftEdgeWebview2Setup.exe"

  DetailPrint "Checking WebView2 runtime..."
  ReadRegStr $0 HKLM "${WEBVIEW2_KEY_W}" "pv"
  StrCmp $0 "" 0 runtime_ok
  ReadRegStr $0 HKLM "${WEBVIEW2_KEY_L}" "pv"
  StrCmp $0 "" 0 runtime_ok
  ReadRegStr $0 HKCU "${WEBVIEW2_KEY_L}" "pv"
  StrCmp $0 "" 0 runtime_ok
  DetailPrint "WebView2 runtime not found - installing (Evergreen standalone)..."
  ExecWait '"$INSTDIR\WebView2Setup.exe" /silent /install' $1
  DetailPrint "WebView2 installer exit code: $1"
  IntCmp $1 0 runtime_ok 0 runtime_ok
    MessageBox MB_ICONEXCLAMATION|MB_TOPMOST \
      "The WebView2 runtime could not be installed automatically.$\nVelox needs it to display its window.$\n$\nInstall it from: https://developer.microsoft.com/microsoft-edge/webview2/$\nthen run Velox from the Start Menu."
  runtime_ok:
  Delete "$INSTDIR\WebView2Setup.exe"
SectionEnd

Section -Post
  WriteUninstaller "$INSTDIR\Uninstall Velox.exe"
  CreateDirectory "$SMPROGRAMS\Velox"
  CreateShortCut "$SMPROGRAMS\Velox\Velox.lnk" "$INSTDIR\Velox.exe"
  CreateShortCut "$SMPROGRAMS\Velox\Uninstall Velox.lnk" "$INSTDIR\Uninstall Velox.exe"
  CreateShortCut "$DESKTOP\Velox.lnk" "$INSTDIR\Velox.exe"
  WriteRegStr HKCU "${UNINST_KEY}" "DisplayName" "Velox - Download Manager"
  WriteRegStr HKCU "${UNINST_KEY}" "DisplayVersion" "${PRODUCT_VERSION}"
  WriteRegStr HKCU "${UNINST_KEY}" "Publisher" "Velox Project"
  WriteRegStr HKCU "${UNINST_KEY}" "DisplayIcon" "$INSTDIR\Velox.exe"
  WriteRegStr HKCU "${UNINST_KEY}" "InstallDir" "$INSTDIR"
  WriteRegStr HKCU "${UNINST_KEY}" "UninstallString" '"$INSTDIR\Uninstall Velox.exe"'
  WriteRegStr HKCU "${UNINST_KEY}" "QuietUninstallString" '"$INSTDIR\Uninstall Velox.exe" /S'
  WriteRegDWORD HKCU "${UNINST_KEY}" "NoModify" 1
  WriteRegDWORD HKCU "${UNINST_KEY}" "NoRepair" 1
SectionEnd

Section "un.Uninstall"
  !insertmacro CLOSEVELOX
  Delete "$INSTDIR\Velox.exe"
  Delete "$INSTDIR\WebView2Setup.exe"
  Delete "$INSTDIR\Uninstall Velox.exe"
  RMDir "$INSTDIR"
  Delete "$SMPROGRAMS\Velox\Velox.lnk"
  Delete "$SMPROGRAMS\Velox\Uninstall Velox.lnk"
  RMDir "$SMPROGRAMS\Velox"
  Delete "$DESKTOP\Velox.lnk"
  DeleteRegKey HKCU "${UNINST_KEY}"
SectionEnd
