# OPEN SOURCE AUDIT — external components considered for Velox

Date: 2026-11-21. Scope: every third-party component proposed by the open-source
acceleration mission, plus the licences already carried by the engine's dependency
tree (Cargo.lock, 200 crates). Rule applied throughout: nothing is integrated before
its licence, version and maintenance status are recorded here; uncertain compatibility
= no integration until resolved.

## A. External download/media projects (candidates)

| Project | Repo / version | Licence | Integration proposal | Decision |
| --- | --- | --- | --- | --- |
| aria2 | github.com/aria2/aria2 (1.37.0 line, active) | **GPL-2.0-or-later** (+ OpenSSL exception) | none by linking — GPL is incompatible with embedding in a MIT-licensed Rust binary; a bundled GPL binary as a subprocess would impose GPL obligations on that binary's distribution inside our installer | **NOT integrated.** Architecture studied only (segment scheduling, per-host limits, checkpoint ideas already reflected in our engine). Motrix demonstrates the MIT-app-bundles-aria2-subprocess pattern; we choose not to carry GPL binaries in Velox installers. |
| yt-dlp | github.com/yt-dlp/yt-dlp (2026.10.26 tested live) | **Unlicense** (public-domain equivalent) | external subprocess, user-installed, never bundled — implemented in `engine-http::extract` | **Already integrated** (extractor adapter, D1/D2 fixes 2026-11-21). Unlicense imposes no attribution obligation; we credit it in docs/EXTRACTORS.md anyway. |
| FFmpeg | ffmpeg.org — LGPL-2.1-or-later core, **optional GPL components** (x264/x265/fdkaac etc. flip the whole build to GPL) | LGPL-2.1+ (chosen build) / GPL (rejected build) | future: subprocess for progressive→adaptive media merging only (`-c copy`, no transcoding), preserving original quality | **DEFERRED** to the media-merge cycle (docs/FFMPEG_INTEGRATION.md is the spec). Requirements recorded: LGPL build only (e.g. BtbN `…-win64-lgpl`), user-provided or installer-offered binary, no GPL-enabled build ever shipped, LGPL §4 notices + source offer maintained. |
| libcurl | curl.se (curl licence: MIT-derived) | curl licence | second networking stack | **NOT integrated** — reqwest/hyper/rustls already provide streaming, redirects, TLS, pooling; a second stack adds attack surface and maintenance with no measurable gain (mission §4: no stack unless advantage). |
| Xtreme Download Manager | github.com/subhra74/xdm (GPL-3) | GPL-3 | study only | Java stack, GPL — architecture reading only (browser-integration interception rules). |
| JDownloader | jdownloader.org (GPL-3 components + proprietary parts) | mixed | study only | No reuse; plugin/extractor taxonomy read as reference. |
| Motrix | github.com/agalwood/Motrix (MIT) | MIT | study only | Useful as evidence for the aria2-subprocess distribution pattern (which we decline) and for UI-level queue concepts (post-gate). |
| uGet | ugetdm.com (LGPL-2.1) | LGPL-2.1 | study only | C/GTK — architecture reading only. |
| FDM / IDM | proprietary | proprietary | public documentation only | **No decompilation, no binary extraction, no protected-code reproduction** (mission rule). Publicly documented behaviors (browser handoff, site grabber concepts) are implemented independently. |

## B. Licences already carried by our dependency tree (verified from Cargo.lock, 2026-11-21)

| Crate (version) | Licence | Role |
| --- | --- | --- |
| reqwest 0.13.5 | MIT OR Apache-2.0 | HTTP transport |
| hyper 1.12.0 | MIT OR Apache-2.0 | HTTP core (via reqwest) |
| rustls 0.23.46 (+ rustls-platform-verifier 0.7.1) | MIT OR Apache-2.0 OR ISC | TLS (verification ON by default) |
| ring 0.17.14 | ISC-style + OpenSSL derivative notice | crypto backend (carries its own licence file in the source distribution) |
| tokio 1.54.1 | MIT OR Apache-2.0 | async runtime |
| rusqlite 0.43.0 (libsqlite3-sys) | MIT (bindings); SQLite itself is **public domain** | state store |
| serde 1.0.229 / serde_json | MIT OR Apache-2.0 | serialization |
| url 2.5.8 / bytes 1.12.2 / time 0.3.55 | MIT OR Apache-2.0 | parsing/bytes/time |
| cookie_store 0.22.1 / reqwest_cookie_store 0.10.0 | MIT OR Apache-2.0 | RFC 6265 jar |
| async-trait 0.1.92 / thiserror 2.0.21 / tracing 0.1.44 | MIT OR Apache-2.0 | ergonomics/logging |
| tempfile 3.27.0 (dev) | MIT OR Apache-2.0 | tests |
| tauri 2.x (app only) | MIT OR Apache-2.0 | desktop shell |

Engine source licence: MIT (LICENSE-MIT in repo root). All of the above are
MIT/Apache/ISC-compatible; none impose copyleft on our distribution.

## C. Distribution requirements (current state)

1. `docs/THIRD-PARTY-NOTICES.md` — generated overview of the dependency licences (checked in).
2. `scripts/gen-license-report.sh` — repeatable report: scans the cargo registry sources
   of every locked crate and emits `docs/dependency-licence-report.txt` (licence field
   per crate; flags any `GPL`/`AGPL`/`LGPL` hit for review).
3. Bundled binaries policy: **nothing third-party is bundled** in Velox installers today
   (WebView2 Evergreen = Microsoft freeware installer, redistributed per its terms with
   its own EULA shown at install time; yt-dlp/FFmpeg remain user-provided externals).
4. Copyright notices: yt-dlp credited in docs/EXTRACTORS.md; no third-party code was
   copied into the engine (see docs/EXTERNAL_VELOX_AUDIT.md for the similarity audit).

## D. Uncertainties (documented, not integrated)

- FFmpeg distribution build provenance: LGPL builds must be verified per release
  (configure flags are not stamped into the binary). Procedure in
  docs/FFMPEG_INTEGRATION.md: verify `ffmpeg -version` does NOT list `--enable-gpl`,
  keep the build manifest with the installer, and provide the source link.
- ring's OpenSSL-derived code carries an additional notice requirement — covered by the
  notices file (ring ships its own LICENSE; the cargo registry copy is preserved by
  `cargo vendor`/registry caching).
