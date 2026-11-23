; Velox — UNIFIED per-user NSIS installer (Phase 5: ONE Velox-Setup-x64.exe).
; Replaces the former online/offline variant pair: this template installs the
; COMPLETE application (Velox.exe, browser extension + native host, yt-dlp,
; LGPL ffmpeg/ffprobe, WebView2 bootstrapper fallback, licenses/notices).
;
; Tauri updater contract (docs/PHASE0-ROOT-CAUSES.md RC8, Phase 4 flow):
;   - launched BY THE UPDATER with /P, /S and/or /UPDATE flags → UPDATE MODE:
;     no taskkill, WAIT for Velox.exe to exit (the app exits itself after
;     pausing downloads), silent install, then START the new Velox.exe.
;   - interactive run: graceful close first (taskkill WITHOUT /F sends
;     WM_CLOSE — the app's CloseRequested handler pauses downloads), bounded
;     wait, then an explicit prompt if Velox is still running. Force-kill is
;     never used on user data.
; Usage of user PATH is not required: the app discovers bundled binaries
; beside its own executable (app/src-tauri/src/main.rs does that).
Unicode true
ManifestDPIAware true
RequestExecutionLevel user

!define PRODUCT_NAME "Velox"
!define PRODUCT_VERSION "0.5.0"
!define IDENTIFIER "io.velox.downloadmanager"
!define UNINST_KEY "Software\Microsoft\Windows\CurrentVersion\Uninstall\${IDENTIFIER}"
!define WEBVIEW2_KEY_L "SOFTWARE\Microsoft\EdgeUpdate\Clients\{F3017226-FE2A-4295-8BDF-00C3A9A7E4C5}"
!define WEBVIEW2_KEY_W "SOFTWARE\WOW6432Node\Microsoft\EdgeUpdate\Clients\{F3017226-FE2A-4295-8BDF-00C3A9A7E4C5}"

!include "MUI2.nsh"
!include "FileFunc.nsh"

Name "${PRODUCT_NAME} ${PRODUCT_VERSION}"
OutFile "/tmp/installer/Velox-Setup-x64.exe"
InstallDir "$LOCALAPPDATA\Programs\Velox"
InstallDirRegKey HKCU "${UNINST_KEY}" "InstallDir"
VIProductVersion "0.5.0.0"
VIAddVersionKey ProductName "${PRODUCT_NAME}"
VIAddVersionKey FileVersion "0.5.0.0"
VIAddVersionKey ProductVersion "0.5.0.0"
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

Var UpdateMode      ; 1 = launched by the updater (/P, /S, /UPDATE)
Var RestartAfter    ; 1 = start Velox.exe after a successful update install

; Exit code of `tasklist /FI "IMAGENAME eq Velox.exe" | find /I "Velox.exe"`:
; 0 = running, 1 = not running. NO force kill anywhere in this script.
!macro VELOX_RUNNING _out
  nsExec::Exec 'tasklist /NH /FI "IMAGENAME eq Velox.exe" | find /I "Velox.exe"'
  Pop ${_out}
!macroend

; Graceful close for INTERACTIVE runs: WM_CLOSE via taskkill (no /F) lets the
; app's CloseRequested handler pause active downloads and exit by itself.
!macro CLOSEVELOX_GRACEFUL
  DetailPrint "Closing a running Velox instance (downloads are paused safely)..."
  nsExec::Exec 'taskkill /IM Velox.exe'
  Pop $0
  StrCmp $0 0 0 done_close
  StrCpy $1 0
wait_close:
  Sleep 500
  IntOp $1 $1 + 1
  !insertmacro VELOX_RUNNING $2
  StrCmp $2 0 0 done_close
  IntCmp $1 60 wait_close done_close wait_close   ; ~30 s
  MessageBox MB_RETRYCANCEL|MB_ICONQUESTION \
    "Velox is still running (a download may be active).$\nClosing it now pauses downloads safely — but you can also close it yourself first.$\n$\nRetry?" \
    IDRETRY wait_close
  Abort "Velox must be closed before installing."
done_close:
!macroend

; UPDATE MODE: the app pauses its downloads and exits by itself; we simply
; wait (bounded). If it refuses to exit we do NOT kill it — the update aborts
; and the app keeps running (honest failure, no data loss).
!macro CLOSEVELOX_UPDATEWAIT
  DetailPrint "Waiting for Velox to hand over for the update..."
  StrCpy $1 0
wait_upd:
  Sleep 500
  IntOp $1 $1 + 1
  !insertmacro VELOX_RUNNING $2
  StrCmp $2 1 0 done_upd
  IntCmp $1 240 wait_upd 0 0                      ; ~120 s
  DetailPrint "Velox did not exit within 120 s — update aborted, nothing was touched."
  SetErrorLevel 2 ; honest abort: app did not hand over, nothing was touched
  Quit
done_upd:
!macroend

Function .onInit
  StrCpy $UpdateMode 0
  StrCpy $RestartAfter 0
  ${GetOptions} $CMDLINE "/UPDATE" $R0
  ${IfNot} ${Errors}
    StrCpy $UpdateMode 1
  ${EndIf}
  ${GetOptions} $CMDLINE "/P" $R0
  ${IfNot} ${Errors}
    StrCpy $UpdateMode 1
  ${EndIf}
  ${GetOptions} $CMDLINE "/S" $R0
  ${IfNot} ${Errors}
    StrCpy $UpdateMode 1
    SetSilent silent
  ${EndIf}
  ${GetOptions} $CMDLINE "/R" $R0
  ${IfNot} ${Errors}
    StrCpy $RestartAfter 1
  ${EndIf}
  ; /NS and /D are accepted and ignored (Tauri NSIS compatibility; our layout
  ; always installs shortcuts and the destination is the registered one).
FunctionEnd

Section "-Install" SecInstall
  SetOutPath "$INSTDIR"
  StrCmp $UpdateMode 1 0 interactive
    !insertmacro CLOSEVELOX_UPDATEWAIT
    Goto files
interactive:
    !insertmacro CLOSEVELOX_GRACEFUL
files:

  ; --- application ---
  File "/oname=Velox.exe" "/tmp/installer/Velox.exe"

  ; --- browser ecosystem: extension assets + native messaging host ---
  File "/oname=velox-browser-host.exe" "/tmp/installer/velox-browser-host.exe"
  SetOutPath "$INSTDIR\extension"
  File "/oname=manifest.json" "/tmp/installer/extension/manifest.json"
  File "/oname=background.js" "/tmp/installer/extension/background.js"
  File "/oname=popup.html" "/tmp/installer/extension/popup.html"
  File "/oname=popup.css" "/tmp/installer/extension/popup.css"
  File "/oname=popup.js" "/tmp/installer/extension/popup.js"
  File "/oname=README.md" "/tmp/installer/extension/README.md"
  SetOutPath "$INSTDIR\extension\icons"
  File "/oname=16.png" "/tmp/installer/extension/icons/16.png"
  File "/oname=32.png" "/tmp/installer/extension/icons/32.png"
  File "/oname=48.png" "/tmp/installer/extension/icons/48.png"
  File "/oname=128.png" "/tmp/installer/extension/icons/128.png"
  SetOutPath "$INSTDIR"
  ; --register without an argument uses the canonical ID derived from the
  ; extension manifest key (a stale literal here would make Chrome refuse NM)
  DetailPrint "Registering the Velox native messaging host (Chromium browsers)..."
  nsExec::ExecToLog '"$INSTDIR\velox-browser-host.exe" --register'
  Pop $0

  ; --- media tooling: bundled yt-dlp + LGPL ffmpeg (YouTube etc.) ---
  File "/oname=yt-dlp.exe" "/tmp/installer/yt-dlp.exe"
  File "/oname=ffmpeg.exe" "/tmp/installer/ffmpeg.exe"
  File "/oname=ffprobe.exe" "/tmp/installer/ffprobe.exe"

  ; --- licenses & notices (LGPL compliance) ---
  File "/oname=THIRD-PARTY-NOTICES.md" "/tmp/installer/THIRD_PARTY_NOTICES.md"
  File "/oname=FFMPEG-LICENSE.txt" "/tmp/installer/ffmpeg-lgpl-LICENSE.txt"

  ; --- verify every required bundled file actually exists (Phase 5) ---
  StrCpy $3 0
  IfFileExists "$INSTDIR\Velox.exe" +2 0
    StrCpy $3 1
  IfFileExists "$INSTDIR\yt-dlp.exe" +2 0
    StrCpy $3 1
  IfFileExists "$INSTDIR\ffmpeg.exe" +2 0
    StrCpy $3 1
  IfFileExists "$INSTDIR\ffprobe.exe" +2 0
    StrCpy $3 1
  IfFileExists "$INSTDIR\velox-browser-host.exe" +2 0
    StrCpy $3 1
  IfFileExists "$INSTDIR\extension\manifest.json" +2 0
    StrCpy $3 1
  IntCmp $3 0 files_ok
    StrCmp $UpdateMode 1 0 verify_ui
      SetErrorLevel 3 ; incomplete install in update mode: report via exit code
      Quit
    verify_ui:
      MessageBox MB_ICONEXCLAMATION|MB_TOPMOST \
        "The installation is incomplete (a required file is missing).$\nPlease re-run the installer or download it again from the release page."
      SetErrorLevel 3
      Quit
  files_ok:

  ; --- WebView2 runtime (interactive UI only when it really fails) ---
  DetailPrint "Checking WebView2 runtime..."
  ReadRegStr $0 HKLM "${WEBVIEW2_KEY_W}" "pv"
  StrCmp $0 "" 0 runtime_ok
  ReadRegStr $0 HKLM "${WEBVIEW2_KEY_L}" "pv"
  StrCmp $0 "" 0 runtime_ok
  ReadRegStr $0 HKCU "${WEBVIEW2_KEY_L}" "pv"
  StrCmp $0 "" 0 runtime_ok
  File "/oname=WebView2Setup.exe" "/tmp/installer/MicrosoftEdgeWebview2Setup.exe"
  DetailPrint "WebView2 runtime not found - installing (Evergreen standalone)..."
  ExecWait '"$INSTDIR\WebView2Setup.exe" /silent /install' $1
  DetailPrint "WebView2 installer exit code: $1"
  IntCmp $1 0 runtime_ok 0 runtime_ok
    StrCmp $UpdateMode 1 0 wv2_ui
      DetailPrint "WebView2 runtime missing — Velox may fail to start. Install it from https://developer.microsoft.com/microsoft-edge/webview2/"
      Goto runtime_ok
    wv2_ui:
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
  ; updater contract: bring the new version back up (the app exited for us)
  StrCmp $UpdateMode 1 0 no_restart
    Exec '"$INSTDIR\Velox.exe"'
  no_restart:
SectionEnd

Section "un.Uninstall"
  ; graceful close (no force kill — the app pauses downloads on WM_CLOSE)
  nsExec::Exec 'taskkill /IM Velox.exe'
  Pop $0
  StrCpy $1 0
un_wait:
  Sleep 500
  IntOp $1 $1 + 1
  !insertmacro VELOX_RUNNING $2
  StrCmp $2 1 0 un_done
  IntCmp $1 60 un_wait un_done un_done          ; ~30 s, then proceed anyway
un_done:
  Delete "$INSTDIR\Velox.exe"
  Delete "$INSTDIR\yt-dlp.exe"
  Delete "$INSTDIR\ffmpeg.exe"
  Delete "$INSTDIR\ffprobe.exe"
  Delete "$INSTDIR\velox-browser-host.exe"
  Delete "$INSTDIR\THIRD-PARTY-NOTICES.md"
  Delete "$INSTDIR\FFMPEG-LICENSE.txt"
  RMDir /r "$INSTDIR\extension"
  DeleteRegKey HKCU "Software\Google\Chrome\NativeMessagingHosts\io.velox.downloadmanager"
  DeleteRegKey HKCU "Software\Microsoft\Edge\NativeMessagingHosts\io.velox.downloadmanager"
  DeleteRegKey HKCU "Software\BraveSoftware\Brave-Browser\NativeMessagingHosts\io.velox.downloadmanager"
  DeleteRegKey HKCU "Software\Opera Software\NativeMessagingHosts\io.velox.downloadmanager"
  DeleteRegKey HKCU "Software\Chromium\NativeMessagingHosts\io.velox.downloadmanager"
  DeleteRegKey HKCU "Software\Vivaldi\NativeMessagingHosts\io.velox.downloadmanager"
  Delete "$INSTDIR\WebView2Setup.exe"
  Delete "$INSTDIR\Uninstall Velox.exe"
  RMDir "$INSTDIR"
  Delete "$SMPROGRAMS\Velox\Velox.lnk"
  Delete "$SMPROGRAMS\Velox\Uninstall Velox.lnk"
  RMDir "$SMPROGRAMS\Velox"
  Delete "$DESKTOP\Velox.lnk"
  DeleteRegKey HKCU "${UNINST_KEY}"
SectionEnd
