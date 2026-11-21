# YOUTUBE VERIFICATION — residential-network checklist (roadmap 5)

Status: **cannot be executed from the build sandbox** (datacenter IP — the bot wall is
IP-scoped; KNOWN-ISSUES #1 reproduces it with standalone yt-dlp, so it is not a Velox
defect). This checklist is for the USER to run from a residential connection.
Result of every scenario goes into KNOWN-ISSUES #1 (one line each: date, IP type,
scenario, outcome). The gate claim "YouTube works" is made ONLY when S1–S4 pass.

## 0. Prerequisites

1. **Residential network** — home Wi-Fi/cable, no VPN, no datacenter, no cloud VM.
   (Verify: your public IP's org should be an ISP, not a hosting company.)
2. **Current yt-dlp** on PATH: `yt-dlp -U && yt-dlp --version` (≥ the tested
   2026.10.26; YouTube changes fast).
3. **A browser session logged into YouTube** (for the cookie remedy in S3) — a throwaway
   account is fine; cookies leave your machine only toward YouTube, same as yt-dlp.
4. **Velox installed** — `velox version` (Release v0.1.0 or later).
5. Optional (S4 only): an **LGPL ffmpeg** on PATH — `ffmpeg -version` must NOT show
   `--enable-gpl`. The engine refuses GPL builds with `GPL_BUILD` by design.

## S0 — baseline: standalone yt-dlp (isolates the network from the engine)

```
yt-dlp -J --no-playlist --no-warnings "https://www.youtube.com/watch?v=XdNjEct5mqc" > nul
```
(Windows `nul`; macOS/Linux `/dev/null`)

- **JSON printed** → residential IP clears the wall; continue to S1.
- **"Sign in to confirm you're not a bot"** → still IP-scoped even at home (mobile
  CGNAT ranges can trigger it): try a different network before blaming anything.

## S1 — honest refusal without an extractor (no network dependency)

```
velox download "https://www.youtube.com/watch?v=XdNjEct5mqc"
```

Expected: failure with `UNSUPPORTED_URL` and guidance naming yt-dlp. Nothing may be
registered as a download. This verifies the classifier; it cannot pass "by accident".

## S2 — watch URL WITH extractor configured (progressive stream, the core case)

Configure the extractor (config file or env per your install; the engine calls
`yt-dlp -J --no-playlist --no-warnings [--add-headers …] <url>` under the hood), then:

```
velox download "https://www.youtube.com/watch?v=XdNjEct5mqc" -o <dest>
```

Expected: resolves to a progressive format, downloads with the extractor-proposed
name, `completed` state. Record the final SHA-256 if the engine printed one.

If the bot wall still fires here but S0 printed JSON → **capture `--verbose` output
and file it** (that combination would be an engine defect: extractor invocation or
header forwarding).

## S3 — cookie remedy (audit D2 — the standard bot-wall remedy)

Log your browser into YouTube, export cookies into Velox (cookie jar file or the
per-download `cookies` config), rerun S2's command from the SAME IP that failed or
succeeded. Expected: cookies reach the extractor process as `--add-headers
Cookie: …` — this exact path is CI-tested (`extractor_receives_user_cookies_as_add_headers`).
Success = the download completes where the cookie-less attempt hit the wall.

## S4 — DASH two-stream + merge (needs ffmpeg; the MediaPipeline)

Only a minority of videos still offer progressive formats; high-quality ones are
DASH (separate video+audio). With ffmpeg configured and the manager's media merge
enabled:

1. Start the two-stream download for the same video (the resolved-job flow — your
   client resolves via yt-dlp and hands Velox the stream URLs).
2. Watch states: both stream rows reach `completed`, then the video row shows
   **`merging`** (SSE events `merging`/`merged` also fire).
3. End state: video row `completed` with the merged file (`<name>.mp4` by default);
   the audio row stays `completed` as the source stream.
4. Play the merged file fully through; jump around the timeline (faststart Moov check).
5. Cancel test: start another two-stream download, cancel during `merging` — the
   ffmpeg child must die (no orphan `ffmpeg` in Task Manager / `ps`), row becomes
   `cancelled`/forgotten.

Expected honest failures to NOTE (not bugs): `MISSING_FFMPEG` (no binary),
`GPL_BUILD` (GPL-flagged ffmpeg — install an LGPL build), `CORRUPT_MEDIA` (merge ran
but output is broken — keep the file for the report).

## What to record per scenario (into KNOWN-ISSUES #1)

`date | IP type (residential/CGNAT) | yt-dlp version | velox version | scenario |
outcome | error kind/message if failed`. Screenshots of any bot-wall text help
upstream debugging; include the `--verbose` tail if S2 failed while S0 passed.

## Already verified WITHOUT a residential network (CI, 2026-11-21)

Classifier refusal (S1's shape), extractor argv/cookies contract (D2), hung-extractor
timeout + kill-no-orphan (D1), merge argv pinning (`-c copy` only), LGPL gate
(`GPL_BUILD`), ffprobe post-validation (`CORRUPT_MEDIA`), kill-safety on cancel.
What residential testing ADDS: proof that YouTube actually serves the wall-free
response to the extractor under real conditions — the only part that cannot run here.
