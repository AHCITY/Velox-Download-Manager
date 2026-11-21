# ROOT-CAUSE REPORT — real file downloads (P0)

Scope: audit and repair of the download-correctness failure reported from the installed
Windows app ("YouTube watch URL downloaded as `watch` at ~1.5 MiB", "Google Drive shared
video URL stopped at ~84 KiB", "completed but not the requested file"). Baseline audited
file set: `engine-http/src/lib.rs`, `engine-core/src/{manager,task,naming,ports,inspect}.rs`.

## 1. Confirmed bugs (fixed)

| # | Bug | Root cause | Fix | Regression test |
| --- | --- | --- | --- | --- |
| B1 | Web page stored as "completed download" | No response-body validation anywhere; HTTP 200 + readable bytes = "success". A YouTube watch page (1.5 MiB chunked HTML) and a Drive interstitial (84 KiB) were persisted as the "file" | (a) 1 KiB bounded body sniff on every inspect probe (`Range: bytes=0-1023`); (b) page gate at add/restart time; (c) worker-level sniff of the real body at offset 0; HTML is accepted **only** on explicit intent (`*.html` target name, URL path `*.html/.htm`, `allow_html=true`) | `html_error_page_with_200_is_rejected`, `html_with_misleading_disposition_is_rejected`, `html_is_stored_when_user_requests_a_page` |
| B2 | Files named `watch` / `view` | `filename_from_url` used the last URL path segment unconditionally — `/watch?v=…` → "watch", `/file/d/<id>/view` → "view" | `filename_from_url_strict`: route-word blocklist (watch/view/uc/open/…), page-script extension blocklist, opaque-id rejection; weak stems only usable with a reliable MIME extension (`watch` + video/mp4 → `watch.mp4`) | `strict_url_names_reject_routes_and_ids`, `choose_weak_stem_gets_mime_extension` |
| B3 | Extension resolution blind | `ext_for_mime` mapped octet-stream→`.bin` (invented extension) and missed most real types; empty `filename*=""` wiped a valid `filename` | octet-stream no longer invents `.bin` (safe fallback `download-<id>` instead); MIME map extended (~50 types); `reconcile_extension` appends only reliable MIME extensions; `filename*=""` falls back to `filename` | `disposition_empty_star_falls_back_to_plain`, `reconcile_appends_reliable_ext_only`, `choose_order` |
| B4 | Compressed transfer could be stored as file bytes | No `Accept-Encoding` control and no `Content-Encoding` validation — a server answering gzip would have its encoded bytes saved as "the file" | every request sends `Accept-Encoding: identity`; inspect and open_range refuse non-identity `Content-Encoding` (`INVALID_RESPONSE`, permanent) | covered by inspect path (engine-http); gate in `page_gate` |
| B5 | Truncation was not a completion-proof failure | Completion checked per-segment ends, but no durable final assertion received==Content-Length | `finalize` now refuses completion unless `received == total` when total is known (`INVALID_RESPONSE: truncated transfer … completion refused`) | `truncated_response_is_never_completed` |
| B6 | Test-suite fragility masked engine behavior | crash-rename test matched "any file ≠ orphan", catching engine residue (`.lock`) | test selects the actual fresh final file (filters `.lock`/`.part-`) | existing `crash_resume` suite, green |

Byte-for-byte SHA-256 verification is asserted in every completing regression scenario
(`http_correctness.rs`: direct MP4, ZIP/PDF, redirect+Content-Disposition, ignore-Range,
premature-close recovery, chunked/unknown length, Drive confirm flow, extractor flow).

## 2. Missing capabilities (implemented)

| # | Capability | Design | Tests / evidence |
| --- | --- | --- | --- |
| M1 | Google Drive share links | `engine-http/src/gdrive.rs`: `/file/d/<id>/view|open|uc` rewritten to the download endpoint; confirm interstitial parsed (form action + hidden `id/export/confirm/uuid` inputs; `Download anyway` anchor fallback; `confirm=t` added when omitted); marker-guarded walk also works for mirrored hosts; specific honest errors for sign-in-required / quota / removed / unparsable pages | unit tests (`gdrive.rs` 6 cases) + fixture E2E (`drive_style_confirm_page_resolves_to_the_real_file`, `drive_view_share_link_…`) + **real Drive download, byte-exact** (see §5) |
| M2 | Media-page extraction (YouTube et al.) | `engine-http/src/extract.rs`: external `yt-dlp` (`-J --no-playlist`), progressive-https stream selection (mp4 > webm > other, by bitrate), HLS/DASH and adaptive-only rejected with explanation, DRM/auth errors passed through verbatim; extractor stays fully separated from the generic engine (`EngineConfig::extractor_path`, CLI `--extractor`, desktop app enables PATH lookup) | unit tests (format selection) + stub-extractor E2E (`engine-http/tests/extractor.rs`, byte-exact stream download) + real-URL behavior (see §5) |
| M3 | New error taxonomy for honesty | `NOT_A_FILE` (web page instead of file), `UNSUPPORTED_URL` (media page without extractor) — both permanent, with user-actionable messages | asserted in regression suite and CLI evidence |
| M4 | Content sniffing primitive | `inspect::sniff_kind` — conservative: confident HTML markers only, BOM/whitespace tolerant, NUL/high-byte → Binary, never misclassifies real binaries | unit tests incl. MP4/ZIP/PDF negatives |
| M5 | `HttpPort::resolve` | New trait method (default = `inspect`) so backends implement URL semantics (Drive, extraction) without any core rewrite; manager + task call `resolve` everywhere | architecture preserved; all pre-existing tests green |

## 3. Preserved architecture (unchanged by design)

- Engine crates, storage (SQLite `engine-state`), part-file persistence, segmentation
  with tail-stealing, resume with validators, retry/backoff, crash recovery, per-host
  connection caps, admission queue — **all untouched**; the whole pre-existing suite
  (115 tests) still passes.
- Desktop app remains an in-process engine with the same JSON-RPC contract; the UI
  change set is copy/font/details only (no styling rework).
- New engine behavior is additive: new config fields are `Option` with serde defaults,
  so old DB rows and configs load unchanged.

## 4. Environment-specific failures (this sandbox)

| Observation | Cause | Status |
| --- | --- | --- |
| `speed.hetzner.de`, `archive.org` unreachable | sandbox egress allowlist | tested against reachable public hosts instead |
| YouTube bot wall: "Sign in to confirm you're not a bot" | YouTube blocks datacenter IPs; verified with **standalone yt-dlp** (same error, independent of Velox) | the engine surfaces it verbatim as `AUTH_REQUIRED`; on residential IPs / with cookies the same path completes — extraction machinery itself is proven byte-exact by the stub-extractor E2E test |
| Sandbox rollback destroyed the previous toolchain + artifacts | environment recycle | code fixes re-applied and verified by the suite; Windows binaries must be rebuilt (CI workflow or toolchain rebuild) |

## 5. Real-world evidence (executed 2026-11-20, engine commit under audit)

| # | Requested URL type | HTTP status | Response MIME | Expected size | Actual size | Final name | SHA-256 | Final state |
| --- | --- | --- | --- | --- | --- | --- | --- | --- |
| E1 | Direct PDF (`www.w3.org … dummy.pdf`) | 200 | application/pdf | 13 264 B | 13 264 B | `dummy.pdf` | `3df79d34…85e6adb4` — **matches curl reference** | completed |
| E2 | Direct binary (`proof.ovh.net/files/1Mb.dat`) | 200 | application/octet-stream | 1 048 576 B | 1 048 576 B | `1Mb.dat` | `30e14955…fcb58` — **matches curl reference** | completed |
| E3 | **Real Google Drive share link** `file/d/0B9P1L--7Wd2vU3VUVlFnbTgtS2c/view?usp=sharing` | 200 (after `/view` → `uc` resolution) | text/plain (real payload) | 5 B | 5 B | `spam.txt` (from Content-Disposition — not "view") | `284e3029…a72cee` — **matches direct `uc?id=` reference** | completed |
| E4 | Bogus Drive file id | 404 | — | — | — | — | — | failed honestly (`HTTP_404`), no file |
| E5 | YouTube watch URL, no extractor | — (rejected pre-network) | — | — | — | — | — | failed honestly (`UNSUPPORTED_URL` + remedy) |
| E6 | YouTube watch URL, extractor set | 200 (extractor stderr) | — | — | — | — | — | failed honestly (`AUTH_REQUIRED`, YouTube bot wall — environment-specific, see §4) |
| E7 | Watch URL via stub extractor (fixture) | 200 | video/mp4 | 524 288 B | 524 288 B | `Sample Clip 1080p.mp4` (extractor title) | byte-exact vs fixture pattern | completed |

The webpage-trap invariant — *a web page is never reported as a completed MP4* — is
enforced by three independent layers (probe sniff, create-time gate, worker body check)
and asserted by the regression suite.

## 6. Remaining limitations (stated plainly)

1. **YouTube downloads require the optional extractor** and, from datacenter/VPN IPs,
   may additionally require cookies (`AUTH_REQUIRED` with yt-dlp's guidance). Not proven
   end-to-end against YouTube itself from this environment; the extraction→download path
   is proven by the stub-extractor E2E test (§5 E7).
2. **Google Drive: only publicly shared single files.** Folder links, org-restricted and
   sign-in-required files are refused with specific errors. Cookie-based Drive flows work
   only when the cookie jar carries valid session cookies.
3. **Adaptive (DASH) media needs ffmpeg** for merging; Velox downloads single-file
   progressive streams only and says so when that is all a page offers.
4. **Content sniffing is prefix-based (1 KiB)**: a server that serves a file-shaped
   binary prefix and HTML afterwards would not be caught at probe time (the worker
   offset-0 check still applies to the real body). Genuine HTML files without any
   top-level marker are stored (conservative policy — no false rejections).
5. **Close-delimited responses with no length information** cannot be distinguished from
   truncation by protocol; completion for unknown-length streams trusts clean EOF
   (RFC 9112 §6.3). The final-size assertion covers every known-length case.
6. **Windows binaries were not rebuilt in this session** (toolchain lost to the sandbox
   rollback). `docs/../.github/workflows/windows.yml` builds them on a real Windows
   runner; artifacts must be produced from this source tree before field use.
