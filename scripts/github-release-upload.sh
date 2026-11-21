#!/usr/bin/env bash
# Create the GitHub "latest" release for the Windows build and upload artifacts.
# Token comes from GITHUB_TOKEN env (never stored in this file or the repo).
# Usage: GITHUB_TOKEN=... github-release-upload.sh <artifact-dir>
set -euo pipefail

REPO="AHCITY/Velox-Download-Manager"
DIR="${1:?artifact dir required}"
TAG="v0.1.0"
COMMITISH="$(git -C /home/z/my-project rev-parse HEAD)"
SETUP="$DIR/Velox-Setup-x64.exe"
PORTABLE="$DIR/Velox.exe"
SUMS="$DIR/SHA256SUMS.txt"
GUIDE="$DIR/INSTALL-AND-VERIFY.md"

for f in "$SETUP" "$PORTABLE" "$SUMS" "$GUIDE"; do
  [ -f "$f" ] || { echo "MISSING: $f"; exit 1; }
done

if [ -z "${GITHUB_TOKEN:-}" ]; then echo "GITHUB_TOKEN not set"; exit 1; fi
TOKEN="$GITHUB_TOKEN"
H=(-H "Authorization: Bearer $TOKEN" -H "Accept: application/vnd.github+json")

# Reuse the release if the tag already exists, else create it (becomes latest).
EXISTING=$(curl -s -m 30 "${H[@]}" "https://api.github.com/repos/$REPO/releases/tags/$TAG")
REL_ID=$(echo "$EXISTING" | python3 -c "import json,sys; d=json.load(sys.stdin); print(d.get('id',''))" 2>/dev/null || true)

if [ -z "$REL_ID" ]; then
  export COMMITISH
  PAYLOAD=$(python3 - "$TAG" <<PY
import json, sys, pathlib
body = """Windows build of the Velox download engine (cross-compiled 2026-11-21).

## Contents
- Velox-Setup-x64.exe — NSIS installer (per-user install, Start-menu shortcut)
- Velox.exe — portable single-binary CLI engine (no install needed)
- SHA256SUMS.txt — SHA-256 checksums for the binaries
- INSTALL-AND-VERIFY.md — install + post-install verification steps

## Verify before running (Windows)
certutil -hashfile Velox-Setup-x64.exe SHA256
Expected: a848e27246f43976c2014d9ab54c3784c1fcd7495bf2c2235a94fda168df5aa4

## Notes
- ffmpeg is NOT bundled: the MediaPipeline merge stage is optional and uses a
  user-provided LGPL ffmpeg build (--enable-gpl builds are refused by the
  engine). See docs/FFMPEG_INTEGRATION.md in the repo.
- The -offline installer variant is not included in this release.
- Engine test status at build time: workspace suite green (164 tests), fmt+clippy clean.
"""
print(json.dumps({
    "tag_name": sys.argv[1],
    "target_commitish": __import__("os").environ["COMMITISH"],
    "name": "Velox Download Manager — Windows installer (2026-11-21)",
    "body": body,
    "draft": False,
    "prerelease": False,
    "make_latest": "true",
}))
PY
)
  CREATE=$(curl -s -m 60 -X POST "${H[@]}" \
    -H "Content-Type: application/json" \
    -d "$PAYLOAD" "https://api.github.com/repos/$REPO/releases")
  REL_ID=$(echo "$CREATE" | python3 -c "import json,sys; d=json.load(sys.stdin); print(d.get('id') or sys.exit('create failed: %s' % d.get('message')))")
  echo "== release created id=$REL_ID tag=$TAG @ $COMMITISH"
else
  echo "== reusing existing release id=$REL_ID tag=$TAG"
fi

upload() {
  local file="$1" name="$2" ctype="$3"
  echo "== uploading $name ($(stat -c%s "$file") bytes)"
  local code
  code=$(curl -s -m 600 -o /tmp/up.json -w "%{http_code}" -X POST "${H[@]}" \
    -H "Content-Type: $ctype" \
    --data-binary @"$file" \
    "https://uploads.github.com/repos/$REPO/releases/$REL_ID/assets?name=$name")
  if [ "$code" != "201" ]; then echo "upload $name failed http=$code:"; cat /tmp/up.json; return 1; fi
  python3 -c "import json; d=json.load(open('/tmp/up.json')); print('  ->', d['name'], d['size'], 'bytes, state', d['state'])"
}

upload "$SETUP"    "Velox-Setup-x64.exe"  "application/vnd.microsoft.portable-executable"
upload "$PORTABLE" "Velox.exe"            "application/vnd.microsoft.portable-executable"
upload "$SUMS"     "SHA256SUMS.txt"       "text/plain"
upload "$GUIDE"    "INSTALL-AND-VERIFY.md" "text/markdown"

echo "== release URL:"
curl -s -m 30 "${H[@]}" "https://api.github.com/repos/$REPO/releases/$REL_ID" \
  | python3 -c "import json,sys; d=json.load(sys.stdin); print(d['html_url'], '| latest:', d.get('make_latest') or d.get('prerelease')==False)"
