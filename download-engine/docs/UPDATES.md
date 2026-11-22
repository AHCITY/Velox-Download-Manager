# In-app auto-updates (v0.3.0+)

Velox uses the official signed Tauri updater (`tauri-plugin-updater`), the same
mechanism other Tauri desktop apps use.

## Flow
1. **Check** — silently at startup (after ~4s), every 6 hours, and on window
   focus (throttled to 30 min). Endpoint:
   `https://github.com/AHCITY/Velox-Download-Manager/releases/latest/download/latest.json`.
2. **Remind** — when a newer version exists: a toast (with an *Install* action),
   an `↑` badge on the Settings nav item, and a dedicated
   **Settings → Updates** section showing the new version and release notes.
3. **Install** — `downloadAndInstall()` downloads the installer, verifies its
   minisign signature against the public key baked into the app, then runs the
   NSIS installer in *passive* mode. The app restarts afterwards
   (`tauri-plugin-process`).

## Trust model
- Artifacts are signed at build time with the release private key
  (`.secrets/velox-updater.key`, gitignored; regenerate = cannot update
  existing installs).
- The public key is embedded in `tauri.conf.json`; unsigned or tampered
  installers are refused by the plugin.
- Downloads come from the project's GitHub releases over HTTPS.

## Release checklist (per version)
1. Bump `version` in `app/src-tauri/tauri.conf.json` + `Cargo.toml` +
   `scripts/installer-online-template.nsi` (`PRODUCT_VERSION`).
2. `scripts/build-velox-msvc.sh` (signing env exported automatically).
3. `scripts/package-update-release.sh <staging-dir>` — copies the signed
   installer + `.sig`, generates `latest.json`, writes `SHA256SUMS.txt`.
4. Write `INSTALL-AND-VERIFY.md` in the staging dir, then
   `scripts/github-release-refresh.sh <staging-dir> v<version>` (uploads the
   installer, portable exe, sums, docs, `.sig` and `latest.json`).
5. Verify: `curl -sL .../releases/latest/download/latest.json` — version and
   non-empty signature; the URL must point at the release asset.
