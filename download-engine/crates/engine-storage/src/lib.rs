//! Storage layer: part files, positional writes, preallocation, durable finalization.
//!
//! Design contract (docs/RECOVERY_DESIGN.md, ADR-0007):
//! - all writes are positional and loop-safe (`write_all_at` semantics);
//! - `sync_data` before any state commit; `sync_all` → rename → directory fsync at finalization
//!   (Windows: best-effort FlushFileBuffers on the directory — see sync_dir);
//! - a `Mutex<File>` wrapper keeps Windows `seek_write` cursor semantics safe.

use std::fs::{File, OpenOptions};
use std::io;
use std::path::{Path, PathBuf};
use std::sync::Mutex;

/// A partially-downloaded file written at arbitrary offsets.
pub struct PartFile {
    file: Mutex<File>,
    path: PathBuf,
}

impl PartFile {
    /// Create (or reopen) a part file. Existing content is preserved (resume).
    /// Unix: created 0600 atomically (audit A13 — partial content is private from birth;
    /// the old create-then-chmod left a 0644 window and ignored chmod failures).
    pub fn create(path: PathBuf) -> io::Result<Self> {
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent)?;
        }
        #[cfg(unix)]
        {
            use std::os::unix::fs::OpenOptionsExt;
            let file = OpenOptions::new()
                .create(true)
                .read(true)
                .write(true)
                .truncate(false)
                .mode(0o600)
                .open(&path)?;
            restrict_permissions(&file);
            Ok(Self {
                file: Mutex::new(file),
                path,
            })
        }
        #[cfg(not(unix))]
        {
            let file = OpenOptions::new()
                .create(true)
                .read(true)
                .write(true)
                .truncate(false)
                .open(&path)?;
            restrict_permissions(&file);
            Ok(Self {
                file: Mutex::new(file),
                path,
            })
        }
    }

    pub fn path(&self) -> &Path {
        &self.path
    }

    /// Preallocate `size` bytes (sparse via set_len/ftruncate). Unallocated regions read as zeros,
    /// which is safe because every byte range is written before completion is claimed.
    pub fn preallocate(&self, size: u64) -> io::Result<()> {
        let f = self.file.lock().unwrap();
        f.set_len(size)
    }

    pub fn truncate_to(&self, size: u64) -> io::Result<()> {
        let f = self.file.lock().unwrap();
        f.set_len(size)
    }

    pub fn is_empty(&self) -> io::Result<bool> {
        Ok(self.len()? == 0)
    }

    pub fn len(&self) -> io::Result<u64> {
        let f = self.file.lock().unwrap();
        Ok(f.metadata()?.len())
    }

    /// Positional write; loops on short writes (write(2) may transfer fewer bytes).
    pub fn write_at(&self, buf: &[u8], offset: u64) -> io::Result<()> {
        let f = self.file.lock().unwrap();
        #[cfg(unix)]
        {
            use std::os::unix::fs::FileExt;
            f.write_all_at(buf, offset)
        }
        #[cfg(windows)]
        {
            // stable primitives only: seek_write_all (FileExt) is still unstable on
            // current toolchains. seek+write is safe here — the Mutex<File> serializes
            // cursor use (this wrapper exists for exactly that reason).
            use std::io::{Seek, Write};
            let mut f = f;
            f.seek(std::io::SeekFrom::Start(offset))?;
            f.write_all(buf)
        }
        #[cfg(not(any(unix, windows)))]
        {
            let _ = (&f, buf, offset);
            compile_error!("unsupported platform for positional writes");
        }
    }

    /// fdatasync-equivalent: flush file *data* to stable storage.
    pub fn sync_data(&self) -> io::Result<()> {
        let f = self.file.lock().unwrap();
        f.sync_data()
    }

    /// fsync-equivalent: flush data *and* metadata (used before final rename).
    pub fn sync_all(&self) -> io::Result<()> {
        let f = self.file.lock().unwrap();
        f.sync_all()
    }
}

#[cfg(unix)]
fn restrict_permissions(f: &File) {
    use std::os::unix::fs::PermissionsExt;
    let _ = f.set_permissions(std::fs::Permissions::from_mode(0o600));
}
#[cfg(not(unix))]
fn restrict_permissions(_f: &File) {}

/// fsync the directory containing `path` so the rename/creation entry is durable.
/// - Unix: fsync(2) on an O_RDONLY directory fd.
/// - Windows: open the directory with FILE_FLAG_BACKUP_SEMANTICS and call
///   FlushFileBuffers. NTFS honors it for directory metadata; on failure (FAT,
///   exotic filter drivers) it is BEST-EFFORT and reported as Ok — documented
///   limitation (RECOVERY_DESIGN §7): Windows renames are not guaranteed durable
///   against power loss, only against process crash.
pub fn sync_dir(path: &Path) -> io::Result<()> {
    let dir = if path.is_dir() {
        path.to_path_buf()
    } else {
        path.parent().map(|p| p.to_path_buf()).unwrap_or_default()
    };
    #[cfg(unix)]
    {
        File::open(&dir)?.sync_all()
    }
    #[cfg(windows)]
    {
        flush_dir_windows(&dir)
    }
    #[cfg(not(any(unix, windows)))]
    {
        let _ = dir;
        Ok(())
    }
}

#[cfg(windows)]
fn flush_dir_windows(dir: &Path) -> io::Result<()> {
    use std::os::windows::fs::OpenOptionsExt;
    use std::os::windows::io::AsRawHandle;
    use windows_sys::Win32::Storage::FileSystem::FlushFileBuffers;
    const FILE_FLAG_BACKUP_SEMANTICS: u32 = 0x0200_0000;
    // FlushFileBuffers requires a handle with GENERIC_WRITE (MSDN) — read-only dirs
    // always failed with ACCESS_DENIED and were silently swallowed before (review fix)
    let f = OpenOptions::new()
        .read(true)
        .write(true)
        .custom_flags(FILE_FLAG_BACKUP_SEMANTICS)
        .open(dir);
    match f {
        Ok(f) => {
            // best-effort: a failure means the platform cannot flush directory
            // metadata; download correctness does not depend on it — the manifest
            // row is the source of truth after restart
            let ok = unsafe { FlushFileBuffers(f.as_raw_handle() as _) };
            if ok == 0 {
                tracing::warn!(
                    path = %dir.display(),
                    error = %io::Error::last_os_error(),
                    "directory metadata flush failed (non-fatal)"
                );
            }
            Ok(())
        }
        Err(e) => {
            tracing::warn!(path = %dir.display(), error = %e, "cannot open directory for flush (non-fatal)");
            Ok(())
        }
    }
}

/// Compute a non-colliding path in `dir`: `name`, then `name(1).ext`, `name(2).ext`, ...
pub fn unique_path(dir: &Path, name: &str) -> PathBuf {
    let candidate = dir.join(name);
    if !candidate.exists() {
        return candidate;
    }
    let stem_ext = split_name(name);
    for i in 1..10_000u32 {
        let candidate = dir.join(format!("{}({}).{}", stem_ext.0, i, stem_ext.1));
        if !candidate.exists() {
            return candidate;
        }
    }
    dir.join(format!("{}.{}.part-dup", stem_ext.0, std::process::id()))
}

fn split_name(name: &str) -> (String, String) {
    match name.rsplit_once('.') {
        // keep dotfiles like ".bashrc" whole: stem must be non-empty
        Some((stem, ext)) if !stem.is_empty() => (stem.to_string(), ext.to_string()),
        _ => (name.to_string(), "bin".to_string()),
    }
}

/// Atomically move a completed part file to its final location and fsync the directory.
/// If the target exists at check time, a unique sibling name is chosen. NOTE: between the
/// existence check and the rename, another process could create the target and the rename
/// (replace-on-both-platforms) would silently overwrite it — a bounded TOCTOU window,
/// documented and accepted (the only caller maps rename errors to a re-download path; full
/// elimination needs renameat2(RENAME_NOREPLACE), which is Linux-only).
pub fn finalize(temp: &Path, desired_final: &Path) -> io::Result<PathBuf> {
    let target = if desired_final.exists() {
        let dir = desired_final.parent().unwrap_or_else(|| Path::new("."));
        unique_path(
            dir,
            &desired_final
                .file_name()
                .unwrap_or_default()
                .to_string_lossy(),
        )
    } else {
        desired_final.to_path_buf()
    };
    std::fs::rename(temp, &target)?;
    sync_dir(&target)?;
    Ok(target)
}

#[derive(Debug)]
pub enum LockError {
    Held(u32),
    Io(io::Error),
}

impl std::fmt::Display for LockError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            // pid 0 = unknown: on Windows the holder's exclusively-locked lockfile
            // cannot be read by a second handle (whole-file LockFileEx region), so
            // the recorded pid is unrecoverable there — say so instead of "pid 0".
            LockError::Held(0) => write!(
                f,
                "download is in use by another process (pid unavailable on this platform)"
            ),
            LockError::Held(p) => write!(f, "download is in use by another process (pid {p})"),
            LockError::Io(e) => write!(f, "io: {e}"),
        }
    }
}

impl std::error::Error for LockError {}

impl From<io::Error> for LockError {
    fn from(e: io::Error) -> Self {
        LockError::Io(e)
    }
}

// ---------------------------------------------------------------------------
// Cross-process download lock: prevents two engine processes from writing the
// same part file concurrently (a real corruption vector, found in the P5 audit).
//
// Ownership model (milestone "Windows readiness"): the holder of the OS-level
// advisory byte-range lock owns the lockfile.
//   - Unix:  flock(fd, LOCK_EX | LOCK_NB)
//   - Windows: LockFileEx(LOCKFILE_EXCLUSIVE_LOCK | LOCKFILE_FAIL_IMMEDIATELY)
//     over the whole 64-bit range.
// The kernel releases the lock when the owning process dies — SIGKILL, crash,
// power loss — so a stale lockfile is claimed by taking the now-free OS lock,
// without any pid heuristics (the previous pid_alive() probe was a no-op stub
// on Windows and could never recover a crashed process's lock there).
// The pid recorded in the file is diagnostics for the `Held(pid)` error only.
// ---------------------------------------------------------------------------

#[derive(Debug)]
pub struct DownloadLock {
    path: PathBuf,
    /// open handle whose OS lock is held for this DownloadLock's lifetime
    /// (releasing the handle releases the lock); `None` only on platforms
    /// without a known advisory-lock primitive, where the text-pid protocol is
    /// used conservatively (never stolen).
    os_lock: Option<std::fs::File>,
}

impl DownloadLock {
    /// Try to acquire the lock for a part file. The lockfile lives in `locks_dir`
    /// (audit D8, 2026-11-21: the user's destination directory must stay clean —
    /// "keep temporary files and lock files internal", mission §5). The basename
    /// keeps per-download uniqueness (`<name>.part-<id>.lock`; ids are global).
    /// Returns `Held(pid)` while another live process holds the OS lock; a lockfile
    /// left by a dead process (crash, SIGKILL) is claimed automatically — the OS
    /// lock is the only arbiter, file presence means nothing.
    pub fn acquire(part_path: &Path, locks_dir: &Path) -> Result<Self, LockError> {
        let lock_path = locks_dir.join(format!(
            "{}.lock",
            part_path
                .file_name()
                .map(|n| n.to_string_lossy().into_owned())
                .unwrap_or_else(|| "part".into())
        ));
        if let Some(parent) = lock_path.parent() {
            std::fs::create_dir_all(parent)?;
        }
        // create-or-open is deliberately not exclusive: the OS lock is the only
        // arbiter, so there is no O_EXCL/create-rename race to defend against.
        let f = OpenOptions::new()
            .read(true)
            .write(true)
            .create(true)
            .truncate(false)
            .open(&lock_path)?;
        match try_os_lock(&f) {
            Ok(()) => {
                // we own the file (and the path — nobody else can claim it while we
                // hold the OS lock): record the pid for diagnostics
                use std::io::Write;
                let mut f = f;
                f.set_len(0)?;
                // cursor is 0 (fresh handle, no reads): no seek needed — and `Seek`
                // for `&File` is unstable on some recent toolchains. One write_all so
                // a concurrent reader never sees a partial pid (diagnostics only).
                f.write_all(format!("{}\n", std::process::id()).as_bytes())?;
                Ok(DownloadLock {
                    path: lock_path,
                    os_lock: Some(f),
                })
            }
            Err(_) => {
                // someone else holds the OS lock: they are alive (a dead holder's
                // lock is released by the kernel)
                Err(LockError::Held(recorded_pid(&lock_path)))
            }
        }
    }

    pub fn lock_path(&self) -> &Path {
        &self.path
    }
}

impl Drop for DownloadLock {
    fn drop(&mut self) {
        // Drop the OS lock handle (releases the kernel lock) and DO NOT unlink:
        // unlinking here re-introduces a two-holder race — a racer that opened the
        // path before our unlink can flock the now-orphaned inode while a third
        // process creates and locks a fresh file at the same path. Ownership is the
        // OS lock alone, so a leftover lockfile is harmless: acquire() claims any
        // file whose OS lock is free (crash-recovery path), and this file's lock is
        // free the moment our handle drops.
        self.os_lock.take();
    }
}

/// Remove lock files older than `max_age` whose OS lock is free. Age-gating makes
/// this race-free: any active acquirer created its lockfile milliseconds before
/// flocking, so an old-mtime file with a free lock has no in-flight racer (the
/// classic flock+unlink two-holder race cannot start on a file nobody just made).
/// Stale-but-recent files are left in place: acquire() claims them safely anyway.
pub fn gc_stale_locks(locks_dir: &Path, max_age: std::time::Duration) -> usize {
    let mut removed = 0;
    let Ok(entries) = std::fs::read_dir(locks_dir) else {
        return 0;
    };
    let now = std::time::SystemTime::now();
    for e in entries.flatten() {
        let p = e.path();
        if p.extension().and_then(|x| x.to_str()) != Some("lock") {
            continue;
        }
        let Ok(meta) = e.metadata() else { continue };
        if !meta.is_file() {
            continue;
        }
        let age_ok = meta
            .modified()
            .ok()
            .and_then(|m| now.duration_since(m).ok())
            .is_some_and(|age| age >= max_age);
        if !age_ok {
            continue;
        }
        // free-lock check WITHOUT creating a residue: flock the file read-only; if the
        // kernel grants it, no live process holds it and the file is safe to delete
        if let Ok(f) = OpenOptions::new().read(true).open(&p) {
            if try_os_lock(&f).is_ok() && std::fs::remove_file(&p).is_ok() {
                removed += 1;
            }
        }
    }
    removed
}

/// Best-effort pid from the lockfile text (diagnostics only; 0 if unreadable).
fn recorded_pid(lock_path: &Path) -> u32 {
    std::fs::read_to_string(lock_path)
        .ok()
        .and_then(|s| s.trim().parse::<u32>().ok())
        .unwrap_or(0)
}

#[cfg(unix)]
fn try_os_lock(f: &std::fs::File) -> io::Result<()> {
    use std::os::unix::io::AsRawFd;
    let ret = unsafe { libc::flock(f.as_raw_fd(), libc::LOCK_EX | libc::LOCK_NB) };
    if ret != 0 {
        let err = io::Error::last_os_error();
        // EWOULDBLOCK == EAGAIN on Linux
        if err.raw_os_error() == Some(libc::EWOULDBLOCK) {
            return Err(io::Error::new(
                io::ErrorKind::WouldBlock,
                "locked by another process",
            ));
        }
        return Err(err);
    }
    Ok(())
}

#[cfg(windows)]
fn try_os_lock(f: &std::fs::File) -> io::Result<()> {
    use std::os::windows::io::AsRawHandle;
    use windows_sys::Win32::Foundation::ERROR_LOCK_VIOLATION;
    use windows_sys::Win32::Storage::FileSystem::{
        LockFileEx, LOCKFILE_EXCLUSIVE_LOCK, LOCKFILE_FAIL_IMMEDIATELY,
    };
    use windows_sys::Win32::System::IO::OVERLAPPED;
    // LockFileEx REQUIRES a non-null OVERLAPPED (the byte-range start offset is read
    // from it): zeroed = lock from offset 0; (MAX, MAX) length covers the whole range
    let mut overlapped: OVERLAPPED = unsafe { std::mem::zeroed() };
    let ok = unsafe {
        LockFileEx(
            f.as_raw_handle() as _,
            LOCKFILE_EXCLUSIVE_LOCK | LOCKFILE_FAIL_IMMEDIATELY,
            0,
            u32::MAX,
            u32::MAX,
            &mut overlapped,
        )
    };
    if ok == 0 {
        let err = io::Error::last_os_error();
        if err.raw_os_error() == Some(ERROR_LOCK_VIOLATION as i32) {
            return Err(io::Error::new(
                io::ErrorKind::WouldBlock,
                "locked by another process",
            ));
        }
        return Err(err);
    }
    Ok(())
}

#[cfg(not(any(unix, windows)))]
fn try_os_lock(_f: &std::fs::File) -> io::Result<()> {
    Err(io::Error::other(
        "no advisory lock primitive on this platform",
    ))
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::time::Duration;

    #[test]
    fn positional_writes_and_size() {
        let dir = tempfile::tempdir().unwrap();
        let p = dir.path().join("x.part");
        let f = PartFile::create(p.clone()).unwrap();
        f.write_at(b"world", 6).unwrap();
        f.write_at(b"hello ", 0).unwrap();
        assert_eq!(f.len().unwrap(), 11);
        f.preallocate(100).unwrap();
        assert_eq!(f.len().unwrap(), 100);
        f.truncate_to(11).unwrap();
        assert_eq!(std::fs::read(&p).unwrap(), b"hello world");
    }

    #[test]
    fn finalize_renames_and_avoids_collision() {
        let dir = tempfile::tempdir().unwrap();
        let temp = dir.path().join("t.part");
        std::fs::write(&temp, b"abc").unwrap();
        let final1 = finalize(&temp, &dir.path().join("f.bin")).unwrap();
        assert_eq!(final1, dir.path().join("f.bin"));
        std::fs::write(&temp, b"def").unwrap();
        let final2 = finalize(&temp, &dir.path().join("f.bin")).unwrap();
        assert_eq!(final2, dir.path().join("f(1).bin"));
        assert!(final1.exists() && final2.exists());
    }

    #[test]
    fn download_lock_exclusive_and_stale_recovery() {
        let dir = tempfile::tempdir().unwrap();
        let part = dir.path().join("x.part-1");
        let locks_dir = dir.path().join("locks");
        let lock = DownloadLock::acquire(&part, &locks_dir).expect("first acquire");
        assert!(lock.lock_path().exists());
        // a second acquire while ours is alive must fail (same process: flock is
        // per open-file-description, so a second fd conflicts)
        match DownloadLock::acquire(&part, &locks_dir) {
            Err(LockError::Held(p)) => {
                // Windows: the holder's pid is NOT readable while its exclusive OS
                // lock covers the whole file — recorded_pid returns 0 (unknown).
                #[cfg(unix)]
                assert_eq!(p, std::process::id());
                #[cfg(windows)]
                assert_eq!(p, 0);
            }
            other => panic!("expected Held, got {other:?}"),
        }
        // a stale lockfile (holder dead: OS lock free but file remains) is CLAIMED
        std::fs::create_dir_all(&locks_dir).unwrap();
        let stale = locks_dir.join("y.part-1.lock");
        std::fs::write(&stale, "999999999\n").unwrap();
        let lock2 = DownloadLock::acquire(&dir.path().join("y.part-1"), &dir.path().join("locks"))
            .expect("stale lock claimed");
        drop(lock2);
        let lock_path = lock.lock_path().to_path_buf();
        drop(lock);
        // the lockfile PERSISTS by design (ownership is the OS lock): what matters is
        // that the LOCK is released
        let _ = lock_path;
        // after release, reacquire works
        let lock3 = DownloadLock::acquire(&part, &locks_dir).expect("reacquire after drop");
        drop(lock3);
    }

    /// Cross-process crash recovery: a child process takes the lock and is SIGKILLed;
    /// the parent must be able to claim the lock immediately (no pid heuristics).
    #[cfg(unix)]
    #[test]
    fn lock_recovered_after_child_process_death() {
        let dir = tempfile::tempdir().unwrap();
        let part = dir.path().join("crash.part");
        let locks_dir = dir.path().join("locks");
        let lock_path = locks_dir.join("crash.part.lock");
        let exe = std::env::current_exe().unwrap();
        let mut child = std::process::Command::new(exe)
            .args([
                "tests::zz_lock_holder_child",
                "--exact",
                "--ignored",
                "--test-threads=1",
            ])
            .env("VELOX_LOCK_HOLDER_PATH", &lock_path)
            .spawn()
            .unwrap();
        // wait for the child to take the lock
        for _ in 0..200 {
            if lock_path.exists() {
                break;
            }
            std::thread::sleep(Duration::from_millis(10));
        }
        assert!(lock_path.exists(), "child never took the lock");
        // second acquire by a THIRD party must see it held (child alive)
        match DownloadLock::acquire(&part, &locks_dir) {
            Err(LockError::Held(_)) => {}
            other => panic!("expected Held while child alive, got {other:?}"),
        }
        // SIGKILL the child: the kernel releases its flock; the lock must now be claimable
        unsafe { libc::kill(child.id() as i32, libc::SIGKILL) };
        let _ = child.wait();
        let reclaimed =
            DownloadLock::acquire(&part, &locks_dir).expect("lock claimed after child death");
        drop(reclaimed);
        let _ = lock_path; // lockfile persists by design; OS lock is what matters
    }

    /// Same scenario for Windows CI (kill via taskkill). Runs only on windows runners.
    #[cfg(windows)]
    #[test]
    fn lock_recovered_after_child_process_death() {
        let dir = tempfile::tempdir().unwrap();
        let part = dir.path().join("crash.part");
        let locks_dir = dir.path().join("locks");
        let lock_path = locks_dir.join("crash.part.lock");
        let exe = std::env::current_exe().unwrap();
        let mut child = std::process::Command::new(exe)
            .args([
                // Same filter as the unix variant: without it the child libtest
                // runner would execute the whole suite and zz_lock_holder_child
                // (#[ignore]) would never run — "child never took the lock".
                "tests::zz_lock_holder_child",
                "--exact",
                "--ignored",
                "--test-threads=1",
            ])
            .env("VELOX_LOCK_HOLDER_PATH", &lock_path)
            .spawn()
            .unwrap();
        for _ in 0..200 {
            if lock_path.exists() {
                break;
            }
            std::thread::sleep(Duration::from_millis(10));
        }
        assert!(lock_path.exists(), "child never took the lock");
        match DownloadLock::acquire(&part, &locks_dir) {
            Err(LockError::Held(_)) => {}
            other => panic!("expected Held while child alive, got {other:?}"),
        }
        let _ = std::process::Command::new("taskkill")
            .args(["/F", "/PID", &child.id().to_string()])
            .status();
        let _ = child.wait();
        let reclaimed =
            DownloadLock::acquire(&part, &locks_dir).expect("lock claimed after child death");
        drop(reclaimed);
    }

    /// The holder subprogram used by lock_recovered_after_child_process_death:
    /// take the lock and park until killed. Not a test — filtered out by name.
    #[test]
    #[ignore = "lock-holder subprogram (spawned by lock_recovered_after_child_process_death)"]
    fn zz_lock_holder_child() {
        if let Ok(p) = std::env::var("VELOX_LOCK_HOLDER_PATH") {
            let path = std::path::PathBuf::from(p);
            if let Some(parent) = path.parent() {
                std::fs::create_dir_all(parent).unwrap();
            }
            let f = OpenOptions::new()
                .read(true)
                .write(true)
                .create(true)
                .truncate(false)
                .open(&path)
                .unwrap();
            try_os_lock(&f).expect("child could not take the lock");
            // park; parent SIGKILLs us
            std::thread::sleep(Duration::from_secs(120));
        }
    }

    #[test]
    fn gc_removes_only_stale_free_locks() {
        let dir = tempfile::tempdir().unwrap();
        let locks_dir = dir.path().join("locks");
        std::fs::create_dir_all(&locks_dir).unwrap();
        // old + free → removed
        let old_free = locks_dir.join("old.free.lock");
        std::fs::write(&old_free, "0\n").unwrap();
        let old = dir.path().join("old.free.lock");
        let _ = std::fs::remove_file(&old);
        // backdate 8 days via libc-free trick: rewrite mtime with filetime? Not a dep.
        // Use the documented interface instead: gc with max_age 0 removes ANY free lock.
        let removed = gc_stale_locks(&locks_dir, std::time::Duration::from_secs(0));
        assert_eq!(removed, 1, "free lock must be collected with max_age=0");
        assert!(!old_free.exists());
        // held → kept even with max_age=0
        let part = dir.path().join("held.part");
        let lock = DownloadLock::acquire(&part, &locks_dir).unwrap();
        let removed = gc_stale_locks(&locks_dir, std::time::Duration::from_secs(0));
        assert_eq!(removed, 0, "a live lock must never be collected");
        assert!(lock.lock_path().exists());
        drop(lock);
    }

    #[test]
    fn sync_dir_works_on_files_and_dirs() {
        let dir = tempfile::tempdir().unwrap();
        assert!(sync_dir(dir.path()).is_ok());
        std::fs::write(dir.path().join("a"), b"1").unwrap();
        assert!(sync_dir(&dir.path().join("a")).is_ok());
    }
}

/// Safety margin required on the destination volume before starting and during a
/// transfer (audit M5, 2026-11-21): 64 MiB — enough for checkpoints and the final
/// rename without claiming the volume must hold twice the file size.
pub const SPACE_MARGIN: u64 = 64 * 1024 * 1024;

/// Free bytes on the volume containing `path`. Errors mean the caller cannot know
/// (treat the destination as unavailable, not as full).
#[cfg(unix)]
pub fn available_space(path: &Path) -> io::Result<u64> {
    use std::ffi::CString;
    let c = CString::new(path.as_os_str().to_string_lossy().as_bytes())
        .map_err(|_| io::Error::new(io::ErrorKind::InvalidInput, "path contains NUL"))?;
    // SAFETY: c is a valid NUL-terminated path; statvfs only reads it.
    let mut fs: libc::statvfs = unsafe { std::mem::zeroed() };
    let rc = unsafe { libc::statvfs(c.as_ptr(), &mut fs) };
    if rc != 0 {
        return Err(io::Error::last_os_error());
    }
    Ok(fs.f_bavail as u64 * fs.f_frsize as u64)
}

#[cfg(windows)]
pub fn available_space(path: &Path) -> io::Result<u64> {
    use std::os::windows::ffi::OsStrExt;
    use windows_sys::Win32::Storage::FileSystem::GetDiskFreeSpaceExW;
    let wide: Vec<u16> = path.as_os_str().encode_wide().chain([0]).collect();
    let mut free: u64 = 0;
    // SAFETY: wide is NUL-terminated UTF-16; free is a valid out-pointer.
    let rc = unsafe {
        GetDiskFreeSpaceExW(
            wide.as_ptr(),
            &mut free,
            std::ptr::null_mut(),
            std::ptr::null_mut(),
        )
    };
    if rc == 0 {
        return Err(io::Error::last_os_error());
    }
    Ok(free)
}

/// True when free space on `path`'s volume is below `margin`. Unqueryable volumes
/// return false — the preflight already made unavailability an explicit failure.
pub fn low_space(path: &Path, margin: u64) -> bool {
    available_space(path).is_ok_and(|free| free < margin)
}

#[cfg(test)]
mod disk_space_tests {
    use super::*;

    #[test]
    fn available_space_reports_the_tmp_volume() {
        let dir = tempfile::tempdir().unwrap();
        let free = available_space(dir.path()).expect("statvfs on a real dir");
        assert!(free > 0, "a writable tempdir must have free space");
        assert!(!low_space(dir.path(), 1024), "more than 1 KiB free");
        assert!(
            low_space(dir.path(), u64::MAX),
            "margin larger than the volume → low"
        );
    }
}
