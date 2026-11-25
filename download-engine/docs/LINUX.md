# Velox on Linux

Velox supports Linux x86_64 and aarch64 as a **per-user application** — no root
required, and your downloads/settings are never touched by updates or
uninstall. Feature parity with Windows is complete (download engine, queues,
browser extension via native messaging, themes, YouTube MP4/MP3 with quality
selection).

## Requirements

- WebKitGTK 4.1 + GTK3 (present on current Ubuntu 22.04+, Debian 12+, Fedora,
  Arch; see Troubleshooting if missing)
- glibc ≥ 2.31 (Ubuntu 20.04 era or newer)
- Optional: `xdg-user-dirs` so the engine finds your localized Downloads folder

## Install

### Online (downloads at install time)

```sh
sh velox-setup-linux-online.sh            # app + YouTube toolchain (~120 MB)
sh velox-setup-linux-online.sh --no-media # app only; add media tools later
```

The script detects your architecture (`x86_64`/`aarch64`), downloads the app
core from the GitHub release and the media tools from their **official**
sources ([yt-dlp](https://github.com/yt-dlp/yt-dlp),
[FFmpeg LGPL builds](https://github.com/BtbN/FFmpeg-Builds)), then installs
per-user.

### Offline (everything bundled)

```sh
tar -xzf Velox-linux-x64-<version>-offline.tar.gz
cd velox-<version> && ./install.sh
```

`yt-dlp` and LGPL `ffmpeg`/`ffprobe` are already in `bin/` — no network needed.

### What gets installed (per-user)

| Path | Contents |
|---|---|
| `~/.local/share/velox/` | app binary, `bin/` (yt-dlp, ffmpeg, ffprobe), `extension/`, licenses |
| `~/.local/bin/velox` | launcher symlink |
| `~/.local/share/applications/io.velox.downloadmanager.desktop` | menu entry |
| `~/.local/share/icons/hicolor/{32x32,128x128,256x256}/apps/` | icons |

**User data lives outside the app directory** (updates and uninstall never
touch it):

| Data | Path |
|---|---|
| Downloads | `~/Downloads/Velox` (changeable in Settings) |
| Settings + themes | `~/.local/share/io.velox.downloadmanager` (XDG) |

## Updating

Re-run the newer installer (online script or offline `install.sh`) — it
overwrites `~/.local/share/velox` only; settings, themes and downloads are
kept. There is no in-app auto-update on Linux by design: the Tauri updater
mechanism is Windows-specific for this app, so the Settings → Updates screen
links to the releases page instead of pretending.

## Uninstall

```sh
~/.local/share/velox/install.sh --uninstall
```

Removes the app, launcher, menu entry and icons. Downloads
(`~/Downloads/Velox`) and settings (`~/.local/share/io.velox.downloadmanager`)
are intentionally preserved; delete them manually if you want a full wipe.

## Browser extension

Settings → Browser works identically to Windows: Velox detects
Chrome/Chromium/Edge/Brave/Vivaldi/Opera (binary on PATH or per-user config
dir present), stages the extension, and registers the native messaging host
via `velox-browser-host --register` (manifests land in
`~/.config/<browser>/NativeMessagingHosts/`). Firefox needs the manual
`about:debugging` flow on every platform — its security model forbids
programmatic installs.

## Troubleshooting

- **"error while loading shared libraries: libwebkit2gtk-4.1.so.0"** — install
  the runtime package: Ubuntu/Debian `sudo apt install libwebkit2gtk-4.1-0`;
  Fedora `webkit2gtk4.1`; Arch `webkit2gtk-4.1`.
- **YouTube downloads fail with bot-wall / sign-in errors** — update the
  toolchain: replace `~/.local/share/velox/bin/yt-dlp` with the latest
  `yt-dlp_linux` from yt-dlp releases (yt-dlp must be updated frequently as
  YouTube changes), or re-run the online installer.
- **MP4 merging or MP3 conversion errors** — check `bin/ffmpeg` exists and is
  executable; replace it from the same BtbN LGPL URL if needed.
- **Wayland** — Velox runs through WebKitGTK; if windows render blank under
  pure Wayland, launch with `GDK_BACKEND=x11`.
- **"velox: command not found"** — add `~/.local/bin` to PATH (the installer
  tells you when this is needed).
- Check the engine's view of the world in **Settings → Advanced** and the
  per-download error history for the exact failure reason.

## Packaging notes (for maintainers)

- The offline tarball and online script are built by CI
  (`.github/workflows/windows.yml` → `linux-desktop` job) on ubuntu-22.04 for
  an old-glibc baseline, with a real startup smoke test on xvfb.
- Tools are installed to `bin/` beside the binary — the same layout as
  Windows — and the app discovers `bin/` first, then the legacy flat layout,
  then PATH.
- AppImage/`.deb` packaging was evaluated and deliberately deferred: the
  linuxdeploy ELF-patching pipeline is fragile with three bundled sidecar
  binaries, while the tar.gz + install.sh covers every distro the deb would.
