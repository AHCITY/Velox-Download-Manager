# STATE.md — Velox Download Manager mission state

Last updated: 2026-11-22 — **UI OVERHAUL SESSION** (complete product rebuild of the web frontend). Keep current; commit before ending every turn.

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
- Browser extension: download-engine/extension/ (MV3, plain JS, pinned key → ID `hpaxoarvxcmmqijm`).
- Native host: crates/velox-host (`velox-browser-host`) — NM stdio ⇄ loopback HTTP JSON-RPC; 5 stdio E2E tests.
- App: loopback bridge in app/src-tauri/src/main.rs (random 127.0.0.1 port + token in app_data/data/bridge.json, 0600; whitelist engine.info/list/status/create/pause/cancel).
- P2 intelligence: SpeedStats tap (mirror broadcast subscriber) → StatusView bps_smoothed/avg/peak; UI details drawer shows avg/peak + etaState labels. Verified live (slow download: smoothed 1.8 / avg 2.1 / peak 4.2 MiB/s).
- Benchmark: docs/BENCHMARK.md — real before/after (0.02s vs 0.03s loopback, throttled 29.03s both) = no regression.
- NOT implemented (designed only): browser-assisted transfers (§4 of comparison), tray/taskbar/notifications (§5).
- GOTCHAS: (1) app crate can NOT build natively on Linux (no GTK) — always build it via scripts/build-velox-msvc.sh (xwin); (2) engine target/debug regrows fast — wipe when disk full, rebuild is ~1 min; (3) NO release publishing this turn (spec §13 forbids without authorization) — Velox.exe with the bridge saved at /tmp/Velox-exe-with-bridge.exe (host-local, not a deliverable).

## Remaining / next candidates

1. Toast matrix per notification pref; media-merge UI exercise needs yt-dlp + ffmpeg on a real machine (honest-unavailable states already in place).
2. Engine-side queue scheduler contract (queues currently orchestrate only while the app is open — stated honestly in the UI).
3. Browser extension + clipboard monitor (no backend today — honest screens).
4. Real-Windows installer test (INSTALLER-TEST-RESULTS.md still says untested) — including a real in-app update 0.2.0→0.3.0 on a Windows box.
5. Long-path `\\?\` helper; GET-206 tolerance; extractor cookie jar (engine queue tasks 9/7).

## Recovery after sandbox reset (checked once, then ask the user)

Not in Git: `.secrets/github-token`, **`.secrets/velox-updater.key` (+ `.pub`)**, `.cargo/`, `.rustup/`, `.xwin-cache/`, `/tmp/*` (llvm debs, nsisx makensis, installer staging, e2e servers, `/tmp/msv` verify crate). Rebuild per `scripts/TOOLCHAIN-SETUP.md` (+ `rustup target add x86_64-pc-windows-msvc` + `cargo install cargo-xwin --version 0.23.1 --locked` + clang-19 debs + nsis 3.13 debs + `npm i -g @tauri-apps/cli`). After `rm -rf download-engine/target` the suite needs a ~10 min rebuild. The updater key is the single most critical secret — losing it means shipping a NEW pubkey with the next release and updating `tauri.conf.json`.
