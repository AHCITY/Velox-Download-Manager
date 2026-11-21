# ENGINE ARCHITECTURE REVIEW — Velox (2026-11-21)

Mandate: keep Velox modular; map the mission's ten conceptual layers onto the existing
architecture; define the adapter-job model; justify keeping or changing each piece.
Rule honored: **no working component is replaced without demonstrated justification.**

## 1. Existing architecture (verified, kept)

```
URL ──▶ engine-core::manager (facade: add/start/pause/resume/cancel/verify/status/events)
          │ classify → inspect probe (sniff, ranges, disposition) → plan → run_task
          │ retry policy, host semaphores, admission queue, events bus
          ▼
        engine-core::task (worker loop, segment supervision, page gate, finalize proofs)
          │                      │
          ▼                      ▼
   engine-http (HttpPort impl:  engine-storage (PartFile, DownloadLock,
   manual redirects, cookie jar,  finalize, sync_dir, GC)
   Drive adapter, yt-dlp adapter)
          │                      │
          ▼                      ▼
   engine-state (SQLite WAL)   engine-api / engine-bridge (HTTP+SSE for UIs)
                                   engine-cli (terminal client)
                                   engine-testserver (fault injection; tests)
```

Verified strengths (from docs/ENGINE-AUDIT-2026-11-21.md): completion proofs,
atomic finalize with directory fsync, origin-scoped redirect credentials, honest error
taxonomy, admission control, panic-safe runners, hash-verified orphan adoption, bounded
memory (13 MB RSS on 256 MiB transfers). These are the reasons the core stays.

## 2. Mapping the mission's ten layers

| # | Mission layer | Where it lives today | Verdict |
| --- | --- | --- | --- |
| 1 | URL intelligence & classification | `engine-core::inspect` + URL classifiers in manager (`media_page_url`, `gdrive::looks_like_share_link`) | keep; extend behind one classifier trait (below) |
| 2 | Metadata & resource discovery | inspect probe (1 KiB bounded sniff, ranges, disposition, ETag) | keep |
| 3 | Website-specific extraction adapters | `engine-http::gdrive` (native), `engine-http::extract` (yt-dlp subprocess) | keep; formalize into an `Extractor` trait so adapters plug in without core changes |
| 4 | Unified download scheduler | manager (host semaphores, admission slots, queue) | keep |
| 5 | Transfer engine | engine-http::follow/open_range + task worker | keep |
| 6 | Segmentation & connection management | SegmentTable, plan.rs, steal logic, host caps | keep |
| 7 | Persistence & recovery | engine-state (SQLite WAL FULL) + storage locks | keep |
| 8 | Integrity validation | sha256/blake3 + verify API | keep; add post-completion presence check (external-audit idea #3) |
| 9 | Media post-processing | **absent** — adaptive-only streams are refused honestly | NEW: `MediaPipeline` stage (yt-dlp resolve → stream download → optional ffmpeg `-c copy` merge) as a manager-level wrapper job, NOT inside the transfer engine |
| 10 | Unified progress & event reporting | Event bus + bridge coalescing + Resolving/Resolved (D4) | keep |

## 3. Adapter job model (the one change of contract proposed)

Every URL enters the engine as a **DownloadJob** regardless of adapter:

```
DownloadJob {
  original_url, headers(user), conflict policy, out_dir, filename?
}
        │ classify
        ▼
Adapter (impl trait `UrlAdapter`) → either
  a) direct_resource: ResourceInfo        → generic transfer path (unchanged), or
  b) resolved_job: { stream_url(s), suggested_name, needs_merge: bool, meta } → transfer + optional MediaPipeline stage
```

Today `HttpPort::resolve` implicitly plays both adapter roles. The refactor introduces
`trait UrlAdapter { fn matches(&url) -> bool; fn resolve(...) -> AdapterResult }` with
three implementations (direct/gdrive/ytdlp) selected by the manager — a **narrow,
evidence-driven change** that removes the growing `match` chain in `resolve` without
touching the transfer/persistence layers. UI never learns adapter internals; it
consumes events + status (unchanged wire contract).

## 4. External process policy (yt-dlp / FFmpeg)

- Processes are started with `kill_on_drop(true)`, bounded timeouts, argv-only secrets
  (never logged), CRLF-sanitized header forwarding — established by D1/D2 (2026-11-21).
- FFmpeg (when added) follows the same rules + the merge stage is a NEW job phase with
  its own events (`Merging { id }`), honest failures, and post-merge ffprobe validation.
  License constraints: LGPL build only, subprocess, never bundled (docs/OPEN_SOURCE_AUDIT.md).

## 5. Rejected changes (and why)

- **aria2 as backend or bundled binary** — GPL-2 obligations on the binary; our engine
  already provides segmentation/resume/checkpoints; no measurable gain demonstrated.
- **libcurl second stack** — duplication without advantage.
- **Rewriting persistence (SQLite→files) or the event bus** — audited solid; churn risk.
- **Replacing naming/sniffing with a library** — our strict-name and page-gate logic is
  more conservative than generic libraries and is fully tested.

## 6. Sequencing (per mission priority)

1. P0 correctness cycles: disk preflight (adopt external-audit idea #1), missing-file
   detection (#3), duplicate-URL policy (#5) — each with tests.
2. P1: `UrlAdapter` trait refactor + MediaPipeline (merge) stage behind it.
3. P2/P3: resume/adaptive-connection tuning guided by benchmarks (loopback limits
   documented; WAN claims only from real-network evidence).
4. P4/P5: third-party notices automation (done: scripts/gen-license-report.sh),
   browser integration AFTER the acceptance gate (Phase 9 mission), UI locked.
