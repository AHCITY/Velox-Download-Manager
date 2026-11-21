# NEXT-MILESTONE-REPORT — Windows readiness, cookies, API v2, live desktop preview

Date: 2026-11-20. Baseline at start: 80 tests green (verified, not assumed).
Workspace: `/home/z/my-project/download-engine` (crates: engine-core/http/storage/state/api/cli/testserver/**bridge**).

## 1. Implemented and verified

**Windows compatibility (high priority)**
- `DownloadLock` rebuilt on OS-level advisory locks: `flock(LOCK_EX|LOCK_NB)` (Unix) / `LockFileEx(LOCKFILE_EXCLUSIVE_LOCK|LOCKFILE_FAIL_IMMEDIATELY)` (Windows). The kernel releases the lock on process death, so stale locks are claimed without pid heuristics — the old Windows path could NEVER recover a crashed holder's lock (`pid_alive` was a stub).
- Ownership model simplified: the OS-lock holder owns the lockfile; the lockfile is never unlinked (unlink-on-drop reintroduced a two-holder race — fixed after independent review).
- Cross-compile verification: `cargo check -p engine-storage --target x86_64-pc-windows-msvc` passes. This check **caught a pre-existing Windows breakage**: `PartFile::write_at` used unstable `seek_write_all` — the code could never have compiled for Windows; replaced with stable seek+write under the existing `Mutex<File>`.
- Windows dir-flush: `FlushFileBuffers` on a `FILE_FLAG_BACKUP_SEMANTICS` handle, opened with write access (review found the read-only open always failed silently), failures logged non-fatally.
- LockFileEx passes a real (zeroed) `OVERLAPPED` — the windows-sys signature requires non-null (review finding; null would crash on first use).
- Cross-process crash-recovery tests: Unix SIGKILL test runs here; the Windows `taskkill` variant ships for the CI runner (`.github/workflows/windows.yml` exists but **has not run on a real Windows runner — see §3**).

**Cookie jar — Netscape compatibility**
- `engine-http::netscape`: parse/import/export of the 7-field Netscape format (`#HttpOnly_` prefix, tabs-in-values, include-subdomains → `CookieDomain::Suffix`, host-only → `HostOnly`, session cookies → `SessionEnd`, expired dropped at import).
- Domain matching delegated to `cookie_store` (RFC 6265); tests prove no leakage to unrelated domains.
- Public-suffix guard: single-label domains ("com", "org") are rejected at import — the jar has no PSL, and such a cookie would match every registrable domain (review finding).
- CLI: `velox cookies import <file>` / `velox cookies export [file]` — smoke-tested end-to-end (import → 0600 JSON jar → byte-identical export).
- `DXDL_COOKIE_JAR` (JSON) untouched; jar saves hardened: per-process tmp name, fsync before rename, corrupt-jar loads log a warning instead of silently emptying.
- Cookie values never appear in project logs/errors/API responses (verified by review; names/counts only).

**API v2 (`engine-bridge`, docs/API_V2.md)**
- `velox serve-http --bind 127.0.0.1 --port 3000 --ui-dir web/ [--sample-url …]`
- `POST /rpc` reuses v1's `handle_line` verbatim → method semantics can never drift. `X-Velox-Api: 2` header; `engine.info` gained additive `default_out_dir`/`max_concurrent_downloads` fields.
- `GET /events` (SSE): sequence-tagged, 512-event replay ring, `Last-Event-ID` reconnect with live-subscribe-BEFORE-replay ordering (gap-free), duplicate filtering, `resync` when history is gone or the client is ahead, progress coalesced to ≤4 Hz (latest wins), structural events flush stale progress, slow clients disconnected (broadcast lag → resync) — the engine never blocks on a slow UI.
- Hardening (from review): 10 s request timeout, 64-connection cap, duplicate-Content-Length / Transfer-Encoding rejection, loopback-only Host guard (403 for foreign origins — CSRF/DNS-rebinding defense), RFC-correct 204 framing, traversal-safe static file serving (tests for encoded and raw `..`).
- 6 bridge tests incl. live SSE delivery + replay + Host guard.

**Desktop UI (`web/index.html`, dark, vanilla JS — no build step)**
- Add bar, active list (name, status chip, progress bar, speed, received/total, ETA, connection count), pause/resume/cancel controls, completed list with verified-hash badge, empty states, settings modal (engine info read-only + client prefs in localStorage), connection pill (Connected / Reconnecting / Disconnected), responsive layout.
- UI speaks the API v2 contract only (no engine logic duplicated); engine-independent disconnect state.

**Desktop research**: `NEXT-MILESTONE-RESEARCH.md` — Tauri 2 recommended (cited sources: tauri.app docs; Electron 80–150 MB/150–300 MB RAM vs Tauri ~2.5–10 MB; WebKitGTK caveats documented). The shipped web UI is deliberately the same asset a Tauri shell would host.

## 2. REAL download demonstration (done, live)

1. `throttled_httpd 96 12000000 0` — a real local HTTP server (96 MiB, throttled 12 MB/s per connection).
2. `velox serve-http` on port 3000 with the UI.
3. Drove the UI with a real headless browser: typed the sample URL, clicked **Add download**.
4. Watched real SSE progress (live snapshot: `DOWNLOADING · 137/256 MiB · 30.9 MiB/s · 8 connections · 4s left` on a second, larger file).
5. Pause verified: byte-frozen at 91.5 MiB → PAUSED. Cancel verified: row removed, part file cleaned.
6. Completed state rendered; file verified on disk: `preview-data/out/file` = **100,663,296 bytes**, sha256 `bbc3ab03e972f9e70ce06a8467c8c664a63665ec4356fb38d25a31f38df6bac9` — **identical to an independent curl of the source**.
7. Collision-safe naming observed (`file(1).bin`), DB persistence across engine restarts observed.
Screenshots: `benchmarks/ui-progress.png`, `benchmarks/ui-live.png`, `benchmarks/ui-completed.png`.
Launcher: `scripts/velox-preview.sh` (starts testserver + engine + UI; prints URLs).

## 3. Implemented but NOT fully verified

- **Real Windows execution.** No Windows runner is reachable from this sandbox. Verified: msvc cross-compile check, careful API usage per MSDN/official docs, cfg(windows) tests shipped. The GitHub workflow must run once on `windows-latest` before any Windows claim.
- windows-gnu target: needs a mingw toolchain (absent here) — only msvc was cross-checked.
- Tauri packaging: recommendation only; no Tauri project was scaffolded (out of scope for this milestone's preview deliverable).
- `run_in_background` smoke coverage of `serve-http` on a hostile network is limited to loopback tests.

## 4. Not implemented (deliberate)

- Engine-side emission of synthetic `queued/started/cancelled` events (the UI maps DB states; engine states are authoritative). Documented in API_V2.md.
- Durable event log (ring + resync covers a desktop UI), per-client event filtering, cookie-store PSL loading, `renameat2(RENAME_NOREPLACE)` for finalize's TOCTOU window (documented, bounded).

## 5. Independent reviews

Two sub-agent reviews (Windows/cookie security; bridge/UI/v1-compat) produced 32 findings. The material ones were fixed and regression-tested (OVERLAPPED, lock-drop unlink race, dir-flush write access, destructive Windows pre-remove on overwrite, PSL import hole, SSE subscribe/replay gap, stale-progress-after-structural, request smuggling, slowloris timeouts, Host guard, UI error/segments field shapes). Accepted stylistic clippy debt remains (pre-existing `&PathBuf` params etc.); no blanket allowances were added.

## 6. Gates (actual commands and results)

- `cargo test --workspace --no-fail-fast` → **93 passed** (baseline 80; +13 new), 0 failures. One timing-flaky test (`rename_orphan_final_file_is_never_adopted_without_evidence`) passed on rerun — pre-existing sensitivity, not touched by this milestone.
- `cargo fmt --all -- --check` → clean. `cargo clippy --workspace --all-targets` → 15 warnings, all pre-existing style class (accepted; listed in §5).
- `cargo check -p engine-storage --target x86_64-pc-windows-msvc` → clean.
- v1 compatibility: `engine-api` untouched semantically (reviewer-verified via git log + tests); v1 stdio tests all pass.
