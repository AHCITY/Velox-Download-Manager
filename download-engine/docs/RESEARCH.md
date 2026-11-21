# RESEARCH — Existing Download Managers, Standards, and Licensing

Date of research: 2026-11-19. All facts below were verified by fetching the cited pages on that date;
anything that could not be verified is marked **unverified**. This document drives design decisions;
it is not a feature checklist. We study behavior and architecture; we do **not** copy code (see §3).

## 1. Studied projects — summary

| Project | Language | License (verified) | Maintenance (as of 2026-11) | UI/engine separation |
|---|---|---|---|---|
| aria2 | C++ | GPL-2.0 | Release 1.37.0; slow cadence, still referenced everywhere | Engine only; GUIs are separate projects |
| AB Download Manager | Kotlin (KMP) | Apache-2.0 | Very active (v1.11.4, Nov 2026) | Yes: `downloader/core` lib + desktop/android apps + REST server |
| Gopeed | Go + Flutter | GPL-3.0 | Active (v2.0 betas) | Yes: Go engine process ↔ HTTP/UDS ↔ Flutter; protocol plugins |
| Motrix | TS/Electron | MIT | Active (v2.4.3, Nov 2026) | Yes: 4 layers with CI-enforced boundaries; wraps bundled aria2 fork |
| Motrix "Turbo" fork | TS/Electron | MIT | Stale mirror of upstream; nothing unique | Same as upstream |
| XDM | Java→Kotlin | GPL-3.0 | Active 9.x rebuild | Yes: `xdm-core` (no UI deps) + `xdm-app` |
| DownThemAll | TS (webext) | GPL-2.0 | Active | None possible: browser downloads API only |
| curl | C | curl (MIT-like) | Very active (8.23.0, Oct 2026) | Library/CLI only |
| qBittorrent | C++/Qt | GPL-2.0/GPL-3.0 | Active (5.3.0) | Qt UI over libtorrent (torrent-specific) |
| Transmission | C | GPL-2/3 (+ exception) | Active (4.1.4) | Yes: libtransmission core + daemon/GUI/web front-ends |
| FDM 3.x (historical) | — | GPL-2.0 (SourceForge `freedownload`, last update 2020-06-19) | Dead | Modern FDM engine is **not published**; only LGPL-2.1 QML UI repo exists |
| Mister Download Manager | Rust | GPL-3.0+ | Active (v0.5.2, Nov 2026) | Yes: `mdm-engine` crate + egui app — closest structural analog |
| TurboDM | Rust/tokio | MIT | Small but active (Nov 2026) | GUI-free `src/engine/` + native GTK4/Win32 UIs |
| uGet | — | **unverified** (repo is a README-only placeholder, "not under active development") | Dormant | — |
| Persepolis | Python | GPL-3.0 | Semi-active | GUI wrapping a downloader |
| pyLoad | Python | AGPL-3.0+ | Pre-release | Core/plugins/webui split |
| Open Download Manager | Java | GPL-3.0 | Minor | Orchestrator over aria2/yt-dlp engines |
| JDownloader | Java | GPLv3 "but partly closed-source" (Wikipedia) | Active-ish, non-free parts | Host-plugin engine + Swing UI |
| yt-dlp | Python | Unlicense | Very active | Extraction framework; downloader backends pluggable |
| GNU Wget / Wget2 | C | GPL-3.0+ | 1.25.0 (2024) / Wget2 2.3.0 (Sep 2026) | CLI only |

Sources: repo pages and manuals fetched directly (github.com/{aria2/aria2, amir1376/ab-download-manager,
GopeedLab/gopeed, agalwood/Motrix, notnauti/motrix, subhra74/xdm, downthemall/downthemall, curl/curl,
qbittorrent/qBittorrent, transmission/transmission, ugetdm/uget, persepolisdm/persepolis, pyload/pyload,
anefzaoui/misterdownloadmanager, xsobhi/turbodm, fossgap/open-download-manager, mirror/jdownloader,
yt-dlp/yt-dlp}, sourceforge.net/projects/freedownload, FreeDownloadManagerTeam/fdm-qml-ui,
aria2.github.io/manual/en/html/aria2c.html, curl.se/docs/manpage.html, en.wikipedia.org/wiki/JDownloader,
en.wikipedia.org/wiki/Wget). gnu.org/wget and ugetdm.com were unreachable from the sandbox.

## 2. What we adopt conceptually (and why)

1. **Engine as a standalone product with thin clients** (aria2, Transmission, ABDM, Gopeed, Mister DM, Motrix v2).
   Every mature success story separates the engine from UI; several (Motrix, ABDM) enforce it in CI/build
   structure. Our workspace is crates-first: no UI crate exists at all; the CLI is a *client*.
2. **aria2's explicit knobs with sane defaults**: per-server connection cap (`-x`, default 1 there — we default
   higher but always capped), min-split-size floor (`-k`, default 20M — our `min_segment_size`), and
   `--always-resume=false` semantics: when resume cannot be established safely, restart from scratch and say why.
3. **aria2's control-file resume, reimagined transactionally**: aria2 persists a versioned control file
   (`<file>.aria2`) periodically. We replace the opaque sidecar with a SQLite manifest (WAL, `synchronous=FULL`)
   — same idea, but transactional, queryable, and multi-download.
4. **curl's transient-error taxonomy** as our default retry classification: timeout, HTTP 408/429/500/502/503/504
   (+ 522/524 style gateway errors), exponential backoff capped, `Retry-After` compliance since 7.66 — this is a
   well-battle-tested default policy set.
5. **qBittorrent/Transmission discipline** (concept only): verify-before-complete; hash failures mean
   "re-fetch that range", not "restart file"; a throttled, cancellable background verify worker.
6. **TurboDM's resume-validation checklist**: on resume, verify file size + ETag/Last-Modified before trusting
   the partial file; data written straight into final position in a preallocated file, no merge step.
7. **ABDM's typed local API** and **Motrix v2's MDXP (JSON-RPC 2.0 + schemas)**: a versioned, documented local
   protocol is the integration boundary. We ship JSON-RPC 2.0 (v1) over stdio first; loopback HTTP later.
8. **IDM-style dynamic segmentation** (observable behavior; IDM is closed-source): few initial connections,
   tails of finished work stolen from slow segments by splitting the largest in-flight range. TurboDM
   ("IDM-style dynamic segmentation, up to 32 connections") and Mister DM ("re-balancing segments") confirm
   this is the state of the art in open implementations.
9. **yt-dlp's chunk sizing** as an anti-throttling knob (chunk-based HTTP downloading) — future knob, not MVP.
10. **DownThemAll's negative result**: chunk staging through browser storage is unreliable; the browser extension
    role is link discovery + handoff to a native engine. Shapes our future extension plan (native host).

## 3. Licensing obligations (binding for this repo)

- **Never copy code or close translations from**: aria2 (GPL-2.0), XDM (GPL-3.0), Gopeed (GPL-3.0),
  Persepolis (GPL-3.0), Open Download Manager (GPL-3.0), Mister DM (GPL-3.0+), DownThemAll (GPL-2.0),
  qBittorrent/Transmission (GPL-2/3), Wget (GPL-3.0+), pyLoad (AGPL-3.0+), FDM 3.x (GPL-2.0).
  A Rust port of GPL logic would be a derivative work; we implement from specifications and our own design.
- **JDownloader** (partly closed despite nominal GPLv3) and **modern FDM** (engine unpublished) and **uGet**
  (license unverified): treat as untouchable; study behavior only.
- **Permissive projects** (curl: MIT-like; Motrix/TurboDM: MIT; ABDM: Apache-2.0; yt-dlp: Unlicense):
  we still implement from first principles, but their documented behaviors (option semantics, error taxonomies)
  are safe reference points. If ever borrowing code, honor NOTICE/patent terms (Apache-2.0) or keep notices (MIT).
- **This project's license**: MIT (ADR-0008). All engine code is original; dependencies are permissive
  (tokio/reqwest/rustls MIT/Apache; rusqlite MIT with public-domain bundled SQLite; blake3 CC0/Apache; sha2 MIT/Apache).
- Study notes (this file) record what was inspected and when, so independent-creation claims stay defensible.

## 4. Key architectural weaknesses observed in existing tools (our targets)

| Weakness | Where observed | Our counter-design |
|---|---|---|
| Opaque binary control files, format drift | aria2 `.aria2` | Transactional SQLite manifest, schema-versioned (ADR-0003) |
| Exit-code-only failure reporting | aria2 CLI | Structured error classification surfaced via API/events (docs/PROTOCOL_COMPATIBILITY.md §5) |
| No validator-verified resume (filename+size treated as identity) | several tools | Strong-ETag If-Range on every range request; validator-missing policy explicit (RECOVERY_DESIGN §5) |
| Fixed connection counts that hurt small files / polite servers | various | Adaptive segment planning: small files get 1 connection; per-host cap; steal-not-spawn (ADR-0006) |
| Spec-published APIs missing lifecycle ops | ABDM REST spec (3 endpoints) | Our API v1 covers create/start/pause/resume/cancel/status/list/verify from day one |
| Engine/UI coupling rot in forks | Motrix "Turbo" (fork-rot, 340 commits behind) | No UI crate exists; engine buildable/testable headless |
| Retry + stdout duplication caveats | curl retry docs | Engine writes to files only; progress is side-channel, never data |
| Data-integrity blind spot at power loss (DB says done, disk disagrees) | general | Strict order: fsync data → rename → fsync dir → DB commit (RECOVERY_DESIGN §6) |

## 5. Standards we implement against (primary sources)

- RFC 9110 (semantics: ranges §14, conditionals §13, validators §8.8), RFC 9112 (framing), RFC 6585 (429),
  RFC 7617 (Basic), RFC 6265 (cookies), RFC 6266 + RFC 8187 (Content-Disposition/filename*),
  RFC 5854 + RFC 6249 (Metalink, future), RFC 8305 (Happy Eyeballs v2, future networking layer).
  Distilled implementable rules live in docs/PROTOCOL_COMPATIBILITY.md.
- Notably: RFC 9110 does **not** define Content-Disposition (RFC 6266 remains authoritative) nor 429 (RFC 6585);
  ranges are computed on the *encoded* representation; If-Range accepts only strong ETags or exact HTTP-dates.

## 6. Session-2 research addendum (2026-11-19, all claims verified against primary sources)

### 6.1 Download manager survey (session 2, with verified repos/licenses/activity)

| Project | Repo | License | Status | Technique most relevant to Velox |
|---|---|---|---|---|
| aria2 | github.com/aria2/aria2 | GPL-2+ (OpenSSL exc.) | maintained, slow (1.37.0, 2023-11) | `Segment` struct vs `SegmentMan` table split; per-connection `DownloadCommand`; manual defaults: `--max-connection-per-server` 1, `--split` 5 |
| XDM 9 | github.com/subhra74/xdm | GPL-3 (8.x was GPL-2) | active (9.0.11, 2026-11) | UI-free `xdm-core` engine; writes HTTP downloads directly to the final file (no temp/assembly) |
| AB Download Manager | github.com/amir1376/ab-download-manager | **Apache-2.0** | very active (v1.11.4, 2026-11) | `downloader/core` + `downloader/monitor` module split — the cleanest modern reference layout |
| Gopeed | github.com/GopeedLab/gopeed | GPL-3 | active (v2.0.0-beta) | Go core + UI over IPC; v2 moving to native FFI |
| Motrix | github.com/agalwood/Motrix | MIT | active | engine-adapter layering over a bundled aria2 fork |
| Persepolis | github.com/persepolisdm/persepolis | GPL-3 | revived (5.2.0, 2025-07) | dropped aria2 for an in-process lib citing the 16-connection limit |
| DownThemAll | github.com/downthemall/downthemall | GPL-2 | webextension, last release 2019 | documented warning: sandboxed environments cannot own the byte-stream |
| uGet | github.com/ugetdm/uget | LGPL (historical) | **closed** | skip |
| curl | github.com/curl/curl | MIT-like | very active (8.23.0, 2026-10) | `-C -` re-derives the resume offset from the local file; `--retry` covers 408/429/5xx + timeouts, honors Retry-After, and *removes data the failed attempt wrote* before retrying — retry and resume compose cleanly |
| wget2 | gitlab.com/gnuwget/wget2 | GPL-3+/LGPL-3+ | active (v2.3.0) | `--chunk-size` fixed-size multithreaded chunks + `--max-threads` pool |
| yt-dlp | github.com/yt-dlp/yt-dlp | Unlicense | very active | `--http-chunk-size` Range-chunking to defeat per-connection throttling |

Clean-room rule restated: behavior/protocol knowledge is learnable from docs for all of these;
**reading GPL source is off-limits** to Velox contributors. FDM and JDownloader2 are treated as
proprietary (observation only). aria2's manual is GFDL text — facts yes, text no.

### 6.2 Protocol facts (RFC 9110/9112/6265, fetched and section-verified)

- **If-Range (§13.1.5)**: entity-tag (strong comparison) or HTTP-date; client MUST NOT send it
  without Range; server MUST ignore If-Range without Range and for non-Range resources; on
  mismatch the server sends the full 200 (never 412). Velox implements the entity-tag form only —
  the HTTP-date form requires a strong Last-Modified validator and is deliberately unsupported.
- **Range (§14.2, §15.3.7, §15.5.17)**: a server MAY ignore Range and answer 200 (the basis of
  Velox's plain-GET fallback); 206 without Content-Range is client-unusable; combining partials
  requires the same strong validator; 416 SHOULD carry `Content-Range: bytes */N` — which Velox
  uses as a legitimate probe outcome for zero-length resources. Velox never requests multiple
  ranges — correct, since clients that cannot process multipart/byteranges MUST NOT ask for them.
- **Retry-After (§10.2.3)**: `HTTP-date / delay-seconds`; the HTTP-date form is now implemented
  (IMF-fixdate; obsolete date forms parse to None → backoff, documented deviation). Past dates →
  retry immediately. Server-provided delays are clamped to `RetryPolicy.max_delay` (a hostile
  `Retry-After: 999999999` cannot park a worker and its per-host permit).
- **RFC 7233 is obsolete**: superseded by RFC 9110 (STD 97) — docs cite §-numbers of 9110.
- **Redirect credential stripping (§15.4 intro)**: on automatic redirect following, a user agent
  SHOULD remove resource-specific fields (Referer, Origin, Authorization, Cookie) — the quoted
  rule has no cross-origin carve-out, but browser practice keeps same-origin credentials working.
  Velox implements the browser-compatible middle ground: full header set on same-origin hops,
  only User-Agent + If-Range/Range cross-origin (documented deviation, PROTOCOL_COMPATIBILITY §3).
- **Cookie persistence (RFC 6265 §5.3 + curl's cookies.txt spec + docs.rs/cookie_store)**: RFC
  6265 defines the storage model but NO on-disk format; the de-facto interchange format is the
  Netscape 7-field cookies.txt. Velox persists via `cookie_store`'s serde JSON (internal format,
  0600) — Netscape import/export is a planned compatibility layer.
- **HTTP-date (§5.6.7)**: IMF-fixdate is the only generated form; recipients MUST also accept
  RFC 850 and asctime. Velox parses IMF-fixdate only (safe fallback for the obsolete forms).

### 6.3 Windows platform facts (Microsoft Learn + std sources)

- **Atomic rename**: Rust `std::fs::rename` on Windows already tries `MoveFileExW(MOVEFILE_REPLACE_EXISTING)`
  and falls back to `SetFileInformationByHandle(FileRenameInfoEx)` with POSIX semantics
  (FILE_RENAME_FLAG_REPLACE_IF_EXISTS | POSIX_SEMANTICS, Win10 1607+). Existing open handles keep
  reading the old file. **Consequence: Velox's pre-rename `remove_file` is Windows-only belt-and-
  braces for open-handle targets, not a correctness requirement.**
- **Flush semantics**: `FlushFileBuffers` requires GENERIC_WRITE; Microsoft recommends unbuffered
  I/O for hot write paths. Directory-metadata durability: open the directory with
  `FILE_FLAG_BACKUP_SEMANTICS` (+ `FILE_FLAG_WRITE_THROUGH`) and flush — community practice,
  not spelled out in current docs (INFERENCE, flagged).
- **LockFileEx**: mandatory byte-range locks, per-handle, released on process death — the right
  primitive to replace Velox's pid-file steal dance on Windows (planned, see WINDOWS_COMPAT.md).
- **Naming**: reserved device names incl. with extensions (NUL.txt ≡ NUL); no trailing dots/
  spaces (Velox's naming.rs already handles these); MAX_PATH 260 pre-1607, opt-out since 1607
  (longPathAware manifest), `\\?\` verbatim paths disable normalization.

### 6.4 Lessons applied to the engine this session

1. curl's "remove what the failed attempt wrote" → Velox's checkpoint-resume from the last durable
   position already matches this semantics (positional overwrite of the tail).
2. aria2/wget2 chunk-floor philosophy → Velox's `min_segment_size` steal floor (identical in kind).
3. XDM's direct-to-final-file → noted as a future design option; Velox keeps the part-file + atomic
   rename model because it composes with hash verification and collision-safe naming.
