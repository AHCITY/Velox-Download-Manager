# ENGINE ROADMAP — sequenced backlog (2026-11-21)

Order follows the master execution control (strict sequence, no parallel missions).
✅ = done and verified; ▶ = current; ⏸ = gated.

## Completed foundation (cycles 1-5, 2026-11-20/21)
- ✅ Engine correctness core: completion proofs, atomic finalize, honest failures,
  origin-scoped redirect credentials, page gate, strict naming (docs/ROOT-CAUSE-REPORT.md)
- ✅ Audit D1-D4 + D8 fixed with evidence tests (docs/ENGINE-AUDIT-2026-11-21.md)
- ✅ Drive adapter verified live ×2; yt-dlp integrated (kill-safe, cookie-aware)
- ✅ Disk preflight + checkpoint re-check + missing-file detection (cycle 5)
- ✅ Licence audit + notices + repeatable report (docs/OPEN_SOURCE_AUDIT.md)
- ✅ External-repo similarity audit (docs/EXTERNAL_VELOX_AUDIT.md)
- ✅ Architecture review (docs/ENGINE_ARCHITECTURE_REVIEW.md)
- ✅ Benchmarks (bounded memory, resume byte-exact) (docs/PERFORMANCE_BENCHMARKS.md)
- ✅ Testing-tier documentation (docs/DOWNLOAD_ENGINE_TESTING.md)

## ▶ Current
1. Windows 2026-11-21 rebuild + installers + SHA256SUMS (cross-build in flight;
   disk pressure being managed).
2. Restore Linux CLI (cleaned for disk) + rerun full suite to re-verify tree.

## Next (engine mission remainder — P0/P1 first)
3. ✅ DONE (2026-11-21): `UrlAdapter` trait refactor (architecture review §3) — narrow, tested, no behavior
   change; unblocks media pipeline cleanly.
4. FFmpeg merge stage (MediaPipeline) per docs/OPEN_SOURCE_AUDIT.md constraints:
   LGPL build, subprocess, kill-safe, `Merging { id }` events, ffprobe post-validation,
   "corrupt media / missing ffmpeg" honest errors. Authorized test media only.
5. ✅ DONE (2026-11-21): YouTube residential-network verification checklist written to
   docs/YOUTUBE_VERIFICATION.md (user-executable copy in download/); cannot
   be executed from this sandbox — bot wall).
6. Adaptive connection policy: only with real-network evidence; keep current static
   caps otherwise.
7. ✅ DONE (2026-11-21): CI workflows at repo ROOT (ci.yml linux + windows.yml); both gates green on 80dfe9e — first green windows run ever (KNOWN-ISSUES #11).

## ⏸ Gated on engine acceptance gate
8. Browser integration mission (Phase 9): native-messaging host, extension, download
   catcher. Gate = acceptance criteria re-verified this cycle (resume/recovery/disk-full
   ✅, integrity ✅, Drive ✅, YouTube partial ⚠ env-limited, deps/licensing ✅,
   benchmarks ✅, regression ✅). The bot-wall item is environment-limited, documented
   in KNOWN-ISSUES.md #1 — the ENGINE gate criteria it can satisfy are satisfied;
   browser work may begin once 1-2 land.
9. UI redesign — locked until the FULL gate including browser-integration mission.

## Maintenance rules (standing)
- Every cycle: fmt + clippy -D warnings + full suite; commit; PROGRESS-LOG entry.
- Backups: source ZIP after every cycle (sandbox rollbacks have hit twice).
- GitHub push when the user resolves auth (never fabricate push success).
