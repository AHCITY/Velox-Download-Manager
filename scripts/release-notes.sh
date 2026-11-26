#!/usr/bin/env bash
# Emits the formal release notes for Velox <version>.
#
# SINGLE SOURCE OF TRUTH for the release page text — used by
# windows.yml (Windows stage), linux.yml (Linux stage) and
# scripts/fix-release-page.sh (page conformance). All three stages write
# the SAME body, so editing notes means editing this file only.
#
# Usage: release-notes.sh <version>
set -euo pipefail

VER="${1:?usage: release-notes.sh <version>}"
REPO_URL="https://github.com/AHCITY/Velox-Download-Manager"

# Quoted heredoc (no shell expansion) + sed placeholder substitution, so the
# markdown backticks never reach the shell.
cat <<'EOF' | sed -e "s/@VER@/$VER/g" -e "s|@REPO@|$REPO_URL|g"
**Velox @VER@** — themes, YouTube format selection, and Linux support.

## Downloads

**Windows (x64)**

1. **Velox Setup** — standard installer. The optional YouTube component downloads yt-dlp and FFmpeg from their official sources during setup.
2. **Velox Setup (Offline)** — complete installer with yt-dlp, FFmpeg (LGPL build), the browser extension and the WebView2 fallback bundled. This is also the auto-update payload.

**Linux (x86_64)**

3. **Velox @VER@ for Linux (Offline)** — app, yt-dlp and FFmpeg bundled. Extract and run `install.sh` (per-user install, no root required).
4. **Velox Online Installer for Linux** — shell installer; downloads the app core and the YouTube toolchain from official sources (`--no-media` skips the toolchain).

**Checksums and metadata**

- `SHA256SUMS.txt` — SHA-256 checksums for every asset
- `latest.json` — signed auto-update manifest (windows-x86_64)
- `Velox-Setup-x64-Offline.exe.sig` — updater signature
- `Velox-linux-x64-@VER@.tar.gz` — app core archive, fetched by the online installer

## Verify

```
sha256sum -c SHA256SUMS.txt          # Linux
certutil -hashfile <file> SHA256     # Windows
```

## Requirements

- Windows 10 or later; WebView2 is installed automatically when missing.
- Linux with WebKitGTK 4.1 and GTK3 (present on current Ubuntu, Debian, Fedora and Arch). See [docs/LINUX.md](@REPO@/blob/main/docs/LINUX.md) for installation, updates and troubleshooting.
EOF
