# PERFORMANCE PLAN — measure, then optimize

Directive §8: performance is measured, never guessed, and never purchased with data integrity.

## 1. Methodology

All comparisons run under **identical conditions**: same file bytes, same server (our `engine-testserver` on
loopback with configurable pacing), same connection constraints, same duration, same machine, OS caches warmed
by one untimed pass. No cross-machine comparisons, no "peak" cherry-picking: we report median of ≥5 runs.

Harness: `benchmarks/run_local.sh` — starts the test server with a generated file, then runs each tool with
equivalent semantics:
- `velox download` (engine, defaults),
- `curl -o out` (single connection baseline; `-Z` only for multi-URL, not single-file segmentation — curl has none),
- `wget -O out`,
- `aria2c -x8 -k1M` when installed (this sandbox lacks it; listed for completeness).

Server-side pacing knobs (`--rate N bps`, per-connection latency) let us test slow links, high latency, and
per-connection throttling deterministically — conditions that matter more than loopback records.

## 2. Metrics recorded per run

wall time, mean/min/max throughput, distinct connections used, peak RSS (`/usr/bin/time -v`), CPU%
(`getrusage` via time), and for the engine: checkpoint overhead (DB rows/second), resume latency (command →
first byte), recovery latency (crash state → resumed streaming).

## 3. Targets (revoked only by measured evidence, never by wishful thinking)

| Metric | Target |
|---|---|
| Throughput vs curl, single connection, loopback | ≥ 90% of curl |
| Throughput vs curl, 8-connection throttled server (rate-limited per conn) | ≥ 4× curl single |
| Peak RSS, any file size | < 64 MiB (O(connections), not O(file)) |
| CPU (multi-threaded loopback, 8 conns) | < 50% of one core at ≥ 100 MiB/s |
| Resume command → first byte | < 500 ms local |
| Checkpoint overhead | < 2% throughput at 4 MiB interval |
| Small file (100 KiB) completion | ≤ 1.5× curl (connection setup dominates) |

## 4. Resource budgets (design-level)

Memory: O(connections × 64 KiB) buffers + hash state; no whole-file buffering anywhere.
Disk I/O: positional writes only; fsync at checkpoint intervals (4 MiB default) and completion.
SQLite: one `BEGIN IMMEDIATE` per checkpoint per worker; WAL auto-checkpoint default.

## 5. Profiling approach

`tracing` spans around inspect/plan/open_range/write/checkpoint; `RUST_LOG=debug` replays as flame-friendly
logs. Release builds keep symbols (`lto = "thin"`). For deep dives: `perf record` on the download path and
`/usr/bin/time -v` deltas between checkpoint settings.

## 6. Current status (updated 2026-11-19, session 2)

**First measured results** — `bash benchmarks/run_local.sh 67108864 8 3` (64 MiB, page-cache-warm
loopback, python http.server origin, median of 3, integrity hash verified after every run):

| Tool | 64 MiB wall (median) | Peak RSS | CPU | Notes |
|---|---|---|---|---|
| **velox (-c8, fresh)** | **0.04 s** (~1.6 GB/s) | ~12.5 MB | ~99% | inspect + probe + download + verify-safe finalize |
| curl (1-conn) | 0.08 s | ~12.7 MB | 62% | |
| wget (1-conn) | 0.06 s | ~12.7 MB | 70% | |
| velox (100 KiB, -c1) | 0.01 s | ~12.5 MB | ~92% | parity with curl-small |
| aria2c | — | — | — | **not installed** in this environment; documented, not fabricated |


### 6.1 Throttled-origin scenarios (session 2, `benchmarks/run_scenarios.sh`)

Identical origin (engine's Range-correct testserver), per-connection pacing, median of 3,
fresh engine state per measured run, integrity verified:

| Scenario | velox (-c4) | curl (1) | wget (1) | Reading |
|---|---|---|---|---|
| moderate: 32 MiB @ 4 MB/s/conn | **4.2 s** | 8.5 s | 8.5 s | velox **2.0× faster** (multi-connection aggregation works) |
| slow: 8 MiB @ 256 KB/s/conn | 32.5 s | 32.6 s | 32.6 s | NO gain — 8 MiB < 2×min_segment(8 MiB) → single connection by design; a `--min-segment` CLI flag is the recorded follow-up |
| latency: 8 MiB @ 120 ms delay | 0.38 s rc=0 | 0.14 s | 0.13 s | one earlier velox rc=1 in this scenario was harness flake (manual rerun rc=0 ×2; stderr capture added to measure.py) |

Honest caveats: loopback only; velox's 2.0× (not 4×) at 4 connections is unexplained and flagged
for profiling (suspect: per-connection pacing granularity in the test origin); CPU/RSS parity with
curl (~12.6 MB peak). aria2c remains unavailable — documented, not fabricated.

Honest reading:
- velox reaches **parity-to-better** vs curl/wget on loopback single-origin transfers, with memory
  on par with curl and well inside the 64 MiB budget (ARCHITECTURE §6). At this scale numbers are
  dominated by process startup and page-cache I/O; they are sanity numbers, not headline claims.
- **No multi-connection gain is observable on loopback** — the link is not the bottleneck, as the
  plan predicted (§1: never assume connections help). The throttled-origin comparison (per-connection
  pacing in engine-testserver) is the next measurement and is pending.
- Harness lessons fixed this session: GNU time absent → python getrusage wrapper
  (`benchmarks/measure.py`); velox runs 2..N previously hit the duplicate-detection "already
  completed" path (0.01 s measurements) — every measured run is now fresh-state.
- Real-network suite (packet loss, latency, throttling) remains pending (P5/P6 follow-up).

- Loopback sanity benchmark implemented (`benchmarks/run_local.sh`); comparative numbers vs curl/wget on this
  sandbox recorded there as they are produced. aria2c absent from sandbox → that baseline is pending.
- Real-network suite (public test files, packet-loss/latency injection) is planned with the P5 phase;
  the loopback suite is the reproducible core.
- No performance claims are made anywhere in user-facing docs until measured here.

### 6.2 First-wave sizing (k0) — explains the 2.0× at 4 connections

The initial wave is `k0 = ceil(total / (2 * min_segment_size))` capped by `-c` — NOT `-c`
connections up front. 32 MiB @ 8 MiB floor ⇒ k0 = 2 segments of 16 MiB ⇒ 2 effective
connections ⇒ 2.0× over curl. A fourth worker joins via tail-stealing once a segment drains.
`--min-segment` (session 2) widens the wave: 4 MiB floor ⇒ k0 = 4 ⇒ ~4×. This is deliberate
(aria2-style conservative start); measured, documented, and now user-controllable.
