# RELEASE ERRORS PLAYBOOK — read BEFORE every release push

Last updated: 2026-11-26 (after the 4 failed `v0.7.0` tag attempts, runs #69/#71/#73/#75).

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

- Release page shows: `Velox-Setup-x64-unified.exe`, `Velox-Setup-x64-unified.exe.sig`,
  `latest.json`, `SHA256SUMS.txt`, `Velox-linux-x64-<ver>-offline.tar.gz`,
  `Velox-linux-x64-<ver>.tar.gz`, `velox-setup-linux-online.sh`.
- `latest.json`: version == shipped version, URL points at THIS tag's asset,
  `pub_date` ISO, signature matches the `.sig` file content.
- The `.sig` trusted comment contains `version:<ver>` (the updater
  cross-checks this — E-8/E-9).
- `SHA256SUMS.txt` covers every asset and the numbers match a fresh
  `sha256sum` download.
- Release notes contain both the Windows and Linux sections.

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
**History:** run #71 failed in 2m50s for exactly this; fixed by 913c906.

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

## Known non-errors (do not chase)

- `cargo build` re-downloading 294 crates: no cargo cache is configured —
  expected, see the time budget. (Caching is a future optimization, NOT a
  blocker; it was deliberately deferred mid-release once.)
- `ffmpeg.zip` ~179 MB re-downloaded per desktop run: expected, same as above.
- Desktop-app unit-test step "velox-app is a standalone crate": it only
  compiles where the Tauri stack exists; on Linux CI it is covered by the
  `linux-desktop` build instead.
- The `desktop-online` job builds an installer that is NOT on the release
  page: by design (release page policy: unified installer + updater trio +
  checksums + Linux distributions).
