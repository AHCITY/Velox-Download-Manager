# VELOX — PROGRESS LOG (append-only; newest at bottom; every agent turn MUST append)

## 2026-11-20 ~17:40–18:05 (setup session)
- Windows toolchain rebuilt after sandbox rollback: cargo-xwin 0.23.1 installed
  (CARGO_HOME=/home/z/my-project/.cargo); clang-cl + lld-link 19.1.7 extracted from
  Debian debs to /tmp/llvmroot (rebuild recipe: scripts/TOOLCHAIN-SETUP.md); makensis
  3.13 extracted to /tmp/nsisdeb (NSISDIR honored).
- xwin SDK download hit ENOSPC (disk 100%); freed 3.4 GB (removed Linux target/ + junk).
- Phase-1 probe setup (scripts/xwin-setup-phase1.sh) launched to complete SDK+CRT
  splat; then delete dl/+unpack/ before the real build.
- NSIS installer script drafted at /tmp/installer/installer.nsi (per-user, Start Menu +
  desktop shortcuts, HKCU uninstall registration, WebView2 Evergreen standalone bundled
  offline: /tmp/installer/MicrosoftEdgeWebview2Setup.exe 294,099,168 B,
  sha256 47bc51387aff95c1275ce38cfbbb5fba…).
- MISSION.md + this log created; backup plan: download/ + backup/ + scripts/ persist
  across recycles; no external git remote available (documented).

## 2026-11-20 ~18:20 (cycle 1 — baseline audit + toolchain fixes)
- Toolchain fixes during Windows build bring-up (all documented in scripts/TOOLCHAIN-SETUP.md):
  missing clang resource dir (libclang-common-19-dev) broke ring's C build
  (stdalign.h not found); missing llvm-19 deb broke cc-rs (llvm-lib). Both fixed,
  build proceeding.
- Code audit of engine P0 behaviors (verified in source, not from reports):
  * PRESENT + tested: Retry-After honoring, 429 backoff (retry.rs/manager), ETag/
    If-Range/Last-Modified identity checks (task.rs check_identity), 416/Content-Range
    handling, truncated-transfer completion proof (finalize refuses Completed unless
    received == total when length known), hash-before-rename + durable rename + sync_dir,
    collision-safe final names, DISK_FULL→PAUSED-with-checkpoint policy (task.rs on_failed),
    gdrive adapter (classify/id/confirm-form/usercontent flow), strict URL naming
    (naming.rs strict_segment).
  * GAP FOUND: Windows ENOSPC not mapped — std::fs on Windows reports ERROR_DISK_FULL(112)
    / ERROR_HANDLE_DISK_FULL(39), from_io only mapped POSIX 28. On Windows a full disk
    would be classified generic Io (retryable) instead of DiskFull (pause+checkpoint).
  * FIX APPLIED: crates/engine-core/src/error.rs from_io now maps 112/39 → DiskFull
    under #[cfg(windows)] (POSIX 39=ENOTEMPTY/112=EHOSTDOWN untouched). Test to be run
    with the Linux suite once the Windows build frees disk.
  * Deferred (documented): disk-space PREcheck before starting (std has no statvfs;
    reactive ENOSPC→DISK_FULL→paused already covers correctness; precheck is UX sugar).


## 2026-11-20 ~18:40 (cycle 1 complete)
- Baseline verified on HEAD: cargo test --workspace = 133 passed / 0 failed
  (fmt + clippy -D warnings green). Coverage confirmed by reading tests:
  truncation, range-ignoring, wrong Content-Range, premature-close recovery,
  chunked unknown-length, HTML gates, Drive confirm interstitial, crash/resume.
- NEW TEST: http_correctness::overlong_body_completes_with_exactly_claimed_bytes —
  server claiming 1 MiB while sending 2 MiB must complete with exactly the first
  claimed MiB (RFC 7230 framing), hash-verified. PASS. Suite now 134 tests.
- FIX: from_io maps Windows ERROR_DISK_FULL(112)/ERROR_HANDLE_DISK_FULL(39) →
  DISK_FULL (cfg(windows)); previously only POSIX ENOSPC(28) → on Windows a full
  disk was misclassified as generic retryable Io.
- Installer pipeline COMPLETE: Velox.exe 13,807,104 B (sha ea1fec36…),
  Velox-Setup-x64.exe 5,217,400 B online (sha b328f625…),
  Velox-Setup-x64-offline.exe 299,179,674 B (sha 4dd5744b…); delivered to
  download/windows-build-20261120/ with SHA256SUMS.txt + INSTALL-AND-VERIFY.md.
- Night plan: no cron tool in this runtime (twice: "No such tool available") —
  deterministic watchdog (scripts/nightly-watchdog.sh, nohup, 30-min cycle,
  14 h max) guards fmt/clippy/tests and archives failures; deeper agent cycles
  require a user message in the morning. Documented honestly.
- **2026-11-20 18:26 [watchdog]** SKIPPED test gate (only 109156KB free disk)

## 2026-11-21 ~11:20 (cycle 2 — sandbox rollback recovery + Phase-1 audit + fixes)
- ROLLBACK #2: git history reset to initial commit; engine tree reduced to 2 files;
  .cargo/.rustup + my-project/scripts wiped; /tmp toolchain gone; watchdog died.
  RECOVERED from backup/velox-source-20261120-cycle1.zip (113 files) — the backup
  strategy worked exactly as designed. Rust toolchain reinstalled into workspace paths
  (cargo/rustc 1.100, both targets); cargo-xwin reinstalled; rust-env.sh recreated.
  Restored tree committed (b8df3c6) + baseline suite verified 133/0 green.
- PHASE 1 AUDIT COMPLETE: docs/ENGINE-AUDIT-2026-11-21.md. Solid areas evidenced
  (completion proofs, durability, redirect scoping, admission control, cookie jar,
  naming). Confirmed defects D1-D7 with file:line evidence; non-issues documented.
- PHASE 3 FIXES (all test-verified, suite now 140 passed / 0 failed, fmt+clippy clean):
  * D1 (P1): yt-dlp orphan-process leak — kill_on_drop(true) + configurable
    extractor_timeout. E2E: hung extractor surfaces TIMEOUT and its PID is reaped.
  * D2 (P1): user cookies/referer/UA/auth now forwarded to the extractor process as
    --add-headers (CRLF-sanitized; inline RFC 4648 base64 for Basic, no new deps).
    E2E: cookie reaches extractor argv through the full manager flow.
  * D3 (P1): origin-scoped credentials at resolve level (RequestHeaders::
    scoped_credentials) applied to Drive confirm-walk targets + resolved stream URLs —
    page-controlled hosts can no longer harvest the user's cookies/auth.
  * D4 (P2): Resolving/Resolved events (engine + wire bridge) — extraction phase is
    now observable before any row exists.
- DEFERRED (documented in audit): D5 mp4-preference decision, D6 verify() event
  contract, D7 page-fetch hardening.

## 2026-11-21 ~11:15 (cycle 3 — D8 lock-file residue + real-world evidence)
- REAL-WORLD E2E (engine CLI, live internet): Google Drive share link
  file/d/0B9P1L--7Wd2vU3VUVlFnbTgtS2c/view → resolved → `spam.txt` (5 B, content
  byte-verified). EXPOSED DEFECT D8: `spam.txt.part-1.lock` left in the user's OUT dir
  (mission §5 violation).
- D8 FIXED (no unlink-on-drop — the existing comment documents a real flock+unlink
  two-holder race): lock files moved to `<data_dir>/locks/` (internal; basenames keep
  per-download uniqueness via global ids); race-free GC of stale locks (mtime-gated,
  flock-checked) at manager startup; storage tests updated; new integration test
  `out_dir_is_clean_after_completion` (out dir must hold EXACTLY the final file).
  Re-verified with a fresh real Drive download: out/ clean, lock internal.
- YouTube real probe (2026.10.26, fresh logs): datacenter IP → "Sign in to confirm
  you're not a bot … Use --cookies" — confirms ROOT-CAUSE §4: IP-scoped bot wall, and
  the remedy (cookies) is exactly what D2 wired through. Not an engine defect.
- Suite: 142 passed / 0 failed; fmt + clippy -D warnings clean. Commit 7092612 (cycle2)
  then this cycle.

## 2026-11-21 ~11:35 (cycle 4 — Phase 5 benchmarks + docs)
- Benchmark suite (scripts/run-benchmarks.sh + bench-server.py, Range-capable loopback):
  256 MiB payload. RSS 12.8-13.1 MB on 256 MiB downloads → memory bounded (mission §8);
  single 0.12s vs segmented 0.31s on loopback (expected — loopback serializes; honest
  scope statement, no IDM/FDM comparison); 4×64 MiB concurrent 0.36s; SIGKILL@~60MiB →
  resume → byte-exact SHA-256 vs source. benchmarks/BENCH-2026-11-21.md.
- Measure harness lost to rollback #2 was the cause of the first empty run (recreated).
- Docs synced: audit D8 entry + fresh YouTube bot-wall log; EXTRACTORS.md documents the
  D2 header pass-through + kill_on_drop.

## 2026-11-21 ~12:20 (cycle 5 — mission docs + disk-preflight correctness cycle)
- Missions 2-4 delivered: docs/EXTERNAL_VELOX_AUDIT.md (objective similarity audit vs
  qamarabbas408/velox-download-manager: no code matches; MIT, active since Aug 2026;
  ideas adopted: disk preflight, missing-file detection, dup-URL policy [already
  present], browser-assistance concept for Phase 9), docs/OPEN_SOURCE_AUDIT.md
  (aria2 GPL-2 → not integrated; yt-dlp Unlicense ✓; FFmpeg LGPL build-only spec;
  curl rejected; dep licences verified), docs/ENGINE_ARCHITECTURE_REVIEW.md (10-layer
  mapping; UrlAdapter trait + MediaPipeline proposed, core kept),
  docs/dependency-licence-report.txt + scripts/gen-license-report.sh (155 OK, 0 GPL).
- CYCLE 5 ENGINE WORK (audit M5, adopted from external-audit ideas, P0 disk handling):
  * engine-storage::available_space (statvfs / GetDiskFreeSpaceExW) + SPACE_MARGIN
  * manager add(): destination preflight — create dir, refuse DiskFull BEFORE any row
    when total+64MiB > free; "destination unavailable" distinct from "low disk space"
  * task worker: periodic re-check at each checkpoint → honest DiskFull failure,
    resumable state preserved
  * StatusView.file_missing: completed rows whose final file vanished surface honestly
    (state never rewritten); verify() refuses missing files explicitly
  * duplicate-URL policy verified already-present (find_same_destination → existing_id)
- Suite: 146 passed / 0 failed (4 new tests); fmt + clippy -D warnings clean.
- Windows build #3: SDK splat was truncated by rollback (stale DONE marker, 114 MB
  without CRT headers) → full re-splat; build failed once on an in-flight editor error,
  re-running after cycle-5 code landed.

## 2026-11-21 ~15:30 (rollback #3 + restore + GitHub prep)
- ROLLBACK #3 at session start: git history reset to a NEW initial commit (5a93517),
  engine tree reduced to 4 .rs files, cargo/rustup wiped, scripts wiped (only
  installer-online-template.nsi survived). Survived: cycle-5 ZIP backup, windows-build
  artifacts, xwin SDK splat.
- RESTORED from backup/velox-source-20261121-cycle5.zip (122 files: all cycle-5 code,
  docs, progress log). rustup reinstalling. All build/watchdog/bench/licence scripts
  recreated from session records and committed (they are now part of the repo).
- GitHub prep (user request): remote origin added
  (https://github.com/AHCITY/velox-download-manager.git — name adjustable),
  scripts/github-push.sh (token via env only, never stored, push verified against
  remote head), root .gitignore hardened (.cargo/.rustup/.xwin-cache/backup/download/
  .env/upload excluded; 133 files tracked). Git-over-HTTPS egress VERIFIED working
  (git ls-remote to github.com OK). Awaiting: repo creation + PAT from user.
- **2026-11-21 15:30 [watchdog]** GREEN: fmt+clippy clean, 146 tests passed

## 2026-11-21 ~16:00 (GitHub sync LIVE)
- Push SUCCEEDED and verified: remote main == local 8321808 (7 commits: engine
  cycle-5, scripts, docs, LICENSE merge, watchdog GREEN line). Repo renamed by
  user to AHCITY/Velox-Download-Manager; remote URL updated to canonical case.
- Token workflow: fine-grained PAT via GITHUB_TOKEN env only (scripts/github-push.sh),
  never stored in files or remote URL. Required permissions: Contents R+W,
  Workflows R+W (windows.yml in tree).
- From now on: every verified milestone ends with a commit + push. GitHub is the
  durable source of truth (3rd sandbox rollback proved local-only storage unsafe).

## 2026-11-21 ~16:15 (roadmap 3 — UrlAdapter refactor)
- NEW: crates/engine-core/src/url_adapter.rs — UrlKind {Direct, Media}, UrlAdapter
  trait (parse/classify, no I/O), HttpUrlAdapter = the manager URL gate ported
  VERBATIM (http/https only, host required, no embedded credentials; identical
  messages), validate() via static adapter. 8 unit tests incl. trait-object usage.
- manager.rs::validate_url now delegates to the seam (call sites untouched, zero
  behavior change — incl. documented lenient-parse parity for `https:///path`).
- lib.rs re-exports UrlAdapter/HttpUrlAdapter/UrlKind.
- Suite: 154 passed / 0 failed (146 prior + 8 new), fmt+clippy -D warnings clean.
- Media classification intentionally returns Direct for all URLs today; DASH/HLS
  detection arrives with the MediaPipeline milestone (roadmap 4).
- **2026-11-21 16:04 [watchdog]** FAILED — logs archived: /home/z/my-project/backup/watchdog-failures

## 2026-11-21 ~16:50 (roadmap 4 — MediaPipeline merge core)
- NEW crate crates/engine-media (manager-level wrapper per review §9): MediaPipelinePort
  (subprocess port, faked in tests), SubprocessPort (tokio::process, kill_on_drop +
  cancel-trigger select), build_merge_args (PINNED `-c copy` argv, transcode impossible),
  verify_lgpl (rejects --enable-gpl/--enable-nonfree per audit §D), validate_probe_json
  (≥1 stream + format section), MediaPipeline::discover (honest MISSING_FFMPEG early),
  merge() with Merging/Merged events. 10 tests incl. REAL-PROCESS kill-safety (sleep).
- Kill-safety semantics: watch trigger true OR channel-close both kill the child
  (manager gone → child dead). Call sites hold senders for the run duration.
- engine-core: ErrorKind + MissingFfmpeg/GplBuild/CorruptMedia/MergeFailed (all
  Permanent), events + Merging{id}/Merged{id,path}; engine-bridge wires both to SSE
  ("merging"/"merged").
- NEW docs/FFMPEG_INTEGRATION.md — the spec the audit referenced (was missing):
  licence policy + verification procedure, kill-safety contract, event contract,
  honest errors, argv shape, testing rules, non-goals.
- Suite: workspace GREEN (164 total incl. 10 new), fmt+clippy -D warnings clean.
- Next slice: manager job-type wiring (merge as post-download phase) + yt-dlp resolve.

## 2026-11-21 ~16:25 (Windows installer published to GitHub Releases)
- Release v0.1.0 (tagged @ 01e3fe5, make_latest): Velox-Setup-x64.exe (5.2 MB),
  Velox.exe portable (13.8 MB), SHA256SUMS.txt, INSTALL-AND-VERIFY.md.
- Verified: local sha256 == SHA256SUMS.txt BEFORE upload; round-trip download from
  GitHub hashes identical (a848e272…). Release notes document certutil verification,
  LGPL-ffmpeg-only policy (no bundling), and that the -offline variant is not included
  (cleaned for disk pressure earlier).
- Script: scripts/github-release-upload.sh (token via env only).
- **2026-11-21 16:34 [watchdog]** GREEN: fmt+clippy clean, 164 tests passed

## 2026-11-21 ~17:15 (roadmap 4 slice 2 — merge phase wired into the manager)
- engine-core: NEW MediaMergePort trait (ports.rs — same seam pattern as HttpPort);
  DownloadManager gains media_merge + merge_triggers fields, set_media_merge()/
  has_media_merge(), and merge_streams(video_id, audio_id): both rows must be
  COMPLETED with files on disk → video row MERGING (event) → port.merge
  (`-c copy` + ffprobe via engine-media) → COMPLETED with merged path; honest
  failures set FAILED+MERGE_FAILED, cancel sets CANCELLED. Output naming: stem + ext
  (media_merge.output_ext, default mp4), collision → name(1).ext unless overwrite.
  Merged row hash fields cleared honestly (stream hashes ≠ merged artifact).
- cancel() now kills in-flight merges via the trigger (60s settle); pause() during
  MERGING refuses honestly (ffmpeg has no resume).
- config: DownloadConfig.media_merge: Option<MediaMergeSpec { audio_url, output_ext }>
  — declared contract for the resolved-job flow (#[serde(default)] keeps old rows).
- engine-state: states::MERGING.
- engine-media: MergePort<P> adapter implements MediaMergePort (delegates to
  MediaPipeline); discover_subprocess() for hosts.
- 6 new manager tests (happy path + events, not-configured honesty, failure marking,
  cancel-during-merge kill-and-forget, collision numbering, output_ext spec). Suite:
  170 passed / 0 failed, clippy -D warnings clean.
- CI (roadmap 7): workflows moved to repo ROOT (nested download-engine/.github was
  invisible to GitHub — windows.yml had NEVER run); ci.yml added. First runs: ci ✅
  SUCCESS on f3b8ac7; windows ❌ at the test step — 5 PRE-EXISTING windows-specific
  failures (preflight destination, extractor stubs, crash-resume kill) surfaced by the
  first real Windows run; desktop job correctly skipped (dispatch-only; app/ sources
  not in repo). Windows test fixes = next work item.

## 2026-11-21 ~17:35 (windows CI gate fixes)
- 5 pre-existing Windows-only failures from the first real windows.yml run:
  1-2. preflight "unavailable destination" tests used /proc paths — Linux-only trick
     (Windows creates C:\proc\... happily). Replaced with a cross-platform blocker:
     a path UNDER an existing FILE (create_dir_all fails on every OS). [manager.rs
     preflight_tests + tests/integration.rs]
  3. crash-resume kill_before_first_checkpoint used a fixed 150 ms sleep — on Windows
     process startup can outlast it, so the kill landed before the manifest row was
     written. Now polls for row existence (up to 15 s) then kills immediately — same
     intent, deterministic on every OS. [engine-cli/tests/crash_resume.rs]
  4-5. extractor stub tests are Unix-only fixtures (#!/bin/sh stubs, /proc/{pid}
     liveness) — gated #[cfg(unix)] with KNOWN-ISSUES #11 tracking Windows equivalents
     (powershell stub + tasklist liveness).
- KNOWN-ISSUES: #6/#8/#9 closed (GitHub sync, UrlAdapter, Windows rebuild); #11 added
  (windows gate not green until windows.yml passes).
- Local suite still 170/0; pushed for CI re-run on both platforms.

## 2026-11-21 ~17:50 (roadmap 5 — YouTube verification checklist)
- NEW docs/YOUTUBE_VERIFICATION.md: user-executable residential-network checklist —
  S0 baseline (standalone yt-dlp isolates the IP question), S1 honest refusal,
  S2 extractor+download (core case), S3 cookie remedy (D2 path), S4 DASH two-stream
  + merge incl. cancel-during-merging kill test; honest-failure taxonomy
  (MISSING_FFMPEG/GPL_BUILD/CORRUPT_MEDIA); per-scenario recording format for
  KNOWN-ISSUES #1; lists exactly what CI already proved vs what residential testing
  adds. Copy delivered to download/YOUTUBE_VERIFICATION.md for the user.
- **2026-11-21 16:51 [watchdog]** GREEN: fmt+clippy clean, 170 tests passed

## 2026-11-21 ~18:05 (windows gate round 2 — lock + third extractor fixture)
- 90f719f windows run: the 5 earlier fixes PASS; 3 more failures surfaced (they were
  behind the earlier aborts): (a) stub_extractor_resolves_watch_url… — third Unix
  #!/bin/sh fixture, cfg(unix)-gated like the others; (b) download_lock_exclusive…
  — Held(pid) reads 0 on Windows because the holder's exclusive whole-file LockFileEx
  region makes the lockfile unreadable: exclusivity WORKS, only diagnostics differ;
  test now asserts pid==0 on windows, Display says "pid unavailable on this platform"
  (honest, no fake pid 0); (c) lock_recovered_after_child_process_death (windows
  variant) spawned the child test binary WITHOUT the test-filter args, so the
  #[ignore]d lock-holder never ran — args added (same as unix variant).
- All three are test/fixture-level; no product code change except the honest
  LockError::Held(0) Display wording. Suite 170/0, clippy clean.
- **2026-11-21 17:05 [watchdog]** GREEN: fmt+clippy clean, 170 tests passed
- **2026-11-21 17:21 [watchdog]** GREEN: fmt+clippy clean, 170 tests passed

## 2026-11-21 ~19:20 (roadmap 7 COMPLETE — both CI gates green, first ever)
- windows.yml GREEN on 80dfe9e: fmt, clippy -D warnings, build, FULL workspace tests,
  CLI stdio smoke, API v2 smoke (Host guard), crash-recovery focus (windows lock
  variant + serialized crash_resume). ci.yml green on every commit since a65f6f2.
- Rounds: (1) /proc paths, kill-sleep race, extractor fixtures gated; (2) third unix
  fixture, lock Held(0) honesty, child test-filter args, unix-only imports; (3)
  crash-test child stderr captured (DXDL_HOME created first — my 263952e regression,
  caught locally before CI), 60 s row deadline, crash tests serialized on CI.
- KNOWN-ISSUES #11 closed (residual: windows extractor-stub equivalents to write).
- Engine acceptance-gate item "regression suite green on BOTH platforms" now holds on
  GitHub-hosted real Windows runners.
- **2026-11-21 17:35 [watchdog]** FAILED — logs archived: /home/z/my-project/backup/watchdog-failures
- **2026-11-21 17:52 [watchdog]** SKIPPED gate (only 0KB free disk)
- **2026-11-21 18:06 [watchdog]** FAILED — logs archived: /home/z/my-project/backup/watchdog-failures
- **2026-11-21 18:22 [watchdog]** SKIPPED gate (only 1397252KB free disk)

## 2026-11-21 — Mission cycle 2: merge stage wired end-to-end (P0 fix)
- engine-api: `download.merge {video_id, audio_id}`; `engine.info` reports `merge_capable`.
- engine-cli: `velox merge` subcommand; `wire_media_merge()` discovers LGPL ffmpeg on PATH at serve/serve-http/merge startup (honest stderr either way).
- Tauri app: engine-media dep + merge-port discovery injected at setup.
- Tests: workspace 171 passed / 0 failed / 1 ignored; fmt + clippy -D warnings clean. Audit: docs/AUTONOMOUS_CYCLE_INITIAL_AUDIT.md.
- Known follow-up (next cycle): merge output still written to final path by the pipeline; temp+.part rename + MERGING startup sweep queued (P0-2).
- **2026-11-21 18:40 [watchdog]** SKIPPED gate (only 966648KB free disk)
- **2026-11-21 18:52 [watchdog]** SKIPPED gate (only 355848KB free disk)
- **2026-11-21 19:10 [watchdog]** FAILED — logs archived: /home/z/my-project/backup/watchdog-failures
- **2026-11-21 19:22 [watchdog]** SKIPPED gate (only 1214116KB free disk)
