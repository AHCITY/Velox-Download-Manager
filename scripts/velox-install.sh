#!/bin/sh
# Velox per-user install (Linux) — no root required.
#
# Layout (mirrors docs/LINUX.md):
#   app:      ~/.local/share/velox/           (Velox, bin/, extension/, licenses)
#   launcher: ~/.local/bin/velox              (symlink)
#   menu:     ~/.local/share/applications/io.velox.downloadmanager.desktop
#   icons:    ~/.local/share/icons/hicolor/{32x32,128x128,256x256}/apps/
#   user data stays OUTSIDE the app dir (survives updates and uninstall):
#     downloads: ~/Downloads/Velox
#     settings+themes: ~/.local/share/io.velox.downloadmanager
#
# Usage:  ./install.sh [--uninstall]
set -e

APP_DIR="${HOME}/.local/share/velox"
BIN_DIR="${HOME}/.local/bin"
APPS_DIR="${HOME}/.local/share/applications"
ICON_BASE="${HOME}/.local/share/icons/hicolor"
DESKTOP_ID="io.velox.downloadmanager"

if [ "$1" = "--uninstall" ]; then
  rm -rf "$APP_DIR"
  rm -f "$BIN_DIR/velox" "$APPS_DIR/$DESKTOP_ID.desktop"
  for s in 32x32 128x128 256x256; do
    rm -f "$ICON_BASE/$s/apps/$DESKTOP_ID.png"
  done
  command -v update-desktop-database >/dev/null 2>&1 && update-desktop-database "$APPS_DIR" 2>/dev/null || true
  echo "Velox removed (user data untouched):"
  echo "  downloads: ~/Downloads/Velox"
  echo "  settings + themes: ~/.local/share/io.velox.downloadmanager"
  exit 0
fi

SRC="$(cd "$(dirname "$0")" && pwd)"
[ -f "$SRC/Velox" ] || { echo "ERROR: Velox binary not found next to install.sh"; exit 1; }

mkdir -p "$APP_DIR" "$BIN_DIR" "$APPS_DIR"
# copy everything EXCEPT this script's own tarball siblings that don't belong
cp -r "$SRC/extension" "$APP_DIR/" 2>/dev/null || true
[ -d "$SRC/bin" ] && cp -r "$SRC/bin" "$APP_DIR/"
cp "$SRC/Velox" "$APP_DIR/Velox"
[ -f "$SRC/THIRD-PARTY-NOTICES.md" ] && cp "$SRC/THIRD-PARTY-NOTICES.md" "$APP_DIR/"
[ -f "$SRC/FFMPEG-LICENSE.txt" ] && cp "$SRC/FFMPEG-LICENSE.txt" "$APP_DIR/"
[ -f "$SRC/README-LINUX.txt" ] && cp "$SRC/README-LINUX.txt" "$APP_DIR/"

chmod 755 "$APP_DIR/Velox"
if [ -d "$APP_DIR/bin" ]; then
  for t in "$APP_DIR"/bin/*; do
    [ -f "$t" ] && chmod 755 "$t"
  done
fi

ln -sf "$APP_DIR/Velox" "$BIN_DIR/velox"

for pair in "32x32:$SRC/icons/32x32.png" "128x128:$SRC/icons/128x128.png" "256x256:$SRC/icons/128x128@2x.png"; do
  size="${pair%%:*}"; icon="${pair#*:}"
  [ -f "$icon" ] || continue
  mkdir -p "$ICON_BASE/$size/apps"
  cp "$icon" "$ICON_BASE/$size/apps/$DESKTOP_ID.png"
done

cat > "$APPS_DIR/$DESKTOP_ID.desktop" <<EOF
[Desktop Entry]
Type=Application
Name=Velox
GenericName=Download Manager
Comment=Fast, resumable multi-connection download manager
Exec=$APP_DIR/Velox
Icon=$DESKTOP_ID
Terminal=false
Categories=Network;FileTransfer;
StartupWMClass=Velox
EOF

command -v update-desktop-database >/dev/null 2>&1 && update-desktop-database "$APPS_DIR" 2>/dev/null || true
command -v gtk-update-icon-cache >/dev/null 2>&1 && gtk-update-icon-cache -qtf "$ICON_BASE" 2>/dev/null || true

echo "Velox installed:"
echo "  app:     $APP_DIR"
echo "  command: $BIN_DIR/velox  (menu entry: Velox)"
case ":$PATH:" in
  *":$BIN_DIR:"*) ;;
  *) echo "  note: add $BIN_DIR to your PATH to use 'velox' from the terminal";;
esac
[ -d "$APP_DIR/bin" ] && echo "  YouTube: yt-dlp + FFmpeg bundled in $APP_DIR/bin (ready)"
echo "Launch 'Velox' from your app menu, or run: $BIN_DIR/velox"
