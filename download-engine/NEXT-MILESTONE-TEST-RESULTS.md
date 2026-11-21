# NEXT-MILESTONE-TEST-RESULTS

All commands were actually executed. Date: 2026-11-20.

## Rust workspace
- `cargo test --workspace --no-fail-fast` → **93 passed, 0 failed** (baseline 80 verified at start of the milestone).
  - New: 7 Netscape-cookie tests (parse semantics, import/expiry, Suffix+HostOnly domain matching, roundtrip, session cookies, no-value-leak), 6 bridge tests (v1-route identity, live SSE + replay + seq ids, Host guard, traversal safety, wire-event mapping), 1 cross-process SIGKILL lock-recovery test (engine-storage), plus plan/lock test updates.
  - `rename_orphan_final_file_is_never_adopted_without_evidence`: passed on rerun (pre-existing timing sensitivity under parallel load).
- `cargo fmt --all -- --check` → exit 0.
- `cargo clippy --workspace --all-targets` → 15 warnings, all pre-existing stylistic classes (`&PathBuf` params, complex return types, large enum variants); no new ones of substance; no blanket allowances added.
- `cargo check -p engine-storage --target x86_64-pc-windows-msvc` → **clean** (this check caught and fixed the pre-existing unstable `seek_write_all` usage that broke Windows compilation).
- `cargo check -p engine-storage --target x86_64-pc-windows-gnu` → not verifiable here (mingw toolchain absent; documented).

## Cookie import/export (live CLI smoke test)
- Wrote a Netscape file, `velox cookies import /tmp/cookies.txt` → "imported 1 cookie(s)", jar written `-rw-------` (0600), `velox cookies export /tmp/exported.txt` → byte-identical Netscape line with `#HttpOnly_`/domain/path/secure/expiry preserved.

## API v2 (bridge tests + live)
- 6/6 bridge tests green (reuses v1 `handle_line`; notification → 204; Host guard 403; traversal 404; SSE seq ids + hello; replay path).
- v1 stdio suite: unchanged and green (part of the 93).

## Real end-to-end demo (headless browser against the live stack)
- UI loaded at http://localhost:3000, sample URL added through the interface.
- Live SSE progress observed: `DOWNLOADING · 137/256 MiB · 30.9 MiB/s · 8 connections · 4s left`.
- Pause via UI: byte-frozen at 91.5 MiB → PAUSED. Cancel via UI: row + part file removed.
- Completed: `preview-data/out/file` = 100,663,296 bytes; sha256 `bbc3ab03e972f9e70ce06a8467c8c664a63665ec4356fb38d25a31f38df6bac9` — identical to an independent `curl` re-download of the source (verified with `sha256sum … | uniq -c` → 2 identical).
- Engine + DB persistence across restarts observed; collision-safe naming (`file(1).bin`).
- Screenshots: `benchmarks/ui-progress.png`, `benchmarks/ui-live.png`, `benchmarks/ui-completed.png`.

## Not runnable in this environment
- The `windows-latest` CI job (real Windows runner) — the cfg(windows) crash-recovery test and full Windows test run await one CI pass.
- mingw (gnu) Windows toolchain check.
