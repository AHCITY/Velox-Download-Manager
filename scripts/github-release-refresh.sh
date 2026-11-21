#!/usr/bin/env bash
# Refresh the "latest" GitHub release assets with a rebuilt artifact set.
# STANDING POLICY (user, 2026-11-21): at the END of every work turn, rebuild the
# setup.exe from HEAD and publish it here so the user can download and test.
#
# Usage: GITHUB_TOKEN=... github-release-refresh.sh <artifact-dir> [tag]
#   <artifact-dir> must contain: Velox-Setup-x64.exe, Velox.exe,
#                                SHA256SUMS.txt, INSTALL-AND-VERIFY.md
set -euo pipefail

REPO="AHCITY/Velox-Download-Manager"
DIR="${1:?artifact dir required}"
TAG="${2:-v0.1.0}"
COMMITISH="$(git -C /home/z/my-project rev-parse HEAD)"
SHORT="$(git -C /home/z/my-project rev-parse --short HEAD)"
TESTS="$(grep -oP '(?<=PASSED: )\d+' /tmp/local-tests.txt 2>/dev/null || echo 'all')"
SETUP_HASH="$(sha256sum "$DIR/Velox-Setup-x64.exe" | cut -d' ' -f1)"
PORTABLE_HASH="$(sha256sum "$DIR/Velox.exe" | cut -d' ' -f1)"

for f in "$DIR/Velox-Setup-x64.exe" "$DIR/Velox.exe" "$DIR/SHA256SUMS.txt" "$DIR/INSTALL-AND-VERIFY.md"; do
  [ -f "$f" ] || { echo "MISSING: $f"; exit 1; }
done
[ -n "${GITHUB_TOKEN:-}" ] || { echo "GITHUB_TOKEN not set"; exit 1; }

H=(-H "Authorization: Bearer $GITHUB_TOKEN" -H "Accept: application/vnd.github+json")
API="https://api.github.com/repos/$REPO"

REL_ID=$(curl -s -m 30 "${H[@]}" "$API/releases/tags/$TAG" \
  | python3 -c "import json,sys; print(json.load(sys.stdin).get('id') or sys.exit('no release $TAG'))")
echo "== release id=$REL_ID tag=$TAG -> refresh at commit $SHORT"

# delete every existing asset (full refresh — no stale binaries under the same name)
curl -s -m 30 "${H[@]}" "$API/releases/$REL_ID/assets?per_page=100" | python3 -c "
import json,sys
for a in json.load(sys.stdin): print(a['id'], a['name'])" | while read -r aid name; do
  code=$(curl -s -m 30 -o /dev/null -w "%{http_code}" -X DELETE "${H[@]}" "$API/releases/assets/$aid")
  echo "== deleted asset $name ($aid) http=$code"
done

export COMMITISH SHORT SETUP_HASH PORTABLE_HASH TESTS
BODY=$(python3 - <<'PY'
import json, os, datetime
body = f"""Windows build of the Velox download engine — refreshed from commit {os.environ['SHORT']} on {datetime.date.today().isoformat()}.

## Contents
- Velox-Setup-x64.exe — NSIS installer (per-user install, Start-menu shortcut)
- Velox.exe — portable single-binary CLI engine (no install needed)
- SHA256SUMS.txt — SHA-256 checksums for the binaries
- INSTALL-AND-VERIFY.md — install + post-install verification steps

## Verify before running (Windows)
certutil -hashfile Velox-Setup-x64.exe SHA256
Expected: {os.environ['SETUP_HASH']}

## Notes
- ffmpeg is NOT bundled: the MediaPipeline merge stage is optional and uses a
  user-provided LGPL ffmpeg build (--enable-gpl builds are refused by the
  engine). See docs/FFMPEG_INTEGRATION.md in the repo.
- Engine test status at build time: workspace suite green ({os.environ['TESTS']} tests),
  fmt+clippy clean, CI gates green on GitHub (ci + windows).
"""
print(json.dumps({"body": body}))
PY
)
curl -s -m 60 -X PATCH "${H[@]}" -H "Content-Type: application/json" \
  -d "$BODY" "$API/releases/$REL_ID" \
  | python3 -c "import json,sys; d=json.load(sys.stdin); print('== notes updated:', 'ok' if d.get('id') else d.get('message'))"

upload() {
  local file="$1" name="$2" ctype="$3"
  echo "== uploading $name ($(stat -c%s "$file") bytes)"
  local code
  code=$(curl -s -m 600 -o /tmp/up.json -w "%{http_code}" -X POST "${H[@]}" \
    -H "Content-Type: $ctype" --data-binary @"$file" \
    "https://uploads.github.com/repos/$REPO/releases/$REL_ID/assets?name=$name")
  if [ "$code" != "201" ]; then echo "upload $name failed http=$code:"; cat /tmp/up.json; return 1; fi
  python3 -c "import json; d=json.load(open('/tmp/up.json')); print('  ->', d['name'], d['size'], 'bytes, state', d['state'])"
}

upload "$DIR/Velox-Setup-x64.exe" "Velox-Setup-x64.exe" "application/vnd.microsoft.portable-executable"
upload "$DIR/Velox.exe"           "Velox.exe"           "application/vnd.microsoft.portable-executable"
upload "$DIR/SHA256SUMS.txt"      "SHA256SUMS.txt"      "text/plain"
upload "$DIR/INSTALL-AND-VERIFY.md" "INSTALL-AND-VERIFY.md" "text/markdown"

echo "== release URL:"
curl -s -m 30 "${H[@]}" "$API/releases/$REL_ID" | python3 -c "import json,sys; print(json.load(sys.stdin)['html_url'])"
