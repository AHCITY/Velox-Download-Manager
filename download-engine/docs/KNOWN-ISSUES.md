# KNOWN ISSUES — Velox engine (live list)

Honest, current as of commit 9a1a496 (2026-11-21). Fixed items live in
docs/ROOT-CAUSE-REPORT.md and docs/ENGINE-AUDIT-2026-11-21.md, not here.

## Environment-limited (not engine bugs; verify from a residential network)

1. **YouTube bot wall from datacenter IPs.** Reproduced 2026-11-21 with yt-dlp
   2026.10.26 directly: "Sign in to confirm you're not a bot … Use --cookies". Standalone
   yt-dlp fails identically → IP-scoped, not a Velox defect. User-side remedy (cookies)
   is wired through the engine as of cycle 2 (D2: cookie/referer/UA/auth forwarded to
   the extractor process). Claiming YouTube "works" requires a residential-IP test.

## Accepted design decisions (documented, revisit triggers noted)

2. **Adaptive-only media streams are refused, not merged** (extract.rs): merging needs
   FFmpeg; adding it is planned as the MediaPipeline stage (docs/ENGINE_ARCHITECTURE_
   REVIEW.md §9) with an LGPL-build-only, subprocess, ffprobe-validated design
   (docs/FFMPEG_INTEGRATION spec pending that cycle). Until then, users get an honest
   "adaptive stream needs merging" error instead of a broken file.
3. **mp4 preferred over higher-bitrate webm in format selection** (audit D5):
   container-compatibility-first decision.
4. **verify() event contract inconsistency** (audit D6): `Event::Verified{ok:true}` with
   no expected checksum while row says verified=false. Wire-contract change deferred.
5. **fetch_page_body treats mid-page transport errors as EOF** (audit D7): downstream
   error remains honest (NOT_A_FILE + page_reason); hardening deferred.

## Open work (tracked, not defects)

6. **GitHub sync blocked**: no remote configured, no `gh` CLI, no credentials in this
   sandbox. All commits are local; push requires the user's auth (paused per user
   instruction). Notify-on-return.
7. **Browser integration (Phase 9 mission)**: gate checklist not yet fully re-run this
   cycle; native-messaging host + extension design drafted in
   docs/ENGINE_ARCHITECTURE_REVIEW.md + external-audit ideas; implementation starts
   only after the engine acceptance gate.
8. **UrlAdapter trait refactor**: proposed (architecture review §3), not yet merged.
9. **Windows 2026-11-21 rebuild**: in progress at commit time — installers to be
   produced and SHA256SUMS refreshed once the cross-build lands on a clean disk
   (two ENOSPC interruptions documented in PROGRESS-LOG.md).
10. **Nightly watchdog died with sandbox recycles** (processes don't survive); it is
    relaunched manually at end of each working turn. No cron service exists in this
    runtime (documented 2026-11-20).
