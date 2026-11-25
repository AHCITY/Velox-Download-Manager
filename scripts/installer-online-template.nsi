; Velox — ONLINE per-user NSIS installer.
; The CORE app (Velox.exe, browser extension + native messaging host,
; WebView2 fallback, licences) is bundled — small download. The OPTIONAL
; "YouTube Video Downloader" component (yt-dlp + LGPL ffmpeg/ffprobe,
; ~120 MB) is fetched AT INSTALL TIME from the official sources by
; velox-fetch.ps1, so the online setup is genuinely online and users who
; do not want media tooling never download it.
;
; Build parameters (same contract as installer-unified-template.nsi):
;   BUILD_STAGING  directory holding the input files (Velox.exe, extension/, …)
;   BUILD_OUTPUT   the installer file to produce
;   BUILD_ICON_DIR directory of icon.ico
;   PRODUCT_VERSION app version (lockstep with tauri.conf.json)
;   SEP            path separator for JOINING build paths ("\ on Windows hosts)
; Staging must also contain velox-fetch.ps1 (scripts/velox-online-fetch.ps1).
;
; Tauri updater contract (identical to the unified installer):
;   /P, /S, /UPDATE → UPDATE MODE: wait for the app to exit (no taskkill),
;   silent install, /R restarts the new version. The auto-updater payload
;   remains the UNIFIED installer; this online setup is for humans.
;   In update mode the media component skips the network when the tools
;   already exist beside the exe (updates stay fast); it only downloads
;   when they are genuinely missing.
Unicode true
ManifestDPIAware true
RequestExecutionLevel user

!ifndef PRODUCT_VERSION
  !define PRODUCT_VERSION "0.7.0"
!endif
!ifndef BUILD_STAGING
  !define BUILD_STAGING "/tmp/installer"
!endif
!ifndef BUILD_OUTPUT
  !define BUILD_OUTPUT "${BUILD_STAGING}${SEP}Velox-Setup-x64-Online.exe"
!endif
!ifndef BUILD_ICON_DIR
  !define BUILD_ICON_DIR "/home/z/my-project/download-engine/app/src-tauri/icons"
!endif
!ifndef SEP
  !define SEP "\"
!endif
!define PRODUCT_NAME "Velox"
!define IDENTIFIER "io.velox.downloadmanager"
!define UNINST_KEY "Software\Microsoft\Windows\CurrentVersion\Uninstall\${IDENTIFIER}"
!define WEBVIEW2_KEY_L "SOFTWARE\Microsoft\EdgeUpdate\Clients\{F3017226-FE2A-4295-8BDF-00C3A9A7E4C5}"
!define WEBVIEW2_KEY_W "SOFTWARE\WOW6432Node\Microsoft\EdgeUpdate\Clients\{F3017226-FE2A-4295-8BDF-00C3A9A7E4C5}"

!include "MUI2.nsh"
!include "FileFunc.nsh"

Name "${PRODUCT_NAME} ${PRODUCT_VERSION}"
OutFile "${BUILD_OUTPUT}"
InstallDir "$LOCALAPPDATA\Programs\Velox"
InstallDirRegKey HKCU "${UNINST_KEY}" "InstallDir"
VIProductVersion "${PRODUCT_VERSION}.0"
VIAddVersionKey ProductName "${PRODUCT_NAME}"
VIAddVersionKey FileVersion "${PRODUCT_VERSION}.0"
VIAddVersionKey ProductVersion "${PRODUCT_VERSION}.0"
VIAddVersionKey CompanyName "Velox Project"
VIAddVersionKey FileDescription "Velox online installer"
VIAddVersionKey LegalCopyright "MIT License"

!define MUI_ICON "${BUILD_ICON_DIR}${SEP}icon.ico"
!define MUI_UNICON "${BUILD_ICON_DIR}${SEP}icon.ico"
!define MUI_ABORTWARNING
!define MUI_FINISHPAGE_RUN "$INSTDIR\Velox.exe"
!define MUI_FINISHPAGE_RUN_TEXT "Launch Velox"

!insertmacro MUI_PAGE_WELCOME
!insertmacro MUI_PAGE_COMPONENTS
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
; wait (bounded). If it refuses to exit we do NOT kill it — the update aborts.
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
  SetErrorLevel 2
  Quit
done_upd:
!macroend

; Download the optional YouTube toolchain via the staged PowerShell fetcher.
; Skips the network entirely when the tools already exist (fast updates and
; quick re-runs). Never fatal: Velox works for direct downloads without them.
!macro FETCH_MEDIA_TOOLS
  IfFileExists "$INSTDIR\bin\yt-dlp.exe" 0 do_fetch
  IfFileExists "$INSTDIR\bin\ffmpeg.exe" 0 do_fetch
  IfFileExists "$INSTDIR\bin\ffprobe.exe" 0 do_fetch
    DetailPrint "YouTube tooling already present — skipping the download."
    Goto fetch_done
do_fetch:
  DetailPrint "Downloading the YouTube toolchain (yt-dlp + LGPL FFmpeg, ~120 MB)..."
  CreateDirectory "$INSTDIR\bin"
  nsExec::ExecToLog 'powershell -NoProfile -ExecutionPolicy Bypass -File "$INSTDIR\velox-fetch.ps1" -DestDir "$INSTDIR\bin"'
  Pop $0
  IntCmp $0 0 fetch_done 0 fetch_done
    StrCmp $UpdateMode 1 0 fetch_ui
      DetailPrint "YouTube tooling could not be downloaded (exit $0) — Velox is updated; retry later or install yt-dlp/FFmpeg manually."
      Goto fetch_done
    fetch_ui:
      MessageBox MB_ICONEXCLAMATION|MB_TOPMOST \
        "The YouTube toolchain (yt-dlp / FFmpeg) could not be downloaded.$\n$\nVelox is installed and works for direct downloads. To add YouTube support later, run this installer again with an internet connection, or see Settings in the app.$\n$\nDetails are in the install log."
fetch_done:
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
  ; /NS and /D are accepted and ignored (Tauri NSIS compatibility)
FunctionEnd

; ---- Core (required) ----
Section "Velox download manager" SEC_CORE
  SectionIn RO
  SetOutPath "$INSTDIR"
  StrCmp $UpdateMode 1 0 interactive
    !insertmacro CLOSEVELOX_UPDATEWAIT
    Goto files
interactive:
    !insertmacro CLOSEVELOX_GRACEFUL
files:

  ; --- application ---
  File "/oname=Velox.exe" "${BUILD_STAGING}${SEP}Velox.exe"

  ; --- browser ecosystem: extension assets + native messaging host ---
  File "/oname=velox-browser-host.exe" "${BUILD_STAGING}${SEP}velox-browser-host.exe"
  SetOutPath "$INSTDIR\extension"
  File "/oname=manifest.json" "${BUILD_STAGING}${SEP}extension${SEP}manifest.json"
  File "/oname=background.js" "${BUILD_STAGING}${SEP}extension${SEP}background.js"
  File "/oname=popup.html" "${BUILD_STAGING}${SEP}extension${SEP}popup.html"
  File "/oname=popup.css" "${BUILD_STAGING}${SEP}extension${SEP}popup.css"
  File "/oname=popup.js" "${BUILD_STAGING}${SEP}extension${SEP}popup.js"
  File "/oname=README.md" "${BUILD_STAGING}${SEP}extension${SEP}README.md"
  SetOutPath "$INSTDIR\extension\icons"
  File "/oname=16.png" "${BUILD_STAGING}${SEP}extension${SEP}icons${SEP}16.png"
  File "/oname=32.png" "${BUILD_STAGING}${SEP}extension${SEP}icons${SEP}32.png"
  File "/oname=48.png" "${BUILD_STAGING}${SEP}extension${SEP}icons${SEP}48.png"
  File "/oname=128.png" "${BUILD_STAGING}${SEP}extension${SEP}icons${SEP}128.png"
  SetOutPath "$INSTDIR"
  DetailPrint "Registering the Velox native messaging host (Chromium browsers)..."
  nsExec::ExecToLog '"$INSTDIR\velox-browser-host.exe" --register'
  Pop $0

  ; --- licences & notices (bundled; FFMPEG-LICENSE.txt arrives with the
  ;     optional download when the component is selected) ---
  File "/oname=THIRD-PARTY-NOTICES.md" "${BUILD_STAGING}${SEP}THIRD_PARTY_NOTICES.md"

  ; --- online media fetcher (runs only when the component is selected) ---
  File "/oname=velox-fetch.ps1" "${BUILD_STAGING}${SEP}velox-fetch.ps1"

  ; --- verify the bundled core actually landed ---
  StrCpy $3 0
  IfFileExists "$INSTDIR\Velox.exe" +2 0
    StrCpy $3 1
  IfFileExists "$INSTDIR\velox-browser-host.exe" +2 0
    StrCpy $3 1
  IfFileExists "$INSTDIR\extension\manifest.json" +2 0
    StrCpy $3 1
  IfFileExists "$INSTDIR\velox-fetch.ps1" +2 0
    StrCpy $3 1
  IntCmp $3 0 files_ok
    StrCmp $UpdateMode 1 0 verify_ui
      SetErrorLevel 3
      Quit
    verify_ui:
      MessageBox MB_ICONEXCLAMATION|MB_TOPMOST \
        "The installation is incomplete (a required file is missing).$\nPlease re-run the installer or download it again from the release page."
      SetErrorLevel 3
      Quit
  files_ok:

  ; --- WebView2 runtime (bundled fallback; interactive UI only on failure) ---
  DetailPrint "Checking WebView2 runtime..."
  ReadRegStr $0 HKLM "${WEBVIEW2_KEY_W}" "pv"
  StrCmp $0 "" 0 runtime_ok
  ReadRegStr $0 HKLM "${WEBVIEW2_KEY_L}" "pv"
  StrCmp $0 "" 0 runtime_ok
  ReadRegStr $0 HKCU "${WEBVIEW2_KEY_L}" "pv"
  StrCmp $0 "" 0 runtime_ok
  File "/oname=WebView2Setup.exe" "${BUILD_STAGING}${SEP}MicrosoftEdgeWebview2Setup.exe"
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

; ---- Optional: YouTube Video Downloader (yt-dlp + LGPL FFmpeg, online) ----
Section "YouTube Video Downloader (yt-dlp + FFmpeg, downloaded at install)" SEC_MEDIA
  SetOutPath "$INSTDIR"
  !insertmacro FETCH_MEDIA_TOOLS

  ; verify what the fetcher produced (informational: failures are already
  ; reported honestly by the macro; this only annotates the log)
  IfFileExists "$INSTDIR\bin\yt-dlp.exe" +2 0
    DetailPrint "note: bin\yt-dlp.exe not present (component failed or skipped)"
  IfFileExists "$INSTDIR\bin\ffmpeg.exe" +2 0
    DetailPrint "note: bin\ffmpeg.exe not present (component failed or skipped)"
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

; ---- component descriptions (shown on the components page) ----
LangString DESC_CORE ${LANG_ENGLISH} "Velox itself: the download manager app, browser extension and native messaging host. Required."
LangString DESC_MEDIA ${LANG_ENGLISH} "YouTube Video Downloader support: downloads yt-dlp and LGPL FFmpeg (~120 MB) from their official sources during installation and installs them beside Velox. Needed for downloading and merging YouTube video/audio; leave unchecked to skip the download."

!insertmacro MUI_FUNCTION_DESCRIPTION_BEGIN
  !insertmacro MUI_DESCRIPTION_TEXT ${SEC_CORE} $(DESC_CORE)
  !insertmacro MUI_DESCRIPTION_TEXT ${SEC_MEDIA} $(DESC_MEDIA)
!insertmacro MUI_FUNCTION_DESCRIPTION_END

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
  Delete "$INSTDIR\bin\yt-dlp.exe"
  Delete "$INSTDIR\bin\ffmpeg.exe"
  Delete "$INSTDIR\bin\ffprobe.exe"
  RMDir "$INSTDIR\bin"
  Delete "$INSTDIR\yt-dlp.exe"
  Delete "$INSTDIR\ffmpeg.exe"
  Delete "$INSTDIR\ffprobe.exe"
  Delete "$INSTDIR\velox-browser-host.exe"
  Delete "$INSTDIR\velox-fetch.ps1"
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
