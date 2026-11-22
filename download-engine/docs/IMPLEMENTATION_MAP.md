# UI Overhaul — Implementation Map (internal, 2026-11-22)

## Architecture facts (audited)
- The Tauri desktop app serves `download-engine/web/` as its frontend (`tauri.conf.json: frontendDist: "../../web"`). The web dir IS the product UI. Single `index.html` (532 lines) — the thing being replaced.
- One JSON-RPC 2.0 contract, two transports: Tauri IPC `invoke('rpc', {body})` (desktop, engine in-process) or HTTP `POST /rpc` + SSE `/events` (`velox serve-http`, browser preview).
- Events (engine-bridge::wire_event): state-name events (`queued`, `downloading`, `paused`, `retrying`, `completed`, `failed`, `cancelled`) + `progress` + `verified` + `resolving`/`resolved` (url-keyed, no row yet) + `merging`/`merged`.
- API methods: engine.info, download.inspect/create/start/resume/pause/cancel/remove(=cancel today)/list/status/verify/merge.
- StatusView already includes: connections_active, segments, recent_errors, **file_missing** (M5 audit) — UI never consumed it.
- DownloadConfig (per-create): out_dir, filename, headers, cookies, referer, auth, max_connections, bandwidth_limit_bps (real per-download token bucket), verify (expected checksum), overwrite, auto_start, allow_html, insecure_tls, media_merge.
- engine-state already has delete_download(id); manager.remove_partial refuses completed rows; NO API removes a completed record or deletes its file → the user's exact complaint.

## Backend additions (minimal, UI-integration only)
1. `manager.remove(id, delete_file)` — cancel if running, delete record, optionally delete the final file (completed rows allowed; temp file cleaned).
2. engine-api: `download.remove {id, delete_file?}` → real removal (NOT cancel alias). `download.cancel` keeps its semantics (cancel + keep record).
3. StatusView: expose `bandwidth_limit_bps` (from config_json) so Network shows real per-download limits.
4. Tauri: `settings_get`/`settings_set` (JSON file in app_data_dir; localStorage fallback in browser), `open_path` validation extended: downloads_root OR canonical path recorded as a download dest_dir (DB-validated, still no arbitrary FS access).
5. Tests for 1–3 in engine-core/engine-api.

## Screens → contract (all real or honestly unavailable)
| Screen | Real functionality |
|---|---|
| Home | live counts, active rows, aggregate speed, activity feed (events), engine info |
| Downloads | full CRUD: pause/resume/start/cancel/remove/delete-file/open/reveal/copy/verify/details/errors; search/sort/filter/group/multi-select/bulk; file_missing surfaced |
| Queues | persisted frontend-orchestrated queues driving real start/pause when slots free (honest: no engine scheduler, app-open only) |
| Browser | honest integration status (no extension backend) + paste capture; future-ready |
| Network | live aggregate/per-download speed, connections, throughput chart (event-sampled), per-download limits (read-only after create), concurrency read-only |
| Devices | honest: no device backend; shows engine storage/network facts only |
| History | persistent rows (completed/failed/cancelled), search/date filter, re-download, remove entry |
| Vault | completed + existence (file_missing/verify), categories, open/reveal/verify/re-download |
| Settings | persisted prefs + read-only engine values (shown with actual values) |
| Media | resolving→inspecting→downloading→merging→merged stage rendering; merge action when merge_capable |

## UI architecture (no build system, CSP-safe)
`web/index.html` + `web/assets/{app.css, api.js, store.js, icons.js, ui.js, screens/*.js, app.js}` — ES modules, event-driven store, render-on-diff. Icons: inline Lucide path subset. Fonts: system stack.
