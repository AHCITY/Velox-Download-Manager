# ROADMAP — phases, status, completion criteria

Directive §14 phases mapped to this workspace. Status: ✅ done · 🚧 partial · ⬜ not started
(updated 2026-11-19, session 2 — validation session).

| Phase | Scope | Status | Completion criteria |
|---|---|---|---|
| P0 Research & architecture | Repo inspection, downloader/standards/storage research, docs, ADRs | ✅ | docs/ complete with real decisions; workspace builds |
| P1 Single-connection HTTP engine | Inspection, streaming, redirects, auth, progress, safe files, manifest | ✅ | integration tests: full download, unknown length, chunked, redirects, CD naming, 404/429/503 behavior, resume |
| P2 Segmented engine | Range probe, concurrent segments, adaptive split/steal, fallbacks | ✅ | tests: correct assembly, per-host cap, no-range fallback, mechanical steal detection, mid-range faults |
| P3 Persistent recovery | Transactional manifest, resume validation, reconcile, finalize order | ✅ | **out-of-process SIGKILL tests** (mid-download, during-resume, pre-checkpoint, rename-orphan) byte-exact; manifest-vs-file consistency invariants asserted |
| P4 Failure management | Classification, backoff+jitter, Retry-After, stall handling | 🚧 | classification/backoff/Retry-After(delta-seconds)/stall tested; policy surface (per-class overrides) + HTTP-date Retry-After remain |
| P5 Performance & compatibility | Benchmark suite vs curl/wget(/aria2), profiling, tuning | 🚧 | harness ready (benchmarks/run_local.sh); results recorded in PERFORMANCE_PLAN §6; tuning after measurements |
| P6 Multi-source & advanced networking | Mirrors, Metalink, interface awareness, failover, bonding research | ⬜ | mirror engine behind resource-identity checks (RFC 6249); interface adapters per platform |
| P7 Platform adapters | Windows/macOS/Android storage+network specifics, disk-space API, keyring | ⬜ | fallocate/SetEndOfFile adapters, free-disk pre-check, credential keyring, Android storage |
| P8 Additional protocols | FTP/SFTP evaluation, BitTorrent subsystem (separate), extensible protocol trait | ⬜ | only after HTTP engine stable; license review per dependency |
| P9 User interfaces | Desktop/Android/browser extension/remote web on API v1+ | ⬜ | **explicitly last** (directive §1); API stability is the prerequisite |

## Session 2 validation results (2026-11-19)

- Integration triage fixed 3 engine bugs (see SESSION-REPORT.md §3): 200-to-full-range acceptance
  (RFC 9110 §14.2), plain-GET when the probe proves ranges unsupported, inspect-level retry with
  Retry-After (delta-seconds), plus boundary normalization of fully-written unmarked segments and
  the Content-Length-on-206 trap (partial length ≠ total, RFC 9110 §14.4).
- Scheduler property tests: no double-writes, exact coverage, exact partitioning, min-split floors.
- HTTP compatibility audit extended: HEAD-unsupported, weak/missing/changed ETags, Last-Modified
  identity, 410 expiring URLs, zero-length resources, chunked framing, broken 206-without-CR,
  stall detection, 503.

## Dependency order

P2→P3→P4 are the reliability spine (validated). P5 gates any optimization claims.
P6 depends on P5 numbers (does multi-connection to mirrors actually help?). P9 depends on a frozen,
documented API v1 plus P4/P5 maturity.

## Next concrete tasks (priority order)

1. Policy surface: configurable per-class retry overrides + `--fresh`/`--no-resume` CLI flags.
2. HTTP-date form of Retry-After; persistent cookie jar (RFC 6265 store file, 0600).
3. Benchmark medians (loopback + throttled server pacing) recorded in PERFORMANCE_PLAN §6.
4. Adopt-if-verified path for rename-orphans when a checksum exists (currently: always re-download — safe).
5. Metalink4 parser + mirror engine with piece-hash verification (RFC 5854 §4.1.3).
6. `engine-network` crate skeleton: interface enumeration (Linux first), bind-to-interface research.

## Session 2 overnight addendum (2026-11-19)

- Audits (2 independent sub-agent reviews) → fixed: H3 validator-less length splice (silent truncation),
  A7 etag-verified 200 acceptance, A6 retryable statuses mid-download, limiter hang (oversized chunk),
  lock-steal TOCTOU (atomic rename steal), worker-panic Ctrl::Failed, origin-triple auth scoping,
  cross-origin header stripping, insecure_tls propagation (dead flag → wired), 0600 part files,
  URL userinfo rejection, BEGIN IMMEDIATE transactions, cancel-vs-finalize race, finalize
  truncate-before-hash, collision-name row write-back, Retry-After clamp + HTTP-date parsing.
- New features: evidence-based rename-orphan adoption (checksum+size+hash), persistent RFC 6265
  cookie jar (0600 JSON, DXDL_COOKIE_JAR), throttled benchmark harness.
- New docs: WINDOWS_COMPAT.md (audited, NOT validated), UI_READINESS.md, RESEARCH.md §6 addendum.
- Gates: clippy clean after --fix, rustfmt clean, 74 tests green (see OVERNIGHT-TEST-RESULTS.md).
