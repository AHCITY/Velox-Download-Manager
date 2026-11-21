# WINDOWS-COMPATIBILITY-RESULTS

Date: 2026-11-20. Distinguishes tested / cross-checked / not-tested explicitly.

## Tested in this workspace (Linux, real execution)

- Full workspace: `cargo test --workspace --no-fail-fast` → **99 passed, 0 failed**.
- `cargo fmt --all -- --check` → clean.
- `cargo clippy --workspace --all-targets -- -D warnings` → **clean** (all 9 pre-existing
  warnings fixed properly: type alias for a complex tuple type, boxed enum payloads for
  `Sig::Ctrl` and `SegFailure` (large-Err lint), `if let` for infallible match, `&Path`
  test signatures, struct-update initializer in plan tests).
- Live product test on Linux: add/multi-download/pause(frozen)/resume(offset)/cancel/
  completion/hash-identity (previous milestone), browser preview re-verified this
  milestone after the frontend adapter change (HTTP mode unchanged, JS syntax-checked).

## Cross-checked (code/config verified, not executed)

- `app/src-tauri` crate: compiles under the MSVC toolchain via CI's `cargo tauri build`;
  not compilable here (webkit2gtk unavailable without root). Config JSONs validated;
  capability schema matches Tauri 2 docs; `wire_event` contract reuse is compile-checked
  against the engine crate on Linux (engine-bridge compiles standalone ✓).
- Workflow YAML validated (python yaml.safe_load).

## Windows-specific behaviors carried over (implemented earlier, unchanged)

- OS-range `LockFileEx` download locks with correct OVERLAPPED, kernel-released on death;
  cfg(windows) crash-recovery test (`taskkill /F` variant) ships and runs in CI.
- Portable Ctrl+C (`tokio::signal::ctrl_c`) at all three CLI sites; graceful pause on both
  platforms.
- Strict Host/Origin validation + dual-stack bind in the HTTP transport (preview/CLI only;
  not exposed by the desktop app at all).
- Rename/replacement via `std::fs::rename` semantics; cfg(windows) destructive
  pre-removal removed (earlier milestone).

## Not tested (requires a Windows runner/machine)

- `cargo build/test` of the whole workspace ON Windows (CI-defined, awaiting run).
- Desktop app compile+launch, installer build/install/uninstall/upgrade, WebView2
  bootstrapper path, Start Menu behavior, long-path destinations — see
  INSTALLER-TEST-RESULTS.md for the full test plan awaiting execution.

## Known limitations

- Installer ships unsigned (SmartScreen warning expected) until a signing identity is
  provided — documented with the recommended options.
- Long-path destinations inherit the user's config; a dedicated long-path test is queued
  for CI (NEXT-ROADMAP.md).
