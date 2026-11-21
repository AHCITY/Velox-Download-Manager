# ENGINE AUDIT — 2026-11-21 (Phase 1 of the engine-first mission)

Auditor pass over the full restored tree (commit b8df3c6). Every finding below was
confirmed by reading the actual code paths (file:line) — none taken from prior reports.
The engine's P0-correctness core (completion proofs, durability, honest failure
taxonomy, origin-scoped redirect credentials, admission control, panic-safe runners,
hash-proven orphan adoption) **checks out and was not rewritten**.

## Verified-solid (evidence highlights — do not touch)

| Area | Evidence |
| --- | --- |
| Completion proof | `task.rs:1116-1131` refuses Completed unless received==total; hash-before-rename; truncate-before-hash for streams |
| Atomic finalization | explicit `overwrite` conflict policy; collision-safe rename; `sync_all` → rename → `sync_dir` (`task.rs:1196-1228`) |
| DB-commit honesty | rename+commit-failure → FAILED, never Completed (`task.rs:1235-1249`) |
| Redirect credential scoping | origin triple incl. scheme+port; loop detection; RFC 9110 §15.4 across redirects (`engine-http/src/lib.rs:99-186`, audit A1) |
| Compression corruption gate | `Accept-Encoding: identity` + Content-Encoding refusal in inspect AND open_range |
| Retry policy | Retry-After honored but clamped to max_delay (worker-permit protection); jitter; shift-capped exp (`retry.rs`) |
| State store | SQLite WAL + synchronous=FULL + busy_timeout; checkpoint/snapshot API (`engine-state`) |
| Part-file durability | sync_data before state commits; Windows dir-flush best-effort (`engine-storage`) |
| Admission/queue | non-blocking start() with QUEUED state; queued-pause honored after admission (`manager.rs:379-526`) |
| Process hygiene | cross-process DownloadLock; panic-safe runner marks FAILED instead of zombie (`manager.rs:527-550`) |
| Cookie jar | RFC 6265 store; 0600 + fsync + atomic rename, pid-suffixed tmp (review 8b) |
| Naming | strict URL stems, disposition parsing, MIME-reconciled extensions, sanitize blocks traversal/devices; `choose_filename` sanitizes extractor titles too (D8 disproven) |

## Confirmed defects (prioritized)

### D1 (P1) — yt-dlp orphan-process leak on extraction timeout
`engine-http/src/extract.rs:59-85`: `tokio::process::Command` is built WITHOUT
`.kill_on_drop(true)`. When the 120 s timeout fires, the future holding the `Child` is
dropped and — per tokio's default — **the child process is NOT killed**: a hung yt-dlp
keeps running as an orphan (CPU/network/memory leak), one per timed-out add.
Fix: `.kill_on_drop(true)` + make the timeout configurable (`EngineConfig::extractor_timeout`,
default 120 s) so a regression test can provoke the path with a sleeping stub extractor.

### D2 (P1) — user cookies/referer never reach the extractor
`extract.rs:43` discards `RequestHeaders` (`_hdrs`). A logged-in user's cookies (the
standard remedy for YouTube's "Sign in to confirm you're not a bot", age-gated and
members-only media — see docs/ROOT-CAUSE-REPORT.md §4/E6) are never passed to yt-dlp.
The CLI/UI cookie feature is therefore dead for media pages.
Fix: forward `Cookie`, `Referer`, `User-Agent` (and `Authorization` for Basic/Bearer)
as `--add-headers` arguments to the extractor process. Never write them to stdout
metadata; argv is process-internal (documented exposure note).

### D3 (P1) — explicit credentials forwarded to page-controlled hosts across resolve hops
`engine-http/src/lib.rs:294-312` + `manager.rs:161-202`: credential scoping exists
**within** one `follow()` call (redirects), but `resolve()` fans out to
`inspect(confirm_url)` and the engine later downloads the **resolved stream URL** with
the SAME user headers (`task.rs` builds RequestHeaders from the stored config). A
Drive-style confirm interstitial is page-controlled: a hostile mirror can craft a
confirm form whose action points anywhere, and the user's Cookie/Authorization for the
ORIGINAL site would be sent there. Same applies to the extractor path (youtube.com
cookies would flow to googlevideo.com / any redirect target).
Fix (two layers, keeping the jar's RFC 6265 domain scoping intact):
- `RequestHeaders::scoped_credentials(original_url, target_url)` — strips Cookie /
  Authorization / extra auth-bearing headers cross-origin (keeps User-Agent, Range,
  If-Range, Referer only when same-origin);
- apply it in `try_confirm_walk` / `resolve_gdrive` for the confirm target, and in the
  task layer for the resolved stream URL (original row URL vs resolved URL origin).

### D4 (P2) — extraction/resolve silence for up to 120 s (×5 retry attempts worst case)
`manager.rs:resolve_with_retry` emits nothing during resolve. The UI gets no feedback
while yt-dlp runs (mission: "Report extraction and download progress separately").
Fix: new `Event::Resolving { url }` broadcast at resolve start (+ `Event::Resolved` on
success carrying `resolve_note`). DB states unchanged (no migration); the UI gate is
locked (Phase 6) — engine emits; UI consumes later.

### D5 (P3, documented decision) — format selection prefers mp4 over quality
`extract.rs:197-199`: `(pref, tbr)` ordering means mp4@640 kb/s beats webm@700 kb/s.
Decision: KEEP — container compatibility first (browser/players, correct extension),
documented here; revisitable behind a config knob in the UI phase.

### D6 (P3, noted) — verify() Event/row inconsistency when no expected hash
`manager.rs:776-784`: with no configured checksum, `matches=None` → `row.verified=false`
but `Event::Verified{ok:true}`. Behavior is defensible (nothing to compare) but the
event contract should be `ok: matches` or carry `matches: Option<bool>`. Deferred to the
API-stability review (event wire change).

### D7 (P3, noted) — `fetch_page_body` treats mid-page transport errors as EOF
`lib.rs:230-238`: a reset during the interstitial fetch yields a truncated page →
honest `NOT_A_FILE` with `page_reason` (no false completion; no corruption). Accepted;
upgrading to a hard error would need a retry-capable page fetch (later).

## Out of scope confirmed-NON-issues (checked, evidence)
- Memory growth with file size: segments write bounded chunks (`segments.rs span/advance`
  clamps; no whole-file buffers) — property tests cover.
- 429/Retry-After: honored at resolve AND range layers, clamped (`retry.rs` tests).
- Truncation/overlong bodies: both covered (`truncated_response_is_never_completed`,
  `overlong_body_completes_with_exactly_claimed_bytes`).
- Windows ENOSPC: fixed 2026-11-20 (`error.rs` 112/39 mapping).
- URL-embedded credentials: rejected (`manager.rs:222`).
- TLS verification: on by default; `insecure_tls` is an explicit per-download opt-in.

### D8 (P2, found 2026-11-21 by the real-Drive E2E) — lock files left in the user's destination
`engine-storage::DownloadLock` created `<name>.part-<id>.lock` beside the part file and
never unlinked it (deliberately: unlink-on-drop re-introduces the classic flock+unlink
two-holder race — see the code comment). Result: the real Google Drive download left
`spam.txt.part-1.lock` in the user's out dir (mission §5: "Keep temporary files and lock
files internal"). FIX: lock files moved to `<data_dir>/locks/` (internal; basename keeps
per-download uniqueness via global ids; OS flock remains the only arbiter) + race-free
startup GC (mtime-gated ≥7 days + flock-checked — an active acquirer's file is fresh, so
an old free lock can never have an in-flight racer). Integration test
`out_dir_is_clean_after_completion` pins the invariant; re-verified live.

## YouTube real-world failure — Phase 2 investigation (continued next cycle)
REPRODUCED 2026-11-21 with yt-dlp 2026.10.26 fresh logs:
`ERROR: [youtube] jNQXAC9IVRw: Sign in to confirm you're not a bot … Use --cookies-from-browser or --cookies`.
Prior evidence (ROOT-CAUSE §4/E6): from datacenter IPs YouTube serves the bot wall
("Sign in to confirm you're not a bot") — **standalone yt-dlp fails identically**, so
the failure is IP-scoped, not an engine bug; the extraction→download path is proven
byte-exact by the stub-extractor E2E. This cycle adds D2 (cookie pass-through), which
is the user-side remedy, plus a re-verification attempt with real yt-dlp + logs.
