# EXTRACTORS.md — optional media extractor (yt-dlp)

Velox is an HTTP download manager. A YouTube `watch`, `shorts` or `youtu.be` URL is a
**web page**, not a media file — a plain HTTP GET of it returns HTML, which the engine now
correctly refuses (`NOT_A_FILE`). To download from media pages, Velox can delegate URL
resolution to an **external yt-dlp executable**, then download the resolved stream with
the ordinary HTTP engine (segmentation, resume, hashing all unchanged).

## What it does

1. A watch-page URL is classified (`UNSUPPORTED_URL` if no extractor is configured).
2. If `extractor_path` is set, the engine runs
   `yt-dlp -J --no-playlist --no-warnings [--add-headers H]… <url>` (bounded by
   `extractor_timeout`, default 120 s; the child is killed on expiry — no orphan
   processes). The user's per-download `Cookie`, `Referer`, `User-Agent` and
   `Authorization` headers are forwarded as `--add-headers` (2026-11-21, audit D2) —
   this is the remedy for YouTube's datacenter-IP bot wall ("Sign in to confirm
   you're not a bot") and for age-gated media. Values are CRLF-sanitized; argv is
   process-internal and never logged. `kill_on_drop` guarantees no hung extractor
   survives the timeout.
3. The best **progressive https stream** (video+audio in one file, real `.mp4`/`.webm`
   extension) is selected; HLS/DASH manifests and adaptive video-only/audio-only halves
   are rejected honestly (merging them requires ffmpeg, which Velox does not bundle).
4. The resolved stream URL is downloaded by the generic engine — the row keeps the
   original page URL, the file gets the extractor-proposed title as its name.

## What it does NOT do

- **No DRM bypass.** DRM-protected media is refused (`UNSUPPORTED_URL`).
- **No paywall/account bypass.** Private, members-only, age-gated media surfaces the
  extractor's authentication error (`AUTH_REQUIRED`).
- **No bundling.** The extractor is never shipped inside Velox binaries or installers
  in this build; it is an external, user-installed program.

## Licensing

[yt-dlp](https://github.com/yt-dlp/yt-dlp) is released under the
[Unlicense](https://unlicense.org) (public-domain equivalent): it can be executed and
redistributed without restrictions. Velox invokes it as an unmodified external process
(no linking, no source reuse), so Velox's own MIT licensing is unaffected. Note:
`yt-dlp` itself depends on optional FFmpeg for merging/ transcoding — Velox does not
require or ship FFmpeg (progressive streams only).

## Installation on Windows

Pick one; the binary must be reachable via `PATH` (the desktop app looks up `yt-dlp`
on PATH), or configured explicitly (CLI: `--extractor <path>`):

```powershell
winget install yt-dlp.yt-dlp      # winget (recommended)
scoop install yt-dlp              # scoop
choco install yt-dlp              # chocolatey
```

or download the standalone `yt-dlp.exe` from
<https://github.com/yt-dlp/yt-dlp/releases>, place it in a directory on `PATH`
(e.g. `%LOCALAPPDATA%\Microsoft\WindowsApps`), and verify with `yt-dlp --version`.

## Update strategy

YouTube changes frequently; an outdated extractor fails with extraction errors. The
extractor is updated independently of Velox:

- `yt-dlp -U` (self-update for the standalone binary), or
- `winget upgrade yt-dlp.yt-dlp` / `scoop update yt-dlp` / `choco upgrade yt-dlp`.

Velox never auto-updates the extractor and does not pin a version.

## Configuration

| Surface | Setting |
| --- | --- |
| Desktop app | enabled by default, resolves `yt-dlp` from `PATH` |
| CLI | `velox download <url> --extractor yt-dlp` (or bare name / full path) |
| Engine config | `EngineConfig::extractor_path: Option<PathBuf>` — `None` disables extraction |

## Google Drive vs media pages (related)

Google Drive share links (`/file/d/<id>/view`) are handled **natively by the HTTP
engine** — no extractor involved: the view page is rewritten to the download endpoint
and the virus-scan confirm interstitial is walked automatically. Publicly shared files
download directly; private, quota-exceeded and removed files fail with specific,
honest messages. See `docs/ROOT-CAUSE-REPORT.md` §3.
