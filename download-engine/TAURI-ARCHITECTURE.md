# TAURI-ARCHITECTURE

Implemented this milestone. Sources applied: Tauri 2 docs (windows-installer,
capabilities, calling-rust, architecture), Microsoft WebView2 evergreen guidance,
Microsoft code-signing options.

## Layout

```
app/src-tauri/            # standalone crate (NOT a member of the engine workspace,
  Cargo.toml              #   so the engine's test matrix stays GUI-independent)
  tauri.conf.json         # NSIS bundle, per-user install, WebView2 downloadBootstrapper
  capabilities/main.json  # "main-cap": core:default + event + window, window ["main"] only
  src/main.rs             # commands, event pump, lifecycle
  icons/                  # generated icon set (ICO + PNGs)
web/                      # the SAME frontend serves both transports
```

## Engine integration — in-process, one wire contract

- `DownloadManager` is created in `setup()` with `EngineConfig` pointed at the per-user
  app-data dir and `%USERPROFILE%\Downloads\Velox`. No HTTP server exists in the
  packaged app; `velox serve-http` remains for browser preview/CLI/remote use.
- **Commands** (registered via `generate_handler!`): `rpc` (routes the frontend's
  JSON-RPC envelope through the existing `engine_api::handle_line` — the exact same
  request/response contract as the HTTP API), `default_download_dir`, `open_path`
  (validated: must exist AND live inside the downloads root).
- **Events**: `engine_bridge::wire_event` (the SSE wire mapping) is reused verbatim —
  event names and payload shapes identical to the HTTP preview, so the frontend has
  ONE event contract on both transports. No invented progress; the pump forwards the
  engine's broadcast hub.
- **Frontend**: `web/index.html` detects `window.__TAURI__` (`withGlobalTauri: true`,
  no bundler needed) and switches transport per call: `invoke('rpc', …)` + named event
  listeners in the desktop app; `fetch('/rpc')` + `EventSource` in the browser. Browser
  preview behavior is unchanged.

## Lifecycle

- **Single instance**: `tauri-plugin-single-instance` — a second launch focuses the
  existing window instead of starting a competing engine (which would contend for the
  SQLite database and per-download OS locks).
- **Close policy** (implemented): the first CloseRequested is intercepted
  (`api.prevent_close()`); all running downloads are paused and the pause operations are
  AWAITED under a bounded timeout (10 s per download, 30 s overall); then the window
  closes. A repeated close request while shutdown is in flight is honored immediately
  (atomic flag — no second task, no deadlock). Engine checkpoints are durable on their
  cadence, so nothing is lost even if the bound is hit. No pause task outlives the exit.
- **Paths**: state in `app_data_dir()`, downloads in `download_dir()/Velox` — never in
  the installation directory (per Microsoft packaging best practices).

## Security posture

- Restrictive capability scoped to window `main`; no filesystem/shell plugins exposed;
  remote origins get nothing by default. CSP set in tauri.conf.json.
- `open_path` validates the FINAL path that will be opened (reveal resolves the parent
  first), then canonicalizes both sides — symlink/junction traversal cannot escape the
  downloads root, and a missing path fails the command instead of opening Explorer.
- Every RPC passes through the engine's existing validation (URL/destination checks,
  strict error mapping).
