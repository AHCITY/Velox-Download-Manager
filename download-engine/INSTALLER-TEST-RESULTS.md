# INSTALLER-TEST-RESULTS

Date: 2026-11-20.

## Status

**The installer was not built in this workspace and no installation test was executed.**
Building `Velox-Setup-x64.exe` requires the Windows MSVC toolchain + NSIS (Tauri bundler);
this Linux sandbox cannot produce it (`ring` needs clang-cl for cross-compile; webkit2gtk
and apt are unavailable for a native build). No artifact is fabricated.

What EXISTS and is committed:
- The complete Tauri 2 application (code, config, capabilities, icons) ready to build.
- The CI desktop job that builds and uploads the installer on `windows-latest`.
- The exact local build commands (RELEASE-AND-UPDATE-GUIDE.md).

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
