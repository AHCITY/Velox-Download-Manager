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
