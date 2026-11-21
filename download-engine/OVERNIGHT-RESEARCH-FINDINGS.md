# Overnight research findings (verified against primary sources; full citations in docs/RESEARCH.md §6)

## Applied to the engine this session
- RFC 9110 §10.2.3: HTTP-date Retry-After implemented (IMF-fixdate; server delays clamped).
- RFC 9110 §15.4: cross-origin redirect credential stripping (origin-triple scoping).
- RFC 6265: persistent cookie jar via cookie_store (serde JSON, 0600; Netscape export planned).
- Microsoft docs: std::fs::rename already uses POSIX-semantics rename on Win10 1607+ → the
  Windows pre-rename remove is belt-and-braces; LockFileEx identified as the proper Windows lock.
- curl's retry+resume composition (offset re-derivation, attempt-data removal) matches Velox's
  checkpoint-resume semantics — behavior parity confirmed.

## Key survey facts (repo + license + activity verified)
- AB Download Manager (Apache-2.0) is the cleanest modern reference; aria2 (GPL-2+) manual-only
  for Velox; XDM 9 / Gopeed / Persepolis (GPL-3) behavior-only; FDM & JDownloader proprietary.
- wget2 uses fixed-size chunk + thread pool; curl has no single-file segmentation (its
  --parallel is per-transfer) — Velox's tail-stealing scheduler remains differentiated.
- uGet closed; DownThemAll documents why segment logic cannot live in a webextension sandbox.

## Clean-room boundaries restated
Facts/behavior from any project's docs are learnable; GPL source code is off-limits to read for
contributors; proprietary tools are observation-only.
