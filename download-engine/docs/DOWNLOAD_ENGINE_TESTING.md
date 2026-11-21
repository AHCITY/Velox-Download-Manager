# DOWNLOAD ENGINE TESTING — tiers, coverage and how to run

Velox distinguishes five test tiers explicitly. Results are only claimed for tiers
that were actually executed (see PROGRESS-LOG.md for per-cycle evidence).

## Tier 1 — Unit tests (pure, mocked transport)

Where: `#[cfg(test)]` modules inside each crate (`engine-core/src/{naming,retry,inspect,
segments,error,ports,manager}.rs`, `engine-http/src/{extract,gdrive,netscape}.rs`,
`engine-storage/src/lib.rs`).
Covers: URL naming rules, retry/backoff math, format selection, confirm-page parsing,
range arithmetic, credential scoping, lock semantics, disk-space helpers, error mapping.
Run: `cargo test --workspace --lib`.

## Tier 2 — Local integration tests (real reqwest ↔ fault-injecting local server)

Where: `crates/engine-core/tests/{integration,http_correctness,recovery,concurrency}.rs`,
`crates/engine-http/tests/*.rs`, backed by `engine-testserver` (fault injection:
truncation, overlong bodies, premature close, redirects, ignore-Range, chunked,
HTML-with-200, gzip lies, stalls).
Covers: the P0 catalog — byte-exact SHA-256 on every completing scenario, resume after
kill, page gate, truncated-never-completed, overlong-exact-completion, Drive confirm
flow (fixture), stub-extractor flow, admission/queue, crash-rename adoption,
out-dir cleanliness, disk preflight, missing-file detection.
Run: `cargo test --workspace` (all local tiers together).

## Tier 3 — External integration tests (live internet; run from the sandbox only)

Where: manual/CLI runs recorded in PROGRESS-LOG.md (not in `cargo test`, so CI is not
dependent on egress). Evidence files: benchmarks/*.log, PROGRESS-LOG entries.
Covers: real Google Drive share link (2026-11-21 ×2, byte-verified), real yt-dlp probe
(bot wall reproduced with fresh logs), real direct-file downloads (benchmarks).
Honesty rule: environment-limited failures (datacenter-IP bot wall) are documented as
environment limits, never as engine bugs — and never glossed as success.

## Tier 4 — Manual verification (real Windows host, real browsers)

Where: `download/windows-build-20261120/INSTALL-AND-VERIFY.md` + INSTALLER-TEST-RESULTS.md
(from the 2026-11-20 build; the 2026-11-21 build repeats the same checklist).
Covers: installer flow, WebView2 bootstrapping, app launch, real downloads, update paths.

## Tier 5 — Performance benchmarks

Where: `benchmarks/BENCH-2026-11-21.md`, `scripts/run-benchmarks.sh` (+ bench-server.py).
Scope-limited honestly: loopback measures memory/CPU/correctness/concurrency/resume, NOT
WAN behavior. Reproduce with one command; results land in /tmp/bench/measure/results.txt.

## Regression discipline

After every meaningful change: `cargo fmt --all` → `cargo clippy --workspace
--all-targets -- -D warnings` → `cargo test --workspace`. A cycle is not complete until
all three are green (PROGRESS-LOG.md records each cycle's counts).
