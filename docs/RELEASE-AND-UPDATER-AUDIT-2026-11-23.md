# Auto-updater audit + v0.5.0 release preparation (2026-11-23)

Questions asked: (1) confirm the extension is verified, working and complete —
that it catches downloads swiftly and sends them to the desktop application;
(2) thoroughly check the auto-updater before touching the GitHub setup files;
(3) update all setup files on GitHub so they can be previewed after download.

## 1. Extension — verified end-to-end, this time against the REAL engine

Live chain in a real browser (Chrome for Testing 143 under Xvfb), driven over
CDP, with the actual `velox serve-http` engine behind the same bridge contract
the desktop app publishes:

```
OK: Velox extension target running at chrome-extension://ffnpjmbhfmaamhlbohbmamnmoaikpdmc/background.js
extension runtime id: ffnpjmbhfmaamhlbohbmamnmoaikpdmc (matches the derived ID)
hello reply: hello_ack connected:true | round-trip 7 ms
capture latency: 37 ms (download started -> download.create at the bridge)
status reply: recent=[{name: big.bin, state: completed, total: 2097152}]
```

- A 2 MB browser download was intercepted by the extension and arrived at the
  app bridge **37 ms** after the download started (auto mode), bearer-token
  verified.
- The bridge handed it to the **real Velox engine** (`download.create` →
  `download.start`): the engine row reached `state:"completed"`, 2,097,152 /
  2,097,152 bytes, and the file physically landed at
  `~/.local/share/velox/out/big.bin`.
- Context-menu/popup/status paths (the `download.list`/`engine.info`
  round-trips) all returned live engine data.

The extension is complete and working: manifest (MV3 + `nativeMessaging`),
service worker, popup, context menus, interception, native-messaging host,
installer registration — nothing left in the chain.

(Testing note: Chrome for Testing resolves native-messaging manifests from the
`<user-data-dir>/NativeMessagingHosts` directory; end-user Chrome/Edge/Brave/
Vivaldi use the fixed per-browser dirs that `velox-browser-host --register`
already writes. No product code change was needed for this.)

## 2. Auto-updater — the suspicion was right, but the bugs are in the PIPELINE

The in-app updater code itself is sound and stayed sound: strict semver guard
(never offers equal/older releases), one announcement per release, consent +
pause-active-downloads flow, no force-close, honest `unavailable` state in
browser preview, updater plugin registered, `updater:default` +
`process:default` capabilities present, version comparison unit-tested.

What made updates impossible in practice:

1. **`windows.yml` never produced updater artifacts.** The desktop job was
   dispatch-only, built without the signing key (so `createUpdaterArtifacts`
   silently produced no signature), generated no `latest.json`, and uploaded
   only workflow artifacts — never a GitHub release.
2. **No `latest.json` anywhere** → the app's endpoint
   (`releases/latest/download/latest.json`) 404s → every update check errors.
3. **The signing private key existed only in gitignored `.secrets/`** and was
   wiped in a sandbox recycle → the pubkey pinned in 0.4.0
   (`35F9AED0D3C28C15`) can never sign again. 0.4.0 clients could never have
   received a verified update.
4. Version pins duplicated by hand (`0.4.0` in the NSIS template, app crate,
   tauri.conf) could drift.
5. The packaging script hardcoded `.secrets/` paths and unconditionally
   checksummed an optional file (crashed without it).

### Fixes (all applied and validated)

- **New updater keypair** generated with `tauri signer generate`; private key
  tracked at `scripts/velox-updater.key` (this repo's existing convention — the
  extension key is tracked the same way; this is a provenance key for unsigned
  dev builds, not a production secret), new pubkey pinned in
  `tauri.conf.json`. Consequence stated plainly: 0.4.0 installs pin the old
  key and cannot auto-update to 0.5.0 — they install the new setup manually.
- **`windows.yml` reworked**: desktop job now also runs on `v*` tags and signs
  the build (`TAURI_SIGNING_PRIVATE_KEY` read from the tracked key); the
  signature is uploaded as an artifact; a new **release job** (tag-only,
  `contents: write`) assembles and publishes the GitHub release with
  `Velox-Setup-x64.exe` + `.sig`, `latest.json` (tag-pinned asset URL),
  `Velox-Extension.zip` and `SHA256SUMS.txt`.
- **v0.5.0 everywhere**: tauri.conf.json, app crate + Cargo.lock, NSIS
  template (`PRODUCT_VERSION`, `VIProductVersion`, version keys).
- **Packaging script repaired**: key path now `scripts/` first with `.secrets/`
  fallback; checksums built from what exists.
- **Dry-run validated locally**: the full packaging path signed a real file
  with the new key — the signature's trusted comment carries
  `version:0.5.0` (exactly what tauri-plugin-updater 2.14+ cross-checks) — and
  produced a contract-correct `latest.json` (version, ISO pub_date,
  windows-x86_64 signature + URL).

## 3. GitHub setup files — one command away

The sandbox lost its GitHub token in a recycle, so it cannot push; everything
is committed locally on `main` (ahead of origin by a fast-forward) and
`scripts/publish-release.sh` ships it all:

```
GITHUB_TOKEN=ghp_... scripts/publish-release.sh
```

That pushes `main` + the `v0.5.0` tag; CI then builds, signs and **publishes
the release automatically** (installer, signature, latest.json, extension zip,
checksums). After it runs, the setup files live at
`https://github.com/AHCITY/Velox-Download-Manager/releases/tag/v0.5.0`.

## Limits

- Push/publish: not executable here (no token) — handed off with the script.
- The Tauri GUI cannot run in this sandbox, so the updater's install flow was
  verified at the contract level (signature/manifest/plugin wiring) and the
  extension's app flow against the real engine + a contract-faithful bridge.
- Windows registry registration paths follow Chromium's documented layout but
  were not executed on Windows hardware.
