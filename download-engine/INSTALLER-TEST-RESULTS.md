# INSTALLER-TEST-RESULTS

Date: 2026-11-22 (refresh; original 2026-11-20).

## Status

**Installers ARE built and published from this workspace** (the 2026-11-20 note is
superseded — the cargo-xwin + clang-cl cross toolchain and NSIS are now provisioned
here; see scripts/TOOLCHAIN-SETUP.md):

- v0.3.1: `Velox-Setup-x64.exe` published, updater-manifest verified.
- v0.4.0 (2026-11-22): online `Velox-Setup-x64.exe` (6.0 MB) AND offline
  `Velox-Setup-x64-offline.exe` (147.5 MB, bundles WebView2 bootstrapper, yt-dlp,
  LGPL ffmpeg/ffprobe, extension + native host with install-time NM registration).
  Published as the "latest" release with signed `latest.json`; public asset URLs
  verified HTTP 200.

What is STILL untested (honest):
- A real Windows-machine install/uninstall pass (per-user install, WebView2
  bootstrap path, Start-menu shortcuts, native-host registration end-to-end).
- A real in-app update hop (e.g. 0.3.1 → 0.4.0 via the updater on Windows).

What EXISTS and is committed:
- The complete Tauri 2 application (code, config, capabilities, icons).
- CI desktop job (`windows.yml`) that builds the installer on `windows-latest`.
- NSIS v0.4.0 UNIFIED template (scripts/installer-unified-template.nsi — the
  single Velox-Setup-x64.exe, Phase 5; replaces the former online/offline pair)
  + packaging/signing scripts used for the releases.

## Test plan (execute on a Windows 10 22H2 x64 / Windows 11 x64 machine after the CI run)

1. Build: push → CI `desktop` job → download `Velox-Setup-x64.exe` artifact.
2. Install: run the setup (per-user, no admin expected); accept the default
   `%LOCALAPPDATA%\Programs` location; verify WebView2 bootstrapper completes if runtime
   is absent (test on a VM without Edge if feasible).
3. Launch from the Start Menu shortcut "Velox" — a native window titled
   "Velox — Download Manager" appears; the connection pill reads Connected (in-process
   engine, no terminal involved).
4. Add a real HTTP test file URL; verify progress, speed, ETA, connection count render.
5. Pause → bytes freeze; Resume → progress continues from the frozen offset.
6. Add a second download; Cancel it — row disappears, no orphan part files.
7. Complete the first download; verify the file exists in `Downloads\Velox`; verify its
   SHA-256 against the source.
8. Use the ⌘ action → Explorer opens the download folder (validated open_path).
9. Close the app; relaunch from Start Menu — history persisted, completed row intact,
   no phantom active rows.
10. Interrupt (kill via Task Manager) mid-download; relaunch; resume — continues from the
    part-file offset (engine recovery).
11. Upgrade: install a newer version over 0.2.0 — settings and history preserved.
12. Uninstall via Windows Settings → Apps — binaries, shortcuts, registration removed;
    downloaded files and app data (`%APPDATA%\io.velox.downloadmanager`) are NOT silently
    deleted (NSIS per-user uninstaller behavior; document the data-retention choice).
13. Unicode/spaces destination paths; second drive letter (D:\) destination.
14. Record SmartScreen behavior for the unsigned installer (expected: "More info → Run
    anyway").

Result columns to fill after execution: PASS/FAIL per step, OS build, runtime presence,
installer hash, app version. Do not mark any step pass without execution.
