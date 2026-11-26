# Linux

Velox ships two distributions for x86_64 Linux — an **offline archive** (everything bundled) and an **online installer** (downloads components during setup). Both install per-user; no root is required.

## Requirements

- x86_64 Linux — Ubuntu 22.04+, Debian 12+, Fedora 38+, Arch or an equivalent distribution
- WebKitGTK 4.1 and GTK3 — preinstalled on the desktop editions of those distributions; otherwise:

| Distribution | Install command |
| --- | --- |
| Debian / Ubuntu | `sudo apt install libwebkit2gtk-4.1-0 libgtk-3-0` |
| Fedora | `sudo dnf install webkit2gtk4.1 gtk3` |
| Arch | `sudo pacman -S webkit2gtk-4.1 gtk3` |

## Install — offline

Everything is bundled: the app, yt-dlp and FFmpeg (LGPL build). No network needed.

```bash
tar -xzf 3-Velox-linux-x64-<version>-offline.tar.gz
cd velox-<version>
./install.sh
```

## Install — online

Downloads the app core from the release and the YouTube toolchain from the official upstream sources (yt-dlp, BtbN FFmpeg LGPL build):

```bash
chmod +x 4-velox-setup-linux-online.sh
./4-velox-setup-linux-online.sh             # full install incl. YouTube toolchain (~120 MB)
./4-velox-setup-linux-online.sh --no-media  # skip the YouTube toolchain
```

## Launch

- Application menu: **Velox**
- Terminal: `~/.local/bin/velox`

## Installed layout

| Path | Content |
| --- | --- |
| `~/.local/share/velox/` | app binary, `bin/` tools, browser extension, licenses |
| `~/.local/bin/velox` | launcher (symlink) |
| `~/.local/share/applications/io.velox.downloadmanager.desktop` | menu entry |
| `~/.local/share/icons/hicolor/` | menu icons |
| `~/.local/share/io.velox.downloadmanager/` | settings and themes — outside the app dir, survives updates and uninstall |
| `~/Downloads/Velox/` | completed downloads |

## Update

Install the new version the same way (offline archive or online installer). The app directory is replaced in place; settings, themes and downloads are kept. The in-app auto-updater is currently Windows-only.

## Uninstall

```bash
./install.sh --uninstall
```

Run it from any extracted release directory. User data (settings, themes, downloads) is never removed.

## Verify downloads

From the directory holding the downloaded release assets:

```bash
sha256sum -c SHA256SUMS.txt
```

## Troubleshooting

- **App does not start, missing WebKitGTK** — install the packages listed under Requirements, then start Velox again.
- **YouTube downloads fail** — check that `~/.local/share/velox/bin/yt-dlp` exists (the online installer skips it with `--no-media`). To refresh the tool, replace that file with the official `yt-dlp_linux` binary from the yt-dlp releases page.
- **Merge/MP3 conversion unavailable** — `bin/ffmpeg` and `bin/ffprobe` must sit beside the app binary. The offline archive bundles them; the online installer fetches them unless `--no-media` was used.
- **Checksum mismatch** — re-download the asset; a failed download never masquerades as complete.
