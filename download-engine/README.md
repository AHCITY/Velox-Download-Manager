# Velox — next-generation download engine

**Engine first, UI last.** Velox is a cross-platform download engine written in Rust: segmented HTTP/HTTPS
downloading with dynamic work stealing, transactional crash-safe resume, strict integrity rules, and a
versioned JSON-RPC API. The CLI (`velox`) is merely its first client — no UI framework code exists in this
repository.

## Status (session 1, 2026-11-19)

Implemented and tested: single-connection + segmented downloading, range probing with fallbacks, redirect
handling, Content-Disposition naming, retry with backoff/Retry-After, pause/resume, crash recovery against a
SQLite manifest (WAL, synchronous=FULL), bandwidth limiting, SHA-256/BLAKE3 verification, fault-injection test
server, JSON-RPC v1 API, CLI. See docs/ROADMAP.md for what remains (mirrors, Metalink, network-interface
intelligence, extra protocols, UI).



## Desktop application (Tauri 2)

The Windows desktop app lives in `app/src-tauri/`: the engine runs IN-PROCESS behind
Tauri commands; the same `web/` frontend switches transport automatically (Tauri IPC in
the app, HTTP/SSE in the browser preview). Build on Windows with
`cargo tauri build` (see RELEASE-AND-UPDATE-GUIDE.md); CI produces `Velox-Setup-x64.exe`
on every push to main.

## Desktop preview (API v2 + web UI)

```bash
cargo build --release -p engine-cli -p engine-testserver --example throttled_httpd
./scripts/velox-preview.sh          # Windows: powershell -File scripts\velox-preview.ps1
# then open http://localhost:3000 — the demo download URL is shown in the UI
```

## API v2 (HTTP + SSE)

- `POST /rpc` — one JSON-RPC 2.0 request per body (same methods as v1; `X-Velox-Api: 2`)
- `GET /events` — Server-Sent Events: sequence ids, 512-event replay, `Last-Event-ID`
  reconnect, progress coalesced to ≤4 Hz, loopback-only (strict Host/Origin validation)
- See `docs/API_V2.md`.

## Cookies

- Persistent jar: `$DXDL_HOME/cookies.json` (RFC 6265 store, 0600); `DXDL_COOKIE_JAR=path|off`
- Netscape files: `velox cookies import cookies.txt` / `velox cookies export [file]`

## Quick start

```sh
cargo run -p engine-cli --release -- download https://example.com/bigfile.bin -o ~/Downloads
cargo run -p engine-cli -- list
cargo run -p engine-cli -- status <id>
cargo run -p engine-cli -- resume <id>
cargo run -p engine-cli -- verify <id>
cargo run -p engine-cli -- serve        # JSON-RPC v1 over stdio
cargo test  --workspace                 # unit + integration (local test server only, no external net)
```

Data dir: `$DXDL_HOME` → `$XDG_DATA_HOME/velox` → `~/.local/share/velox`.

## Layout

| Crate | Role |
|---|---|
| `engine-core` | orchestration, segment scheduler (tail stealing), retry, integrity, naming |
| `engine-http` | reqwest(+rustls/ring) transport: inspection, ranges, manual redirects |
| `engine-storage` | part files, positional writes, durable finalization |
| `engine-state` | SQLite manifest (downloads/segments/errors) |
| `engine-api` | JSON-RPC 2.0 over stdio (v1) |
| `engine-cli` | `velox` binary |
| `engine-testserver` | std-only fault-injecting HTTP server for tests/benchmarks |

## Engineering documents

docs/RESEARCH.md · ARCHITECTURE.md · PROTOCOL_COMPATIBILITY.md · RECOVERY_DESIGN.md ·
PERFORMANCE_PLAN.md · SECURITY.md · ROADMAP.md · DECISIONS/ (ADRs)

## Principles (binding)

Reliability over speed · measure, don't guess · never fake progress · never mark complete before
verification and finalization · never assume ranges/resume/multi-connection help · engine has no UI code ·
MIT licensed, clean-room from GPL projects (docs/RESEARCH.md §3).

License: MIT.
