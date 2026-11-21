#!/usr/bin/env bash
# Scenario benchmarks: throttled bandwidth + latency, identical conditions per tool.
# Raw output appended to benchmarks/bench-scenarios.log; medians quoted in PERFORMANCE_PLAN §6.
# Usage: benchmarks/run_scenarios.sh
set -uo pipefail
ROOT="$(cd "$(dirname "$0")/.." && pwd)"
source /home/z/my-project/scripts/rust-env.sh
OUT="$ROOT/benchmarks/out"
LOG="$ROOT/benchmarks/bench-scenarios.log"
RUNS="${RUNS:-3}"
mkdir -p "$OUT"

echo "=== building release binary ==="
cargo build --release --manifest-path "$ROOT/Cargo.toml" -p engine-cli || exit 1
cargo build --release --manifest-path "$ROOT/Cargo.toml" -p engine-testserver --example throttled_httpd || exit 1

measure() { # name, cmd...
    local name="$1"; shift
    for i in $(seq 1 "$RUNS"); do
        echo "$name run$i $(python3 "$ROOT/benchmarks/measure.py" "$@")" | tee -a "$LOG"
    done
}

scenario() { # label size_mib rate_bps delay_ms
    local label="$1" size="$2" rate="$3" delay="$4"
    echo "=== scenario: $label (size=${size}MiB rate=${rate}B/s/conn delay=${delay}ms) ===" | tee -a "$LOG"
    ./target/release/examples/throttled_httpd "$size" "$rate" "$delay" > "$OUT/server-url.txt" 2>/dev/null &
    local srv=$!
    sleep 0.5
    local url
    url=$(head -n1 "$OUT/server-url.txt")
    for i in $(seq 1 "$RUNS"); do
        # every measured run is a fresh download (runs 2..N otherwise hit the
        # duplicate-detection "already completed" shortcut — see run_local.sh lesson)
        rm -rf "$OUT/dxdl-home" "$OUT/s.out"
        echo "velox(-c4) [$label] run$i $(python3 "$ROOT/benchmarks/measure.py" env DXDL_HOME="$OUT/dxdl-home" \
            "$ROOT/target/release/velox" download "$url" -o "$OUT" -c 4 --name s.out --overwrite)" | tee -a "$LOG"
    done
    measure "curl(1)  [$label]" curl -s -o "$OUT/c.out" "$url"
    measure "wget(1)  [$label]" wget -q -O "$OUT/w.out" "$url"
    kill "$srv" 2>/dev/null || true
    wait "$srv" 2>/dev/null || true
    sleep 0.2
}

scenario "moderate-32mib" 32 4000000 0
scenario "slow-8mib"      8  256000   0
scenario "latency-8mib"  24  0        120

echo "=== done; medians quoted in docs/PERFORMANCE_PLAN.md §6 ===" | tee -a "$LOG"
