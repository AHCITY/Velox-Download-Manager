# Velox Browser Extension

Official companion extension for the Velox desktop download manager
(Chromium browsers — Manifest V3). It sends URLs to the Velox **desktop app**
through a native-messaging host; the desktop app remains the single source of
truth for downloads.

## What it does
- **Download with Velox** context-menu items (link / page / selected URL).
- Popup: connection status, URL input + Add, aggregate speed, active count,
  recent downloads, interception preference, reconnect.
- Optional interception of browser downloads (Ask = default; Auto; Off).
- Optional clipboard suggestion (explicit opt-in — never reads silently).
- Badge: active download count, `!` when disconnected.

## The extension ID
The `key` field in `manifest.json` pins the ID Chrome computes for this
extension:

    ffnpjmbhfmaamhlbohbmamnmoaikpdmc

It is stable across reinstalls and derived from the shipped key (see
`engine-core/src/extension.rs`, which tests the derivation against this file).
An earlier build pinned `hpaxoarvxcmmqijm`, which did NOT match the shipped
key — Chrome then refused every native-messaging connection ("Access to the
native messaging host is forbidden") and the extension could never talk to the
app. Never hand-edit the ID; it always follows the key.

## Install
The desktop app automates this: **Browser screen → Install extension**
(stages the extension, registers the native host, opens the browser). What it
cannot automate — Chromium's security model — is the last step in the browser:

1. Enable **Developer mode** on the extensions page
   (`chrome://extensions`, `edge://extensions`, `brave://extensions`,
   `vivaldi://extensions`, `opera://extensions`).
2. Click **Load unpacked** and pick the staged folder the app shows
   (Windows: `%LOCALAPPDATA%\Velox\extension`;
   Linux: `~/.local/share/io.velox.downloadmanager/extension`).

**Quick start** (per-browser button) additionally launches the browser with
`--load-extension` for an immediate session load. Google removed that flag
from **branded Chrome in version 137** — on Chrome the guided steps above are
the reliable path; Edge, Brave, Vivaldi, Opera and Chromium still honor it.

Firefox is not supported yet: it forbids programmatic extension installs and
its MV3 event pages differ from Chromium service workers.

## Native messaging host
`velox-browser-host` (installed beside `Velox.exe`) speaks Chrome's NM framing
and forwards a whitelist of methods to the app's loopback bridge. Registration:

    velox-browser-host --register

Without an argument it registers the canonical ID above (a stale literal can
no longer be registered by accident). It writes the host manifest into the
per-user config dirs of every supported browser:

- Windows registry (HKCU): `Google\Chrome`, `Microsoft\Edge`,
  `BraveSoftware\Brave-Browser`, `Opera Software`, `Chromium`, `Vivaldi`
  → `NativeMessagingHosts\io.velox.downloadmanager` pointing at
  `%LOCALAPPDATA%\Velox\native-host.json`.
- Linux: `NativeMessagingHosts/io.velox.downloadmanager.json` inside
  `~/.config/{google-chrome, microsoft-edge, BraveSoftware/Brave-Browser,
  chromium, vivaldi}`. Opera reads the Google Chrome path, so it is covered.
- macOS: Chrome and Brave under `~/Library/Application Support/...`.

The registration file allowlists exactly one origin:
`chrome-extension://ffnpjmbhfmaamhlbohbmamnmoaikpdmc/` — no other page or
extension can launch the host.

## Start Velox, then reload the extension
The popup shows *connected · v<app version>*; the badge turns into the active
count. The desktop app publishes the loopback bridge at startup (random port +
32-byte token in a `0600 bridge.json` under its app-data dir); the host reads
it and forwards the whitelisted method set (`engine.info`, `download.list`,
`download.status`, `download.create`, `download.pause`, `download.cancel`).
No credentials, no broad host permissions, no remote commands.

## Files
- `manifest.json` — MV3, minimum Chrome 102, permissions: contextMenus,
  notifications, alarms, storage, downloads; optional: clipboardRead.
- `background.js` — service worker: NM port + capped-backoff reconnect,
  request-id protocol, context menus, interception, popup data.
- `popup.html/css/js` — compact companion UI (no build step required).
- `icons/` — Velox brand icon set.

## Build
None — plain JS, load the folder as-is. (The reference project builds with
vite/TS; this implementation is deliberately build-free to keep the extension
auditable and lightweight.)
