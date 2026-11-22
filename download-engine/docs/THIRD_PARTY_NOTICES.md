# Third-Party Notices & Provenance

## qamarabbas408/velox-download-manager (MIT)

Reference repository studied during the browser-ecosystem turn (2026-11-22):
https://github.com/qamarabbas408/velox-download-manager

**Ideas adapted** (with design changes to fit our architecture — no source files
were copied):

| Idea | Origin (their files) | Our adaptation |
|---|---|---|
| Versioned NM envelope protocol (4-byte LE framing, request ids, hello/ready handshake) | `crates/velox-protocol/src/lib.rs` | `crates/velox-host/src/main.rs` — kinds renamed to our conventions (`hello_ack` vs `ready`), 4 MiB cap (theirs: 64 MiB), payload = our JSON-RPC methods |
| Host-manifest registration for Chrome/Edge/Brave incl. Windows HKCU registry | `src-tauri/src/native_host.rs` | `velox-browser-host --register` subcommand (they register from the app at startup) |
| Pinned-key extension identity for a stable unpacked dev ID | their manifest `key` + allowlist comment | our `extension/manifest.json` key + `scripts/velox-extension-key.pem` |
| App-owned local IPC endpoint; extension never stores download state | `src-tauri/src/browser_host.rs` (unix socket / named pipe, NM passthrough) | our loopback HTTP JSON-RPC bridge with bearer token (reuses the EXISTING engine contract instead of a second protocol in the app) |
| Chunked page-context fetch w/ acks (browser-assisted downloads) | `extension/src/pageAssistFetch.ts` + `assist_*` protocol | designed only — see docs/FEATURE_COMPARISON.md §4; not implemented |

Their LICENSE (MIT) requires notice retention for "substantial portions" of
source; since we copied no source, this file records the intellectual provenance
of the adapted ideas anyway.

## Everything else
All engine, UI, extension and host code in this repository is original to the
AHCITY/Velox-Download-Manager project.
