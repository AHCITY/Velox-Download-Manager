#!/bin/bash
# Verify the v0.5.0 release and its assets.
# Usage: bash scripts/release-check.sh
set -euo pipefail
TOKEN=$(cat /home/z/my-project/.secrets/github-token)
REPO=AHCITY/Velox-Download-Manager
TAG=v0.5.0
curl -s -H "Authorization: Bearer $TOKEN" \
  "https://api.github.com/repos/$REPO/releases/tags/$TAG" > /tmp/rel.json
python3 - <<'PY'
import json
d = json.load(open('/tmp/rel.json'))
if 'assets' not in d:
    print('no release yet:', d.get('message'))
    raise SystemExit(0)
print('release:', d.get('name'), '| draft:', d.get('draft'), '| prerelease:', d.get('prerelease'))
print('url:', d.get('html_url'))
for a in d.get('assets', []):
    print(f"  asset: {a['name']:<44} {a['size']:>12,} bytes  dl={a['download_count']}")
PY
