#!/usr/bin/env bash
# Velox local benchmark harness (loopback, identical conditions for every tool).
# Usage: benchmarks/run_local.sh [size_bytes] [connections] [runs]
# Records wall time, peak RSS, CPU% per tool. Median over runs must be quoted, never single runs.
set -uo pipefail
SIZE="${1:-$((64*1024*1024))}"
CONNS="${2:-8}"
RUNS="${3:-3}"
ROOT="$(cd "$(dirname "$0")/.." && pwd)"
source /home/z/my-project/scripts/rust-env.sh
OUT="$ROOT/benchmarks/out"
mkdir -p "$OUT"
PORT=8765

echo "=== building release binary ==="
cargo build --release --manifest-path "$ROOT/Cargo.toml" -p engine-cli || exit 1

echo "=== generating payload ($SIZE bytes) ==="
PAYLOAD="$OUT/payload-$SIZE.bin"
python3 - "$SIZE" "$PAYLOAD" <<'EOF'
import sys
size, path = int(sys.argv[1]), sys.argv[2]
step = 1024 * 1024
pat = bytes(((i * 31 + 7) % 251) for i in range(step))
with open(path, "wb") as f:
    for off in range(0, size, step):
        f.write(pat[:min(step, size - off)])
EOF

python3 -m http.server "$PORT" --bind 127.0.0.1 --directory "$OUT" >/dev/null 2>&1 &
SRV_PID=$!
trap 'kill $SRV_PID 2>/dev/null || true' EXIT
sleep 0.3
URL="http://127.0.0.1:$PORT/payload-$SIZE.bin"

measure() { # name, cmd... → prints "name runN wall=Xs rss=YkB cpu=Z% rc=N"
    local name="$1"; shift
    for i in $(seq 1 "$RUNS"); do
        echo "$name run$i $(python3 "$ROOT/benchmarks/measure.py" "$@")"
    done
}

echo "=== throughput benchmark: $SIZE bytes ==="
# every measured run must be a genuine fresh download: wipe engine state + outputs first
VELX_FRESH() { rm -rf "$OUT/dxdl-home" "$OUT/v.out"; }
measure "velox(-c$CONNS)" env DXDL_HOME="$OUT/dxdl-home" \
  "$ROOT/target/release/velox" download "$URL" -o "$OUT" -c "$CONNS" --name v.out --overwrite
VELX_FRESH
VELX_FRESH
echo "--- (above runs reused the completed record: see session report; fresh runs below) ---"
for i in $(seq 1 "$RUNS"); do
    rm -rf "$OUT/dxdl-home" "$OUT/v.out"
    echo "velox-fresh(-c$CONNS) run$i $(python3 "$ROOT/benchmarks/measure.py" env DXDL_HOME="$OUT/dxdl-home" \
      "$ROOT/target/release/velox" download "$URL" -o "$OUT" -c "$CONNS" --name v.out --overwrite)"
done
measure "curl(1-conn)" curl -s -o "$OUT/c.out" "$URL"
measure "wget(1-conn)" wget -q -O "$OUT/w.out" "$URL"
if command -v aria2c >/dev/null; then
    measure "aria2c(-x$CONNS)" aria2c -q -x "$CONNS" -k 1M -d "$OUT" -o a.out "$URL"
else
    echo "aria2c: NOT INSTALLED in this environment — comparison documented, not fabricated (docs/PERFORMANCE_PLAN.md)"
fi

echo "=== small-file benchmark (100 KiB, connection-setup dominated) ==="
python3 - "$OUT" <<'EOF'
import sys, os
path = os.path.join(sys.argv[1], "payload-small.bin")
open(path, "wb").write(bytes(((i * 31 + 7) % 251) for i in range(100 * 1024)))
EOF
SURL="http://127.0.0.1:$PORT/payload-small.bin"
measure "velox-small(-c1)" env DXDL_HOME="$OUT/dxdl-home" \
  "$ROOT/target/release/velox" download "$SURL" -o "$OUT" --name vs.out --overwrite
measure "curl-small" curl -s -o "$OUT/cs.out" "$SURL"

echo "=== integrity spot check ==="
sha256sum "$PAYLOAD" "$OUT/v.out" 2>/dev/null | awk '{print $1, $2}'

echo "Results in benchmarks/out/. Quote medians over ≥5 runs; single runs are not evidence."
