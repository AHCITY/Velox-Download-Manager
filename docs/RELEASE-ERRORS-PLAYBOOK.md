# RELEASE ERRORS PLAYBOOK — read BEFORE every release push

Last updated: 2026-11-26 — the 4 failed `v0.7.0` tag attempts (#69/#71/#73/#75)
are fully diagnosed from job logs (verified failure map below), AND the
published v0.7.0 page has been conformed to the download policy (sequenced,
numbered downloads + labels, online installer attached, formal notes —
see E-21..E-24 for the page-cleanup pitfalls hit along the way).

Every release so far needed more than one push. This file is the accumulated,
verified knowledge of WHY each class of release failed and the exact fix, plus
the staged release procedure that now replaces the "one combined tag push"
approach. Read the [Error catalogue](#error-catalogue) top-to-bottom before
pushing anything, and check a failed run's log against it before inventing a
new theory — the answer is usually already in here.

---

## The staged release procedure (v0.7.0 onward) — ONE push at a time

The single biggest structural problem was that ONE tag push gated FIVE jobs
together (`windows` gate → `desktop` + `desktop-online` + `linux-desktop` →
`release`). Any Linux hiccup killed the Windows release too, and every retry
meant moving the tag. Starting with v0.7.0 the pipeline is SPLIT and STAGED:

```
Push 1 (Windows):  push branch  release/windows/v0.7.0   (from main)
                   -> windows.yml: engine gate -> desktop (+desktop-online)
                      -> release job CREATES the GitHub release v0.7.0
                         (installer + .sig + latest.json + SHA256SUMS)

Push 2 (Linux):    push branch  release/linux/v0.7.0     (from main, ONLY
                   after the Windows release exists)
                   -> linux.yml: build + xvfb smoke test
                      -> upload Linux assets INTO the existing v0.7.0 release
                         (appends checksums + Linux section to the notes)
```

Rules:

1. **Windows first, Linux second — never both at once.** The Linux upload job
   hard-fails if the release does not exist yet, so the order is enforced by CI.
2. **Trigger = branch push, not a tag.** `release/windows/**` and
   `release/linux/**` are the only release triggers. Tag pushes trigger
   nothing anymore (this prevents CI from re-running against a tag the release
   job itself just created).
3. **The version comes from `download-engine/app/src-tauri/tauri.conf.json`.**
   Both workflows derive `TAG=v<that version>`. Bump the version there, in the
   app `Cargo.toml`, AND re-pin `app/src-tauri/Cargo.lock` (see error E-10).
4. **Never move a published tag.** Retrying = push the release branch again
   (or a new `release/windows/v0.7.0-2` branch). The release job is
   idempotent: if the release already exists it re-uploads assets with
   `--clobber` instead of failing.
5. Both release jobs are safe to re-run on the same branch; every upload uses
   `--clobber`, checksum merging strips stale lines for the platform being
   re-uploaded.

### Time budget (cold runners, NO cargo caching anywhere)

| Stage | Wall time |
|---|---|
| `windows` engine gate (fmt+clippy+build+tests+smokes) | ~11–13 min |
| `desktop` (app build + NSIS + sign + verify) | ~8–12 min (fat-LTO link alone 2.5–3.5 min) |
| `desktop-online` | runs in parallel with `desktop`, not on the critical path |
| `release` job | ~1–2 min |
| `linux-desktop` (apt deps + app build + smoke + package) | ~15–20 min |
| `linux-release` | ~1–2 min |

So: Windows release live ~25–35 min after Push 1; Linux assets ~20 min after
Push 2. **Never diagnose a "stuck" release before this much time has passed.**

### Pre-push checklist

- [ ] `tauri.conf.json` version == app `Cargo.toml` version == `Cargo.lock`
      `velox-app` version (E-10).
- [ ] `scripts/velox-updater.key` exists in the repo and is non-empty (E-7).
- [ ] Windows main-branch CI is green for the commit you branch from.
- [ ] No unreleased colons in new YAML step names (E-1).
- [ ] You are on `main` and up to date; release branches are created FROM
      `main` at the exact commit you want to ship.

### Post-release verification (what "done" means)

- Release page shows, in this order (numbered names make GitHub's alphabetical
  listing display the downloads in order; labels carry the same numbers):
  `1-Velox-Setup-x64.exe` (label "1. Velox Setup"), `2-Velox-Setup-x64-Offline.exe`
  (label "2. Velox Setup (Offline)"), `3-Velox-linux-x64-<ver>-offline.tar.gz`,
  `4-velox-setup-linux-online.sh`, then the unnumbered metadata:
  `latest.json`, `SHA256SUMS.txt`, `Velox-Setup-x64-Offline.exe.sig`,
  `Velox-linux-x64-<ver>.tar.gz`.
- Release notes are rendered by `scripts/release-notes.sh` — the single source
  of truth for the page text; windows.yml, linux.yml and release-page.yml all
  write the SAME body (idempotent).
- The online installer is published by the release job (needs:
  [desktop, desktop-online]) — both installers ship on every release.
- `release-page.yml` is a self-healing guard: it re-conforms the page of the
  version in tauri.conf.json on every main push and no-ops when the page is
  already conforming.
- `latest.json`: version == shipped version, URL points at THIS tag's asset,
  `pub_date` ISO, signature matches the `.sig` file content.
- The `.sig` trusted comment contains `version:<ver>` (the updater
  cross-checks this — E-8/E-9).
- `SHA256SUMS.txt` covers every asset and the numbers match a fresh
  `sha256sum` download.
- Release notes contain both the Windows and Linux sections.

---

## Verified failure map — the four v0.7.0 tag runs (read this first)

Pulled from the actual job logs of each run (2026-11-25). The decisive lesson:
**the Windows side was green in #71, #73 AND #75** — every one of the four
failures was a different Linux job, one step further each time. A shared
release gate turned three platform fixes into four destroyed release
attempts.

| Run | windows gate | desktop | desktop-online | linux-desktop (failed step) | release | Root cause |
|---|---|---|---|---|---|---|
| #69 (766712e) | ✅ | ❌ lock gate ~3 min | ❌ lock gate | ❌ build (E-02) | skipped | E-10 (stale `app/Cargo.lock`) + E-2 (cfg(windows) dep table) |
| #71 (913c906) | ✅ | ✅ | ✅ | ❌ build (E-2) | skipped | E-2 only → fixed by b7d9e78 |
| #73 (b7d9e78) | ✅ | ✅ | ✅ | ❌ xvfb smoke test (E-3) | skipped | E-3 only → "fixed" by 7373ea4 |
| #75 (7373ea4) | ✅ | ✅ | ✅ | ✅ smoke PASSED → ❌ **packaging** | skipped | **E-17** (untested verification grep) |

Notes that rewrite earlier assumptions:

- Run #75's smoke test **PASSED** with `WEBKIT_DISABLE_COMPOSITING_MODE=1 +
  LIBGL_ALWAYS_SOFTWARE=1` (the ubuntu-22.04 WebKitGTK is pre-DMABUF). The
  extra hardening in `linux.yml` stays — it protects against future runner
  image bumps — but E-3 layer 1 was NOT the last blocker.
- Run #75 died in the packaging step at a **verification grep that could
  never match** (E-17) after 17.7 s of successful tarring, silently — with
  no error output at all. Untested verify-checks are as dangerous as
  untested builds.
- The old pipeline's `release` job was skipped in every failed run: the
  Windows installer existed as artifacts in all of #71/#73/#75 and was never
  published because Linux kept gating it. This is E-12 in its purest form.

---

## Error catalogue

Legend: each entry = signature you see in logs/UI → root cause → fix →
where it bit us. Ordered roughly by how often they recur.

### E-1 · Workflow runs with ZERO jobs after a YAML edit
**Signature:** the push shows a run that starts and instantly ends with no
jobs, or Actions says "no jobs ran" / workflow invalid.
**Cause:** a plain-scalar step/job name containing a colon (`:`) — YAML
parses it as a mapping key and the workflow definition silently breaks.
**Fix:** no colons in unquoted step names, or quote the whole name.
**History:** v0.6.0 tag run had ZERO jobs for exactly this reason; the whole
release looked "ignored".
**Rule:** after ANY workflow edit, confirm the run actually contains jobs
within ~1 minute of pushing.

### E-2 · Linux job cannot compile `velox-app` — "unresolved import serde/tokio/engine-*"
**Signature:** `error[E0432]` / `unresolved import` for crates that are
clearly in `Cargo.toml`, ONLY on the Linux build.
**Cause:** a `[target.'cfg(windows)'.dependencies]` table earlier in
`Cargo.toml` — everything after the target header inherits the windows-only
scope. Platform-independent deps listed under it never build on Linux.
**Fix:** keep ONLY genuinely windows-only deps (e.g. `winreg`) inside the
target table; move serde/tokio/engine crates back to `[dependencies]`.
**History:** run #73 (commit b7d9e78 fixed it after run #69).

### E-3 · xvfb startup smoke test fails — app never creates its data dir
**Signature:** `STARTUP SMOKE TEST FAILED: app data dir was never created`
after 60–90 s.
**Causes (three separate layers, ALL must be handled):**
1. WebKitGTK ≥ 2.42 uses the DMABUF renderer, which crashes in headless
   VMs → **must export `WEBKIT_DISABLE_DMABUF_RENDERER=1`** (the older
   `WEBKIT_DISABLE_COMPOSITING_MODE=1` alone is NOT enough on current
   ubuntu-22.04 runners).
2. No GPU → also export `LIBGL_ALWAYS_SOFTWARE=1` (+ keep
   `WEBKIT_DISABLE_COMPOSITING_MODE=1`).
3. Slow first boot → poll up to 90 s, not 10.
**Fix (current shape in `linux.yml`):** set all three env vars, optionally
wrap in `dbus-run-session` when available, poll up to 75×1 s for
`~/.local/share/io.velox.downloadmanager/data` (the `data` subdir — the
ENGINE creates it during Tauri setup, so it is proof the whole app booted,
not just the window), dump the app log on failure.
**History:** commit 7373ea4 fixed layers 2–3 (runs #73→#75) but run #75 still
failed; layer 1 (DMABUF) is applied in linux.yml from v0.7.0 onward — if the
smoke test fails again, read the app-smoke.log dump (now automatic) before
theorizing.

### E-4 · Windows flaky test failures in `crash_resume` under Defender
**Signature:** `crash_resume` integration tests fail intermittently, often
with the parent process unable to open the child's log file, ONLY on
windows-latest.
**Cause:** Windows Defender scans every freshly-built exe; parallel spawns of
`velox.exe` children race the scanner.
**Fix:** `cargo test -p engine-cli --test crash_resume -- --test-threads=1`
(the workflow already does this — do not "optimize" it back).
**History:** 2026-11-21 CI evidence, cost at least one full tag run.

### E-5 · makensis "no files found" for staged files
**Signature:** NSIS compile error `File: "staging/Velox.exe" — no files found`
on the Windows runner, while the same template works locally on Linux.
**Cause:** NSIS `util.cpp get_dir_name` splits File paths on the HOST
separator only. A template written with forward slashes works on a Linux
build host and breaks on Windows (and vice versa).
**Fix:** the template joins paths via `${SEP}` (default `\`, Linux local
builds pass `-DSEP=/`). Never hand-write absolute staged paths in the `.nsi`.
**History:** commit eec7c6f; cost run #59.

### E-6 · "installer is INCOMPLETE — missing ..." in the 7-Zip verify step
**Signature:** the `Verify the installer actually contains every required
component` step throws missing items that ARE in the staging dir.
**Cause:** 7-Zip's NSIS handler lists inner paths with the HOST separator:
the Windows runner prints `extension\README.md` while the required list uses
`extension/README.md`.
**Fix:** normalize the listing before matching:
`$listing = (& $seven l <exe>) -replace '\\','/'` (already in the workflow).
**History:** v0.6.0 runs #59 → #61 (fixed by d3a3337).

### E-7 · Updater signature missing / key wiped
**Signature:** `updater signature missing — TAURI_SIGNING_PRIVATE_KEY was not
picked up`, or a release ships with no `.sig`.
**Cause:** the minisign private key is the single most critical release
secret. It used to live only in gitignored `.secrets/` and was lost in
sandbox recycles; a lost key means shipping a NEW pubkey (old clients can
never auto-update again — that stranded every 0.4.0 install).
**Fix:** the key is tracked at `scripts/velox-updater.key` (dev-provenance
convention of this repo). CI reads it via `$env:GITHUB_WORKSPACE`
(see E-8) and the job hard-throws if the `.sig` is not produced.
**Rule:** before every release, `git show HEAD:scripts/velox-updater.key` on
a fresh clone and confirm it is non-empty.

### E-8 · CI cannot find the updater key (relative path)
**Signature:** desktop job fails reading `scripts\velox-updater.key` with a
path like `..\..\scripts\...` in the error.
**Cause:** the job computed the key path relative to the working directory
(`download-engine`), which resolves differently between gate and desktop
jobs.
**Fix:** always `Join-Path $env:GITHUB_WORKSPACE 'scripts\velox-updater.key'`.
**History:** first v0.5.0 tag run; fixed in f1a84b3, cost one tag move.

### E-9 · Updater endpoint 404 / "no update available" for a shipped release
**Signature:** `releases/latest/download/latest.json` 404s; in-app checks
error out.
**Cause:** a release published without `latest.json`, or `latest.json` whose
asset URL points at a different tag, or a `.sig` whose trusted comment
`version:` does not match `latest.json`'s version (the updater 2.14+
cross-checks it).
**Fix:** the release job generates `latest.json` with the tag-pinned asset
URL and the shipped version, and ALWAYS signs with
`tauri signer sign --app-version <ver>`. Never hand-edit `latest.json`.
**History:** v0.4.0 shipped with no updater artifacts at all; every 0.4.0
install had to manually reinstall 0.5.0.

### E-10 · Desktop build fails fast (~3 min) at `cargo fetch --locked`
**Signature:** `error: the lock file ... needs to be updated but --locked was
passed` in the desktop/desktop-online job, minutes into the run.
**Cause:** version bumped in `Cargo.toml`/`tauri.conf.json` but
`app/src-tauri/Cargo.lock` not re-pinned. The desktop jobs build `--locked`
as a reproducibility gate, so they refuse.
**Fix:** after ANY version bump run `cargo update -p velox-app --precise
<newver>` (or a build) inside `app/src-tauri` and COMMIT the lock file.
**History:** run #69's desktop AND desktop-online jobs both died here ~3 min
in; fixed by 913c906 (pin velox-app 0.7.0 in app Cargo.lock).

### E-11 · Chocolatey NSIS install flaky
**Signature:** `choco install nsis` succeeds but `makensis.exe` missing, or
choco itself times out.
**Fix:** the workflow falls back to the direct SourceForge
`nsis-3.10-setup.exe /S` install and hard-fails only if makensis is still
absent. Do not remove the fallback.
**History:** recurring on windows-latest; the fallback saved at least two runs.

### E-12 · One platform's failure poisons the whole release (STRUCTURAL)
**Signature:** tag run red, release page empty, even though the Windows
installer built fine — because `release` needed `[desktop, linux-desktop]`.
**Cause:** monolithic tag pipeline (the root cause of "never pushed in 1
try"): 4 consecutive failed v0.7.0 tag runs (#69 #71 #73 #75), each moving
the tag, each blocked by a DIFFERENT job.
**Fix:** the staged split described at the top — Windows release first,
Linux assets appended second, independent failure domains.
**History:** the entire v0.7.0 saga; this playbook + split pipelines are the
direct outcome.

### E-13 · `gh release create` fails on retry ("already exists")
**Signature:** re-pushing a release branch after a partial success fails the
release job at `gh release create`.
**Fix:** current jobs check `gh release view <TAG>` first: create when
absent, `gh release upload --clobber` when present. All release-side
operations must stay idempotent.
**Rule:** retries must never require tag surgery (see also rule 4 above).

### E-14 · The old "delete remote tag → retag → push" pattern
**Signature:** historical releases needed it (v0.5.0, v0.7.0 x4).
**Why it is now forbidden:** it re-runs ALL jobs against the moved tag,
invalidates any already-published asset URLs, and clients that saw the first
release can hold stale metadata. It was only ever "safe" when NO release
existed yet. With the staged flow, retries are branch re-pushes instead.

### E-15 · Stale hardcoding in release tooling
**Signature:** verification scripts check the WRONG tag (`release-check.sh`
hardcoded `v0.5.0` through the v0.6.0 cycle), notes templates hardcode
versions, `PRODUCT_VERSION` duplicated in the NSIS template.
**Fix:** derive everything from `tauri.conf.json` (CI already derives
PRODUCT_VERSION at build time). Audit any release-adjacent script for a
hardcoded version before shipping.

### E-16 · Sandbox/credential loss between sessions
**Signature:** the sandbox recycles and the GitHub token / updater key /
build toolchain vanish; pushes become impossible mid-flow.
**Mitigation:** release-critical secrets are repo-tracked
(`scripts/velox-updater.key`); the release flow is now designed to be driven
by BRANCH PUSHES (which the GitHub connector can do), not local git; this
playbook lives in the repo, not in the sandbox.
**Rule:** assume the sandbox can die at any moment; never leave release
state only in `/tmp` or the sandbox home.

### E-17 · Packaging dies silently at a verification grep — the E-17 class
**Signature:** a `run:` step prints NOTHING after the group banner and exits
1; the commands before it (tar/sed/chmod) visibly succeeded. No error line
anywhere in the log.
**Cause (verified, killed run #75 after 17.7 s of successful tarring):** the
packaging step verified the substituted online-installer script with
`grep -q "releases/download/v$VER" file`. But
`scripts/velox-setup-linux-online.sh` resolves the version at RUNTIME via its
own `VER` variable — after sed the file contains the assignment
`VER="0.7.0"`, and the URL line still reads `releases/download/v$VER` as
literal shell source. Grep can never match the expanded URL. `set -e` killed
the step with zero output.
**Fix (in linux.yml):** grep the embedded assignment
(`grep -q "VER=\"$VER\"" file`) and `bash -n` the generated script.
**Rule:** EVERY verification grep in a release pipeline must be executed once
locally against the real artifact/script before it ships. A check that was
never run once is a landmine — verify-checks fail silently exactly when
everything else works, which is the worst possible time.

### E-18 · `! grep -q ...` under `set -e` can NEVER fail the step
**Signature:** a negative check ("placeholder must be gone", "tool must NOT
be bundled") that provably should abort does nothing.
**Cause:** bash errexit explicitly ignores commands whose return value is
inverted with `!`. The step sails past a violated invariant and ships it.
**Fix:** use the explicit form in every negative check:
`if grep -q "pattern" file; then echo "ERROR: ..."; exit 1; fi`
(applied to the placeholder check and the "core must not bundle ffmpeg /
yt-dlp" checks in linux.yml).
**Rule:** audit every `! cmd` in release workflows; under `set -e` they are
decorations, not checks.

### E-19 · `curl` without `-f` ships error pages as binaries
**Signature:** none up front — the download "succeeds" (exit 0) and the file
exists; later verification greps like `tar -tzf | grep bin/yt-dlp` still pass
because the FILE NAME is right even when the CONTENT is an HTML error page.
**Cause:** transient 4xx/5xx (rate limits, asset renaming, CDN hiccups) make
`curl -sL` write the error body to the output path and exit 0.
**Fix:** `curl -fL --retry 3` for every binary fetched into a release
package, plus a size sanity check (`test "$(stat -c%s f)" -gt 1000000`) for
files that are known to be megabytes (yt-dlp ~4 MB, ffmpeg tarball ~90 MB).
**History:** the v0.7.0 linux packaging step fetched yt-dlp/ffmpeg with bare
`-sL` — one GitHub hiccup would have shipped a broken "yt-dlp" inside the
offline tarball with every check green.

### E-20 · The smoke HARNESS itself breaks the app ("unknown path" panic)
**Signature:** `thread 'main' panicked ... Failed to setup app: error
encountered during setup hook: unknown path` — the app dies before creating
its data dir; the smoke test reports "data dir was never created".
**Cause:** "unknown path" is `tauri::Error::UnknownPath` — a `dirs::*_dir()`
resolver returned `None` (main.rs setup calls `app_data_dir()` and
`download_dir()`; the latter goes through the xdg-user-dirs machinery). The
binary was byte-identical to a run where the smoke test PASSED — the delta
was the smoke step's own "defensive improvements": wrapping in
`dbus-run-session`, overriding `XDG_RUNTIME_DIR`, adding
`WEBKIT_DISABLE_DMABUF_RENDERER`. Any of these changes the environment the
resolvers see and can flip `None`.
**Fix (linux.yml):** the smoke env is FROZEN to the proven set —
`WEBKIT_DISABLE_COMPOSITING_MODE=1 LIBGL_ALWAYS_SOFTWARE=1` and nothing else,
plain `xvfb-run -a timeout 90` invocation. Diagnostics only: the app log is
dumped on failure and `RUST_BACKTRACE=1` is exported (output-only, no
semantics).
**Rule:** treat the smoke harness like production config: never add
environment "hardening" to it without a green run that proves it. Diagnose
with logs, not with new env vars.
**Follow-up (app, NOT a release blocker):** `download_dir()` panicking at
setup when a Linux user lacks xdg-user-dirs is a real-world robustness bug —
fall back to `$HOME/Downloads` in a future version instead of `?`.

### E-21 · GitHub silently mutates release-asset names (spaces, parentheses)
**Signature:** the upload returns 201 but the stored name is not the name you
sent: `Velox Setup 0.7.0.exe` comes back as `Velox.Setup.0.7.0.exe` (spaces →
dots) and `Velox Setup 0.7.0 (offline).exe` as `Velox.Setup.0.7.0.offline.exe`
(parentheses REMOVED). No error anywhere.
**Cause:** GitHub asset-name sanitization — spaces become dots, `(` and `)`
are stripped. Verified empirically on v0.7.0 (2026-11-26); the literal
"(offline)" requested for the page can therefore never exist as a FILE name.
**Fix:** use the numbered/hyphen name policy (`1-Velox-Setup-x64.exe`,
`2-Velox-Setup-x64-Offline.exe`, …). Human-readable naming lives in the asset
LABEL (`2. Velox Setup (Offline)`) — labels DO render as the bold display
text on the release page and may contain spaces/parentheses.
**Rule:** after EVERY asset upload/PATCH, compare the response `name` against
what you sent; never trust the request to round-trip.

### E-22 · Release-page asset order is ALPHABETICAL — re-uploading cannot re-sequence
**Signature:** assets appear in a jumbled order; deleting and re-uploading
them in the desired order changes nothing.
**Cause:** the release page (and `expanded_assets`) lists assets sorted by
name, case-insensitively — upload order is irrelevant. Latest-first metadata
names (`latest.json`, `SHA256SUMS.txt`) sort before `Velox-*`, and
`Velox-linux-*` sorts before `Velox-Setup-*` (`l` < `s`), burying the main
Windows installer at the bottom.
**Fix:** sequence by NAMING: number the four downloads (`1-…`/`2-…` Windows,
`3-…`/`4-…` Linux) so digits sort before letters and the platform order is
explicit; keep `latest.json`/`SHA256SUMS.txt`/`.sig`/core tarball unnumbered
metadata after them. Update linux.yml/windows.yml/templates so future CI
produces conforming names natively, and `release-page.yml` (self-healing
no-op when conforming) guards older pages.

### E-23 · Renaming an asset breaks three things at once (updater URL, sums, .sig)
**Signature:** after a rename the in-app updater 404s (E-9 family);
`sha256sum -c SHA256SUMS.txt` fails on the renamed file; the signature file
no longer sits beside its payload.
**Cause:** three artifacts reference an asset NAME, not its bytes:
`latest.json` `platforms.*.url`, the corresponding line in
`SHA256SUMS.txt`, and the `.sig` sibling naming.
**Fix (rename-only conformance):** retarget the URL inside `latest.json` and
re-upload it (NO re-signing — the minisign signature covers the payload
BYTES; a rename changes none, so the signature stays valid); regenerate
`SHA256SUMS.txt` by downloading EVERY asset fresh and hashing it (E-19
discipline — verify the remapped sums, don't trust the remap); rename the
`.sig` alongside its installer. The sig's embedded `file:` trusted comment
lags behind the new name — informational only, the updater cross-checks
`version:` binding.
**History:** the v0.7.0 page cleanup (2026-11-26) — verified end-to-end:
updater URL resolves, all 7 sums verify OK.

### E-24 · Unauthenticated GitHub API from the sandbox rate-limits mid-release
**Signature:** reads suddenly return `{"message":"API rate limit exceeded…"}`
and a JSON parse "fails" — even though the call worked minutes ago.
**Cause:** anonymous API calls share a 60-requests/hour budget per IP; a
release session exceeds it fast.
**Fix:** authenticate EVERY call (`Authorization: Bearer <token>`), including
reads. 5,000 req/h authenticated.


### E-25 · Theme PR: `BUILTIN_THEMES` key ≠ theme id (CI red + broken theme at runtime)
**Signature:** `web/tests/themes.test.mjs` fails asserting
`Object.keys(BUILTIN_THEMES)`; the new theme silently never applies at runtime
(`applyTheme('soft-canopy')` misses).
**Cause:** the object literal keyed the theme by its camelCase JS variable
(`softCanopy`) while the theme id is `soft-canopy`. `getTheme(id)` looks up by
id — a key/id mismatch is invisible until the test pins the key list.
**Fix:** key the map by the id: `{ dark, light, purple, blue, 'soft-canopy': softCanopy }`.
**Rule:** a built-in theme map key MUST equal the theme id; the node test pins it.

### E-26 · windows.yml: `$size = ath]::Round(...)` — corrupted `[math]::Round`
**Signature:** release builds would die parsing the pwsh step (`ath]::Round` is
not valid PowerShell); slipped through because the `desktop`/`desktop-online`
jobs are SKIPPED on normal main pushes and the last release predated the edit.
**Cause:** a bad edit in the 1c9d2ef release-page cleanup.
**Fix:** `[math]::Round` (both offline + online size logs). **Rule:** run the
release workflow on a throwaway branch before trusting an edited release job.

### E-27 · windows.yml: upload-artifact path pointed inside `download-engine/`
**Signature:** `if-no-files-found: error` — the artifact upload fails although
the installer was built.
**Cause:** the installers are copied to the WORKSPACE ROOT (`Copy-Item ... .`),
but the artifact `path:` said `download-engine/Velox-Setup-x64-*.exe`.
**Fix:** artifact paths are workspace-root-relative here.

### E-28 · Online installer aborts mid-way: "Error opening file for writing: velox-browser-host.exe"
**Signature:** install dies around the browser-ecosystem staging (the reported
"browser-… something" error); Velox never starts because the install never
finished.
**Cause:** the native-messaging host is spawned by the BROWSER, not by
Velox.exe — closing Velox (CLOSEVELOX_GRACEFUL) leaves a stale
`velox-browser-host.exe` running, and the NSIS `File` command cannot overwrite
a running executable.
**Fix:** both installer templates detect the stale host (tasklist) and
force-terminate it (taskkill /F) before staging. Force is safe: the host is a
headless stdio forwarder holding no download state (the engine owns it all).

### E-29 · Online installer "hangs" during the ~120 MB FFmpeg download
**Signature:** users see the installer sitting mid-way with no visible progress
(then give up or kill it).
**Cause:** PowerShell 5 `Invoke-WebRequest` renders a progress bar per chunk;
that UI work can dominate large downloads.
**Fix:** `$ProgressPreference = 'SilentlyContinue'` at the top of
velox-online-fetch.ps1 (already retried 3x with backoff).

### E-30 · YouTube downloads finish as TWO separate files + a merge error
**Signature:** video + audio rows both complete; the video row then fails with
`GPL_BUILD` or `MISSING_FFMPEG`; the user ends with 2 files and no merged mp4.
**Cause (root):** the LGPL license gate refused EVERY ffmpeg advertising
`--enable-gpl` — which is what winget/gyan/distro builds ship. Since Velox
merely INVOKES a user-installed PATH binary (it does not distribute it), the
refusal was over-broad and broke merging for most users.
**Fix:** the gate now applies only to BUNDLED binaries (inside the app's tools
dir — those we distribute). PATH-resolved binaries run ungated
(`LicenseGate::Skip`), regression-tested in engine-media. The bundled layout
keeps the LGPL gate (a swapped binary is still caught at merge time).

---

## Quick triage table (fastest path from symptom to fix)

| Symptom | Go to |
|---|---|
| Run started, zero jobs | E-1 |
| Linux `error[E0432]` unresolved imports | E-2 |
| `STARTUP SMOKE TEST FAILED` | E-3 |
| `crash_resume` red on Windows only | E-4 |
| makensis `no files found` | E-5 |
| `installer is INCOMPLETE` | E-6 |
| `.sig` missing / key errors | E-7, E-8 |
| Updater 404 / version mismatch | E-9 |
| `--locked` fetch failure ~3 min in | E-10 |
| makensis missing entirely | E-11 |
| Release red but Windows built fine | E-12 |
| `gh release create` already exists | E-13 |
| Release-check reports wrong tag | E-15 |
| Packaging step exits 1 with NO output | E-17 |
| Negative check never aborts | E-18 |
| Downloaded tool is corrupt/error page | E-19 |
| Uploaded asset name comes back different | E-21 |
| Re-uploading does not fix the download order | E-22 |
| Renamed asset: updater 404 / sums fail | E-23 |
| Reads start returning rate-limit JSON | E-24 |
| Theme PR: tests fail on `BUILTIN_THEMES` keys | E-25 |
| Release job dies parsing pwsh (`ath]::Round`) | E-26 |
| Artifact upload says file missing after a build | E-27 |
| Online installer dies writing velox-browser-host.exe | E-28 |
| Online installer seems stuck on the FFmpeg download | E-29 |
| YouTube: 2 files + merge error (`GPL_BUILD`/`MISSING_FFMPEG`) | E-30 |

## Known non-errors (do not chase)

- `cargo build` re-downloading 294 crates: no cargo cache is configured —
  expected, see the time budget. (Caching is a future optimization, NOT a
  blocker; it was deliberately deferred mid-release once.)
- `ffmpeg.zip` ~179 MB re-downloaded per desktop run: expected, same as above.
- Desktop-app unit-test step "velox-app is a standalone crate": it only
  compiles where the Tauri stack exists; on Linux CI it is covered by the
  `linux-desktop` build instead.
- The `desktop-online` job builds the ONLINE installer, published by the
  release job alongside the offline payload (download policy since the
  v0.7.0 page cleanup: numbered 1-4 downloads + unnumbered metadata).
  Until v0.7.0 the release job skipped it — release-page.yml attaches it
  to older releases automatically.
