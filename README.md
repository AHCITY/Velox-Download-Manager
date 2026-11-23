# Velox Download Manager

[![ci](https://github.com/AHCITY/Velox-Download-Manager/actions/workflows/ci.yml/badge.svg)](https://github.com/AHCITY/Velox-Download-Manager/actions/workflows/ci.yml)
[![windows](https://github.com/AHCITY/Velox-Download-Manager/actions/workflows/windows.yml/badge.svg)](https://github.com/AHCITY/Velox-Download-Manager/actions/workflows/windows.yml)
[![License: MIT](https://img.shields.io/badge/License-MIT-blue.svg)](LICENSE)

**A native, resumable download manager for Windows.** Rust download engine, Tauri 2 desktop UI, browser extension integration — with YouTube support powered by yt-dlp and an LGPL FFmpeg stream-copy pipeline.

Velox is not a wrapper around a CLI downloader: the engine runs **in-process** — segmented HTTP/HTTPS transfers, transactional crash-safe resume, strict integrity verification, and a JSON-RPC API, all in Rust. The desktop shell and the browser extension are thin clients over that one engine.

## Features

- **Segmented, multi-connection downloads** with dynamic work stealing and per-host connection limits
- **Pause / resume / crash recovery** — transactional checkpointing (SQLite, WAL, `synchronous=FULL`); a killed process resumes exactly where it stopped, byte-for-byte
- **Integrity verification** — optional SHA-256 / BLAKE3 checksums; failed downloads never masquerade as complete
- **YouTube & adaptive media** — URLs are resolved through [yt-dlp](https://github.com/yt-dlp/yt-dlp): progressive streams download directly; adaptive (video-only + audio-only) streams are downloaded separately and merged with **stream-copy** (`-c copy`) — original quality, no transcoding — and validated with ffprobe. DRM and paywalls are never bypassed.
- **Google Drive support** — confirm-page walks for large-file interstitials
- **Browser extension** — one-click native-messaging host setup for Chrome / Edge / Brave; the right-click "download with Velox" flow
- **Signed auto-updates** — minisign-verified updater; installing an update asks first, pauses active downloads, and never force-closes the app
- **Honest error handling** — no fake progress, no silent failures; every state the UI shows is the engine's real state

## Install

Download **`Velox-Setup-x64-unified.exe`** from the [latest release](https://github.com/AHCITY/Velox-Download-Manager/releases/latest) and run it (Windows 10 / 11, per-user install, no admin required).

The installer ships everything:

| Bundled | Purpose |
| --- | --- |
| `Velox.exe` | the desktop app (Tauri 2 / WebView2) |
| `yt-dlp.exe` | media URL extraction (Unlicense) |
| `ffmpeg.exe` / `ffprobe.exe` | LGPL builds, stream-copy merge + validation |
| `extension/` + `velox-browser-host.exe` | browser integration (auto-registered) |
| `WebView2Setup.exe` | fallback runtime installer (used only if WebView2 is missing) |

The app discovers its bundled tools next to its own executable — **no PATH configuration needed**. User data (history, settings, downloads) lives outside the install directory and survives upgrades and uninstall.

Linux/macOS: build from source (below). There is no packaged installer for them yet.

## Build from source

Prerequisites: **Rust stable**, **Node.js** (only for linting the web assets — the frontend is plain ES modules, no build step), and the usual Tauri/Linux webkit deps when building the desktop app on Linux:

```bash
git clone https://github.com/AHCITY/Velox-Download-Manager.git
cd Velox-Download-Manager/download-engine

# engine + CLI
cargo build --release
cargo run -p engine-cli --bin velox -- serve-http --port 8080 --ui-dir ../web   # browser preview

# Windows desktop app (cross-build from Linux, see scripts/TOOLCHAIN-SETUP.md)
../scripts/build-velox-msvc.sh
```

Run the test suites:

```bash
cargo test --workspace          # 26 suites, incl. a real-HTTP adaptive-media e2e
(cd web && node --test)     # updater version-guard + queue orchestrator tests
../scripts/e2e-ui.sh               # 33-check browser E2E (needs the debug CLI binary)
```

## Project layout

```
download-engine/
├── crates/
│   ├── engine-core/      # DownloadManager, task state machine, planning, naming
│   ├── engine-http/      # reqwest backend, probing, yt-dlp extraction, Drive walk
│   ├── engine-media/     # LGPL-gated FFmpeg merge pipeline (stream-copy only)
│   ├── engine-state/     # SQLite persistence (WAL, crash-safe)
│   ├── engine-api/       # JSON-RPC v1 contract
│   ├── engine-bridge/    # SSE/event wire protocol
│   └── velox-host/       # Chrome/Edge/Brave native-messaging host
├── app/src-tauri/        # Tauri 2 desktop shell (engine runs in-process)
└── web/                  # frontend: plain HTML/CSS/JS ES modules
scripts/                  # build, packaging, release and CI helper scripts
```

## Roadmap

- Online/offline installer variant selection (postponed deliberately — one complete installer ships today)
- Mirror/Metalink support, multi-protocol planning
- Packaged Linux builds (AppImage/deb)
- Media playlist support (currently single-media URLs)

## Contributing

Contributions are welcome — issues, bug reports and pull requests alike.

1. **Open an issue first** for anything larger than a small fix, so we can agree on the approach.
2. **Keep PRs focused**: one logical change per PR, with a short description of what and why.
3. **Run the gates before pushing** — CI runs them, and they are the definition of "done":

   ```bash
   cargo fmt --all --check
   cargo clippy --workspace --all-targets -- -D warnings
   cargo test --workspace
   ```

4. **UI changes**: run `scripts/e2e-ui.sh` and keep the 33 checks green.
5. **License**: by contributing you agree that your contributions are licensed under the [MIT License](LICENSE) that covers this project. Do not submit GPL/nonfree FFmpeg builds — the merge pipeline refuses them by policy.

## Acknowledgements

- [yt-dlp](https://github.com/yt-dlp/yt-dlp) — media extraction (invoked as an external executable, never bundled into the engine)
- [FFmpeg](https://ffmpeg.org) — LGPL builds only, stream-copy merging
- [Tauri](https://tauri.app) — desktop shell

## License

[MIT](LICENSE) © 2026 AHCITY
