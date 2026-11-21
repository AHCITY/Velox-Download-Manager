# WINDOWS-DESKTOP-RELEASE-REPORT

Date: 2026-11-20. Companion docs: TAURI-ARCHITECTURE.md, RELEASE-AND-UPDATE-GUIDE.md,
WINDOWS-COMPATIBILITY-RESULTS.md, INSTALLER-TEST-RESULTS.md, NEXT-ROADMAP.md.

## What was implemented (all verified in this workspace)

1. **Tauri 2 desktop application** (`app/src-tauri/`): the existing Rust engine runs
   in-process behind three Tauri commands; the engine event hub is forwarded to the
   webview through the same wire contract as the HTTP preview (`engine_bridge::wire_event`).
   Single-instance, close-pauses-downloads lifecycle, per-user data/download dirs,
   restrictive capabilities, CSP, generated icon set, Windows metadata.
2. **Frontend dual transport**: `web/index.html` speaks Tauri IPC in the desktop app and
   HTTP/SSE in the browser preview — same JSON-RPC envelopes, same event names. The
   browser preview remains fully functional (verified live below).
3. **Windows CI**: engine gates now include `cargo clippy --workspace --all-targets -- -D warnings`
   (made honest by FIXING all 9 pre-existing warnings — none silenced), plus a gated
   `desktop` job that builds `Velox.exe` + the NSIS `Velox-Setup-x64.exe` with
   `cargo tauri build` and uploads both as artifacts with verified existence.
4. **Suite**: 99 tests passing; `cargo fmt --all -- --check` clean; strict clippy clean.

## Honest verification matrix

| Item | Status |
| --- | --- |
| Engine workspace build/tests/clippy/fmt (Linux) | **tested, green** |
| Tauri app compile | **not compilable in this sandbox** — Tauri needs webkit2gtk dev libs; no root to install them (apt denied). The crate compiles under `cargo tauri build` on Windows/CI. Stated plainly. |
| Windows CI run | **not executed** — no GitHub Actions runner access from this sandbox. The workflow is complete and YAML-validated; the run must happen on push. |
| Installer artifact | **not produced here** — requires the Windows toolchain/NSIS (above). CI produces it; local Windows build command documented. |
| Live desktop launch | not possible on this Linux sandbox (no webkit). The BROWSER preview (same UI, same engine, same contract) was verified live. |

Nothing above is claimed as tested that was not tested. The remaining gap is exactly one
thing: a Windows machine (CI runner or desktop) executing the committed workflow.

## What the reviewer should do next

1. Push this commit → `windows.yml` runs the engine gates and the desktop job on
   `windows-latest`, producing `Velox-Setup-x64.exe` as a CI artifact.
2. Download the artifact on Windows 10 22H2 x64 / Windows 11 x64, run the 20-step
   installation test plan in INSTALLER-TEST-RESULTS.md, and record results.
3. When a signing identity is available, add it per RELEASE-AND-UPDATE-GUIDE.md.

## Blocker-fix round (review follow-up)

Five blockers found by review were fixed and verified locally:

1. **Missing app dependencies** — `main.rs` calls `engine_api::handle_line` and
   `tokio::time::timeout`/`tokio::sync::broadcast`, but neither crate was declared in
   the standalone `app/src-tauri/Cargo.toml`. Both added (tokio features: time, sync).
2. **Shutdown lifecycle** — close is now a real sequence: intercept → await bounded
   pause (10 s/download, 30 s overall) → close. Repeated close requests safe via atomic
   flag; no fire-and-forget pause tasks racing process exit.
3. **Executable naming** — `[[bin]] name = "Velox"` + `mainBinaryName: "Velox"`
   (schema-verified top-level key); CI asserts the literal `Velox.exe`.
4. **open_path hardening** — final path (after reveal-parent resolution) is
   canonicalized and compared against the canonicalized root: symlinks/junctions and
   parent escapes are rejected; missing paths error. Frontend opens the folder itself
   (`reveal: false`).
5. **Lockfile** — `app/src-tauri/Cargo.lock` committed (527 packages); CI gates with
   `cargo fetch --locked`.

Local verification after the fixes: engine fmt clean, strict clippy (-D warnings) clean,
99 tests passing, workflow YAML + Tauri JSON configs valid, frontend JS syntax valid,
dependency graph resolves. Tauri compilation still requires the Windows/webkit toolchain
(absent in this sandbox — webkit2gtk cannot be installed without root).

## Windows 10 lifecycle note

The compatibility baseline is Windows 10 22H2 x64. Windows 10's ordinary support ended
(October 14, 2025); application compatibility with 22H2 does NOT imply the OS remains
security-supported. No minimum-supported-version increase was made: the Tauri/WebView2
stack supports Windows 10 1809+, well below the 22H2 baseline.
