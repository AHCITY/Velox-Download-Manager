# Phase 0 — Root-Cause Report (2026-11-22)

Scope: verified by reading code + live release evidence; no behavior assumed from
crate/file existence. Global rules honored: no engine rewrite, existing architecture
extended in place.

## 1. YouTube / adaptive media

| # | Finding | Evidence |
|---|---|---|
| RC1 | `pick_stream()` accepts only progressive https streams (`vcodec != none && acodec != none`); adaptive video-only/audio-only formats and m3u8/dash are skipped. Adaptive-only media → honest UNSUPPORTED_URL error. | `crates/engine-http/src/extract.rs:235-298` (filter at 243, 248; error at 330) |
| RC2 | The merge machinery EXISTS end-to-end but is never driven automatically: `DownloadConfig.media_merge` (`MediaMergeSpec { audio_url, output_ext }`) is parsed (manager.rs:1728 test) yet **`audio_url` is consumed nowhere at runtime** — only in a test. `merge_streams(video_id, audio_id)` exists and is tested (LGPL gate, `-c copy`, ffprobe validation, cancel-kill, MERGING→COMPLETED row transition) but is invoked only via the manual `download.merge` / CLI `merge` two-id command. | `crates/engine-core/src/config.rs:122-133`, `manager.rs:783+`, `engine-api/src/lib.rs:261-280`, `engine-cli/src/main.rs:711-739` |
| RC3 | yt-dlp extraction itself is solid (JSON `-J`, timeout+kill_on_drop, header forwarding, DRM/auth honesty) and stays. | `extract.rs:112-231` |
| RC4 | ffmpeg/ffprobe port (`SubprocessPort`) is kill-safe and LGPL-gated; app injects it at startup via PATH discovery. Missing: beside-the-exe discovery priority is only implemented for yt-dlp (main.rs PATH-prepend covers ffmpeg indirectly on Windows — works but implicit), and **no Windows `CREATE_NO_WINDOW` flag on any subprocess spawn**. | `engine-media/src/pipeline.rs:150-173, 276+`, `app/src-tauri/src/main.rs:715-756` |

**Repair shape (Phase 2)**: extend `resolve_media` to also select the best
video-only + audio-only adaptive pair; thread it into `DownloadConfig.media_merge`;
add the missing orchestrator in `DownloadManager` (auto stream-2 download after
stream-1 completion, auto-merge, audio row cleanup). All within the existing
manager/port architecture — no second engine, no new merge implementation.

## 2. Windows console windows

RC5: three spawn sites create console-subsystem children from the GUI app without
`CREATE_NO_WINDOW` → the reported flashing windows:
- yt-dlp extractor: `extract.rs:132` (tokio Command)
- ffmpeg/ffprobe: `pipeline.rs:283` (tokio Command)
- native-host registration: `main.rs:370` (std Command `.output()`)

(Browser launches in `extension_setup` are intentionally visible — untouched.)

## 3. Updater

RC6: the published `latest.json` is CORRECT (verified live: version 0.4.0, valid
minisign signature, correct asset URL). Check-side version comparison is done by
tauri-plugin-updater itself (semver, cross-checked against the signature's trusted
comment). So the reported false/repeat notifications are NOT a manifest bug.

RC7 (primary): **install/restart race.** `updater.js` calls
`pendingUpdate.downloadAndInstall(...)` then `setTimeout(relaunch, 500ms)`. On
Windows the plugin's install step LAUNCHES the NSIS installer; `Finished` means
"installer started", not "files replaced". Relaunching 500 ms later can start the
OLD exe again (installer still copying / killed-by-NSIS `CLOSEVELOX`), so the
freshly started app still reports the old version → updater offers the same
release again → loop. It also force-closes the app without consent (Phase 4
complaint), bypassing the app's CloseRequested pause-and-wait shutdown path.

RC8: the updater downloads **the custom makensis installer** as its artifact. That
template does not understand Tauri NSIS update flags (`/S`, `/P`, `/R`, `/NS`,
`/UPDATE`) — NSIS ignores unknown args, so a "passive" update pops the full wizard
UI, and `/R` (restart) is not honored. Updater + installer must speak the same
contract (Phase 5 fix: teach the ONE unified installer the Tauri NSIS arg
convention while keeping its own UX for interactive runs).

RC9 (minor UX): after `downloadAndInstall` the JS state stays `available`
(stale badge until next check); startup check exists (+4.5 s) and cadence is 6 h
with 30-min focus throttle — acceptable, kept; Settings shows installed version
only via generic info (improvement planned).

## 4. Installer

RC10: offline template already bundles Velox.exe, velox-browser-host.exe,
extension/, yt-dlp.exe, ffmpeg.exe, ffprobe.exe, WebView2 bootstrapper; per-user
install; uninstall preserves user data (correct). Gaps for the unified-installer
requirement: no `/S /P /R` updater-arg support (RC8), product name still says
"offline variant" (online template is a second variant → Phase 5 collapses to ONE
`Velox-Setup-x64.exe`), no bundled THIRD-PARTY-NOTICES, no post-install file
verification, output name `Velox-Setup-x64-offline.exe`.

## 5. Storage / B2

Deferred to Phase 1 execution (df/du report + round-trip proof). Credentials are
handled only via env vars loaded from an untracked `.secrets/b2.env`; nothing is
printed, committed, or bundled.

## Non-issues (verified working, left untouched)

- Engine core: segmentation, resume, fsync-before-commit, retries, persistence,
  integrity verification (26 suites / 193 tests green at audit time).
- Google Drive flow, HTTP/SSE + Tauri-IPC dual transport, event wire contract.
- Tauri single-instance, close-request pause-and-wait shutdown (bounded 30 s).
- Signing/packaging scripts (`package-update-release.sh`) — correct manifest
  generation; only the installer-contract side needs work.
