#!/bin/bash
# Verify a release and its assets.
# Usage: bash scripts/release-check.sh [tag]   (default: latest release)
set -euo pipefail
TOKEN=$(cat /home/z/my-project/.secrets/github-token)
REPO=AHCITY/Velox-Download-Manager
if [ $# -ge 1 ]; then
  TAG="$1"
  ENDPOINT="tags/$TAG"
else
  ENDPOINT="latest"
fi
curl -s -H "Authorization: Bearer $TOKEN" \
  "https://api.github.com/repos/$REPO/releases/$ENDPOINT" > /tmp/rel.json
python3 - <<'PY'
import json
d = json.load(open('/tmp/rel.json'))
if 'assets' not in d:
    print('no release yet:', d.get('message'))
    raise SystemExit(0)
print('release:', d.get('name'), '| tag:', d.get('tag_name'), '| draft:', d.get('draft'), '| prerelease:', d.get('prerelease'))
print('url:', d.get('html_url'))
for a in d.get('assets', []):
    print(f"  asset: {a['name']:<44} {a['size']:>12,} bytes  dl={a['download_count']}")
# v0.6.0+ release policy: the unified installer plus exactly what the signed
# auto-updater needs — anything else is drift
expected = {'Velox-Setup-x64-unified.exe', 'Velox-Setup-x64-unified.exe.sig',
            'latest.json', 'SHA256SUMS.txt'}
actual = {a['name'] for a in d.get('assets', [])}
if d.get('tag_name') == 'v0.6.0':
    missing = expected - actual
    extra = actual - expected
    if missing: print('MISSING expected assets:', missing)
    if extra:   print('UNEXPECTED assets (release-page clutter):', extra)
    if not missing and not extra: print('asset policy: OK (unified installer + updater trio only)')
PY
