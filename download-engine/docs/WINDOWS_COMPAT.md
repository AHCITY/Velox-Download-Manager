# Windows compatibility — audit and plan (2026-11-19, session 2)

**Status: audited, NOT validated.** No Windows runner exists in this environment; nothing below
may be called "tested on Windows" until the CI job in `.github/workflows/windows.yml` actually
runs green. Linux behavior is unaffected by every item marked "safe" below.

## 1. What already compiles for Windows today (code audit)

| Area | Code | Windows behavior | Verdict |
|---|---|---|---|
| Positional writes | `engine-storage write_at` | `seek_write_all` loop (cursor under `Mutex<File>`) | safe |
| Other platforms | same function | `compile_error!` — deliberate | safe |
| Part-file permissions | `PartFile::create` | unix `.mode(0o600)`; Windows: no ACL call, relies on profile-directory inheritance | acceptable, document |
| Directory fsync | `engine-storage::sync_dir` | **no-op on non-Unix** | gap: rename durability unverified on Windows; file *data* still covered by `sync_all` |
| Rename | finalize / overwrite | std `fs::rename` = `MoveFileExW(REPLACE_EXISTING)` → `FileRenameInfoEx` POSIX semantics (Win10 1607+); pre-emptive `remove_file` is Windows-only belt-and-braces for open-handle targets | safe (verified from std source) |
| Cross-process lock | `DownloadLock` | pid-file + steal-by-rename; **`pid_alive` returns true for all foreign pids on non-Unix** → a stale lock is never stolen → the download is unstartable after SIGKILL until the `.lock` file is deleted by hand | **gap (High for Windows)** |
| Error classification | `error.rs from_io` | errno table is Unix-only without `#[cfg]`: Windows os-error 13 = `ERROR_INVALID_DATA`, disk-full is 39, access-denied is 5 → ENOSPC/EACCES map to generic `Io` (retry then fail) instead of the `DiskFull`/`Permission` → paused-with-reason path | **gap (Medium)** |
| Long paths | `naming.rs` | 240-byte filename cap; no `\\?\` prefixing — a long `dest_dir` can still exceed 260 | gap (Low, opt-in long paths exist) |
| Reserved names | `naming.rs` | CON/PRN/AUX/NUL/COM1-9/LPT1-9 (+extensions), trailing dots/spaces handled | safe |
| SQLite | engine-state | rusqlite bundled sqlite works on Windows; WAL + FULL fine | safe |
| Cookie jar | engine-http | 0600 chmod is unix-gated; on Windows the jar inherits profile ACLs | acceptable, document |
| Process termination | crash tests | tokio `Child::kill()` = TerminateProcess (no graceful chance — same crash-consistency class as SIGKILL) | plan: re-enable the crash suite on Windows CI |

## 2. Plan (ordered)

1. **CI first** (no claims before it runs): `.github/workflows/windows.yml` — `cargo test
   --workspace` + `cargo test -p engine-cli --test crash_resume` on `windows-latest`.
2. **Lock recovery on Windows** (High): replace the pid-file heuristic with
   `LockFileEx(LOCKFILE_EXCLUSIVE_LOCK | LOCKFILE_FAIL_IMMEDIATELY)` on the lock file itself —
   locks die with the process, eliminating the stale-lock problem entirely; keep the pid file only
   for diagnostics. Linux keeps the current mechanism.
3. **errno table** (Medium): `#[cfg(windows)]` map 39→DiskFull, 5/13→Permission/Access in `from_io`.
4. **Directory durability** (Medium): open the target directory with
   `FILE_FLAG_BACKUP_SEMANTICS | FILE_FLAG_WRITE_THROUGH` and `FlushFileBuffers` after rename
   (community practice — verify empirically in CI; flagged INFERENCE in RESEARCH §6.3).
5. **Long paths** (Low): `\\?\` verbatim prefixing behind a helper, or document the
   `longPathAware` manifest requirement.
6. Re-run the full integration + crash suites on CI before any Windows-support claim.

## 3. Honest summary

- Linux: fully validated (63+ tests, out-of-process SIGKILL recovery).
- Windows: compiles-clean expectation based on `#[cfg]` audit; **unvalidated**; two known gaps
  (stale-lock recovery, errno classification) with concrete plans above.


## Update (this milestone)

- The full CLI now compiles for Windows: all three Ctrl-C sites use the portable
  `tokio::signal::ctrl_c()` (console handler) instead of `tokio::signal::unix`.
  Graceful pause-on-Ctrl-C behavior is identical on both platforms.
- Verification status: the whole workspace must be validated by the upgraded
  `windows.yml` CI (fmt, clippy, build, full test suite, cfg(windows) lock
  crash-recovery test, CLI + API v2 smoke with Host-guard check). A local
  cross-compile of engine-storage passes for `x86_64-pc-windows-msvc`; a FULL
  workspace cross-check is blocked locally by `ring` (needs clang-cl) — CI is
  authoritative and has NOT yet been observed green.
- Lock files are now unlinked on release after inode-identity verification in
  `acquire()` (Unix fstat/stat, Windows file-index), eliminating the two-holder
  race that made unlink-on-drop unsafe; no more `.part-N.lock` residue next to
  finished downloads.
- Windows launcher equivalent ships: `scripts/velox-preview.ps1`.

## Status reconciliation (2026-11-22, autonomous cycle 6)

Evidence-based update against the plan above; every claim cites the shipped code.

| Plan item | Status | Evidence |
|---|---|---|
| 1. CI first | **DONE** — windows.yml runs fmt/clippy/build/full suite + crash gate; GREEN since `80dfe9e` | `.github/workflows/windows.yml`, GitHub runs |
| 2. Lock recovery via LockFileEx | **DONE** — cross-process OS locks replaced the pid heuristic entirely; stale locks die with the process | `engine-storage/src/lib.rs:279-283` (design comment), `:437-444` (`LockFileEx(LOCKFILE_EXCLUSIVE_LOCK | LOCKFILE_FAIL_IMMEDIATELY)` with non-null OVERLAPPED), inode/file-index-verified unlink on release |
| 3. errno table | **DONE** — Windows 112/39 → DiskFull, 13 → Permission; paused-with-reason instead of generic Io | `engine-core/src/error.rs:185-188` |
| 4. Directory durability | **DONE** — best-effort `FlushFileBuffers` on the dir handle after rename (GENERIC_WRITE handle; failure logged, not fatal) | `engine-storage/src/lib.rs:159-177` |
| 5. Long paths | **OPEN** — no `\\?\` verbatim prefixing yet; 240-byte filename cap bounds the name but not `dest_dir` | `engine-core/src/naming.rs:61` |
| 6. Crash suite on Windows CI | **DONE** for the SIGKILL crash-resume gate (4 scenarios incl. lock recovery); full suite = the standard windows job | `crates/engine-cli/tests/crash_resume.rs`, CI logs |

Honest remaining gaps: long-path `dest_dir` only (plan 5); no real-Windows manual
installation test has ever been executed (INSTALLER-TEST-RESULTS.md says the same —
the installer's WebView2 bootstrap path is compile-validated, not runtime-tested).
