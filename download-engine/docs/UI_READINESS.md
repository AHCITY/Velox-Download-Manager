# UI integration readiness — JSON-RPC v1 API audit (2026-11-19, session 2)

Transport: line-delimited JSON-RPC 2.0 over stdio (`velox serve`). The engine/UI separation holds:
the API layer touches only `DownloadManager` + events, no storage internals. This document is the
checklist a desktop UI implementer should read before wiring anything.

## 1. What the API can already support (verified against engine-api/src/lib.rs)

| UI need | Method | Notes |
|---|---|---|
| Add URL | `download.create` | returns id, filename, resumability, total size; duplicate URL returns the existing record's id (`existing_id`) |
| Inspect before add | (CLI `inspect` calls manager directly) | DONE (session 2): `download.inspect` exists — previews without registering |
| Start/resume | `download.start` | idempotent while running |
| Pause | `download.pause` | settles before returning (checkpointed) |
| Resume | `download.start` | |
| Cancel | `download.cancel` | settles then deletes row + part file; refuses if the task will not settle (session-2 fix) |
| Delete record | `download.remove` | alias of cancel — **naming gap: "remove" does not keep history** |
| List | `download.list` | full rows; fine for small N, no pagination |
| Progress/speed | `download.status` | includes live speed + connections while running; segments included |
| Verify | `download.verify` | recomputes the checksum of a completed download |
| Errors | `error` field | `{code, class, message}`; classification (permanent/temporary) is machine-readable |
| History | `download.list` | rows persist after completion; error ring buffer per download via `download.status.recent_errors` |
| Events | (stdout stream) | Progress/DownloadState/Completed/Failed/Restarting broadcast on the same channel as responses — a UI must demultiplex by presence of `"method"`-shaped fields; **gap: events are not JSON-RPC notifications, they are raw JSON lines** |

## 2. Gaps to close before a UI is built (priority order)

1. **Event framing** — emit real JSON-RPC notifications (`{"jsonrpc":"2.0","method":"engine.event","params":{...}}`)
   with a `seq` counter; today's raw event lines force brittle client parsing. Back-compat: keep
   raw lines until v2, gate by `engine.info.capabilities`.
2. **`download.inspect`** — preview without registering (the CLI already has the logic).
3. **Queue management** — nothing orders downloads: no queue position, no max-concurrent-downloads
   gate (audit A8), no priorities. For a UI, add `EngineConfig.max_concurrent_downloads` + FIFO
   admission + `download.move`/priority field.
4. **Pagination/filtering on `download.list`** (state filter, window).
5. **Batch progress** — one status poll per download per second does not scale past ~50 rows;
   add a delta-progress notification or a `download.list_summary`.
6. **`download.remove` should be distinct from cancel** (keep the record; delete only on explicit
   remove) — needed for the history view.
7. **id echo strictness** (audit A11): reject non-string/number/null ids; do not reply to
   notifications (JSON-RPC 2.0 spec conformance before any UI depends on it).
8. **Line-size cap** (audit A10, 1 MiB) and per-request timeout on `download.create` so a hostile
   URL cannot freeze the control channel (audit A9; retry clamp already landed).

## 3. Stable parts (do not change)

- Method names, `engine.info.api_version = "1"` semantics, error envelope `{code, class, message}`.
- `AddedDownload` shape (`id/filename/total_size/resumable/existing_id/resumed_from`).
- `StatusView` field names and types (segments array included).
- Event payload field names (Progress: id/received/total/speed_bps/connections).

## 4. Recommendation

Freeze v1 as-is for embedding/scripting; implement the §2 items in a v2 flag-gated pass, and do
not start UI work before items 1–3 (events, inspect, queue) exist — a UI without them will be
rewritten.


## Update (this milestone)

- The preview stack ships IN the repo: `scripts/velox-preview.sh` (Unix) and
  `scripts/velox-preview.ps1` (Windows PowerShell) — port-conflict detection, clean
  child shutdown, predictable data dir (`preview-data/`).
- Reliability fixes affecting the UI path: `download.start` no longer blocks the API
  when the queue is full (rows become `queued`); pause-while-queued is honored;
  stale SSE progress can no longer follow a terminal event; the connection cap is
  enforced; Host/Origin validation is strict loopback-only.
