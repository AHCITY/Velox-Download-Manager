# NEXT-MILESTONE-RESEARCH — Windows readiness, cookies, API v2, desktop UI

Companion documents: `docs/API_V2.md` (protocol spec), `docs/WINDOWS_COMPAT.md` (platform notes),
`NEXT-MILESTONE-REPORT.md` (what shipped and what was verified).

## 1. Desktop framework comparison (for the future packaged Velox app)

Sources opened and cited: tauri.app official docs (webview versions, Linux graphics quirks),
independent benchmark write-ups (gethopp.app, blog.openreplay.com), community benchmarks.

| Criterion | Tauri 2 (Rust) | Electron | wry/tao direct | Neutralino | Native (WinUI/GTK) |
| --- | --- | --- | --- | --- | --- |
| Installer/bundle | ~2.5–10 MB (system webview reused) | 80–150 MB (bundles Chromium + Node) | ~5 MB (no IPC layer, all hand-rolled) | ~2 MB (system webview, thin API) | smallest |
| RAM | ~40–100 MB typical | 150–300 MB typical | lowest | low | lowest |
| Rust engine integration | **first-class** (Rust backend in-process; our crate = a Tauri command module) | via Node native addons or sidecar processes (our case: sidecar `velox serve-http` + HTTP calls — works, extra process) | full control, most glue code | sidecar HTTP like Electron | FFI side |
| UI stack | HTML/CSS/JS — **the existing `web/` UI carries over unchanged** | same | same | same | platform-specific rewrite |
| Windows story | WebView2 (preinstalled on Win 11, auto-installable on Win 7+; officially documented) | bundled Chromium = identical everywhere | WebView2 by hand | WebView2 by hand | WinUI only |
| Linux story | WebKitGTK: documented rendering quirks/fragmentation (v2.tauri.app/develop/debug/linux-graphics) | Chromium = consistent | WebKitGTK | WebKitGTK | GTK rewrite |
| Packaging/updates | tauri-bundler + built-in updater (signed, per-platform) | electron-builder + squirrel | manual | small tooling | per-platform |
| Risk | webview rendering variance (Linux); IPC security surface to review | bundle weight; Node security surface; RAM | everything is manual | smaller ecosystem | two UIs to maintain |

**Recommendation: Tauri 2**, with the engine kept as an independent crate set. The decisive
points, in order: (1) the Rust engine integrates in-process rather than as a sidecar; (2) the
existing browser UI (`web/index.html`) is reused as-is — the API v2 transport in this milestone
was designed so the Tauri window points at the same `POST /rpc` + `GET /events` contract
(embedded via a local port), making the packaged app a *thin shell* around code that already
works; (3) bundle size and RAM are within a download manager's dignity (~3 MB vs ~85 MB).

Interim shipped UI: a real static `web/index.html` served by `velox serve-http` (engine-bridge).
This is deliberately the same UI Tauri would host — no throwaway work.

Engine independence rule (kept): the frontend talks JSON-RPC/SSE over HTTP. It contains zero
scheduling/segmentation/retry/storage logic; if the engine dies, it renders a disconnected
state and retries.

## 2. Cookie jar: Netscape compatibility

- Existing jar: `cookie_store` 0.22 (RFC 6265 storage model, reqwest integration) — domain,
  path, expiry and secure/HttpOnly semantics are already correct there; the milestone work is
  *format* compatibility, not a new matching engine.
- Netscape format facts (de-facto standard shared by curl `-b cookies.txt`, wget, yt-dlp,
  browser extensions): 7 tab-separated fields (domain, include-subdomains, path, secure,
  expires unix-seconds with 0 = session, name, value); `#HttpOnly_` line prefix marks HttpOnly;
  `#` otherwise starts a comment; values may contain tabs (parse value as everything after the
  6th tab).
- Import maps onto `Cookie::try_from_raw_cookie` + explicit `CookieDomain::HostOnly/Suffix`
  override, preserving the file's declared subdomain semantics; expired entries are dropped at
  import; session cookies (0) map to `SessionEnd`.
- Export re-emits that exact format; HttpOnly is re-encoded via the `#HttpOnly_` prefix.
- Files: 0600 on Unix (atomic temp+rename, same pattern as the JSON jar); values never appear
  in logs or error strings (the only error path is a static "cookie store poisoned" message).
- `DXDL_COOKIE_JAR` (JSON, existing behavior) is untouched; Netscape files are imported INTO
  the jar via `velox cookies import`, so there is one live format and no dual-jar drift.

## 3. API v2 event transport

Full design + protocol examples: `docs/API_V2.md`. Summary of the research trade-off:
JSON-RPC notifications on stdio (v1-style) cannot serve a desktop webview (no multi-client,
no reconnect); WebSocket needs a hand-rolled resume protocol; gRPC is disproportionate. SSE +
`POST /rpc` gives platform-level reconnect (`Last-Event-ID`) with ~200 lines of dependency-free
tokio code and *reuses v1's `handle_line`*, eliminating semantic drift.

## 4. Windows filesystem behavior (primary sources: Microsoft Learn + Rust std docs)

- `std::fs::rename` on Windows fails if the destination exists (Linux replaces it): the engine
  already removes an existing target on the explicit-overwrite path, and finalize() never
  renames onto an existing name — verified during this milestone.
- Advisory locks: POSIX `flock` releases on process death; the Windows equivalent is
  `LockFileEx` with `LOCKFILE_EXCLUSIVE_LOCK | LOCKFILE_FAIL_IMMEDIATELY` over the whole range —
  also released by the kernel when the process dies. This made pid-heuristic stale-lock
  recovery unnecessary on BOTH platforms (the old `pid_alive()` was a no-op stub on Windows and
  could never recover a crashed holder's lock there).
- `FlushFileBuffers` on a directory handle (opened with `FILE_FLAG_BACKUP_SEMANTICS`) is the
  best-effort Windows equivalent of directory fsync; failure is non-fatal and documented — the
  manifest row remains the crash-recovery source of truth.
- `File::seek_write_all` (Windows `FileExt`) is **still unstable** on current Rust toolchains —
  the cross-compile check caught that the pre-existing code could not build for Windows at all;
  replaced with stable `seek` + `write_all` under the existing `Mutex<File>` (whose stated
  purpose is exactly to serialize cursor use).
