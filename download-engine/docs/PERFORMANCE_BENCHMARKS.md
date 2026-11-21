# PERFORMANCE BENCHMARKS — Velox engine

Methodology, current results and reproduction steps. Raw evidence lives in
`benchmarks/BENCH-2026-11-21.md`; the harness is `scripts/run-benchmarks.sh`
(Range-capable loopback server: `scripts/bench-server.py`).

## Methodology

- Local loopback against a Python ThreadingHTTPServer honoring `Range` (scripts/bench-server.py),
  non-compressible deterministic payload, fresh `DXDL_HOME` per scenario.
- Measurements: wall time, peak RSS (`getrusage(RUSAGE_CHILDREN)`), CPU%, plus SHA-256
  integrity of every produced file against the source payload.
- Engine commit recorded with every run (results are meaningless without it).

## Latest results (commit 9a1a496-era engine; loopback)

| Scenario | Result |
| --- | --- |
| Single-connection 256 MiB | 0.12 s wall, RSS 12.8 MB, CPU 98% — sha256 == source |
| Segmented 8-connection 256 MiB | 0.31 s wall, RSS 13.1 MB, CPU 51% — sha256 == source |
| 4 concurrent downloads × 64 MiB (4 conns each) | 0.36 s wall aggregate |
| SIGKILL mid-transfer (~60 MiB) → resume of 128 MiB file | 0.11 s wall, sha256 == source |

## What this does and does NOT demonstrate

Demonstrated (sandbox-verifiable):
- **Bounded memory**: RSS stays ~13 MB on 256 MiB transfers — memory does not grow with
  file size (streaming segments, fixed chunks, no whole-file buffers).
- **Correct bytes under every scenario** (segmented, concurrent, resumed-after-kill).
- **Resume works and is cheap** after SIGKILL (state survives, only the remainder moves).
- No runaway threads/CPU.

NOT demonstrated here (honest scope limits):
- WAN throughput and the value of segmentation under latency/loss — a loopback
  seg-8 run was SLOWER than single-conn (0.31 s vs 0.12 s) because loopback + a
  threaded Python server serialize; segmentation targets real-WAN conditions which
  this sandbox cannot simulate. No IDM/FDM comparisons are claimed.
- Adaptive connection counts need real-network evidence before implementation (mission
  Part 5: implement only demonstrable improvements).

## Reproduce

```bash
scripts/run-benchmarks.sh          # ~2 min; writes /tmp/bench/measure/results.txt
```

Requirements: built release CLI (`cargo build --release -p engine-cli`), Python 3.
