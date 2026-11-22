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

## Remaining / next candidates

1. Toast matrix per notification pref; media-merge UI exercise needs yt-dlp + ffmpeg on a real machine (honest-unavailable states already in place).
2. Engine-side queue scheduler contract (queues currently orchestrate only while the app is open — stated honestly in the UI).
3. Browser extension + clipboard monitor (no backend today — honest screens).
4. Real-Windows installer test (INSTALLER-TEST-RESULTS.md still says untested).
5. Long-path `\\?\` helper; GET-206 tolerance; extractor cookie jar (engine queue tasks 9/7).

## Recovery after sandbox reset (checked once, then ask the user)

Not in Git: `.secrets/github-token`, `.cargo/`, `.rustup/`, `.xwin-cache/`, `/tmp/*` (llvm debs, nsisx makensis, installer staging, e2e servers). Rebuild per `scripts/TOOLCHAIN-SETUP.md` (+ `rustup target add x86_64-pc-windows-msvc` + `cargo install cargo-xwin --version 0.23.1 --locked` + clang-19 debs + nsis 3.13 debs). After `rm -rf download-engine/target` the suite needs a ~10 min rebuild.
