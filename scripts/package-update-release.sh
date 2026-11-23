#!/usr/bin/env bash
# Package a Velox release for the in-app updater:
#   1. sign the makensis-built installer with the release key (tauri signer sign)
#   2. generate latest.json (tauri-plugin-updater contract: signature = .sig CONTENT)
#   3. stage installer + Velox.exe + sig + latest.json + SHA256SUMS.txt
# Usage: package-update-release.sh <installer.exe> <staging-dir>
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

VERSION=$(python3 -c "import json;print(json.load(open('$APP/tauri.conf.json'))['version'])")
mkdir -p "$OUT"
cp "$SETUP" "$OUT/Velox-Setup-x64.exe"

# sign the EXACT installer bytes that will be uploaded — the updater verifies
# the downloaded file against this signature (minisign, pubkey baked into the app)
rm -f "$OUT/Velox-Setup-x64.exe.sig"
# tauri's signer expects the RAW base64 key content (no "untrusted comment" line)
# whether via -k or TAURI_SIGNING_PRIVATE_KEY — pass `tail -1` of the key file.
# --app-version embeds version:X.Y.Z in the trusted comment; the updater plugin
# (2.14+) cross-checks it against latest.json's announced version, so a tampered
# manifest cannot pair a new version number with an old artifact.
TAURI_SIGNING_PRIVATE_KEY="$(tail -1 "$KEY")" TAURI_SIGNING_PRIVATE_KEY_PASSWORD="" \
  tauri signer sign --app-version "$VERSION" "$OUT/Velox-Setup-x64.exe" >/dev/null
[ -f "$OUT/Velox-Setup-x64.exe.sig" ] || { echo "signing produced no .sig"; exit 1; }

# portable engine binary (download-only convenience; not used by the updater)
EXE="$APP/target/x86_64-pc-windows-msvc/release/Velox.exe"
[ -f "$EXE" ] && cp "$EXE" "$OUT/Velox.exe"

SIGNATURE=$(python3 -c "import sys;print(open(sys.argv[1]).read().strip())" "$OUT/Velox-Setup-x64.exe.sig")
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
            "url": "https://github.com/AHCITY/Velox-Download-Manager/releases/latest/download/Velox-Setup-x64.exe",
        },
    },
}
pathlib.Path(out, "latest.json").write_text(json.dumps(manifest, indent=2) + "\n")
print("latest.json:", version, "| signature", len(signature), "chars")
PY
cd "$OUT"
# Velox.exe is optional (only present after an msvc cross-build) — list what exists
SUMS="Velox-Setup-x64.exe"
[ -f "Velox.exe" ] && SUMS="$SUMS Velox.exe"
sha256sum $SUMS > SHA256SUMS.txt
echo "== staged in $OUT (app version $VERSION)"
