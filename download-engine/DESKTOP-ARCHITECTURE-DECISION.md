# DESKTOP-ARCHITECTURE-DECISION

Date: 2026-11-20. Required decision: build the Windows installer now, or defer?

## Decision: DEFER the installer. Ship the engine + API v2 + web UI as the product core; make Tauri 2 packaging the next dedicated milestone.

This is not a rejection of the Windows desktop goal — it is the honest gate outcome. The
request's own criteria for a "proper first release" cannot all be met and *tested* today:

1. **No Windows build/test environment.** An installer (MSI/NSIS via Tauri bundler) must be
   BUILT on Windows and TESTED on Windows: install → launch → download → pause/resume →
   uninstall → WebView2 runtime handling. This sandbox has no Windows runner, no clang-cl,
   no NSIS/WiX. Anything produced here would be an untested artifact — exactly the
   "fake, incomplete, or broken installer" the request forbids.
2. **Windows CI has not run even once.** Packaging on top of an unverified Windows base
   multiplies risk; the correct order is CI-green → packaging milestone.
3. **Single-instance, updater, and auto-start behaviors are design work** that deserves its
   own milestone rather than being rushed.

## Architecture decision (for the future packaging milestone)

**Tauri 2 with the engine IN-PROCESS as a Rust library — not a sidecar process.**

- The engine crates are already library-shaped (`engine-core` + `engine-http` behind
  traits). A Tauri app can own a `DownloadManager` directly and expose it to the webview as
  Tauri commands — no HTTP hop, no port management, no process lifecycle babysitting, and
  crucially **no unauthenticated local HTTP service** exposed beyond the app (the current
  `serve-http` transport remains for the browser preview and remote/CLI use only).
- The existing vanilla `web/index.html` UI carries over: it already speaks a strict
  request/response + event-stream model. The Tauri shell maps `POST /rpc`-equivalents to
  commands and the SSE stream to an event channel; the UI code changes little.
- WebView2 on Windows 10 x64: documented auto-install; single-instance via tauri-plugin;
  updates via tauri-bundler signed installers; data in `%APPDATA%\Velox`; default download
  dir `Downloads\Velox` (respecting XDG on Linux).
- Security: engine commands are invoked through the Tauri IPC allowlist (origin-pinned),
  replacing the loopback HTTP surface entirely inside the packaged app.

## What this milestone did to make packaging straightforward

- Engine fully usable as a library (independent of any frontend; verified: engine crates
  contain no UI references).
- The web UI is self-contained (single file, no build step) and engine-independent —
  it renders a clean disconnected state when the backend is absent.
- All CLI lifecycle paths are now portable (Ctrl-C, serve/serve-http).
- Reliability fixes this milestone (queue non-blocking starts, pause-while-queued honored,
  panic-safe runner, cancel-vs-completed races, stale-SSE-progress suppression) remove the
  classes of bugs that would otherwise surface first in a packaged app.

## Installer gate checklist (next milestone's entry criteria)

1. `windows.yml` green on a real runner (all gates).
2. Tauri scaffold with engine-in-process commands; local smoke on a Windows machine or CI.
3. MSI/NSIS build + install/uninstall + WebView2 runtime path verified on Windows 10 x64.
4. Single-instance + data-dir + default-download-dir behaviors implemented and tested.
