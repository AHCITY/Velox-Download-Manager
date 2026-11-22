# In-app auto-updates (v0.3.0+)

Velox uses the official signed Tauri updater (`tauri-plugin-updater`), the same
mechanism other Tauri desktop apps use.

## Flow
1. **Check** — silently at startup (after ~4s), every 6 hours, and on window
   focus (throttled to 30 min). Endpoint:
   `https://github.com/AHCITY/Velox-Download-Manager/releases/latest/download/latest.json`.
2. **Guard** — the plugin trusts the manifest's version claim, so the frontend
   re-verifies it: a release is offered ONLY when it is strictly NEWER than the
   running app (semver, `web/assets/update-version.js` — unit-tested). This is
   the fix for the "update offered when already current" bug. The same release
   is announced once per install (localStorage-guarded), not once per check.
3. **Remind** — when a genuinely newer version exists: a toast (with an
   *Install* action), an `↑` badge on the Settings nav item, and a dedicated
   **Settings → Updates** section showing the installed version, the available
   version, and the time of the last check.
4. **Install (Phase 4 — never force-closes)** — the user confirms explicitly;
   if downloads are active the dialog says how many and offers to postpone.
   On confirmation: active downloads are PAUSED and awaited →
   `downloadAndInstall()` verifies the minisign signature and runs the NSIS
   installer in update mode → the app EXITS ITSELF → the installer (which
   waits for Velox to exit, `scripts/installer-unified-template.nsi`) finishes
   and starts the new version. No `taskkill /F` anywhere; postponing keeps the
   app running with the offer retained in Settings.

## Trust model
- Artifacts are signed at build time with the release private key
  (`.secrets/velox-updater.key`, gitignored; regenerate = cannot update
  existing installs).
- The public key is embedded in `tauri.conf.json`; unsigned or tampered
  installers are refused by the plugin.
- Downloads come from the project's GitHub releases over HTTPS.

## Release checklist (per version)
1. Bump `version` in `app/src-tauri/tauri.conf.json` + `Cargo.toml` +
   `scripts/installer-unified-template.nsi` (`PRODUCT_VERSION`).
2. `scripts/build-velox-msvc.sh` (signing env exported automatically).
3. `scripts/package-update-release.sh <staging-dir>` — copies the signed
   installer + `.sig`, generates `latest.json`, writes `SHA256SUMS.txt`.
4. Write `INSTALL-AND-VERIFY.md` in the staging dir, then
   `scripts/github-release-refresh.sh <staging-dir> v<version>` (uploads the
   installer, portable exe, sums, docs, `.sig` and `latest.json`).
5. Verify: `curl -sL .../releases/latest/download/latest.json` — version and
   non-empty signature; the URL must point at the release asset.
