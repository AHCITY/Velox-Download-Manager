# EXTERNAL VELOX AUDIT — similarity check against qamarabbas408/velox-download-manager

Date: 2026-11-21. Auditor: agent session. Evidence basis: the public repository pages
(README, crates/, src-tauri/src listings, commit history, LICENSE, AGENTS.md) and three
raw source files fetched to /tmp/ext-audit (ext-download.rs 2713 lines, ext-persist.rs,
ext-lib.rs, ext-AGENTS.md, ext-LICENSE). No private communication, no decompilation,
no assumptions beyond what was directly observed.

## 1. What the external project is

- Public GitHub repo `qamarabbas408/velox-download-manager`, branch `main`, MIT licence,
  "Copyright (c) 2026 Qamar Abbas" (LICENSE verified, 21-line MIT text).
- Stack: Tauri 2 + React (Vite/tailwind) + Rust. 79 commits; earliest visible commit
  2026-08-13 (page paginated; older commits exist), latest visible 2026-10-24 (v0.7.0).
- Layout: `src-tauri/src/{browser_host,disk,download,history,lib,main,native_host,persist}.rs`
  (8 files; the engine lives in a 2713-line `download.rs`), workspace `crates/` contains
  only `velox-native-host` and `velox-protocol` (browser-integration support), plus
  `extension/` (browser extension), `scripts/`, `src/` (React frontend).
- README: segmented/multi-connection downloads, clipboard URL capture, global connection
  pool, storage meter, light/dark themes; distributes installers via GitHub Releases,
  SourceForge and the Chrome Web Store.

## 2. Architecture comparison (our project vs external)

| Aspect | External repo | Our project (/home/z/my-project/download-engine) |
| --- | --- | --- |
| Engine location | `src-tauri/src/download.rs`, Tauri types in engine (`AppHandle`, `Emitter`, `State`) | Separate UI-agnostic workspace: engine-core/-http/-storage/-state/-api/-bridge/-cli/-testserver; app is a thin shell in `app/src-tauri` |
| Transport abstraction | reqwest used directly in engine code | `HttpPort` trait + manual redirect layer (`engine-http`), backend swappable |
| Persistence | `persist.rs` + `history.rs` (JSON/state files) | SQLite WAL, synchronous=FULL, checkpoint/snapshot API (`engine-state`) |
| Segmentation | JoinSet-based segments, MAX_SEGMENT_RETRIES=12, RETRY_BACKOFF=800 ms fixed | SegmentTable + plan/steal, RetryPolicy (5 attempts, exponential + jitter, Retry-After clamped), checkpoint cadence by bytes AND period |
| Completion guarantee | progress flush interval 400 ms | completion proofs (received==total, hash-before-rename, truncate-then-hash for streams), atomic finalize + dir fsync |
| Testing | none visible in fetched sources | fault-injecting testserver, 142 tests across tiers, byte-exact assertions |
| Browser integration | native messaging host (`velox-native-host`, `velox-protocol`) + extension + "browser assistance" (page-context fetch streaming) | not yet implemented (planned Phase 9) |

## 3. Code-match findings

- **Identical source files: none found.** No file in our tree shares a name or location
  with theirs (our engine files: manager.rs, task.rs, segments.rs, plan.rs, inspect.rs,
  naming.rs, retry.rs, speed.rs, ports.rs, events.rs, ids.rs, config.rs, error.rs).
- **Matching functions/implementations: none found** in the three source files fetched.
  Their `download.rs` is a single monolith mixing Tauri event emission, segment loops
  and disk checks; our equivalent responsibilities are split across crates with trait
  boundaries and no Tauri imports.
- **Common Rust/download patterns** (reqwest, tokio Semaphore caps, Range resume,
  Content-Disposition parsing, stall timeouts, AtomicU64 counters) appear in both —
  these are generic engineering, not evidence of copying.
- **Constants differ**: 12 retries/800 ms backoff vs our 5 attempts/exponential+jitter;
  400 ms progress flush vs our ≤4 Hz coalescing in the SSE bridge; their
  DEFAULT_MAX_CONN=32 vs our per-host 8 / per-download 8.
- **Dependencies differ**: they use `fs4` (disk free space) and `futures_util` inside the
  engine; ours has neither.
- **Git history**: our repository's history was rebuilt from internal snapshots after
  two sandbox rollbacks (documented in PROGRESS-LOG.md); its earliest commits are
  2026-11-20/21 in this workspace. The external repo's history starts (visibly) Aug
  2026 and is unrelated. No shared remotes, no shared commit hashes.
- **Unconfirmed suspicions**: none recorded. The "Velox" name is a common Latin word;
  both projects chose it independently (our project name comes from the local MISSION.md
  authored before this audit; theirs is credited to "TheDigiOrb"). Nothing observed
  suggests shared code or provenance.

## 4. Licensing findings

- External repo is MIT — code from it COULD legally be reused with the licence text and
  copyright notice preserved. **No reuse has occurred** (nothing to attribute) and none
  is proposed; our architecture is deliberately different.
- Their distribution relies on a Chrome Web Store extension and SourceForge installers —
  distribution model only, no code implication for us.

## 5. Engineering ideas worth adopting (independently implemented)

1. **Disk-space preflight + periodic re-check during transfer** (their 0.7.0 commits):
   we map ENOSPC after it happens (Windows 112/39) but do not refuse to START when the
   destination volume is short, nor re-check periodically. Justified adoption.
2. **"Destination unavailable" vs "low disk space" as distinct error kinds** — sharper
   error taxonomy; aligns with our honest-error principles.
3. **Flag completed downloads whose file is missing from disk** — post-completion
   integrity surfacing; we currently trust the completed row.
4. **Browser-assistance concept (M2–M5 commits)**: on auth/forbidden failures, offer a
   retry that streams through the BROWSER's authenticated page context via the native
   host — an IDM-style capability directly relevant to our Phase 9 mission. Their
   protocol details (hello handshake, client registry, watchdog, race-safe transitions)
   are worth studying when we build ours.
5. **Duplicate-URL policy + destination reservation** — partially covered by our
   collision-safe naming; a per-URL dedup policy is a small, useful addition.
6. **Tag-triggered CI release builds** — our installers are built manually; a CI
   workflow is a maintainability improvement (post-engine-gate).

## 6. Recommendations

1. No action required on similarity: document (this file) and move on.
2. Adopt ideas 1–3 as engine correctness cycles (P0/P2 priorities) with our own
   implementation and tests — engineering ideas are not copyrightable; still, our
   implementation will be written from our own architecture.
3. For Phase 9 (browser integration), study their `velox-protocol` wire format and
   extension structure as ONE reference among several (Chromium/Firefox native-messaging
   docs remain the primary sources). Do not copy their extension UI.
4. Keep this audit updated if further external references are examined.

## 7. Audit limitations

- GitHub API was rate-limited from this IP (unauthenticated); repo metadata (exact
  created_at, stars) came from rendered pages instead — sufficient for this audit.
- Only 3 of their 8 src-tauri files were fetched byte-for-byte; the remaining 5 were
  assessed by name and role. A full byte-level comparison of every file is not
  necessary to support the findings above (architecture, naming and dependency sets
  already differ fundamentally), but a deeper diff can be run on request.
