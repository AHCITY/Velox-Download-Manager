# WINDOWS-READINESS-REPORT

Date: 2026-11-20. Companion: WINDOWS-TEST-RESULTS.md (commands + raw results).

## Verdict

**Windows support is implemented and locally cross-checked, but NOT yet proven on a real
Windows runner.** No Windows-support claim may be treated as final until the upgraded
`.github/workflows/windows.yml` runs green on `windows-latest`. That CI run has NOT been
executed from this environment (no GitHub runner access) — stated plainly, per the
no-unverified-claims rule.

## What makes Windows support real now

1. **The complete CLI compiles for Windows.** Audit finding A confirmed: all three
   `tokio::signal::unix::signal(SignalKind::interrupt())` sites (monitor, serve, serve-http)
   blocked `engine-cli` compilation for Windows. Replaced with the portable
   `tokio::signal::ctrl_c()` (console Ctrl-C handler on Windows). The graceful
   pause-on-Ctrl-C behavior in `monitor_until_done` is preserved exactly and is now
   platform-identical.
2. **Local cross-compile evidence**: `cargo check -p engine-storage --target
   x86_64-pc-windows-msvc` passes. A FULL-workspace Windows cross-check is impossible in
   this sandbox: `ring` (rustls' C code) requires clang-cl, which is not installed —
   documented, not papered over. CI is the authoritative check.
3. **The CI workflow now covers the required gates** (upgraded this milestone):
   `cargo fmt --all -- --check`, `cargo clippy --workspace --all-targets`,
   `cargo build --workspace --all-targets`, `cargo test --workspace --no-fail-fast`
   (which includes the `cfg(windows)` SIGKILL-style lock-recovery test using `taskkill /F`),
   a CLI startup/shutdown smoke (`version` + one v1 request over stdio), and an API v2
   smoke that starts `serve-http` on Windows, makes an RPC round-trip, asserts the strict
   Host guard refuses a foreign Host, and shuts the process down cleanly.
4. **Locking**: OS-level advisory lock (LockFileEx with a real, zeroed OVERLAPPED — the
   windows-sys signature requires non-null; caught by review) over the whole 64-bit range;
   kernel-released on process death; cross-process crash-recovery test ships in both the
   Unix (SIGKILL) and Windows (`taskkill`) variants.
5. **Shutdown/lifecycle**: serve-http selects on `tokio::signal::ctrl_c()`; a Windows
   launcher equivalent ships (`scripts/velox-preview.ps1`) that starts/stops the whole
   preview stack and detects port conflicts.

## Windows-specific behavior investigated

- **Rename/replacement**: `std::fs::rename` replaces existing destinations on both
  platforms (MoveFileExW + MOVEFILE_REPLACE_EXISTING). The previous `cfg(windows)`
  pre-emptive `remove_file` on the overwrite path was destructive and wrong — removed.
- **Error classification**: io::Error mapped to typed `ErrorKind`s remains platform-neutral;
  disk-full and permission errors surface as FAILED rows with the OS message — unchanged.
- **Long paths**: no special handling; destinations inherit the user's config. Windows
  long-path caveats are listed in NEXT-ROADMAP.md as a CI-follow-up (test with a >260-char
  destination once CI is green).
- **Data dir permissions**: Windows uses inherited ACLs (no 0600 concept); the Unix-only
  0600 calls are cfg-gated and never touch Windows. Cookie jar permissions are documented
  per-platform in docs/SECURITY.md.
- **AV/file-lock interactions**: the download lock is byte-range (not share-mode), so real
 -time AV scans that open files read-only do not block it; documented as an area CI smoke
  cannot fully exercise.

## What still blocks a "Windows-ready" claim

- One green CI run on windows-latest (all gates above). Everything is in place; the run
  just needs a GitHub push/PR.
- windows-gnu toolchain: not validated (mingw absent locally; msvc is the CI standard).
