#!/bin/bash
# Audit: makensis A/B — default zlib vs /SOLID lzma on the REAL staged payload.
# Measures: build time, installer size. (Signatures not needed for this test —
# we only measure packaging; the .sig in production covers the final bytes.)
set -e
MAKENSIS=/tmp/nsisx/usr/bin/makensis
export NSISDIR=/tmp/nsisx/usr/share/nsis
TPL=/home/z/my-project/scripts/installer-unified-template.nsi
STAGED=/tmp/installer
OUT=/tmp/installer/out
mkdir -p "$OUT"

echo "=== A: default compressor (current production: zlib) ==="
t0=$(date +%s)
"$MAKENSIS" \
  -DPRODUCT_VERSION=0.6.0 \
  -DBUILD_STAGING="$STAGED" \
  -DBUILD_OUTPUT="$OUT/test-zlib.exe" \
  -DBUILD_ICON_DIR=/home/z/my-project/download-engine/app/src-tauri/icons \
  "$TPL" > "$OUT/makensis-zlib.log" 2>&1 || { tail -20 "$OUT/makensis-zlib.log"; exit 1; }
t1=$(date +%s)
echo "zlib:    build $((t1-t0))s  size $(stat -c%s "$OUT/test-zlib.exe") bytes"

echo "=== B: /SOLID lzma (candidate) ==="
# inject SetCompressor as the first effective line via a wrapper script
cp "$TPL" "$OUT/test-lzma.nsi"
sed -i 's/^Unicode true/Unicode true\nSetCompressor \/SOLID lzma/' "$OUT/test-lzma.nsi"
t0=$(date +%s)
"$MAKENSIS" \
  -DPRODUCT_VERSION=0.6.0 \
  -DBUILD_STAGING="$STAGED" \
  -DBUILD_OUTPUT="$OUT/test-lzma.exe" \
  -DBUILD_ICON_DIR=/home/z/my-project/download-engine/app/src-tauri/icons \
  "$OUT/test-lzma.nsi" > "$OUT/makensis-lzma.log" 2>&1 || { tail -20 "$OUT/makensis-lzma.log"; exit 1; }
t1=$(date +%s)
echo "lzma:    build $((t1-t0))s  size $(stat -c%s "$OUT/test-lzma.exe") bytes"
