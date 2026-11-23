//! Path presentation helpers — pure string logic, no OS-specific APIs.
//!
//! `std::fs::canonicalize` is the right way to VALIDATE a path (it resolves
//! symlinks/junctions and doubles as an existence check), but on Windows it
//! returns a *verbatim* path: `\\?\C:\…`, or `\\?\UNC\server\share\…`. Verbatim
//! paths are understood by the Win32 file APIs only — the shell (`explorer.exe`,
//! `ShellExecuteW`) does not accept them, so a canonicalized path must never be
//! handed to the OS "open" command as-is. `shell_path` strips the prefix for
//! launching/presentation; validation still happens on the canonical form.

use std::path::{Path, PathBuf};

/// The form of `path` that the OS shell / "open with the default app" command
/// understands. Non-verbatim paths are returned unchanged, so this is a no-op
/// on every platform except Windows.
pub fn shell_path(path: &Path) -> PathBuf {
    let s = path.to_string_lossy();
    if let Some(rest) = s.strip_prefix(r"\\?\UNC\") {
        return PathBuf::from(format!(r"\\{rest}"));
    }
    if let Some(rest) = s.strip_prefix(r"\\?\") {
        return PathBuf::from(rest);
    }
    path.to_path_buf()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn verbatim_drive_path_becomes_a_plain_dos_path() {
        assert_eq!(
            shell_path(Path::new(r"\\?\C:\Users\me\Downloads\Velox\file.zip")),
            PathBuf::from(r"C:\Users\me\Downloads\Velox\file.zip")
        );
    }

    #[test]
    fn verbatim_unc_path_keeps_the_unc_form() {
        assert_eq!(
            shell_path(Path::new(r"\\?\UNC\server\share\dir")),
            PathBuf::from(r"\\server\share\dir")
        );
    }

    #[test]
    fn plain_paths_are_returned_unchanged() {
        for p in [
            r"C:\Users\me\Downloads",
            r"\\server\share\dir",
            "/home/me/my files/a.zip",
            "relative/dir/file",
        ] {
            assert_eq!(shell_path(Path::new(p)), PathBuf::from(p));
        }
    }

    #[test]
    fn only_a_leading_verbatim_prefix_is_stripped() {
        // a verbatim marker inside the path is an ordinary name, not a prefix
        assert_eq!(
            shell_path(Path::new(r"C:\dir\\?\odd-name")),
            PathBuf::from(r"C:\dir\\?\odd-name")
        );
    }
}
