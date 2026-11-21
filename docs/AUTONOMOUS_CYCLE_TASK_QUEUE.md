# AUTONOMOUS CYCLE — TASK QUEUE

Rules for the on-duty cycle agent:
- Work top-down; take ONE task, complete it fully (implement + tests + fmt/clippy/test green), commit, log in AUTONOMOUS_CYCLE_PROGRESS.md, then take the next.
- Never leave the tree unbuildable or tests failing at the end of a cycle. Commit only green states.
- Never weaken tests. Never fabricate results. Evidence in the log: real test counts, real commit hashes.
- Disk guard: if free space < 1.5G, `cargo clean` in download-engine and remove /tmp junk before compiling.
- At end of EVERY cycle: push to GitHub (token via env, never stored), verify remote HEAD, then run scripts/github-release-refresh.sh ONLY if a milestone warrants a new installer (engine behavior changed) — otherwise leave the Release as-is.
- When the 8h mission time target is reached AND queue is exhausted: mark mission end in the progress log, delete the cron continuation job, write docs/AUTONOMOUS_CYCLE_FINAL_REPORT.md.

## Queue

1. [P0] Wire merge end-to-end: engine-api `download.merge` method; `velox merge <video-id> <audio-id>` CLI; discover_subprocess + set_media_merge in CLI + Tauri startup; API test. (audit P0-1)
2. [P0] Merge temp-file lifecycle: output `<final>.merging.part` + rename; delete part on cancel/error; startup sweep for stale `merging` rows (MERGE_INTERRUPTED); single-flight guard. (audit P0-2)
3. [P1] Stop-aware probe loop + backoff sleeps (select! on command watch); truncate-to-total at finalize; un-swallow set_len failures. (audit P1-1 + P2)
4. [P1] Auth/redirect e2e tests: cross-origin redirect fixture + auth echo; assert stripped creds + preserved Range/If-Range. (audit P1-4)
5. [P1] Sustained-429 exhaustion test → honest failed row, resumable part, recovery after behavior clears.
6. [P1] Extractor 403 re-resolve: persist resolved_from + format_id; on Permanent 403 re-run resolve once and resume. (audit P1-2)
7. [P1] Extractor cookie jar: pass backend jar as --cookies to yt-dlp alongside --add-headers. (audit P1-3)
8. [P1] Installer hygiene: fix online-template header, pv=="0.0.0.0" detection, DisplayVersion downgrade warning, align version constants, document template-vs-bundler relationship. (audit P1-5)
9. [P2] Plain-GET-206 tolerance (usable full body instead of Restart) + per-chunk HTML sniff in run_segment. (audit P2)
10. [P2] bench-server.py: proper 416 for unsatisfiable ranges; --503-first/--close-after/--stall-after/--redirect fault flags. (audit P2)
11. [P2] WINDOWS_COMPAT.md reconciliation to shipped code + `\\?\` long-path helper + >260-char test. (audit P1-6)
12. [P2] Typed CLI errors (replace string matching on "already completed"); CLI inspect uses manager backend. (audit P2)
13. [P2] Dead states cleanup (inspecting/verifying) or wire them; Verified-event vs verified flag honesty (D6). (audit P2)
14. [P2] config_json corrupt-parse honest error; host semaphore origin keying; permit-release during backoff. (audit P2)
15. [Research+P2] yt-dlp upstream compat check (current releases, changed flags); document in EXTRACTORS.md if anything relevant.

## Explicit non-goals (standing)

- No GPL/nonfree integration; no bundling ffmpeg in the installer; no transcoding when stream copy suffices.
- No new heavy dependencies without justification; no full rewrite of DownloadManager.
- No release without full suite green; v0.1.0 stays historical.
