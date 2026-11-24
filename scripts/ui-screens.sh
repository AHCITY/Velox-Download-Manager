#!/bin/bash
# Multi-window-size visual audit of the refined UI (P4/P5): serves the REAL
# engine (velox serve-http --ui-dir web) and screenshots each screen at three
# representative window sizes (desktop default, near-minimum, tauri minimum).
set -uo pipefail
ROOT=/home/z/my-project/download-engine
ENGINE=/tmp/velox-keep/velox
OUT=/tmp/screens
rm -rf "$OUT"; mkdir -p "$OUT"
PORT=8791

pkill -f "velox serve-http" 2>/dev/null || true; sleep 0.5
rm -rf /tmp/velox-shots-data
DXDL_HOME=/tmp/velox-shots-data "$ENGINE" serve-http --port $PORT --ui-dir "$ROOT/web" --out /tmp/velox-shots-out > /tmp/velox-shots-engine.log 2>&1 &
ENG=$!
sleep 1.5
curl -sf "http://127.0.0.1:$PORT/config.json" > /dev/null || { echo "engine did not start"; kill $ENG; exit 1; }

# one seeded completed download so the list is not empty
curl -s -X POST "http://127.0.0.1:$PORT/rpc" -H 'content-type: application/json' \
  -d '{"jsonrpc":"2.0","id":1,"method":"download.create","params":{"url":"http://127.0.0.1:9/settings-demo"}}' >/dev/null

shot() { # $1 route, $2 name, $3 w, $4 h
  agent-browser viewport "$3" "$4" >/dev/null 2>&1
  agent-browser open "http://127.0.0.1:$PORT/#$1" >/dev/null 2>&1
  sleep 1.6
  agent-browser screenshot "$OUT/$2-$3.png" >/dev/null 2>&1
  echo "shot: $2 @ ${3}x${4}"
}
for size in "1440 900" "900 650" "720 520"; do
  set -- $size
  W=$1; H=$2
  shot downloads "downloads" "$W" "$H"
  shot browser "browser" "$W" "$H"
  shot settings "settings" "$W" "$H"
  shot queues "queues" "$W" "$H"
done
# overflow probe: does any screen overflow horizontally at the smallest size?
agent-browser viewport "720" "520" >/dev/null 2>&1
for r in downloads browser settings queues network history; do
  agent-browser open "http://127.0.0.1:$PORT/#$r" >/dev/null 2>&1
  sleep 1
  OVF=$(agent-browser eval "document.documentElement.scrollWidth - document.documentElement.clientWidth" 2>/dev/null | tail -1)
  echo "overflow-x @$r (720px): ${OVF}px"
done
kill $ENG 2>/dev/null
echo "screens in $OUT"; ls "$OUT" | head -20
