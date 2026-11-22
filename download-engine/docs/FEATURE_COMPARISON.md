# Velox — Browser Ecosystem & Intelligence Expansion: Feature Comparison

Reference: https://github.com/qamarabbas408/velox-download-manager (MIT) — audited at
`extension/`, `crates/velox-protocol`, `crates/velox-native-host`, `src-tauri/src/{browser_host,native_host,disk,download,history,persist}.rs` (2026-11-22).
Ours: AHCITY/Velox-Download-Manager @ the commit carrying UX 2.0 (v0.3.1).

| Feature | Reference implementation | Our current implementation | Action taken this turn |
|---|---|---|---|
| Browser extension (MV3) | TS+vite build: background SW (360 ln), nativeClient (530 ln), popup, page-assist fetcher, tests | None (Browser screen honestly said "no extension backend") | **IMPLEMENTED** — plain-JS MV3 (no build step): context menu, popup, interception pref, clipboard opt-in, pinned-key stable ID |
| Native messaging | App listens on unix socket / named pipe speaking NM framing; shim is a transparent byte pump; host manifests registered at app startup (Chrome/Edge/Brave, HKCU registry on Windows) | None | **IMPLEMENTED** — `velox-browser-host` (stdio NM ⇄ loopback HTTP JSON-RPC), versioned envelope protocol, token-authed bridge; `--register` writes manifests for Chrome/Edge/Brave |
| Context-menu downloads | `download_request` → app queue, notifications | None | **IMPLEMENTED** — "Download with Velox" on links/page/selection → engine `download.create` (same path as the UI) |
| Browser-assisted fetch (page-context chunks) | `assist_*` + `chunk`/`chunk_ack` protocol, backpressure via outbound queue (64), cancellation | None | **DESIGNED, NOT IMPLEMENTED** (P3) — design in §4 below; needs consent UX + chunk pipeline; deferred to keep this turn's scope shippable |
| ETA / speed intelligence | No smoothed ETA found in reference either (detail drawer displays engine fields) | UI ETA from a single instantaneous speed sample; engine emits per-event `speed_bps` only | **IMPROVED** — engine now tracks smoothed/avg/peak per download (StatusView `bps_smoothed`,`bps_avg`,`bps_peak`); details panel shows avg/peak + ETA states (unknown size / stalled) |
| Segment monitoring | Detail drawer UI | StatusView already exposes `segments` + UI drawer renders them | Already present; kept |
| Disk-space protection | `disk.rs`: fs4 free-space, 256 MiB safety reserve, active-expected summing, ENOSPC classification, preflight at start | `manager.rs` preflight_checked (audit M5) already: probes destination, refuses to start when it cannot hold the file, distinguishes unavailable vs low space | Ours already equivalent at start-time; **added mid-transfer ENOSPC classification to error mapping** so a disk that fills during a transfer fails with a clear kind |
| Duplicate handling | History/persist keyed policies | `find_same_destination` (URL+dest+name) → returns existing id | Already present; unchanged |
| Missing-file detection | — | `file_missing` (audit M5) surfaced in UI | Already ahead of reference; unchanged |
| Tray controls | Tauri tray menu | None | **DESIGNED, NOT IMPLEMENTED** (P4) — see §5 |
| Taskbar progress | Windows taskbar API via tauri plugin | None | **DESIGNED, NOT IMPLEMENTED** (P4) — see §5 |
| Notifications | tauri notification plugin | In-app toasts + prefs | Tray/native notifications folded into P4 design |
| Download recovery | persist.rs + resume logic | Engine already: checkpoints (fsync cadence), pause/resume, retry policy, crash recovery rows | Already present; unchanged |
| Performance | Reference: no published benchmarks | Segmented engine w/ dynamic tail stealing; **measured baseline + post-change benchmark this turn** (see docs/BENCHMARK.md) | Measured; no data-path changes without evidence |

## 3. Native messaging integration (implemented)

```
Chrome/Edge/Brave ──(NM: 4-byte LE + JSON, stdio)──▶ velox-browser-host
      ▲                                            │  HTTP/1.1 POST /rpc
      │ envelope protocol v1                       ▼  Authorization: Bearer <token>
      └─────────────────────────  Velox desktop app loopback bridge (127.0.0.1:<random>)
                                       │ engine_api::handle_line (WHITELIST)
                                       ▼
                              DownloadManager (single source of truth, SQLite)
```

- Discovery: the app writes `bridge.json` `{port, token, app_version, pid}` (0600) under
  `app_data_dir/data/` at startup. The host reads it; a stale file (dead pid) → clear error.
- Whitelist: `engine.info`, `download.list`, `download.status`, `download.create`,
  `download.pause`, `download.cancel` only. Body cap 256 KiB. One request per connection.
- The engine remains the single source of truth; the extension never stores download state
  beyond display caching.

### Envelope protocol v1 (extension ⇄ host; adapted from velox-protocol)
`{version:1, id:"<uuid>", type:"...", payload:{...}}` — kinds:
`hello`, `hello_ack`, `ping`, `pong`, `add_request`, `download_accepted`,
`download_rejected`, `status_request`, `status_response`, `cancel_request`,
`cancel_accepted`, `error`. Max inbound message 4 MiB (Chrome's own limit); the
host rejects oversize/malformed frames without crashing (responds `error`).

## 4. Browser-assisted downloads — design (NOT implemented this turn)
Adapted from the reference's assist protocol with safety changes:
1. Eligibility: engine start fails with an auth/forbidden-kind error AND the URL's
   origin matches an origin the user has an active tab for (extension reports tabs).
2. Consent: explicit per-download prompt (never silent); remember-per-site opt-in.
3. Transfer: extension fetches in the page context (`pageAssistFetch` pattern:
   content-script fetch → base64 chunks → `chunk` envelopes), window ≤ 1 MiB in
   flight, next chunk only after `chunk_ack` (backpressure), cancellable mid-stream.
4. Integration: chunks stream into OUR engine via a dedicated
   `download.assist_begin/chunk/complete` API writing through the normal
   destination/validation/finalize path — no second downloader.
5. Not implemented yet: requires the consent UX + engine ingest endpoint; the
   security review (origin validation, no cookie exfiltration to other origins,
   DRM/paywall refusal) is written but unbuilt.

## 5. Desktop integration — design (NOT implemented this turn)
Tray (tauri TrayIcon): Show/Hide, Pause All, Resume All, aggregate speed tooltip,
Quit. Close-to-tray preference extends the existing CloseRequested pause-and-exit
policy. Taskbar progress: Windows `ITaskbarList3` via `window.set_progress_bar`
(Tauri 2 `set_taskbar_progress` on Windows) with Aggregate state logic
(downloading=normal, paused=paused, error=error, mixed=indeterminate).
Notifications: tauri-plugin-notification behind existing prefs. All four are
contained, low-risk follow-ups; none were faked in code.

## 6. Attribution
Adapted ideas credited to qamarabbas408/velox-download-manager (MIT): NM framing
shape, versioned envelope protocol, host-manifest registration approach, pinned-key
extension identity, page-assist chunk/ack concept. All code in this repository is a
fresh implementation against our engine contract (see docs/THIRD_PARTY_NOTICES.md).
