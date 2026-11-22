# Velox Browser Extension

Official companion extension for the Velox desktop download manager
(Chrome, Edge, Brave — Manifest V3). It sends URLs to the Velox **desktop
engine** through a native-messaging host; the desktop app remains the single
source of truth for downloads.

## What it does
- **Download with Velox** context-menu items (link / page / selected URL).
- Popup: connection status, URL input + Add, aggregate speed, active count,
  recent downloads, interception preference, reconnect.
- Optional interception of browser downloads (Ask = default; Auto; Off).
- Optional clipboard suggestion (explicit opt-in — never reads silently).
- Badge: active download count, `!` when disconnected.

## Install (3 steps)
1. **Load the extension**
   - Open `chrome://extensions` (or `edge://extensions`, `brave://extensions`).
   - Enable *Developer mode* → *Load unpacked* → select this `extension/` folder.
   - The extension ID is pinned by the `key` in `manifest.json`:
     `hpaxoarvxcmmqijm` (stable across reinstalls).
2. **Register the native host** (needs the desktop app installed)
   - Windows: `velox-browser-host.exe --register hpaxoarvxcmmqijm`
     (writes `HKCU\Software\{Chrome,Edge,Brave}\NativeMessagingHosts\io.velox.downloadmanager`
     pointing at `%LOCALAPPDATA%\Velox\native-host.json`).
   - Linux/macOS: `velox-browser-host --register hpaxoarvxcmmqijm`
     (writes `NativeMessagingHosts/io.velox.downloadmanager.json` into the
     Chrome/Edge/Brave config dirs).
   - The registration file allowlists exactly one origin:
     `chrome-extension://hpaxoarvxcmmqijm/` — no other page or extension can
     launch the host.
3. **Start Velox, then reload the extension.** The popup shows
   *connected · v<app version>*; the badge turns into the active count.

The desktop app publishes the loopback bridge (random port + 32-byte token in a
`0600 bridge.json` under its app-data dir) at startup; the host reads it and
forwards a WHITELISTED method set (`engine.info`, `download.list`,
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
