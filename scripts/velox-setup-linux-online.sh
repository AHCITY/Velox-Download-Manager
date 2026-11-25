#!/bin/sh
# Velox ONLINE installer (Linux) — per-user, no root.
# Downloads the app core from the GitHub release, then (by default) the
# YouTube toolchain from the OFFICIAL upstream sources:
#   yt-dlp: https://github.com/yt-dlp/yt-dlp  (release binary, GPLv3+)
#   FFmpeg: https://github.com/BtbN/FFmpeg-Builds  (LGPL build, LGPL-3.0)
# Usage: ./velox-setup-linux-online.sh [--no-media]
set -e

REPO="AHCITY/Velox-Download-Manager"
VER="__VELOX_VERSION__"
ARCH="$(uname -m)"
case "$ARCH" in
  x86_64)  YT_ASSET="yt-dlp_linux";            FF_TARGET="linux64-lgpl" ;;
  aarch64) YT_ASSET="yt-dlp_linux_aarch64";    FF_TARGET="linuxarm64-lgpl" ;;
  *) echo "ERROR: unsupported architecture '$ARCH' (x86_64 and aarch64 are supported)"; exit 1 ;;
esac

WANT_MEDIA=1
[ "$1" = "--no-media" ] && WANT_MEDIA=0

TMP="$(mktemp -d)"
trap 'rm -rf "$TMP"' EXIT

echo "==> Downloading Velox $VER (app core)..."
curl -fL --retry 3 "https://github.com/$REPO/releases/download/v$VER/Velox-linux-x64-$VER.tar.gz" -o "$TMP/core.tar.gz"
mkdir "$TMP/app"
tar -xzf "$TMP/core.tar.gz" -C "$TMP/app"

if [ "$WANT_MEDIA" = 1 ]; then
  echo "==> Downloading the YouTube toolchain (yt-dlp + LGPL FFmpeg, ~120 MB)..."
  BIN="$TMP/app/bin"
  mkdir -p "$BIN"
  curl -fL --retry 3 "https://github.com/yt-dlp/yt-dlp/releases/latest/download/$YT_ASSET" -o "$BIN/yt-dlp" \
    || echo "WARN: yt-dlp download failed — YouTube support can be added later (docs/LINUX.md)"
  if [ -f "$BIN/yt-dlp" ]; then
    curl -fL --retry 3 "https://github.com/BtbN/FFmpeg-Builds/releases/latest/download/ffmpeg-master-latest-$FF_TARGET.tar.xz" -o "$TMP/ffmpeg.tar.xz" \
      || echo "WARN: FFmpeg download failed — merging/MP3 needs it (docs/LINUX.md)"
    if [ -f "$TMP/ffmpeg.tar.xz" ]; then
      mkdir "$TMP/ff"
      tar -xJf "$TMP/ffmpeg.tar.xz" -C "$TMP/ff"
      cp "$TMP/ff"/ffmpeg-master-latest-*/bin/ffmpeg "$BIN/ffmpeg"
      cp "$TMP/ff"/ffmpeg-master-latest-*/bin/ffprobe "$BIN/ffprobe"
      LIC="$(find "$TMP/ff" -name 'LICENSE.txt' | head -1)"
      [ -n "$LIC" ] && cp "$LIC" "$TMP/app/FFMPEG-LICENSE.txt"
    fi
  fi
else
  echo "==> Skipping the YouTube toolchain (--no-media)."
fi

echo "==> Installing (per-user, ~/.local)..."
sh "$TMP/app/install.sh"

[ -x "$HOME/.local/share/velox/Velox" ] || { echo "ERROR: install verification failed"; exit 1; }
echo "==> Done. Launch 'Velox' from your app menu or run: $HOME/.local/bin/velox"
