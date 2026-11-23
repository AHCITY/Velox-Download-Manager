#!/usr/bin/env bash
# Turn-prep: rebuild the Linux rust toolchain, install the tauri CLI (npm) and
# generate the NEW updater signing keypair (the old one died with .secrets/ in
# a sandbox recycle). Logs everything; safe to re-run.
set -uo pipefail
LOG=/home/z/my-project/scripts/ext-turn-prep.log
exec > "$LOG" 2>&1

echo "== [1/3] rust toolchain (project-local)"
bash /home/z/my-project/scripts/setup-rust-linux.sh || exit 1

echo "== [2/3] tauri CLI (npm)"
npm i -g @tauri-apps/cli >/dev/null 2>&1
npx --yes @tauri-apps/cli --version || { echo "tauri-cli FAILED"; exit 1; }

echo "== [3/3] updater keypair (Tauri minisign format)"
KEY=/home/z/my-project/scripts/velox-updater.key
if [ ! -f "$KEY" ]; then
  TAURI_SIGNING_PRIVATE_KEY_PASSWORD="" npx --yes @tauri-apps/cli signer generate -w "$KEY" --password ""
  chmod 600 "$KEY"
fi
ls -la "$KEY" "$KEY.pub"
echo "== round-trip test: sign + verify with the fresh key"
printf 'velox updater key round-trip test\n' > /tmp/rt-test.txt
TAURI_SIGNING_PRIVATE_KEY_PASSWORD="" npx --yes @tauri-apps/cli signer sign -k "$KEY" --password "" /tmp/rt-test.txt >/dev/null 2>&1 && \
npx --yes @tauri-apps/cli signer verify -p "$KEY.pub" /tmp/rt-test.txt 2>/dev/null || \
npx --yes @tauri-apps/cli signer verify -k "$KEY" --password "" /tmp/rt-test.txt
echo "== PREP DONE"
