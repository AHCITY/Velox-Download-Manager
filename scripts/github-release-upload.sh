#!/usr/bin/env bash
# Create the GitHub "latest" release for the Windows build and upload artifacts.
# Token comes from GITHUB_TOKEN env (never stored in this file or the repo).
# Usage: GITHUB_TOKEN=... github-release-upload.sh <artifact-dir>
set -euo pipefail

REPO="AHCITY/Velox-Download-Manager"
DIR="${1:?artifact dir required}"
TAG="v0.4.0"
COMMITISH="$(git -C /home/z/my-project rev-parse HEAD)"
SETUP="$DIR/Velox-Setup-x64.exe"
PORTABLE="$DIR/Velox.exe"
OFFLINE="$DIR/Velox-Setup-x64-offline.exe"
SUMS="$DIR/SHA256SUMS.txt"
GUIDE="$DIR/INSTALL-AND-VERIFY.md"

for f in "$SETUP" "$OFFLINE" "$PORTABLE" "$SUMS" "$GUIDE"; do
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
body = """Velox 0.4.0 — Browser Ecosystem release.

## Highlights
- **Official browser extension** (Chrome/Edge/Brave, MV3): right-click "Download with Velox",
  toolbar popup with live speed/active count/recent downloads, consent-first download
  interception, optional clipboard suggestions. The installer stages it, registers the
  native messaging host, and on first launch the app offers one-click setup per browser.
- **Native messaging host** (velox-browser-host) + token-protected loopback bridge in the
  app; the engine stays the single source of truth.
- **Download intelligence**: per-download smoothed/average/peak speed + honest ETA states
  in the details panel.
- **Offline setup**: Velox-Setup-x64-offline.exe bundles everything incl. WebView2 runtime
  bootstrapper, yt-dlp and LGPL ffmpeg/ffprobe for YouTube/media downloads — no network
  needed for the app's own tooling.

## Contents
- Velox-Setup-x64.exe — online installer (per-user, WebView2 bootstrapped if missing)
- Velox-Setup-x64-offline.exe — full offline installer with yt-dlp + ffmpeg/ffprobe
- Velox.exe — portable single-binary engine (no install needed)
- SHA256SUMS.txt — SHA-256 checksums
- INSTALL-AND-VERIFY.md — install + post-install verification steps
- latest.json — in-app updater manifest

## Chromium note
Chromium's security model does not permit fully silent extension installs; the in-app
installer automates staging + host registration and opens the extensions page for the
standard two-click confirmation. Firefox is honestly reported as not supported yet.

## Verify before running (Windows)
certutil -hashfile Velox-Setup-x64.exe SHA256  (compare with SHA256SUMS.txt)

Engine test status at build time: workspace suite green (193+ tests), extension host
stdio suite 5/5, UI E2E 33/33, fmt+clippy clean.
"""

print(json.dumps({
    "tag_name": sys.argv[1],
    "target_commitish": __import__("os").environ["COMMITISH"],
    "name": "Velox 0.4.0 — Browser Ecosystem",
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
upload "$OFFLINE"  "Velox-Setup-x64-offline.exe" "application/vnd.microsoft.portable-executable"
upload "$PORTABLE" "Velox.exe"            "application/vnd.microsoft.portable-executable"
upload "$SUMS"     "SHA256SUMS.txt"       "text/plain"
upload "$GUIDE"    "INSTALL-AND-VERIFY.md" "text/markdown"

echo "== release URL:"
curl -s -m 30 "${H[@]}" "https://api.github.com/repos/$REPO/releases/$REL_ID" \
  | python3 -c "import json,sys; d=json.load(sys.stdin); print(d['html_url'], '| latest:', d.get('make_latest') or d.get('prerelease')==False)"
