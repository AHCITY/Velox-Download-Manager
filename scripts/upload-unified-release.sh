#!/usr/bin/env bash
# Upload the unified Phase-5 installer as an ADDITIONAL asset on the v0.4.0
# release. The signed Velox-Setup-x64.exe + latest.json pairing stays untouched
# (updater signature = exact bytes of the online installer; replacing either
# would break in-app updates for existing users).
set -uo pipefail
TOKEN="$(cat /home/z/my-project/.secrets/github-token)"
REPO="AHCITY/Velox-Download-Manager"
TAG="v0.4.0"
FILE="/home/z/my-project/download/Velox-Setup-x64.exe"
NAME="Velox-Setup-x64-unified.exe"
SHA="/home/z/my-project/download/Velox-Setup-x64.exe.sha256"

auth() { curl -s -H "Authorization: Bearer $TOKEN" "$@"; }

REL_ID=$(auth "https://api.github.com/repos/$REPO/releases/tags/$TAG" | python3 -c "import json,sys;print(json.load(sys.stdin)['id'])")
[ -n "$REL_ID" ] && echo "release id: $REL_ID"

echo "== uploading $NAME ($(stat -c%s "$FILE") bytes)..."
UP_URL="https://uploads.github.com/repos/$REPO/releases/$REL_ID/assets?name=$NAME"
HTTP=$(curl -s -o /tmp/up-resp.json -w "%{http_code}" -X POST \
  -H "Authorization: Bearer $TOKEN" \
  -H "Content-Type: application/octet-stream" \
  --data-binary @"$FILE" "$UP_URL")
echo "upload HTTP $HTTP"
python3 -c "import json;d=json.load(open('/tmp/up-resp.json'));print('asset:', d.get('name'), d.get('size'), 'state:', d.get('state'))"

echo "== uploading checksum file..."
curl -s -X POST -H "Authorization: Bearer $TOKEN" \
  -H "Content-Type: text/plain" \
  --data-binary @"$SHA" \
  "https://uploads.github.com/repos/$REPO/releases/$REL_ID/assets?name=Velox-Setup-x64-unified.exe.sha256" \
  | python3 -c "import json,sys;d=json.load(sys.stdin);print('asset:', d.get('name'), d.get('size'))"

echo "== done =="
