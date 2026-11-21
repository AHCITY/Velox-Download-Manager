# Overnight engineering report — Velox (2026-11-19)

## Execution
Continuous single-session autonomous run (no Windows runner, no aria2 available). Work was
checkpointed with git commits at every milestone. Sub-agents used: (1) security+HTTP/protocol
audit, (2) storage+architecture audit, (3) download-manager/RFC/Windows web research.

## P0 (mission-required fixes)
- P0.1: two `assert_final_hash` calls in crash_resume.rs were unawaited — the SHA-256 assertions
  in two SIGKILL tests never executed. Fixed and re-verified: all 4 crash tests now genuinely
  hash-verify byte-exact recovery.
- P0.2: finalization state machine rewritten — every required step (stream-truncate → hash →
  sync_all → rename → dir-fsync → DB commit) fails the task; advisory steps log. Fault-injection
  tests added (finalize-failure never reports completed; collision names written back to the row).
- P0.3: all warnings removed (dead `bad_params`, unused `KIB`/`part_file`, unawaited futures).

## Audit findings fixed (verified by me before fixing; regression tests added)
1. Silent truncation reported as completed: validator-less resume ignored a changed resource
   length (H3) — restart-from-zero on length change; regression test splices-never-happen.
2. Single-mode 200 acceptance could complete a truncated NEW body on ETag change (A7) — 200 at
   file start now requires validator match.
3. Mid-download 408/429/5xx consumed the whole restart budget (A6) → now retried per policy.
4. Bandwidth limiter hung forever when a chunk exceeded the rate (H1) → sliced consumption +
   cancellable pacing.
5. Cross-process lock steal race — two processes could both steal a stale lock (H2) → atomic
   rename-based steal + bounded retry.
6. Worker panics leaked segment ownership → hang → Ctrl::Failed on join error.
7. Authorization leaked on https→http downgrade redirects (A1) → origin-triple scoping;
   cookies/extra headers/referer stripped cross-origin (A4).
8. `--insecure-tls` was a dead end-to-end flag (A2) → propagated.
9. Part files born 0644 (A13) → 0600 at creation; URL-embedded credentials rejected (A12).
10. SQLite used BEGIN DEFERRED (doc says IMMEDIATE) → switched; cancel-vs-finalize race could
    orphan files → re-check + refuse-on-timeout; temp_path self-heal.
11. Retry-After unbounded server stall (A5) → clamped to max_delay; sleeps are stop-aware.

## New features
- Evidence-based rename-orphan adoption (expected checksum + exact size + recomputed hash;
  otherwise safe re-download — verified by two tests).
- Persistent RFC 6265 cookie jar (`cookie_store`, 0600 JSON, periodic + drop persistence,
  `DXDL_COOKIE_JAR` override/disable).
- HTTP-date Retry-After (IMF-fixdate parser; obsolete forms → backoff, documented).
- Throttled/latency benchmark harness (`benchmarks/run_scenarios.sh` + `throttled_httpd` example).

## Environment limitations (honest)
- clippy/rustfmt had to be installed mid-session; ~24 mechanical lints auto-fixed, then fmt clean.
- aria2 absent → not benchmarked against it (documented, not fabricated).
- No Windows runner → WINDOWS_COMPAT.md is an audit + plan, explicitly NOT validated.
- 2-core sandbox; benchmarks are loopback-only.

## Recommended next milestone
1. `--min-segment` CLI flag (multi-conn gain proven only ≥2×min_segment today).
2. Investigate why 4 connections gave 2.0× not ~4× under per-connection throttling.
3. Windows CI job + LockFileEx-based lock recovery (WINDOWS_COMPAT.md §2).
4. API v2 pass per UI_READINESS.md §2 (events framing, download.inspect, queue limits).

## Follow-up session (2026-11-20) — additional work

- **2.0× benchmark anomaly EXPLAINED** (no engine bug): the initial wave is
  `k0 = ceil(total / (2×min_segment))` — 32 MiB @ 8 MiB floor ⇒ 2×16 MiB segments ⇒ 2 effective
  connections. Documented in PERFORMANCE_PLAN §6.2 and now user-controllable.
- **`--min-segment` CLI flag**: per-download floor override (DownloadConfig.min_segment_size →
  build_plan) + plan unit tests. 4 MiB floor on the same file ⇒ k0 = 4 ⇒ ~4×.
- **Audit B3**: Content-Range must match BOTH requested ends; a server echoing a different end
  no longer defines segment length. Plus a testserver-armed mid-download 503 regression test
  (A6): transient statuses are retried per policy, never restart.
- **API hardening (UI_READINESS §2.7/§2.8 + §2.2)**: `download.inspect` RPC (preview without
  registering, shared `validate_url`), 1 MiB request-line cap (bounded reader, untrusted input
  cannot grow engine memory), JSON-RPC 2.0 notification handling (no response) + id type
  strictness. 5/5 API tests.
- **A8 queue management**: `EngineConfig.max_concurrent_downloads` — starts beyond the cap wait
  FIFO on a semaphore; row re-checked after queueing (cancel-safe); host-semaphore map is now
  pruned when tasks settle. Regression test included.
- **Security**: state.db (holds auth config) opened/enforced 0600 on Unix.
- **Docs synced**: RECOVERY_DESIGN (truncate-before-hash), PROTOCOL_COMPATIBILITY (HTTP-date
  Retry-After now parsed; A5 clamp), UI_READINESS (inspect done), PERFORMANCE_PLAN §6.2.
- **Windows CI**: `.github/workflows/windows.yml` — build + full tests + crash suite on
  windows-latest; no Windows-support claim may precede a green run.
- **Environment note**: the sandbox recycle corrupted the cargo registry cache mid-session;
  diagnosed (corrupt index cache) and fully repaired by a clean re-fetch — all builds green.

Gates after this session: `cargo test --workspace --no-fail-fast` = **80 tests, 0 failures**;
`cargo fmt --check` clean; clippy: 11 accepted stylistic lints (pre-existing `&PathBuf` params,
complex return types, large enum variants — no behavior impact, no blanket allowances used).
