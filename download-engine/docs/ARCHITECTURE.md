# ARCHITECTURE — Velox Download Engine

Status: implemented as described in this session (see gaps in §8). Engine first, UI last (directive §1).

## 1. Product shape

Velox is an **engine** (a Rust workspace of libraries + a headless CLI client), not an app with embedded
download code. The same engine serves future desktop/Android/browser-extension/remote-web clients through a
versioned API (ADR-0004). No UI framework code can enter `engine-core` because no UI crate exists; the CLI is
a thin client of the core, exactly like the future GUIs will be.

## 2. Workspace layout

```
crates/
  engine-core        domain model + orchestration (no I/O library choices here beyond tokio primitives)
  engine-http        HttpPort implementation on reqwest 0.13 (rustls/ring); inspection, ranges, redirects
  engine-storage     part files, positional writes, preallocation, durable finalization
  engine-state       SQLite persistence (WAL, synchronous=FULL): downloads, segments, error history
  engine-api         JSON-RPC 2.0 over stdio (versioned v1); the contract future clients speak
  engine-cli         `velox` binary — first client; in-process engine + serve mode for the API
  engine-testserver  std-only controllable HTTP server with fault injection (tests + benchmarks)
```

Dependency direction: `engine-http → engine-core → {engine-storage, engine-state}`; `engine-api`,
`engine-cli` depend on core (+ http for wiring). `engine-testserver` depends on nothing (std).

This is a deliberate subset of the directive's suggested tree: crates exist only where responsibilities
genuinely differ (ADR-0001). `engine-mirrors`, `engine-auth`, `engine-protocols` etc. appear when those
features do (ROADMAP).

## 3. Core data flow

```
CLI/API ──▶ DownloadManager (engine-core)
              │  create: inspect ──▶ plan ──▶ reserve names ──▶ insert manifest (SQLite)
              │  start : spawn DownloadTask
              ▼
         DownloadTask (per download)
              │  reconcile state vs file (resume) ─▶ spawn N workers + monitor + checkpoint loop
              ▼
   ┌──── worker ×N ────┐      SegmentTable (Mutex; pending/active ranges, positions)
   │ HttpPort.open_range│◀────▶  take_work / advance / reclaim_tail (work stealing)
   │  206 + If-Range    │        └─▶ dirty checkpoints
   │  chunk().await     │
   │  PartFile.write_at │◀──── engine-storage (Mutex<File>; write_all_at; sync_data)
   └────────────────────┘
              │ monitor: aggregate progress → events; all-done → finalize
              ▼
   finalize: hash (optional) → fsync → rename → fsync dir → DB state=completed  (strict order)
```

## 4. Concurrency model

- **Runtime**: tokio multi-thread runtime. 2 worker tasks per download by default (config: `max_connections`,
  hard-capped by `max_connections_per_host` enforced through a per-host `Arc<Semaphore>` owned by the manager).
- **SegmentTable** (`std::sync::Mutex`, never held across `.await`): the single source of truth for live ranges.
  Workers report `advance(n)` after every successful positional write; the monitor aggregates.
- **Work stealing / dynamic segmentation (ADR-0006)**: workers take pending ranges; when a worker finds the
  queue empty it may *steal the tail* of the largest active segment (`remaining ≥ 2 × min_segment_size`),
  clamping the owner's end. Owners clamp every write under the same lock, so stolen bytes are never double-written.
- **Persistence actor**: `engine-state`'s `Db` wraps a `Mutex<rusqlite::Connection>`; all writes are
  `BEGIN IMMEDIATE` transactions (WAL allows concurrent reads without blocking). Short, millisecond-scale ops —
  called directly, no dedicated thread for MVP (documented limitation if profiling shows contention).
- **Backpressure**: bounded buffers everywhere — workers read ≤ 64 KiB chunks; limiter (token bucket) paces the
  read loop when a bandwidth limit is set; memory use is O(connections × 64 KiB) independent of file size.
- **Events**: `tokio::sync::broadcast` (capacity 256, lag-tolerant) for progress/state events; consumers never
  block the engine.

## 5. Failure model (summary; details in PROTOCOL_COMPATIBILITY §5)

Errors are classified at the boundary: `Permanent` (404, 403 after auth attempted, TLS cert failure, ENOSPC
surface as paused-with-reason, permission) vs `Temporary` (connect/timeout/reset, 408, 429, 5xx) with optional
`Retry-After`. Per-segment retries use exponential backoff + jitter, bounded by `RetryPolicy`
(default: 5 attempts, base 1 s, cap 60 s, `Retry-After` honored). A failed segment resumes from its own
checkpoint — never a whole-file restart unless validators say the resource changed.

## 6. Storage architecture

- One **part file** per download: `<final-name>.part-<id8>` in the destination directory (same volume →
  atomic rename at completion, no merge/copy step).
- Writes are positional (`write_all_at`), loop-safe per `write(2)` semantics; a `Mutex<File>` wrapper keeps
  Windows `seek_write` cursor semantics safe.
- **Durability ladder** (configurable, default = second rung for checkpoints, top rung for completion):
  1. write (page cache) — safe vs process crash;
  2. `sync_data()` + DB checkpoint commit (`synchronous=FULL`) — safe vs power loss to checkpoint granularity;
  3. completion: `sync_all` → rename → fsync directory → DB `completed` commit (RECOVERY_DESIGN §6).
- Preallocation: `set_len` (sparse) by default; true block allocation is a future platform-specific optimization.
- Free-disk-space query is absent from std — ENOSPC is handled as a classified write error in MVP
  (pre-flight disk-space check is planned; ROADMAP P7).

## 7. Platform boundaries

- Networking/TLS: reqwest + rustls(ring) everywhere — no OS TLS APIs in the core.
- Storage: thin `cfg(unix/windows)` surface in engine-storage (`write_all_at` / `seek_write_all`, directory
  fsync is Unix-only no-op on Windows pending a documented equivalent).
- Data dir resolution: `DXDL_HOME` → `$XDG_DATA_HOME/velox` → `~/.local/share/velox` (Unix); `%APPDATA%\velox`
  fallback on Windows (documented, untested on this sandbox).
- Network-interface selection / bonding: deliberately out of the HTTP engine; future `engine-network` crate with
  platform adapters (ROADMAP P6-P7). The core only knows "a transport connects to a host".

## 8. API contract (v1, implemented in engine-api)

JSON-RPC 2.0 over line-delimited stdio (`velox serve`). Methods:
`engine.info`, `download.create`, `download.start`, `download.pause`, `download.resume`, `download.cancel`,
`download.remove`, `download.list`, `download.status`, `download.verify`.
Standard JSON-RPC error codes + engine errors carry `{code, class, message}`. Version negotiation via
`engine.info.api_version = "1"`. Remote/loopback transports are opt-in future work with token auth (SECURITY §5).

## 9. Known gaps vs this document (honest list, updated 2026-11-19 session 2)

- Mirrors/multi-source, Metalink, FTP/SFTP/BitTorrent: not started (ROADMAP P6/P8).
- Network-interface awareness, failover, IPv6 tuning: not started (P6/P7).
- Disk-space pre-check, cross-volume temp handling: partial (ENOSPC classified → paused-with-reason; pre-flight free-space check planned).
- Cookie persistence across sessions: session-only via reqwest store; RFC 6265 store file planned.
- HTTP/2 supported via ALPN (reqwest); HTTP/3 not production-available upstream (research report A2) — tracked.
- Rename-orphan adoption: not implemented; the engine re-downloads (safe path) — verified by test.
- Retry-After HTTP-date form unparsed (delta-seconds supported).
- Windows paths (seek_write_all, dir-fsync equivalent) implemented but untested on this Linux sandbox.

## 10. Concurrency guard added in session 2

`engine-storage::DownloadLock` (`<part>.lock` + owning pid, stale locks stolen after SIGKILL) is
acquired by `manager.start` before opening the part file and held for the task's lifetime: two
engine processes can no longer write the same part file (a corruption vector found by the P5 audit).
