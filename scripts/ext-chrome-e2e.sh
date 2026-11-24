#!/bin/bash
# Real-browser extension E2E: Chrome for Testing (Xvfb) + the REAL
# velox-browser-host + the REAL engine behind the bridge contract.
# Chain verified: extension SW -> chrome.runtime.connectNative -> NM stdio ->
# velox-browser-host -> loopback HTTP bridge (fake bridge proxying the REAL
# engine's /rpc) -> engine -> download lands on disk (sha256 verified).
#
# Everything runs under a sandboxed HOME (/tmp/exttest/home): NM manifests and
# bridge.json land there, nothing pollutes the user's real profile.
set -uo pipefail
PROJ=/home/z/my-project
E2E=/tmp/exttest
HOME_DIR=$E2E/home
mkdir -p "$HOME_DIR" "$E2E/files" "$E2E" "$HOME_DIR/.config"

export HOME="$HOME_DIR"
source "$PROJ/scripts/rust-env.sh"
cd "$PROJ/download-engine"

cleanup() {
  for p in "$CHROME_PID" "$BRIDGE_PID" "$ENGINE_PID" "$SERVE_PID" "${XVFB_PID:-}"; do
    [ -n "${p:-}" ] && kill "$p" 2>/dev/null
  done
  pkill -f "remote-debugging-port=9333" 2>/dev/null
}
trap cleanup EXIT

echo "== [1] velox-browser-host (linux, release)"
HOST_BIN=target/release/velox-browser-host
[ -x "$HOST_BIN" ] || cargo build --release -p velox-host 2>&1 | tail -1
[ -x "$HOST_BIN" ] || { echo "FATAL: host binary missing"; exit 1; }

echo "== [2] stage the extension (the way the app stages it)"
rm -rf "$E2E/staged-extension"
cp -r "$PROJ/download-engine/extension" "$E2E/staged-extension"

echo "== [3] register the NM host for every Chromium browser (unix manifests)"
"$HOST_BIN" --register || { echo "FATAL: register failed"; exit 1; }
ls "$HOME_DIR/.config/google-chrome/NativeMessagingHosts/" 2>/dev/null
python3 - "$HOME_DIR" <<'PY'
import json, sys, pathlib
home = sys.argv[1]
for d in ["google-chrome", "microsoft-edge", "chromium", "BraveSoftware/Brave-Browser", "vivaldi"]:
    p = pathlib.Path(home) / ".config" / d / "NativeMessagingHosts" / "io.velox.downloadmanager.json"
    m = json.loads(p.read_text())
    assert m["name"] == "io.velox.downloadmanager" and m["type"] == "stdio", p
    assert any(o.endswith("/ffnpjmbhfmaamhlbohbmamnmoaikpdmc/") for o in m["allowed_origins"]), p
    assert pathlib.Path(m["path"]).exists(), m["path"]
print("OK: NM manifests valid for chrome, edge, chromium, brave, vivaldi (origin allowlist = derived ID, exe present)")
PY
[ $? -eq 0 ] || { echo "FATAL: manifest validation failed"; exit 1; }

echo "== [4] real engine (serve-http) on 9340"
[ -x target/debug/velox ] || cargo build -p engine-cli 2>&1 | tail -1
ENGINE_PID=""
( target/debug/velox serve-http --bind 127.0.0.1 --port 9340 > "$E2E/engine.log" 2>&1 & echo $! > "$E2E/engine.pid" )
ENGINE_PID=$(cat "$E2E/engine.pid")
sleep 1.5
curl -s -X POST -H 'content-type: application/json' -d '{"jsonrpc":"2.0","id":1,"method":"engine.info"}' http://127.0.0.1:9340/rpc | head -c 120; echo

echo "== [5] fake bridge on 9350 proxying the REAL engine"
rm -f /tmp/exttest/bridge-requests.log
python3 "$PROJ/scripts/ext-test-fake-bridge.py" 9350 > "$E2E/bridge.log" 2>&1 &
BRIDGE_PID=$!
VELOX_FAKE_BRIDGE_PROXY=http://127.0.0.1:9340/rpc python3 "$PROJ/scripts/ext-test-fake-bridge.py" 9350 > "$E2E/bridge.log" 2>&1 &
BRIDGE_PID=$!
sleep 0.8

echo "== [6] fixture file server on 9344 (2 MB deterministic big.bin)"
python3 - <<'PY'
import pathlib
p = pathlib.Path("/tmp/exttest/files/big.bin")
p.write_bytes(bytes(range(256)) * 8192)  # exactly 2 MiB, deterministic
print("fixture:", p.stat().st_size, "bytes")
PY
( cd "$E2E/files" && python3 -m http.server 9344 > "$E2E/files.log" 2>&1 & echo $! > "$E2E/files.pid" )
SERVE_PID=$(cat "$E2E/files.pid")
sleep 0.5

echo "== [7] Chrome for Testing under a hand-launched Xvfb (xauth-less)"
CHROME=/home/z/.cache/puppeteer/chrome/linux-157.0.8103.78/chrome-linux64/chrome
[ -x "$CHROME" ] || { echo "FATAL: chrome binary missing"; exit 1; }
rm -rf "$E2E/profile"
mkdir -p "$E2E/profile/NativeMessagingHosts"
# Chrome (Linux) resolves NM manifests relative to the ACTIVE user data dir:
# with --user-data-dir overridden it probes <profile>/NativeMessagingHosts, so
# seed the same manifest there (real machines use the default dirs — covered
# by the --register step above).
for d in google-chrome google-chrome-for-testing; do
  cp "$HOME_DIR/.config/$d/NativeMessagingHosts/io.velox.downloadmanager.json" \
     "$E2E/profile/NativeMessagingHosts/io.velox.downloadmanager.json" 2>/dev/null
done
Xvfb :99 -screen 0 1280x800x24 > "$E2E/xvfb.log" 2>&1 &
XVFB_PID=$!
sleep 1
export DISPLAY=:99
"$CHROME" \
  --user-data-dir="$E2E/profile" \
  --load-extension="$E2E/staged-extension" \
  --remote-debugging-port=9333 \
  --no-first-run --no-default-browser-check \
  --disable-gpu --no-sandbox \
  about:blank > "$E2E/chrome.log" 2>&1 &
CHROME_PID=$!
sleep 4

echo "== [8] service-worker E2E assertions (CDP)"
node "$PROJ/scripts/ext-test-sw-eval.mjs" 9333 "http://127.0.0.1:9344/big.bin"
SW=$?

echo "== [9] engine verdict: did big.bin land on disk through the whole chain?"
OUT_DIR=$(find "$HOME_DIR" -name "big.bin" -path "*Velox*" 2>/dev/null | head -1)
[ -z "$OUT_DIR" ] && OUT_DIR=$(find "$HOME_DIR" -name "big.bin" ! -path "*/files/*" 2>/dev/null | head -1)
if [ -n "$OUT_DIR" ]; then
  ACTUAL=$(sha256sum "$OUT_DIR" | cut -d' ' -f1)
  EXPECT=$(python3 -c "import hashlib;print(hashlib.sha256(bytes(range(256))*8192).hexdigest())")
  if [ "$ACTUAL" = "$EXPECT" ]; then
    echo "OK: $OUT_DIR is byte-exact through extension -> host -> bridge -> engine"
  else
    echo "FAIL: sha mismatch ($ACTUAL != $EXPECT)"; SW=1
  fi
  grep -o '"state":"[a-z]*"' "$HOME_DIR/.local/share/io.velox.downloadmanager/data/"*.db 2>/dev/null | head -1
  python3 - "$HOME_DIR" <<'PY'
import sqlite3, sys, pathlib, glob
dbs = glob.glob(str(pathlib.Path(sys.argv[1]) / ".local/share/io.velox.downloadmanager/**/*.db"), recursive=True) + \
      glob.glob(str(pathlib.Path(sys.argv[1]) / "Downloads/Velox/../..//io.velox.downloadmanager/**/*.db"), recursive=True)
for db in dbs:
    try:
        c = sqlite3.connect(db)
        rows = c.execute("select final_name, state, received, total_size from downloads").fetchall()
        for r in rows: print("engine row:", r)
    except Exception as e:
        pass
PY
else
  echo "FAIL: big.bin never landed on disk"; SW=1
fi

echo "== verdict: $([ $SW -eq 0 ] && echo 'REAL-CHROME E2E PASSED' || echo 'REAL-CHROME E2E FAILED')"
exit $SW
