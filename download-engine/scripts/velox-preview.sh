#!/usr/bin/env bash
# Velox preview launcher — starts a REAL download demo stack:
#   1. throttled_httpd  (a real local HTTP file server, 96 MiB @ ~12 MB/s per connection)
#   2. velox serve-http (engine + JSON-RPC API v2 + SSE + the web UI)
# Prints both URLs; stops cleanly on Ctrl-C (SIGTERM propagates to children).
# Port conflicts are detected and reported instead of failing mysteriously.
set -u
ROOT="$(cd "$(dirname "$0")/.." && pwd)"
cd "$ROOT"

source /home/z/my-project/scripts/rust-env.sh 2>/dev/null || true

DATA_DIR="$ROOT/preview-data"
UI_DIR="$ROOT/web"
FILE_PORT=8099
UI_PORT=3000
mkdir -p "$DATA_DIR/out"

# never write crash dumps into the repo (a previous launcher bug left one)
ulimit -c 0

VELOX_BIN="$ROOT/target/release/velox"
HTTPD_BIN="$ROOT/target/release/examples/throttled_httpd"

# --- port conflict detection ---
port_busy() {
  (exec 3<>"/dev/tcp/127.0.0.1/$1") 2>/dev/null && { exec 3>&- 3<&-; return 0; } || return 1
}
if port_busy "$UI_PORT"; then
  echo "ERROR: port $UI_PORT is already in use (is a Velox preview already running?)" >&2
  echo "       stop it first, or run with a different port: UI_PORT=3001 $0" >&2
  exit 1
fi
if port_busy "$FILE_PORT"; then
  echo "ERROR: port $FILE_PORT is already in use" >&2
  exit 1
fi

# --- binaries must exist: build them if missing ---
if [ ! -x "$VELOX_BIN" ] || [ ! -x "$HTTPD_BIN" ]; then
  echo "release binaries missing — building (first run takes a few minutes)…"
  cargo build --release -p engine-cli -p engine-testserver --example throttled_httpd || exit 1
fi

PIDS=()
cleanup() {
  for p in "${PIDS[@]:-}"; do kill "$p" 2>/dev/null; done
  wait 2>/dev/null
}
trap cleanup EXIT INT TERM

# 1. real test file server (args: MiB, bytes-per-second per connection, startup delay ms)
if [ "${VELOX_NO_TESTSERVER:-0}" != "1" ]; then
  : > "$DATA_DIR/testserver.log"
  "$HTTPD_BIN" 96 12000000 0 > "$DATA_DIR/testserver.log" 2>&1 &
  PIDS+=($!)
  # the test server binds an EPHEMERAL port and prints the real URL — poll for it
  # (a stale hardcoded port made the engine probe a dead URL: the UI hung forever)
  TEST_URL=""
  for _ in $(seq 1 25); do
    TEST_URL=$(grep -o 'http://[^[:space:]]*/file' "$DATA_DIR/testserver.log" | head -n1)
    [ -n "$TEST_URL" ] && break
    sleep 0.2
  done
  if [ -z "$TEST_URL" ]; then
    echo "ERROR: test file server did not report its URL (see $DATA_DIR/testserver.log)" >&2
    exit 1
  fi
fi

# 2. engine + bridge + UI (downloads land in $DATA_DIR/out)
DXDL_HOME="$DATA_DIR" "$VELOX_BIN" serve-http \
  --bind 0.0.0.0 --port "$UI_PORT" --ui-dir "$UI_DIR" --sample-url "${TEST_URL:-}" &
PIDS+=($!)

echo ""
echo "─────────────────────────────────────────────────────────"
echo " Velox preview"
echo "   UI / API      : http://localhost:$UI_PORT"
echo "   Demo download : $TEST_URL  (96 MiB @ ~12 MB/s per connection)"
echo "   Data dir      : $DATA_DIR  (downloads in $DATA_DIR/out)"
echo "   Stop          : Ctrl-C (children are shut down cleanly)"
echo "─────────────────────────────────────────────────────────"

# wait for the engine process; on exit the trap kills the test server too
wait "${PIDS[-1]}"
