# Velox browser extension — verification and fixes (2026-11-23)

Question asked: the extension is not being installed on Chrome or any other
browser, it is not working and not catching downloads — check it, fix it, and
confirm it is complete.

Verdict: the extension **is complete** (manifest, service worker, popup, icons,
context menus, interception logic, native host, installer wiring all exist),
but the chain between browser and app was broken in **four independent
places**. Each one alone was enough to make the extension appear dead. All
four are fixed and the whole chain is now verified end-to-end in a real
browser.

## Root causes (all fixed)

1. **The pinned extension ID did not match the shipped key.** The manifest's
   `key` field derives the ID Chrome actually assigns:
   `ffnpjmbhfmaamhlbohbmamnmoaikpdmc` (SHA-256 over the public key, first 16
   bytes, hex nibbles mapped to a–p). The code, installer and docs pinned
   `hpaxoarvxcmmqijm`. Chrome compares the native host manifest's
   `allowed_origins` against the real ID, so every `connectNative()` was
   refused as a forbidden origin — the extension could never talk to the app.
   The key was never rotated (one version in the whole history), so the pin
   was simply wrong from the start.
   Fix: the ID is now derived from the shipped key in
   `engine-core/src/extension.rs` and re-derived in tests in both engine-core
   and velox-host — a future key change fails the tests instead of silently
   breaking installs. The app, host and installer all use the canonical value;
   `velox-browser-host --register` now takes the ID as an *optional* argument
   and defaults to the canonical one, so a stale literal can no longer be
   registered.

2. **The manifest was missing the `nativeMessaging` permission.** Without it
   `chrome.runtime.connectNative` does not even exist in the service worker
   (observed live: `TypeError: chrome.runtime.connectNative is not a
   function`), and the old code masked that as "native host not registered".
   Fix: `"nativeMessaging"` added to `manifest.json` permissions; the error
   message now says what is actually missing.

3. **"Quick start" relies on a flag Google removed.** `--load-extension` was
   removed from branded Chrome builds in Chrome 137 (June 2025,
   developer.chrome.com). On a current Chrome the button silently did
   nothing — a big part of "the extension is not being installed". Fix: the
   Browser screen and the setup notes now say plainly that Quick start is
   ignored on Chrome 137+ and that the guided flow (Developer mode →
   Load unpacked → pick the staged folder) is the reliable path. Edge, Brave,
   Vivaldi, Opera and Chromium still honor the flag.

4. **Registration only covered Chrome, Edge and Brave.** Opera (Windows key
   `Opera Software`; on Linux/macOS Opera reads the Google Chrome path),
   Chromium (`~/.config/chromium/...`, `Software\Chromium`) and Vivaldi
   (`Software\Vivaldi`, `~/.config/vivaldi/...`) were never registered, and
   macOS was not handled at all. Fix: velox-host registers and unregisters
   all six browsers (plus Chrome/Brave on macOS), `extension_status` probes
   the same set, browser detection gained Vivaldi (Windows) and
   Chromium/Vivaldi (Linux), and the installer's uninstaller cleans all six
   registry keys.

## Verified in a real browser (Chrome for Testing 143 under Xvfb)

Staged extension + registered host + a fake loopback bridge standing in for
the desktop app (same bridge.json layout, bearer token, whitelist), driven
over CDP:

```
OK: Velox extension target running at chrome-extension://ffnpjmbhfmaamhlbohbmamnmoaikpdmc/background.js (type service_worker)
extension runtime id: ffnpjmbhfmaamhlbohbmamnmoaikpdmc (matches the derived ID)
hello reply: {"id":"t1","payload":{"app_version":"0.4.0-test",...,"connected":true,...},"type":"hello_ack","version":1}
opened download tab: http://127.0.0.1:9344/big.bin
status reply: {"id":"t2","payload":{"active":1,"agg_bps":8192,...,"recent":[...]},"type":"status_response","version":1}
```

The fake bridge's request log shows **a real browser download being caught**:
starting a 2 MB download in the browser fired `chrome.downloads.onCreated`,
the service worker (auto mode) submitted it, and the host forwarded

```
{"auth_ok": true, "body": {"method": "download.create", "params": {"auto_start": true, "url": "http://127.0.0.1:9344/big.bin"}}}
```

followed by `engine.info` + `download.list` (the status round-trip the popup
uses). Every request carried the correct bearer token. Before the
`nativeMessaging` fix the same setup failed with
`connectNative is not a function`; before the ID fix the host would have been
refused as a forbidden origin.

## Also verified outside the browser

- `velox-browser-host --register` (built from the fixed sources) prints the
  canonical ID and writes correct manifests (`path` absolute, exactly one
  `allowed_origins` entry).
- New Rust tests: ID derivation from the shipped manifest (both crates),
  registration-list coverage, host-manifest JSON shape.
- Full gates on the patched tree: fmt, clippy (warnings as errors), workspace
  tests — **209 passed, 0 failed, 1 ignored** (was 203 before the extension
  work: +6 new tests) — plus the JS gate and the 33-check UI E2E, all green.

## Changed files

`extension/manifest.json`, `extension/background.js`, `extension/README.md`
(rewritten), `crates/engine-core/src/extension.rs` (new) + `lib.rs`,
`crates/velox-host/src/main.rs` + `Cargo.toml`, `app/src-tauri/src/main.rs`,
`web/assets/screens/browser.js`, `scripts/installer-unified-template.nsi`,
plus the test tooling `scripts/ext-test-fake-bridge.py` and
`scripts/ext-test-sw-eval.mjs`.

## Limits — what this session could not verify

- The Tauri setup flow (`extension_setup`) itself cannot run here (no GUI
  toolchain), but its ID now comes from the same tested constant, and the
  pieces it drives (staging, register, launch) were exercised individually.
- The Windows registry registration path follows Chromium's documented layout
  but was not executed on Windows (no Windows machine in this sandbox).
- Firefox remains unsupported by design (honest refusal in the UI; the
  manifest is Chromium MV3 only).
- The reference repository (github.com/qamarabbas408/velox-download-manager)
  exists but could not be mirrored from this sandbox (GitHub API rate limit;
  no `extension/manifest.json` at the top of `master`/`main`). The
  implementation here was therefore validated against Chromium's official
  documentation and a live browser instead.

## What a user must do to get a working extension

- **Best:** build/install the next release from this tree — the fixes must be
  shipped in `Velox.exe` + `velox-browser-host.exe` + the staged extension
  (the currently published v0.4.0 binaries still contain the old ID and the
  old manifest).
- **Workaround for an existing v0.4.0 install:** load the extension unpacked
  (its ID is `ffnpjmbhfmaamhlbohbmamnmoaikpdmc`), then run the *installed*
  host with the correct ID — `velox-browser-host.exe --register
  ffnpjmbhfmaamhlbohbmamnmoaikpdmc` — and reload the extension. On Chrome
  137+ use the guided steps (Developer mode → Load unpacked); the Quick start
  flag will not work there.

## References actually used

- developer.chrome.com/docs/extensions/develop/concepts/native-messaging
  (nativeMessaging permission, allowed_origins, manifest locations)
- developer.chrome.com blog "What's happening in Chrome Extensions, June
  2025" (--load-extension removal in Chrome 137)
- Opera forum (Opera reads Google Chrome's NM path on Linux/macOS); Vivaldi
  forum (`~/.config/vivaldi/NativeMessagingHosts`)
