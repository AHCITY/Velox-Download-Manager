# Velox Download Manager

[![CI](https://github.com/AHCITY/Velox-Download-Manager/actions/workflows/ci.yml/badge.svg)](https://github.com/AHCITY/Velox-Download-Manager/actions/workflows/ci.yml)
[![Windows](https://github.com/AHCITY/Velox-Download-Manager/actions/workflows/windows.yml/badge.svg)](https://github.com/AHCITY/Velox-Download-Manager/actions/workflows/windows.yml)
[![License: MIT](https://img.shields.io/badge/License-MIT-blue.svg)](LICENSE)

Velox is an open-source desktop download manager for Windows and Linux, built with a Rust transfer engine and a Tauri 2 interface. It provides segmented HTTP/HTTPS downloads, persistent pause and resume, integrity verification, media extraction, and Chromium browser integration.

## Features

- Multi-connection downloads with per-host connection limits.
- Persistent pause, resume, and crash recovery.
- Optional SHA-256 and BLAKE3 integrity verification.
- Media URL extraction through [yt-dlp](https://github.com/yt-dlp/yt-dlp), with stream-copy merging through LGPL FFmpeg builds.
- Google Drive large-file confirmation handling.
- Chrome, Edge, and Brave integration through a browser extension and native messaging host.
- Signed application updates with signature verification.
- **BitTorrent & magnet links** — paste `magnet:?xt=…` or a `.torrent` file; swarm downloads
  live in the same list with pause/resume/crash-recovery (DHT + trackers, librqbit).
- **Thumbnails for every source** — any media source that has a preview image
  (YouTube, Vimeo, SoundCloud, …) keeps it; the built-in themes ship as editable
  manifests in the installation's `themes/` folder.

## Download

Download the latest installers from [GitHub Releases](https://github.com/AHCITY/Velox-Download-Manager/releases/latest).

**Windows 10 / 11** — run **Velox Setup** or **Velox Setup (Offline)** (per-user install, no admin required). The offline installer includes the desktop application, WebView2 bootstrapper, yt-dlp, FFmpeg/ffprobe, and browser integration components.

**Linux (x86_64)** — use the offline archive or the online installer script (per-user install, no root required). See [docs/LINUX.md](docs/LINUX.md) for requirements, updates, and troubleshooting.

User settings and download history are stored separately from the application installation directory.

## Build

### Requirements

- Rust stable toolchain.
- Node.js for frontend tests.
- Tauri 2 platform dependencies for desktop builds.

### Source

```bash
git clone https://github.com/AHCITY/Velox-Download-Manager.git
cd Velox-Download-Manager/download-engine

# Build the Rust workspace
cargo build --release

# Run the browser preview
cargo run -p engine-cli --bin velox -- serve-http --port 8080 --ui-dir ../web

# Build the Windows desktop application from the repository scripts
../scripts/build-velox-msvc.sh
```

### Tests

```bash
cargo test --workspace
cd web && node --test
```

## Architecture

```text
download-engine/
├── app/src-tauri/       Tauri 2 desktop application
├── crates/
│   ├── engine-core/     Download orchestration and state machine
│   ├── engine-http/     HTTP transfers, probing, media extraction
│   ├── engine-media/    FFmpeg media merge integration
│   ├── engine-state/    SQLite persistence
│   ├── engine-api/      JSON-RPC API
│   ├── engine-bridge/   Event transport
│   └── velox-host/      Browser native-messaging host
└── web/                 HTML, CSS, and JavaScript interface
scripts/                 Build, packaging, release, and CI scripts
```

## License

Velox is distributed under the [MIT License](LICENSE).
