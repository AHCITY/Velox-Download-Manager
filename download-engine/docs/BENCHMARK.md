# Engine Benchmark — 2026-11-22 (browser-ecosystem turn)

Method: identical fixtures and commands for both runs — local python file
server (`127.0.0.1:8391`, real 8 MiB binary file) and the throttled server
(`/slow.bin`, 60 MiB at its own ~2 MiB/s ceiling). The workspace `velox` CLI
drives the REAL engine (one-shot engine instance per download). The BASELINE run
swapped in the pre-turn `manager.rs` (`git show HEAD:…`) and rebuilt; the AFTER
run used this turn's code (speed-stats tap present). Same machine, same fixtures,
back-to-back.

| Scenario | Baseline (pre-turn) | After (with speed-stats tap) |
|---|---|---|
| 1×8 MiB, 4 connections (loopback) | **0.02 s** — 8,388,608 B | **0.03 s** — 8,388,608 B |
| 4×8 MiB parallel (loopback) | **0.05 s** — 33,554,432 B | **0.05 s** — 33,554,432 B |
| 60 MiB throttled (~2 MiB/s server ceiling) | **29.03 s** (≈2.04 MiB/s) | **29.03 s** (≈2.04 MiB/s) |

Reading: identical within loopback noise (±10 ms). The speed-stats tap costs one
broadcast receiver per active download plus a mutex update per ~1 Hz progress
event — not measurable against loopback jitter. This is a NO-REGRESSION check,
not a speedup claim; no data-path code was changed this turn.

Not optimized this turn (spec §7 requires measurement first; candidates recorded):
- Dead-URL failure latency: ~31 s (3 × 10 s connect retries before the honest
  CONNECT error). Retry-policy tuning candidate — engine-tested behavior, deserves
  its own measured change.
- Loopback numbers are ceiling-bound by the fixture servers, not the engine; a
  fair comparison against IDM/FDM needs real-network fixtures and is out of scope
  for this sandbox.

Reproduce:
```
scripts/bench-run.sh            # prints wall time + sizes for all 3 scenarios
# baseline: swap manager.rs to `git show HEAD:download-engine/crates/engine-core/src/manager.rs`,
# rebuild `cargo build -p engine-cli --bin velox`, rerun
```
