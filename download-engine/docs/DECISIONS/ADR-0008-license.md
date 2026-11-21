# ADR-0008 — MIT license; clean-room from copyleft projects

Status: Accepted (2026-11-19)

## Context
Studied projects are dominantly GPL/AGPL (aria2 GPL-2, XDM/Gopeed/Mister DM GPL-3, pyLoad AGPL, FDM 3.x GPL-2,
qBittorrent/Transmission GPL-2/3); a few permissive (curl MIT-like, Motrix/TurboDM MIT, ABDM Apache-2.0,
yt-dlp Unlicense). uGet's license is unverifiable; JDownloader partly closed.

## Decision
- This project: MIT (LICENSE-MIT).
- Strict clean-room: architectures and behaviors studied from docs/READMEs/manuals; no code, no close
  translation, no option-name-compatibility copying from GPL/AGPL sources. Standard engineering facts
  (per-host caps, min-split floors, backoff, Retry-After) are unprotectable ideas and reimplemented from
  RFCs and our own design.
- Dependency licenses verified before adoption (RESEARCH §3); all permissive (MIT/Apache/ISC/CC0/public domain).

## Consequences
+ Maximum adoptability for an open engine; no copyleft entanglement.
− Requires discipline: this file and docs/RESEARCH.md record what was studied and when.
