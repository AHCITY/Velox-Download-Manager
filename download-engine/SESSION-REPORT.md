# Session 1 engineering report — Velox download engine (2026-11-19)

> **Session 2 (validation directive) — see §9 below for the full record.** Sessions 1–2 report;
> §1–§8 are the session-1 record preserved for continuity.

This file records what was delivered in the first development session, per the directive's
"end of session report" requirement (§15). Continuity for the next session: everything below
lives in this repository; state files and scripts survive; processes do not.

## 1. Repository finding

The repository was effectively empty (initial commit + agent workspace dirs). No prior
implementation existed to preserve. The engine was created under `download-engine/`.

## 2. Research (Phase 0)

Three parallel research tracks, all cited and dated in `docs/RESEARCH.md`:
- 20 download managers: verified licenses/maintenance on repo pages (aria2 GPL-2, ABDM Apache-2.0,
  Gopeed GPL-3, Motrix MIT, XDM GPL-3, curl MIT-like, FDM-3.x GPL-2 via SourceForge, modern FDM
  closed except LGPL QML UI, uGet unverifiable, etc.). Clean-room rules recorded (ADR-0008).
- RFCs read from primary sources (9110/9112/6585/7617/6265/6266/8187/5854/6249/8305); distilled
  implementable rules in `docs/PROTOCOL_COMPATIBILITY.md`.
- Storage durability verified from sqlite.org/man7.org/LWN: WAL+synchronous=FULL durability matrix,
  fsync-directory-after-rename requirement, short-write semantics, sparse preallocation.
- Rust ecosystem verified on docs.rs: tokio 1.54 (MSRV 1.85), reqwest 0.13.5 feature renames
  (`rustls-no-provider`), rustls 0.23 ring/aws-lc provider rules, rusqlite 0.43 bundled.

## 3. Documents delivered

docs/: RESEARCH.md · ARCHITECTURE.md · PROTOCOL_COMPATIBILITY.md · RECOVERY_DESIGN.md ·
PERFORMANCE_PLAN.md · SECURITY.md · ROADMAP.md · DECISIONS/ADR-0001..0008 · README.md · LICENSE-MIT.

## 4. Implementation (Phase 1–4 partial, all building)

Workspace `download-engine/` with 7 crates (rustc 1.100, edition 2021):

| Crate | Contents |
|---|---|
| engine-core | config, error classification, retry policy (backoff+jitter+Retry-After), resource inspection model, download planning (ADR-0006), segment table with tail-stealing work stealing, bandwidth limiter, filename resolution/sanitization, task orchestrator (workers/monitor/checkpoints/finalize), DownloadManager facade, events |
| engine-http | reqwest 0.13 + rustls/ring backend: ranged GET probe inspection, manual redirect loop (chain capture, loop detection, host-scoped auth), If-Range, Content-Range parsing, per-chunk stall watchdog |
| engine-storage | part files with positional writes (write_all_at), sparse preallocation, sync_data/sync_all, atomic finalize (rename + directory fsync), collision-safe naming |
| engine-state | SQLite (WAL, synchronous=FULL) manifest: downloads/segments/errors, transactional snapshots, bounded error ring |
| engine-api | JSON-RPC 2.0 v1 over stdio (info/create/start/pause/resume/cancel/remove/list/status/verify) |
| engine-cli | `velox` binary: download/inspect/list/status/pause/resume/cancel/remove/verify/serve, progress display, SIGINT→pause |
| engine-testserver | std-only fault-injecting HTTP server: ranges, 416, wrong/missing Content-Range, chunked, close-delimited, stalls, mid-body closes, 429+Retry-After, redirects, slow drip, per-range slowdown |

## 5. Tests executed

- Unit tests (all green at time of writing): engine-storage (3), engine-state (3), engine-core (17),
  engine-http (content-range parsing), engine-testserver (2), engine-api (2).
- Integration suite `engine-core/tests/integration.rs` (14 scenarios): single-stream hash match,
  segmented multi-connection assembly, per-host cap, close-delimited stream, redirects+disposition
  naming, no-range fallback, Retry-After honoring, pause/resume without full re-download,
  truncated-part-file reconciliation (power-loss simulation), ETag-change restart, slow-tail
  stealing, wrong-Content-Range failure without exposing a file, checksum-mismatch failure keeping
  the part file, cancel cleanup, duplicate detection.
- **Interim integration run results (session cut off mid-run — superseded):** session 1 ended with
  6/15 green; session 2 triaged all 9 failures to root cause and the suite is now 15/15 (see
  session 2 §2). Logs preserved: benchmarks/{integration-run,full-test-run,crash-run}.log.
- Known not yet tested in-suite: true out-of-process SIGKILL crash (planned via `velox` child
  process), bandwidth-limit timing, HTTPS against real servers.
  *(Session 2 update: the SIGKILL suite now exists and passes — see §3 of session 2.)*

## 6. Known issues / limitations (honest)

- Real-network performance numbers not yet produced (aria2 absent in sandbox; loopback harness ready
  in `benchmarks/run_local.sh`).
- Cookie persistence is session-only; persistent jar is roadmap item.
- Windows paths (seek_write_all, dir-fsync equivalent) are implemented but untested on this Linux sandbox.
- Per-download `insecure_tls` flag exists but is only reachable via config/API today (no CLI flag).
- Monitor checkpoints every 500 ms when dirty — fine for correctness, tuning is a Phase-5 task.
- `velox serve` JSON-RPC is sequential per line (documented; concurrency when clients demand it).

## 7. Next concrete tasks

1. Run integration suite to green; add out-of-process crash test.
2. Produce benchmark medians (loopback + throttled), record in PERFORMANCE_PLAN §6.
3. Persistent cookie jar; per-class retry policy surface; `--fresh` flag.
4. Metalink4 parser + mirror engine with piece hashes.
5. `engine-network` crate skeleton (interface enumeration, bind-to-interface research).

---

# Session 2 engineering report — validation directive (2026-11-19)

Continuation directive: validate before extending (P1 tests → P2 crash recovery → P3 HTTP audit
→ P4 scheduler → P5 storage → P6 benchmarks). Preserve the seven-crate architecture; do not hide
failing tests; do not claim completion without evidence.

## 1. Exact commands executed

```
cargo test --manifest-path download-engine/Cargo.toml -p engine-core --test integration
cargo test --manifest-path download-engine/Cargo.toml --workspace --no-fail-fast
cargo test --manifest-path download-engine/Cargo.toml -p engine-cli --test crash_resume
RUST_LOG=velox=debug cargo test ... --test http_compat changed_last_modified -- --nocapture   (triage)
bash benchmarks/run_local.sh [size] [conns] [runs]
```
Logs: benchmarks/{integration-run,full-test-run,crash-run}.log (kept in the repo).

## 2. P1 — integration triage (6/15 → 15/15 green)

Root causes (fixed in the implementation, tests unchanged except where the fixture itself
misrepresented the scenario):

| Bug | Root cause | Fix |
|---|---|---|
| Engine punished servers that answer 200 to ranged requests | The worker required 206 on every ranged request and misread 200 as "resource changed" → restart loop → `RESOURCE_CHANGED` failures (5 tests red) | RFC 9110 §14.2: when the probe proved ranges unsupported and nothing is written, send a **plain GET** (`no_range_header`); a 200 full body at file start is consumed as-is; a 200 past offset still triggers the validator-guarded restart |
| `manager.add` failed on the first 429/503 | Inspection had no transient retry (only `run_task` did) | `inspect_with_retry` in the manager honoring `Retry-After` |
| Retry-After never parsed | Header ignored end-to-end | `ResourceInfo.retry_after` + `parse_retry_after` (delta-seconds; HTTP-date documented as MVP limitation) |
| Latent corruption vector found by analysis | A segment written to its end but un-completed (pause at the exact boundary) resumed as `Range: bytes=(end+1)-(end)` → invalid request → server thread would panic | Boundary normalization in `SegmentTable::new` + `take` (+ regression unit test) |
| Content-Length trap on 206 | A 206 *without* Content-Range fell back to Content-Length — but on 206 that is the **partial** length (RFC 9110 §14.4) → silent truncation | Total is only trusted from Content-Range on 206; otherwise unknown → streaming mode |
| Zero-length resources impossible | 416-with-`bytes */0` probe was rejected by add/start | `probe_usable()` accepts a 416 probe carrying `*/N` |

Fixture corrections (misrepresentations fixed, expectations not weakened): the single-stream test's
server is range-capable so `resumable="yes"` is the correct expectation; the close-delimited test
now also sets `ignore_ranges` to genuinely represent an unknown-length resource; three tests got
deterministic slow-drip pacing (the pause landed after completion at 2 MiB/s); the steal test
switched to **mechanical** evidence (a range request starting off a planned boundary — `range_starts`)
after analysis showed range-scoped slowness makes stolen tails slow too.

## 3. P2 — out-of-process crash recovery (4/4 green, SIGKILL)

`engine-cli/tests/crash_resume.rs` runs the real `velox` binary as a child and kills it with
SIGKILL at: mid-download (≥4 MiB), during resume (double kill), before the first checkpoint, and
a simulated rename-orphan (part file manually moved to the final position). After every kill:
- **Invariant 1**: every manifest position ≤ part-file length (fsync-before-commit holds);
- **Invariant 2**: `downloads.received == Σ segment positions`;
- a fresh process resumes and the final file is **SHA-256 byte-exact**;
- rename-orphans are **never adopted** — the engine re-downloads to a collision-safe name.

Also found and fixed here: `velox resume` started the task and exited immediately (the spawned
task died with the process) — the CLI now stays attached via `monitor_until_done` (same as
`download`), so resume actually completes out-of-process.

## 4. P3 — HTTP compatibility audit (10/10 green)

New scenarios, all against the fault-injecting server: HEAD-unsupported, weak-ETag resume
(mechanically asserts If-Range is never sent), missing-ETag resume, changed Last-Modified →
full restart, 503 retry, expiring URL (410 → permanent failure, no file exposed), zero-length
resource (416 `*/0` probe → empty completed file), chunked framing, RFC-invalid 206-without-
Content-Range (never trusts partial CL; recovers via plain GET), stall detection + retry
(2 s watchdog), mid-body connection closes (2×) with checkpoint recovery.

## 5. P4 — segment scheduler audit (2/2 green)

Property tests (`segments_property.rs`), 200 randomized rounds: no byte written twice within a
run, exact coverage at quiescence, plan always an exact contiguous partition, workers never
build invalid ranges, aggregate accounting matches the write log. Comparison notes vs aria2:
same floor-based splitting philosophy (min-split-size), but aria2 splits eagerly per URI set
while we steal tails on completion — evidence-based connection counts (never raised beyond
measured need) recorded as an open tuning question for P5/P6.

## 6. P5 — storage and integrity audit

- Cross-process **download lock** implemented (`<part>.lock` + owning pid; stale locks stolen
  after SIGKILL — verified by unit test); two `velox resume` processes can no longer double-write
  a part file. Wired into `manager.start`.
- Durability ladder re-verified in code: positional writes (loop-safe), fsync-before-commit,
  sync_all → rename → dir-fsync → DB commit, WAL+synchronous=FULL (crash tests exercise WAL
  recovery under SIGKILL for real).
- Mid-body close recovery regression test added (`closed_early ≥ 2` mechanical assertion).
- Documented non-guarantees updated (RECOVERY_DESIGN §6): rename-orphan adoption is not
  implemented (re-download is the safe path); power-loss claims limited to what
  fsync + SQLite WAL/FULL actually provide; HTTP-date Retry-After unparsed in MVP.

## 7. P6 — benchmarks

Harness: `benchmarks/run_local.sh` + `benchmarks/measure.py` (identical loopback conditions; wall
time, peak RSS via getrusage, CPU%). **aria2c is not installed in this environment — documented,
not fabricated.** First measured results (median of 3, integrity-verified): 64 MiB loopback —
velox 0.04 s / ~12.5 MB RSS / ~99% CPU vs curl 0.08 s / 12.7 MB / 62% and wget 0.06 s / 12.7 MB /
70%; 100 KiB parity with curl. No multi-connection gain on loopback (link not the bottleneck —
as predicted); the throttled-origin comparison is the next measurement. Full table + honest
caveats: PERFORMANCE_PLAN §6. Harness lesson fixed: every measured velox run is now fresh-state
(runs 2..N previously hit the duplicate-detection "already completed" shortcut).

## 8. Remaining known issues

1. Rename-orphan adoption (with verified hash) not implemented — re-download is the current, safe path.
2. HTTP-date Retry-After unparsed; cookie persistence session-only.
3. Windows paths implemented but untested (this sandbox is Linux); directory-fsync is Unix-only.
4. Timing-sensitive tests kept only with generous bounds; the steal test's core assertion is mechanical.
5. Comparative benchmark numbers are loopback-only; real-network suite still pending.

## 9. Definition-of-done scorecard (session 2)

- All pre-existing integration tests pass: **15/15** (plus 10 http_compat, 4 crash, 2 property,
  and all unit tests — see benchmarks/full-test-run.log for the authoritative count).
- Crash recovery exercised out-of-process (SIGKILL): **4/4 byte-exact**.
- HTTP fallback behavior validated (plain-GET fallback, stream mode, 200-full-range acceptance).
- Resource identity validated on resume (If-Range strong-only; weak/missing/changed handled).
- Segment scheduling passes boundary + concurrency property tests.
- Integrity verification works (checksum mismatch fails the download; verified hash recorded).
- No known silent-corruption scenarios left unaddressed (the three found — full-range 200
  splicing, 206-partial-CL truncation, concurrent-process double-write — are fixed and tested).
