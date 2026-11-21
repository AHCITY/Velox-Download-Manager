# AUTONOMOUS CYCLE — INITIAL AUDIT (2026-11-21)

Mission start: 2026-11-21 (late), continuing via scheduled cycles through 2026-11-22.
Repo: AHCITY/Velox-Download-Manager · HEAD at audit: `69810c0` (main) · GitHub CI: linux GREEN, windows GREEN (crash gate included).

## Method

Three independent read-only audit agents inspected engine-core/storage/cli/http,
engine-http extractor + media pipeline, and the Windows/installer/desktop layers.
Every finding below carries file:line evidence from the actual tree. Claims from
progress docs were cross-checked against code, not trusted.

## Actual repository state

- Workspace: 7 crates (state, storage, http, core, media, api, bridge, cli, testserver) + Tauri 2 desktop app (`download-engine/app/src-tauri`) + single-file web UI (`download-engine/web/index.html`).
- Engine suite: 164 tests green at last full run; fmt + clippy `-D warnings` clean; Linux + Windows CI green (incl. SIGKILL crash-resume gate).
- Windows v0.1.0 installer published on GitHub Releases earlier today; a rebuilt installer from HEAD is being staged this cycle.

## Working functionality (verified in code, not just docs)

- Segmented multi-connection HTTP(S) downloads with per-segment checkpointing, tail stealing, fsync-before-commit; completion requires `received == total`; checksum verification path; HTML/encoding gates; Content-Range both-end validation; span clamping.
- Crash recovery: SIGKILL mid-download resume byte-exact (crash_resume.rs, 4 scenarios, Windows + Linux); rename-orphan adoption with hash proof; cross-process OS locks (LockFileEx on Windows) with honest Held(pid) display.
- Admission control (global + per-host semaphores), disk preflight, DiskFull → paused-not-failed, Retry-After-aware retry (5 attempts, 1→60s, jitter), redirect loop detection (10 hops) with cross-origin auth stripping.
- yt-dlp extractor contract: subprocess JSON protocol, argv pinning, timeout+kill, LGPL ffmpeg gate, ffprobe validation — all CI-tested contracts (real YouTube serving remains unverified from datacenter IP; honest per YOUTUBE_VERIFICATION.md).
- JSON-RPC API (inspect/info/create/start/pause/cancel/list/status/verify), SSE bridge, per-user NSIS installer with WebView2 bootstrap.

## Confirmed defects (evidence-backed, prioritized)

### P0-1 — Merge machinery is unreachable end-to-end (three agents converged)
- `set_media_merge()` has zero non-test callers; `merge_streams` returns "media pipeline not configured" in production (manager.rs:137, :637-770).
- engine-api has no merge method (engine-api/src/lib.rs:135-235); CLI has no merge subcommand; Tauri crate does not depend on engine-media; `MediaMergeSpec.audio_url` config is dead (config.rs:118-134, read nowhere).
- Extractor refuses adaptive streams unconditionally (engine-http/src/extract.rs:249,333) so DASH can never reach merge even if wired.
- YOUTUBE_VERIFICATION.md S4 instructs a two-stream merge flow that no interface can execute today.

### P0-2 — Merge phase is not crash-recoverable
- No startup sweep of `merging` rows: after a crash mid-merge the row is stuck forever (merge_triggers are in-memory; manager.rs:117-141, :864-887).
- ffmpeg writes directly to the final path with `-y` (engine-media/src/pipeline.rs:50-67): a killed merge leaves a truncated file at the destination; retry picks `name(1).ext` and garbage accumulates; post-restart cancel() deletes only temp_path, leaving the orphaned partial output.

### P1 (high)
1. **Pause/cancel dead during resolve/probe retries** — probe loop ignores the command watch (task.rs:211-234); retry budget can exceed pause/cancel deadlines (30/60s) → user-visible hang, state diverges from intent.
2. **yt-dlp URL expiry unhandled** — expired googlevideo URL → 403 → classified Permanent, no re-resolve path (extract.rs:216-219; error.rs:209).
3. **Cookie jar never reaches extractor** — `velox cookies import` populates the backend jar but `extractor_header_args` reads only per-request headers (extract.rs:45-80; cli main.rs:764-835).
4. **Auth forwarding untested e2e** — `saw_auth_header` recorded by testserver but no test reads it; a regression in same-origin scoping would leak credentials silently (engine-testserver/src/lib.rs:140,338).
5. **Installer hygiene** — online template header says "OFFLINE variant" (copy-paste provenance hazard); WebView2 detection accepts broken runtime `pv="0.0.0.0"`; no downgrade protection; version skew installer 0.2.0 vs engine 0.1.0 (installer-online-template.nsi:1-4, :57-62).
6. **WINDOWS_COMPAT.md contradicts shipped code** — claims no-op fsync + pid-file heuristic; code has FlushFileBuffers + LockFileEx (engine-storage/src/lib.rs:159-165, :432-463). Long paths: no `\\?\` prefixing anywhere.

### P2 (medium)
- `inspecting`/`verifying` dead states (engine-state/src/lib.rs:25,28); `Event::Verified{ok:true}` vs `verified=false` when no checksum expected (manager.rs:1050/1055, known D6).
- Finalize doesn't truncate known-length residue tails; preallocate/set_len failures swallowed (task.rs:356-359, :1160-1165).
- `config_json` corrupt parse silently drops verify spec (manager.rs:463); no single-flight guard on merge_streams (manager.rs:637-709) — moot until P0-1 fixed, then real.
- Worker HTML gate only checks file offset 0 — a 206 with correct Content-Range but HTML body passes unchecked mid-file (task.rs:902-905).
- Host semaphore keyed by host_str not origin; permit held through retry backoff sleeps (manager.rs:168-179; task.rs:719-725).
- CLI `inspect` builds a second backend; error matching by string ("already completed") instead of typed kind (cli main.rs:491, :371).
- Plain-GET answered 206-without-Content-Range burns restart budget (task.rs:876-881).
- Extractor spawn failure misclassified as UNSUPPORTED_URL (extract.rs:149-158).
- bench-server.py malformed on unsatisfiable/suffix-overflow ranges (negative Content-Length; seek crash) and supports no fault injection (scripts/bench-server.py:19-42).
- No connection-pool tuning (idle sockets unbounded per host); no global retry budget (flapping server sustains indefinitely — acceptable but worth an event).
- UrlAdapter.classify is a stub returning Direct always; real media classification lives in engine-http/gdrive.rs — parallel classification points (url_adapter.rs:57-62; gdrive.rs:31-64).
- Per-download connection count never grows beyond initial wave (plan.rs:58-61) — documented conservative, but the "grows via stealing" comment misleads.

## Windows-specific risks

- Long-path (>260 total) unhandled; filename capped at 240 but dest_dir unbounded (naming.rs:61).
- App close during a merge would orphan ffmpeg (no pdeathsig equivalent; shutdown only pauses rows with running task — main.rs:185-194). Blocked on P0-1 being reachable, then real.
- Crash-mid-merge recovery (P0-2) applies to Windows shutdown too.
- Installer templates drift from Tauri bundler NSIS config; INSTALLER-TEST-RESULTS.md honestly records that no real installation test was executed.

## Test results at audit time

- Workspace suite: 164 passed, 0 failed (last full local run this session on this tree).
- fmt --check / clippy -D warnings: clean.
- GitHub CI (both workflows): green on 69810c0.

## Prioritized engineering plan (cycles)

1. **Cycle 1 — Wire merge end-to-end (P0-1):** `download.merge {video_id, audio_id}` in engine-api dispatch; `velox merge` CLI command; `MergePort::discover_subprocess()` + `set_media_merge()` in CLI and Tauri startup (honest stderr when ffmpeg absent); API-level test.
2. **Cycle 2 — Merge safety (P0-2):** output to `<final>.merging.part` + rename on success; delete part on cancel/error; startup sweep marking stale `merging` rows failed with MERGE_INTERRUPTED; single-flight guard.
3. **Cycle 3 — Responsiveness + finalization belt:** stop-aware probe loop and backoff sleeps (select! against command watch); truncate-to-total at finalize with honest errors; un-swallow set_len failures.
4. **Cycle 4+ (from backlog):** auth/redirect e2e tests; sustained-429 exhaustion test; extractor 403 re-resolve; extractor cookie jar; installer hygiene batch; WINDOWS_COMPAT.md reconciliation + `\\?\` long-path helper; plain-GET-206 tolerance; HTML-sniff per-chunk; bench-server 416 fix + fault flags; dead-state cleanup; typed CLI errors.

## Honest limitations

- Sandbox disk is tight (~5G free after cleanup): cross-build must coexist with test compiles; cycles clean target dirs when needed.
- Cross-compiled Windows exe cannot be smoke-run here (no Windows); correctness rests on the workspace suite + CI + installer hash round-trip.
- Real YouTube/ffmpeg end-to-end cannot be exercised from a datacenter IP (bot wall) — contracts are CI-tested; the merge flow becomes testable locally once Cycle 1 lands (ffmpeg present in sandbox for integration tests).
