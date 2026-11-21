# VELOX — OVERNIGHT AUTONOMOUS ENGINEERING MISSION (state file, read first)

Owner: Z. Window: 2026-11-20 evening → 2026-11-21 morning (~09:00).
This file is the SINGLE SOURCE OF TRUTH for any agent turn resuming the mission.
Read PROGRESS-LOG.md (append-only) for what already happened. Never redo completed
work listed there.

## Goal

Take Velox from P0-correctness to a trustworthy, reliable download manager
(comparable in reliability to IDM/FDM, no unproven speed claims). A downloader is
trustworthy ONLY if: ordinary files keep correct names/contents, interruption
recovery works, supported links resolve to real media, unsupported links fail
honestly with reasons, and the desktop app behaves consistently.

## Project layout

- Engine crates: `download-engine/` (workspace: engine-core, engine-*, velox CLI)
- Frontend (shared browser-preview + desktop UI): `download-engine/web/index.html`
- Tauri desktop app: `download-engine/app/src-tauri/` (standalone workspace,
  locked Cargo.lock; engine runs IN-PROCESS via Tauri commands)
- Docs: `download-engine/*.md` (ROOT-CAUSE-REPORT, EXTRACTORS, roadmaps, test results)
- Build scripts: `/home/z/my-project/scripts/` (persist across sessions!)
- Installer inputs: `/tmp/installer/` (WebView2 standalone exe, installer.nsi) —
  /tmp may vanish on recycle; WebView2 re-download command is in scripts/TOOLCHAIN-SETUP.md
- Deliverables: `/home/z/my-project/download/` — NEVER delete existing artifacts there.

## How to work (every turn)

1. `git -C /home/z/my-project/download-engine status` and read the last 3 entries of
   PROGRESS-LOG.md. Do not duplicate finished work.
2. Pick the highest-priority item from the BACKLOG below that is feasible in ≤50 min.
3. Implement in small steps. Run the relevant tests after every change:
   `source /home/z/my-project/scripts/rust-env.sh && cd /home/z/my-project/download-engine && cargo test --workspace`
   (gates: `cargo fmt --all -- --check`, `cargo clippy --workspace --all-targets -- -D warnings` must stay green)
4. Independent review every change set: read the diff as a reviewer hunting for
   incorrect completion states, silent corruption, range/resume bugs, races, leaks,
   credential leaks, wrong filenames, false support claims, Windows-specific breaks.
5. Append PROGRESS-LOG.md (timestamped, what/why/evidence). Commit:
   `git add -A && git commit -m "<cycle>: <what>"`
6. If a tool/host is unavailable, document the blocker in PROGRESS-LOG.md and move to
   the next feasible item. NEVER fake success. NEVER mark untested things complete.

## Hard rules

- Never mark an incomplete/invalid response as completed; verify bytes (SHA-256) in tests.
- Keep web/index.html dual-transport contract (Tauri IPC + HTTP/SSE) intact.
- Keep engine API wire contract stable (`engine_bridge::wire_event`).
- No placeholder features. No hardcoded test-server success in production code.
- Disk is 9.9 GB total: keep `du` in check; clean `download-engine/target` if > 3 GB.
- The Windows toolchain (if present: .xwin-cache/xwin/DONE + /tmp/llvmroot) must not be
  deleted. If missing, see scripts/TOOLCHAIN-SETUP.md to rebuild (~15 min).

## BACKLOG (work top-down; strikethrough/annotate completed in PROGRESS-LOG.md)

- [P0] Baseline verify: full `cargo test --workspace` green on current HEAD; record counts.
- [P0] Integrity gate: engine must verify final size vs Content-Length/final-bytes; on
  mismatch mark FAILED with reason (not completed). Add tests (truncated-server fixture).
- [P0] 416/429/Retry-After handling: exponential backoff honoring Retry-After; test fixture.
- [P0] Disk-space precheck + ENOSPC → clear FAILED state with reason; test.
- [P0] ETag/If-Range/Last-Modified revalidation on resume; tests.
- [P1] Chunked/unknown-length downloads: accurate progress without Content-Length; test.
- [P1] Multi-GByte simulated download test (sparse file) — memory flat, temp→final rename atomic.
- [P1] Duplicate-filename policy (append (1), (2)...); Unicode/long-path test on Linux; document Windows long-path behavior.
- [P1] Crash-recovery torture: SIGKILL mid-download → restart → resume from part offset; verify bytes.
- [P1] Auth/cookies: support user-supplied Cookie/Referer/User-Agent per download in CLI+UI; ensure no log leakage.
- [P1] Google Drive public-file adapter (download-only, no auth): confirm/normalize forms,
  large-file confirm token, document limits (no scraping of auth-gated content).
- [P1] yt-dlp adapter robustness: extractor availability probe, honest failure reporting, version drift handling.
- [P2] HLS/DASH investigation: document legal/technical scope in docs/EXTRACTORS.md; implement ONLY where a maintained lib is usable; else document restriction.
- [P2] Scheduling/queue: per-download start/delay option; bandwidth cap option (kbps); test.
- [P2] Benchmark single vs segmented on local slow-server fixture; record numbers in docs.
- [P2] Browser-extension/native-messaging design doc (NO implementation unless trivially safe); include security analysis.
- [P2] UI: only functional fixes surfaced by testing (errors readable, states truthful).

## Deliverables due 2026-11-21 morning (morning wrap cron job consolidates)

1. Updated source ZIP in download/ 2. Windows installer + exe (if toolchain) 3. updated
architecture/compat docs 4. provider support matrix 5. test report w/ SHA-256 evidence
6. independent code-review report 7. perf numbers 8. known limitations (P0/P1) 9. build
instructions 10. commit list.
