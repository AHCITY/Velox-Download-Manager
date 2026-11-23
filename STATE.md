# STATE.md — Velox Download Manager mission state

Last updated: 2026-11-23 — **RELEASE TURN**: v0.5.0 shipped via CI (signed installer + latest.json + extension zip). Keep current; commit before ending every turn.

## Where things are

| Path | Purpose |
|---|---|
| `download-engine/web/` | **THE PRODUCT UI** — Tauri loads it via `frontendDist: "../../web"`. `index.html` + `app.js` shell + `assets/{app.css, icons.js, api.js, store.js, ui.js}` + `assets/screens/{home,downloads,addDialog,queues,network,browser,devices,history,vault,settings}.js` |
| `download-engine/docs/IMPLEMENTATION_MAP.md` | UI-overhaul audit → screen/contract map (read this first next session) |
| `download-engine/crates/engine-core/src/manager.rs` | `remove(id, delete_file)` (any state incl. completed), StatusView.bandwidth_limit_bps |
| `download-engine/crates/engine-api/src/lib.rs` | download.remove = record removal (NOT the old cancel alias); delete_file param |
| `download-engine/crates/engine-state/src/lib.rs` | delete_download cascades segments + errors |
| `download-engine/app/src-tauri/src/main.rs` | Tauri shell: rpc/open_path (DB-record-scoped)/default_download_dir/settings_get/settings_set; engine in-process; bounded close-pause |
| `download-engine/crates/engine-core/tests/remove.rs` | removal semantics tests (5) |
| `scripts/installer-online-template.nsi` | NSIS installer template (online bootstrapper variant) |
| `scripts/build-velox-msvc.sh` / `scripts/github-release-refresh.sh` / `scripts/github-push.sh` | build / publish / push |
| `scripts/slow-server.py` | throttled HTTP server for UI e2e (on :8392) |
| `scripts/rust-env.sh` / `scripts/TOOLCHAIN-SETUP.md` | toolchain env + rebuild recipe |
| `.secrets/github-token` | token (0600, gitignored) — recreate from chat if wiped |
| `download/windows-build-20261122-a/` | current installer staging (this session) |
| `download/mission-reports/` | copies of mission docs |
| GitHub release `v0.1.0` | durable artifact copies |

## Progress

- Engine cycles 1–7: complete (see `docs/AUTONOMOUS_CYCLE_PROGRESS.md`).
- **UI overhaul session (this one)**:
  - Audited the real stack: Tauri serves `web/`; one JSON-RPC contract over Tauri IPC (desktop) or HTTP POST /rpc + SSE /events (browser preview); events = DB state names + progress/resolving/resolved/merging/merged; the engine already tracked `file_missing` (audit M5) but the old 532-line UI ignored it.
  - Rebuilt the frontend: 9 screens, dark-only design system, real backend integration everywhere. The user's core complaint (can't manage deleted/missing downloads) fixed by surfacing `file_missing` + the new remove/delete-file APIs.
  - Backend additions (minimal, UI-integration only): `manager.remove(id, delete_file)` for ANY state incl. completed (artifact deleted BEFORE record; cascades segments/errors; emits `removed` event); `download.remove` re-routed to it (cancel keeps its semantics); StatusView exposes `bandwidth_limit_bps`; Tauri `settings_get`/`settings_set`; `open_path` trust anchor extended to engine-DB-recorded destinations. Gates green (fmt, clippy -D warnings, remove+api tests).
  - E2E in headless browser against `velox serve-http` + real file servers: dialog add with inspect preview, completion, resume wiring, live progress (speed/ETA/connections), file-missing badge, delete-file with confirmation (disk + DB verified), settings persistence across reload, Ctrl+A bulk bar, invalid-URL inline errors, context menus. Screenshots reviewed (Home/Downloads/Settings).
  - **Recorded gotcha**: rAF is throttled to zero in background/headless webviews — `store.notify` uses `setTimeout(0)`.
  - Commit `9cffc19` pushed + remote verified. Windows build launched after it; installer refresh + publish + round-trip follow.

## v0.5.0 release turn (2026-11-23) — CI-driven signed release

- Pushed the extension-fix + release-pipeline stack (3 commits) and tagged v0.5.0;
  windows.yml builds the signed NSIS installer on the tag and a release job publishes
  installer + .sig + latest.json + Velox-Extension.zip + SHA256SUMS.txt.
- CI bug found on the first tag run: the desktop job read the updater key via ..\..
  relative to working-directory download-engine → one level ABOVE the repo root
  ("Cannot find path 'D:\a\Velox-Download-Manager\scripts\...'"). Fixed by resolving
  via $env:GITHUB_WORKSPACE (commit f1a84b3); the tag was moved onto it
  (delete remote tag → retag → push — no release existed yet, so safe).
- Second tag run hit the known Windows flake class in crash_resume: parent opened the
  manifest right after SIGKILL → SQLITE_BUSY, and the row-insert-before-segment-plan
  window tripped "segments planned". Fixed in tests (commit 818f01b): db() retries
  busy opens ~5s; empty segments accepted while received==0. Tag moved again.
- SHIPPED + VERIFIED (run 45219262512, all jobs green): release v0.5.0 contains
  latest.json (version 0.5.0; sig trusted-comment version:0.5.0 — require_signed_version
  satisfied; tag-pinned asset URL), Velox-Setup-x64.exe 4.3 MB + .sig, Velox-app-x64.exe,
  Velox-Extension.zip, SHA256SUMS.txt.
- Push flow: scripts/publish-release.sh with the token at .secrets/github-token
  (0600, gitignored). CI watch: scripts/ci-poll.sh; release verify: scripts/release-check.sh.
- The GitHub connector (account AHCITY) can read + create branches but CANNOT
  git-push/tag — token-based push is the reliable path. Temporary connector probe
  branch deleted after the push.
- CAVEAT: 0.4.0 and older installs pin the pre-recycle updater key (lost) — they
  cannot auto-update to 0.5.0; one manual install of Velox-Setup-x64.exe required.

## Auto-updater (added 2026-11-22, v0.3.0)

- **Mechanism**: official `tauri-plugin-updater` 2.14.1 + `tauri-plugin-process` (relaunch), registered in `app/src-tauri/src/main.rs`. Endpoints: `releases/latest/download/latest.json`. Passive NSIS install mode. Permissions in `capabilities/main.json`: `updater:default`, `process:default`.
- **Signing**: `.secrets/velox-updater.key` (+ `.pub`) — **if lost, existing installs can never receive updates again**. Build/packaging must pass the RAW base64 (last line of the key file) via `TAURI_SIGNING_PRIVATE_KEY` (paths are NOT accepted; the "untrusted comment" line breaks decoding).
- **CRITICAL**: the plugin cross-checks the signature's trusted-comment `version:` against latest.json's announced version (`require_signed_version`) — ALWAYS sign with `tauri signer sign --app-version "$VERSION"` (done in `scripts/package-update-release.sh`).
- **Release flow**: bump version in `tauri.conf.json` + app `Cargo.toml` + NSIS template → `build-velox-msvc.sh` → copy NEW exe to `/tmp/installer/Velox.exe` (the NSIS template embeds THAT path, not target/) → `makensis` → `package-update-release.sh <installer> <staging>` (signs + generates latest.json + SHA256SUMS) → write INSTALL-AND-VERIFY.md → `github-release-refresh.sh <staging> v<version>` (creates the release if missing).
- **GOTCHA**: changing `web/` does NOT re-embed assets on rebuild — `touch app/src-tauri/src/main.rs` before `build-velox-msvc.sh`, or the exe ships the OLD frontend (this bit us once; v0.3.0 published binaries are the fixed ones).
- **Verification**: `/tmp/msv` crate replicates `verify_signature` 1:1 (main+global sig + version cross-check); used to validate the published artifacts. Frontend flow E2E-tested with a mock `__TAURI__` (check → toast+badge → progress → relaunch).
- `docs/UPDATES.md` documents the whole flow.

## UX Overhaul 2.0 (v0.3.1, 2026-11-22) — done + gotchas

- Implemented + pushed (commit da81f07). Downloads-first shell, unified filter list, search bug fixed
  (missing `notify()`), full-width rows, Settings with real default-dir picker (tauri-plugin-dialog,
  `dialog:default` capability — only backend change), network screen absorbed Home content.
- E2E: `scripts/e2e-ui.sh` — 33 checks vs the REAL engine (`velox serve-http`, random ports, self-contained
  slow fixture). 33/33 on 4 of last 5 runs. GOTCHAS (all bit us): (1) the slow-server reads
  `/tmp/vlx-e2e/files/slow.bin` — it now self-creates the fixture; (2) the makensis template embeds
  `/tmp/installer/Velox.exe` — COPY THE FRESH EXE THERE BEFORE makensis (bit us: v0.3.1 first package had a stale exe);
  (3) tauri-build does not re-embed changed web/ assets unless `main.rs` is touched (see updater section);
  (4) engine `download.cancel` REMOVES the partial record (row leaves the list — UI asserts that, not a chip);
  (5) dead-URL create/inspect takes ~31s (3x10s connect retries) before the honest CONNECT error — slow, not hung;
  (6) E2E port guard: pkill stale servers before starting (a leftover server hijacking the fixed port caused
  phantom failures); ports are now randomized per run.
- Release v0.3.1 published + verified (endpoint 0.3.1, minisign main+global VALID, version-bound, hash round-trip).

## Browser ecosystem turn (2026-11-22, post-v0.3.1) — done + gotchas

- Reference repo audited (cloned to /tmp, deleted after) → docs/FEATURE_COMPARISON.md + docs/THIRD_PARTY_NOTICES.md.
- Browser extension: download-engine/extension/ (MV3, plain JS, pinned key → ID `ffnpjmbhfmaamhlbohbmamnmoaikpdmc`, re-derived from the shipped key — the old `hpaxoarvxcmmqijm` literal was WRONG and broke every connectNative; fixed in the 2026-11-23 extension audit).
- Native host: crates/velox-host (`velox-browser-host`) — NM stdio ⇄ loopback HTTP JSON-RPC; 5 stdio E2E tests.
- App: loopback bridge in app/src-tauri/src/main.rs (random 127.0.0.1 port + token in app_data/data/bridge.json, 0600; whitelist engine.info/list/status/create/pause/cancel).
- P2 intelligence: SpeedStats tap (mirror broadcast subscriber) → StatusView bps_smoothed/avg/peak; UI details drawer shows avg/peak + etaState labels. Verified live (slow download: smoothed 1.8 / avg 2.1 / peak 4.2 MiB/s).
- Benchmark: docs/BENCHMARK.md — real before/after (0.02s vs 0.03s loopback, throttled 29.03s both) = no regression.
- NOT implemented (designed only): browser-assisted transfers (§4 of comparison), tray/taskbar/notifications (§5).
- GOTCHAS: (1) app crate can NOT build natively on Linux (no GTK) — always build it via scripts/build-velox-msvc.sh (xwin); (2) engine target/debug regrows fast — wipe when disk full, rebuild is ~1 min; (3) NO release publishing this turn (spec §13 forbids without authorization) — Velox.exe with the bridge saved at /tmp/Velox-exe-with-bridge.exe (host-local, not a deliverable).

## v0.4.0 turn (2026-11-22) — in-app extension installer + offline setup

- App drives browser setup: browser_detect / extension_setup / extension_status commands (main.rs); bridge last-activity timestamp drives "extension connected".
- Chromium allows NO silent unpacked install — app stages extension to %LOCALAPPDATA%\Velox\extension, registers the NM host (velox-browser-host --register), opens chrome://-extensions page; Quick start = --load-extension session load. Firefox honestly unsupported.
- Browser screen rewritten (was "not implemented"); first-launch dialog gated by prefs.browserSetupSeen.
- Installers: NSIS v0.4.0 templates bundle extension+host both variants; offline adds yt-dlp.exe + LGPL ffmpeg/ffprobe (BtbN win64-lgpl). makensis from debs: NSISDIR=/tmp/nsisroot/usr/share/nsis /tmp/nsisroot/usr/bin/makensis (sandbox recycle wipes apt).
- GOTCHA: shell cwd RESETS to /home/z/my-project between commands — always absolute paths or cd first (caused the "Velox.exe missing" scare).
- GOTCHA: cargo xwin needed for ANY msvc cross-target build (plain cargo build --target msvc lacks link.exe).
- Released v0.4.0 (user-authorized): online 6.0MB + offline 147.5MB + portable exe + latest.json (signed) + SHA256SUMS.

## Maintenance turn (2026-11-22, user away) — checks + small fixes

- FULL workspace gates: fmt clean, 26 suites / **193 tests ALL CLEAN** (`--no-fail-fast`; earlier fail-fast pass had hidden the rest behind one flake).
- FIXED flaky `crash_resume::kill_before_first_checkpoint` (full-workspace run): a fast download can COMPLETE before the kill lands under parallel load; `check_manifest_consistency` now skips completed manifests (segments are cleared on completion by design; final byte-exactness is hash-checked elsewhere). Kill windows widened (test 3: 2→4 MiB, test D: 2→8 MiB). 6 pre-fix runs couldn't reproduce standalone — fix is by-construction correct.
- FOUND+FIXED UI bug: `location.hash` was read only at boot — no `hashchange` listener, so browser back/forward and pasted deep links (#settings) never re-rendered. Now the hash is the single navigation source of truth (verified live in headless browser: hash nav re-renders without reload).
- QOL: Browser screen's 3s status-poll timer now stops on nav-away (screens can export stop* hooks, called by the router); first-launch dialog dynamic-import dedupe.
- Docs: README finally documents the extension + installers; INSTALLER-TEST-RESULTS.md refreshed (installers ARE built+published; real-machine install + real update hop still honestly pending); STATE "Remaining" un-staled (clipboard monitor ≠ extension's own opt-in clipboard).
- Verified clean: UI⇄engine method contract (11 methods, start/resume are engine-side aliases), event contract (DownloadState passes DB state strings through), settings.js "not implemented" rows are honest (global limit/proxy genuinely absent), headless renders of Downloads/Browser(preview note)/Settings all clean, E2E 33/33 after all frontend edits.

## Phase 0-7 production-reliability round (2026-11-22)

Root causes (full report: download-engine/docs/PHASE0-ROOT-CAUSES.md, RC1-RC10):
- RC1/2: `pick_stream` progressive-only + `MediaMergeSpec.audio_url` consumed NOWHERE at runtime (merge machinery existed but manual-only).
- RC5: no CREATE_NO_WINDOW on yt-dlp (extract.rs), ffmpeg/ffprobe (pipeline.rs), velox-browser-host --register (main.rs) → console flashes on Windows.
- RC7: updater.js relaunched 500ms after downloadAndInstall → installer/relaunch race → same update re-offered; installer taskkill /F force-closed the app.
- RC8: updater downloaded the CUSTOM NSIS installer that ignored Tauri /P /S /R /UPDATE flags (full wizard popped mid-"passive" update).
- RC6 (non-cause): live latest.json verified CORRECT (0.4.0, valid sig).

Fixes shipped (all tests green):
- extract.rs: `pick_plan` — progressive fast path kept; best video-only + best audio-only pair (m4a-pref) with `merge_container` (h264+aac→mp4, vp9/opus→mkv, never transcode); ResourceInfo.media carries MediaMergeSpec {audio_url, output_ext, output_name, audio_name}.
- manager.rs: add() threads info.media into cfg; runner spawns media job on Completed; run_media_job = resume-first audio reuse (find_same_destination, no re-probe — googlevideo URLs expire) → auto audio download → merge_streams → audio row+file removed → spec cleared. Honest failures (audio fail keeps video completed + spec; video removed mid-job cancels audio). merge_streams honors spec.output_name (clean final title).
- CREATE_NO_WINDOW (0x08000000) on all three Windows spawn sites.
- updater.js rewrite: strict semver guard (never offer equal/older — the false-update fix), once-per-install announce (localStorage), active-download count + confirm dialog → pause-and-await → downloadAndInstall → graceful self-exit (installer waits) → installer restarts app; postponing keeps the offer. update-version.js extracted pure + node:test suite (4/4).
- installer-unified-template.nsi replaces the online/offline pair (both deleted): ONE Velox-Setup-x64.exe; UPDATE MODE (/P /S /UPDATE) waits for app exit (no taskkill), silent install, auto-restart; interactive upgrade = graceful WM_CLOSE close + bounded wait + prompt (never force-kill); bundled-file verification (exit code 3); LGPL notices + ffmpeg LICENSE bundled; WebView2 Evergreen STANDALONE (294MB, true offline).
- BUILT + VERIFIED: unified installer 440,026,140 B, NSIS-3 Unicode; extracted via 7zz 26.06 — 23 files, Velox.exe + ffmpeg.exe byte-exact vs staging (old p7zip 16.02 parser fails on NSIS-3 Unicode — use 7zz).
- B2 dev cache (GLM-Z.AI, eu-central-003): verified auth+bucket+round-trip; scripts/b2cache.py (idempotent, large-file parts, retries); cached 1.05GB (installer + all bundled exes) + tools/win64; xwin-cache upload in flight.

Gates at round end: cargo fmt ✓, clippy workspace 0 issues, JS tests 4/4, E2E 33/33 (current frontend), engine-core 57/57 (incl. media-job resume + failure), engine-http adaptive suites ✓, engine-api 8/8 (incl. REAL-HTTP media-job e2e), full workspace suite running at log time.

HONEST LIMITS: no real-Windows machine available → installer run/install, console-window invisibility (flag present at all 3 sites, mechanism documented), and a real updater hop are UNTESTED on Windows; sandbox recycled mid-round (toolchain rebuilt via scripts/rebuild-toolchain.sh; GitHub token + velox-updater.key were LOST with it — push and release signing need re-provisioning).

## Remaining / next candidates

1. Toast matrix per notification pref; media-merge UI exercise needs yt-dlp + ffmpeg on a real machine (honest-unavailable states already in place).
2. Engine-side queue scheduler contract (queues currently orchestrate only while the app is open — stated honestly in the UI).
3. Clipboard monitor as a DESKTOP feature (the extension has its own opt-in clipboard suggestion; a standalone app-side monitor does not exist — honest screens).
4. Real-Windows installer test (INSTALLER-TEST-RESULTS.md refreshed 2026-11-22: installers ARE built+published; real-machine install pass + real in-app update hop still pending).
5. Long-path `\\?\` helper; GET-206 tolerance; extractor cookie jar (engine queue tasks 9/7).

## Recovery after sandbox reset (checked once, then ask the user)

Not in Git: `.secrets/github-token`, **`.secrets/velox-updater.key` (+ `.pub`)**, `.cargo/`, `.rustup/`, `.xwin-cache/`, `/tmp/*` (llvm debs, nsisx makensis, installer staging, e2e servers, `/tmp/msv` verify crate). Rebuild per `scripts/TOOLCHAIN-SETUP.md` (+ `rustup target add x86_64-pc-windows-msvc` + `cargo install cargo-xwin --version 0.23.1 --locked` + clang-19 debs + nsis 3.13 debs + `npm i -g @tauri-apps/cli`). After `rm -rf download-engine/target` the suite needs a ~10 min rebuild. The updater key is the single most critical secret — losing it means shipping a NEW pubkey with the next release and updating `tauri.conf.json`.
