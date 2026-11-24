#!/usr/bin/env bash
# Package a Velox release for the in-app updater (UNIFIED installer flow —
# identical asset set to the CI release job):
#   1. sign the makensis-built unified installer with the release key (tauri signer sign)
#   2. generate latest.json (tauri-plugin-updater contract: signature = .sig CONTENT,
#      URL pointing at Velox-Setup-x64-unified.exe)
#   3. stage installer + sig + latest.json + SHA256SUMS.txt (nothing else — the
#      release page stays clean; the unified installer IS the complete package)
# Usage: package-update-release.sh <Velox-Setup-x64-unified.exe> <staging-dir>
set -euo pipefail
SETUP="${1:?installer path required}"
OUT="${2:?staging dir required}"
APP=/home/z/my-project/download-engine/app/src-tauri
# the key is tracked in the repo (dev-build convention, like the extension key);
# .secrets/ remains a legacy fallback location
KEY=/home/z/my-project/scripts/velox-updater.key
[ -f "$KEY" ] || KEY=/home/z/my-project/.secrets/velox-updater.key
[ -f "$SETUP" ] || { echo "missing installer $SETUP"; exit 1; }
[ -f "$KEY" ] || { echo "missing signing key $KEY"; exit 1; }

SETUP_NAME="Velox-Setup-x64-unified.exe"
VERSION=$(python3 -c "import json;print(json.load(open('$APP/tauri.conf.json'))['version'])")
mkdir -p "$OUT"
cp "$SETUP" "$OUT/$SETUP_NAME"

# sign the EXACT installer bytes that will be uploaded — the updater verifies
# the downloaded file against this signature (minisign, pubkey baked into the app)
rm -f "$OUT/$SETUP_NAME.sig"
# tauri's signer expects the RAW base64 key content (no "untrusted comment" line)
# whether via -k or TAURI_SIGNING_PRIVATE_KEY — pass `tail -1` of the key file.
# --app-version embeds version:X.Y.Z in the trusted comment; the updater plugin
# (2.14+) cross-checks it against latest.json's announced version, so a tampered
# manifest cannot pair a new version number with an old artifact.
TAURI_SIGNING_PRIVATE_KEY="$(tail -1 "$KEY")" TAURI_SIGNING_PRIVATE_KEY_PASSWORD="" \
  tauri signer sign --app-version "$VERSION" "$OUT/$SETUP_NAME" >/dev/null
[ -f "$OUT/$SETUP_NAME.sig" ] || { echo "signing produced no .sig"; exit 1; }

SIGNATURE=$(python3 -c "import sys;print(open(sys.argv[1]).read().strip())" "$OUT/$SETUP_NAME.sig")
PUB_DATE=$(date -u +%Y-%m-%dT%H:%M:%SZ)
python3 - "$OUT" "$VERSION" "$SIGNATURE" "$PUB_DATE" <<'PY'
import json, pathlib, sys
out, version, signature, pub_date = sys.argv[1:5]
manifest = {
    "version": version,
    "notes": f"Velox {version} — see the release page for changes.",
    "pub_date": pub_date,
    "platforms": {
        "windows-x86_64": {
            "signature": signature,
            "url": "https://github.com/AHCITY/Velox-Download-Manager/releases/latest/download/Velox-Setup-x64-unified.exe",
        },
    },
}
pathlib.Path(out, "latest.json").write_text(json.dumps(manifest, indent=2) + "\n")
print("latest.json:", version, "| signature", len(signature), "chars")
PY
cd "$OUT"
sha256sum "$SETUP_NAME" "$SETUP_NAME.sig" latest.json > SHA256SUMS.txt
echo "== staged in $OUT (app version $VERSION)"
echo "== assets: $SETUP_NAME + .sig + latest.json + SHA256SUMS.txt"
