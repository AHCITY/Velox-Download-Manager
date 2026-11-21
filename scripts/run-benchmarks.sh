#!/bin/bash
# Velox Phase-5 benchmarks: throughput, concurrency, memory, resume overhead.
# Results: /tmp/bench/measure/results.txt. Local loopback only — reproducible.
set -u
source /home/z/my-project/scripts/rust-env.sh
ENGINE=/home/z/my-project/download-engine/target/release/velox
MEASURE=/home/z/my-project/scripts/measure.py
B=/tmp/bench
OUT=$B/measure
PORT=8943
SZ=$((256 * 1024 * 1024))   # 256 MiB payload

mkdir -p "$OUT"
# build the payload once (non-compressible pattern)
if [ ! -f "$B/payload.bin" ] || [ "$(stat -c%s "$B/payload.bin")" != "$SZ" ]; then
  python3 - "$B/payload.bin" "$SZ" <<'EOF'
import sys
p, size = sys.argv[1], int(sys.argv[2])
block = bytes(range(256)) * 4096  # 1 MiB non-compressible-ish pattern
with open(p, "wb") as f:
    for _ in range(size // len(block)):
        f.write(block)
EOF
fi

python3 /home/z/my-project/scripts/bench-server.py "$B/payload.bin" $PORT >/dev/null 2>&1 &
SRV=$!
sleep 0.5
URL="http://127.0.0.1:$PORT/file"

report() { printf '%-34s %s\n' "$1" "$2" | tee -a "$OUT/results.txt"; }

{
echo "# Velox benchmarks — $(date +%F) (local loopback, 256 MiB payload, engine $(cd /home/z/my-project/download-engine && git rev-parse --short HEAD 2>/dev/null || echo unknown))"
echo
echo "| scenario | result |"
echo "| --- | --- |"
} > "$OUT/results.txt"

# B1 single connection
rm -rf "$OUT/o1" && mkdir -p "$OUT/o1"
R1=$(python3 "$MEASURE" env DXDL_HOME="$OUT/h1" "$ENGINE" download "$URL" -o "$OUT/o1" -c 1)
report "B1 single-conn 256MiB" "$R1"

# B2 segmented (8 connections)
rm -rf "$OUT/o8" && mkdir -p "$OUT/o8"
R8=$(python3 "$MEASURE" env DXDL_HOME="$OUT/h8" "$ENGINE" download "$URL" -o "$OUT/o8" -c 8)
report "B2 segmented 8-conn 256MiB" "$R8"

# B3 integrity of both (the engine's strict namer may rename; hash whatever it produced)
H1=$(sha256sum "$OUT/o1/"* 2>/dev/null | awk '{print substr($1,1,16)}' | head -1)
H8=$(sha256sum "$OUT/o8/"* 2>/dev/null | awk '{print substr($1,1,16)}' | head -1)
HSRC=$(sha256sum "$B/payload.bin" | cut -c1-16)
report "B3 sha256 first16 (src/c1/c8)" "$HSRC / $H1 / $H8"

# B4 concurrency: 4 x 64MiB
python3 - "$B/payload64.bin" "$((64 * 1024 * 1024))" <<'EOF'
import sys
p, size = sys.argv[1], int(sys.argv[2])
block = bytes(range(255, -1, -1)) * 4096
with open(p, "wb") as f:
    for _ in range(size // len(block)):
        f.write(block)
EOF
python3 /home/z/my-project/scripts/bench-server.py "$B/payload64.bin" $((PORT + 1)) >/dev/null 2>&1 &
SRV2=$!
sleep 0.5
U64="http://127.0.0.1:$((PORT + 1))/file"
rm -rf "$OUT/o4" && mkdir -p "$OUT/o4" "$OUT/h4"
T0=$(date +%s.%N)
DPIDS=""
for i in 1 2 3 4; do
  mkdir -p "$OUT/o4/d$i"
  env DXDL_HOME="$OUT/h4/d$i" "$ENGINE" download "$U64" -o "$OUT/o4/d$i" -c 4 >/dev/null 2>&1 &
  DPIDS="$DPIDS $!"
done
# wait ONLY for the download clients — the servers never exit (bare `wait` hung B4 once)
for p in $DPIDS; do wait "$p"; done
T1=$(date +%s.%N)
report "B4 4 concurrent 64MiB (c=4 each)" "wall=$(python3 -c "print(f'{$T1-$T0:.2f}s')")"

# B5 resume overhead: 128MiB, SIGKILL at ~50%, resume
python3 - "$B/payload128.bin" "$((128 * 1024 * 1024))" <<'EOF'
import sys
p, size = sys.argv[1], int(sys.argv[2])
block = bytes(range(128)) * 8192
with open(p, "wb") as f:
    for _ in range(size // len(block)):
        f.write(block)
EOF
python3 /home/z/my-project/scripts/bench-server.py "$B/payload128.bin" $((PORT + 2)) >/dev/null 2>&1 &
SRV3=$!
sleep 0.5
U128="http://127.0.0.1:$((PORT + 2))/file"
rm -rf "$OUT/or" && mkdir -p "$OUT/or" "$OUT/hr"
env DXDL_HOME="$OUT/hr" "$ENGINE" download "$U128" -o "$OUT/or" -c 8 >/dev/null 2>&1 &
PID=$!
for _ in $(seq 1 200); do
  P=$(find "$OUT/or" -name "*.part-*" -size +60M 2>/dev/null | head -1)
  [ -n "$P" ] && break
  sleep 0.05
done
kill -9 $PID 2>/dev/null; wait $PID 2>/dev/null
R5=$(python3 "$MEASURE" env DXDL_HOME="$OUT/hr" "$ENGINE" download "$U128" -o "$OUT/or" -c 8)
report "B5 resume-from-~60MiB (wall incl. verify)" "$R5"
H5=$(sha256sum "$OUT/or/"* 2>/dev/null | awk '{print substr($1,1,16)}' | head -1)
H5S=$(sha256sum "$B/payload128.bin" | cut -c1-16)
report "B5 resumed file sha256 first16" "$H5 (src $H5S)"

kill $SRV $SRV2 $SRV3 2>/dev/null
echo "done" >> "$OUT/results.txt"
cat "$OUT/results.txt"
