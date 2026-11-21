# FFMPEG INTEGRATION — MediaPipeline spec (roadmap 4)

Status: **implemented slice** (engine-media crate, port-tested) / manager wiring next.
Supersedes: the "DEFERRED" placeholder in docs/OPEN_SOURCE_AUDIT.md §B (FFmpeg row).
Governing rules: docs/ENGINE_ARCHITECTURE_REVIEW.md §9, docs/OPEN_SOURCE_AUDIT.md.

## 1. Purpose

Adaptive media (DASH/HLS resolved by yt-dlp) downloads as separate video and audio
streams. Progressive single-file downloads never enter this pipeline. The merge stage
joins the streams into one file, **stream-copying only** (`-c copy`): no transcoding,
original quality preserved, CPU cost near zero.

## 2. Architecture (per review §9)

- MediaPipeline is a **manager-level wrapper job**, NOT inside the transfer engine:
  `crates/engine-media`, depending on engine-core only for events/errors.
- Flow: yt-dlp resolve → engine downloads streams → **merge phase** (this crate) →
  ffprobe post-validation → replace final artifact.
- All ffmpeg/ffprobe interaction goes through a **port** (`MediaPipelinePort`) so tests
  are deterministic: no real binaries or media needed for CI; the default port spawns
  real subprocesses.

## 3. License policy (hard rules, audit §B/§D)

1. **Subprocess only** — ffmpeg is never linked into Velox; no GPL code enters the
   binary. The engine works without ffmpeg installed (feature is optional, honest
   `MISSING_FFMPEG` errors when absent).
2. **LGPL builds only.** At every merge, `ffmpeg -version` output is checked:
   a build whose configuration contains `--enable-gpl` (or `--enable-nonfree`) is
   **rejected** with a `GPL_BUILD` error — even if the user installed it. Verification
   procedure per release we might ship: download the BtbN `…-win64-lgpl` zip, run
   `ffmpeg -version`, confirm the config line has NO `--enable-gpl`, archive the
   version output + zip URL as the build manifest next to the installer, and link the
   corresponding source tarball (LGPL §4/§6 source offer).
3. **User-provided or installer-offered binary.** Velox never bundles ffmpeg inside its
   own download. The installer MAY offer to run the WebView2-style bootstrap for an
   LGPL build; yt-dlp/ffmpeg remain user-owned externals with their own licences.
4. LGPL §4 notices: docs/EXTRACTORS.md credits; the licence report
   (`scripts/gen-license-report.sh`) covers Rust deps only — ffmpeg notices ship beside
   the offered binary download, not in the app.

## 4. Kill-safety contract

- The default port spawns children with **kill_on_drop(true)**: a dropped or aborted
  task cannot leak a running ffmpeg.
- `merge()` takes a `kill` trigger (`tokio::sync::watch`): manager cancel/shutdown sends
  `true`; the port selects on the trigger and `kill()`s the child, then returns
  `Cancelled`. Orphan prevention is verified by a real-process test (`sleep`).
- No shell interpolation: argv vectors only (no `sh -c`), paths passed as arguments;
  hostile filenames cannot inject flags.

## 5. Event contract (engine-core::events)

- `Merging { id }` — emitted when the merge subprocess starts (one per attempt).
- `Merged { id, path }` — emitted only after ffprobe post-validation succeeds.
- Failures emit the standard `Failed { id, kind, message }` with the new kinds below.

## 6. Honest errors (engine-core::ErrorKind)

| kind | meaning |
| --- | --- |
| `MISSING_FFMPEG` | no usable ffmpeg/ffprobe on PATH (or configured path); message names which tool |
| `GPL_BUILD` | the found ffmpeg advertises `--enable-gpl`/`--enable-nonfree` — refused by policy |
| `MERGE_FAILED` | ffmpeg exited nonzero or could not spawn; message carries the stderr tail |
| `CORRUPT_MEDIA` | merge exited zero but ffprobe cannot find playable streams — never report success |

All four classify as `Permanent`: retrying cannot fix them; the caller must act
(install LGPL ffmpeg, re-resolve streams, delete the artifact).

## 7. Merge invocation (fixed shape)

`ffmpeg -y -i <video> -i <audio> -map 0:v:0 -map 1:a:0? -c copy -movflags +faststart
<output>` — the codec flag `-c copy` is constructed by a pure function
(`build_merge_args`) and is not caller-settable; callers provide paths only.
`-movflags +faststart` keeps MP4s streamable; `-map 1:a:0?` tolerates video-only inputs.

## 8. Post-validation

`ffprobe -v error -print_format json -show_format -show_streams <output>` must return
JSON with at least one stream and a format section; anything else (nonzero exit, empty
streams, unparseable output) is `CORRUPT_MEDIA` with the output file left in place for
inspection and the manifest marked failed — never renamed to final.

## 9. Testing

- Port-faked tests: exact argv assertions, MISSING_FFMPEG, GPL_BUILD parsing
  (accept/reject matrices), CORRUPT_MEDIA paths, event emission, error classification.
- Real-process test: kill-safety against `sleep` (cancel → child dead ≤1s), no network,
  no media files.
- Authorized test media only when exercising real merges (never third-party
  copyrighted material).

## 10. Explicit non-goals (this slice)

- No transcoding, no re-encode fallback, no ffmpeg daemon, no server-side installs.
- yt-dlp resolve wiring and the manager job-type integration are the next slice;
  this crate ships the merge/validate/cancel core with its spec and tests.
