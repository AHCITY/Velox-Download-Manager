#!/usr/bin/env python3
"""Licensing + version: THIRD_PARTY_NOTICES.md bundled-tools section, 0.7.0 bumps."""

N = '/home/z/my-project/download-engine/docs/THIRD_PARTY_NOTICES.md'
s = open(N).read()
if 'Bundled media tooling' not in s:
    s += """
## Bundled media tooling (offline installers; optional component in online installers)

The **offline** installers (Windows unified, Linux `-offline.tar.gz`) redistribute
prebuilt binaries of the two tools that power YouTube/media downloads. The
**online** installers instead download the same files from the official sources
below at install time and record the exact URLs in the install log. The Velox
engine itself never links either tool — both are invoked as external executables
(`engine-http::extract`, `engine-media::pipeline`).

### yt-dlp

- Source: https://github.com/yt-dlp/yt-dlp (Windows: `yt-dlp.exe`, Linux x86_64:
  `yt-dlp_linux`, Linux aarch64: `yt-dlp_linux_aarch64` — official release assets)
- License: **Unlicense** for the source; the official **standalone binaries are
  built with PyInstaller and are GPLv3+** (as documented upstream). Redistribution
  with the binary therefore carries the GPLv3 notice above; the corresponding
  source is available at the URL above.
- Used for: resolving media-page URLs (YouTube etc.) into direct stream URLs,
  and (new in 0.7.0) listing the formats a source actually offers.

### FFmpeg / ffprobe

- Source: https://github.com/BtbN/FFmpeg-Builds — the `latest-*-lgpl` builds
  (Windows: `ffmpeg-master-latest-win64-lgpl.zip`, Linux x86_64:
  `ffmpeg-master-latest-linux64-lgpl.tar.xz`)
- License: **LGPL-3.0** (configure without `--enable-gpl`; the license file ships
  in each installer as `FFMPEG-LICENSE.txt` next to the binaries in `bin/`).
- LGPL compliance: this project uses FFmpeg unmodified as separate executables;
  the exact builds downloaded are recorded above, and their complete source is
  published by BtbN at the URL above (build scripts + per-version tarballs).
  Replacement/modification instructions: download the matching tarball, replace
  `bin/ffmpeg` and `bin/ffprobe` beside the app.
- Used for: merging separately downloaded video/audio streams (`-c copy`),
  MP3 extraction (`libmp3lame`, `-q:a 0` VBR best) and stream post-validation
  (ffprobe). MP3 encoding uses libmp3lame (LGPL) which is included in the
  LGPL builds.

## Velox application code

MIT — see LICENSE at the repository root.
"""
open(N, 'w').write(s)
print('notices: bundled-tools section added')

# ---- version bumps ----
import json, re
tc = 'download-engine/app/src-tauri/tauri.conf.json'
d = json.load(open(tc))
assert d['version'] == '0.6.0', d['version']
d['version'] = '0.7.0'
json.dump(d, open(tc, 'w'), indent=2)
open(tc, 'a').write('\n')
print('tauri.conf.json → 0.7.0')

c = 'download-engine/app/src-tauri/Cargo.toml'
t = open(c).read()
t2 = t.replace('version = "0.6.0"', 'version = "0.7.0"', 1)
assert t2 != t
open(c, 'w').write(t2)
print('app Cargo.toml → 0.7.0')

for tpl in ['scripts/installer-unified-template.nsi', 'scripts/installer-online-template.nsi']:
    tt = open(tpl).read()
    tt2 = tt.replace('!define PRODUCT_VERSION "0.6.0"', '!define PRODUCT_VERSION "0.7.0"', 1)
    assert tt2 != tt, tpl
    open(tpl, 'w').write(tt2)
    print(f'{tpl} → 0.7.0')
