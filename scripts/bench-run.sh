#!/usr/bin/env bash
# Real benchmark runs: 1x8MiB, 4x8MiB parallel, 60MiB throttled. Prints measured
# numbers only (wall time + throughput) for docs/BENCHMARK.md.
set -u
ROOT=/home/z/my-project/download-engine
VEL="$ROOT/target/debug/velox"
B=/tmp/vbench
rm -rf "$B" && mkdir -p "$B/data" "$B/out" "$B/files"
python3 - <<'PY'
import random, pathlib
d = pathlib.Path('/tmp/vbench/files')
random.seed(11)
(d / 'sample-archive.zip').write_bytes(bytes(random.getrandbits(8) for _ in range(8 * 1024 * 1024)))
print('fixture ok')
PY
(cd "$B/files" && exec python3 -m http.server 8391 --bind 127.0.0.1) >/dev/null 2>&1 &
FPID=$!
python3 /home/z/my-project/scripts/slow-server.py >/dev/null 2>&1 &
SPID=$!
sleep 1

run1() { # url out_dir label
  local url="$1" out="$2"
  DXDL_HOME="$B/data" "$VEL" download -o "$out" -c 4 "$url" 2>/dev/null
}
# fresh engine instance per scenario via its own DXDL_HOME? The CLI is one-shot:
# each `velox download` call is a standalone engine instance (no server needed).
echo "== scenario 1: 1x8MiB"
S=$(date +%s.%N)
run1 "http://127.0.0.1:8391/sample-archive.zip" "$B/out1" >/dev/null
E=$(date +%s.%N)
python3 -c "print(f'wall: {$E-$S:.2f}s')"
echo "size: $(stat -c %s $B/out1/* 2>/dev/null)"

echo "== scenario 2: 4x8MiB parallel"
S=$(date +%s.%N)
for i in 1 2 3 4; do
  run1 "http://127.0.0.1:8391/sample-archive.zip" "$B/out2-$i" >/dev/null &
done
wait
E=$(date +%s.%N)
python3 -c "print(f'wall: {$E-$S:.2f}s')"
echo "total: $(du -cb $B/out2-* | tail -1 | cut -f1)"

echo "== scenario 3: 60MiB throttled"
S=$(date +%s.%N)
run1 "http://127.0.0.1:8392/slow.bin" "$B/out3" >/dev/null
E=$(date +%s.%N)
python3 -c "print(f'wall: {$E-$S:.2f}s')"
echo "size: $(stat -c %s $B/out3/* 2>/dev/null)"

kill $FPID $SPID 2>/dev/null
echo done
