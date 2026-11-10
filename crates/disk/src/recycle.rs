//! Moving items to the Recycle Bin (on macOS, the user's Trash). The only remove action this
//! crate has: there is no permanent delete, and nothing here empties the bin. The names stay
//! Windows-flavoured on every platform so callers do not change.

use crate::sys;
use std::io;
use std::path::{Path, PathBuf};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum RecycleError {
    NotFound,
    /// The drive has no Recycle Bin (USB or network), so the shell would delete for good.
    /// Nothing is attempted.
    NoRecycleBin,
    /// Open in another app.
    InUse,
    AccessDenied,
    /// Not an absolute path to a file or folder below a drive root.
    InvalidPath,
    /// Refused before the shell saw it: on the never-touch list, outside its cleaning rule,
    /// through a junction, or the profile, AppData, Windows or Program Files folder itself.
    Refused,
    /// The user said no at a shell prompt, for example when an item is too big to recycle.
    Aborted,
    /// Any other shell result code.
    Shell(i32),
}

/// What happened to each requested path. A partial move reports both lists.
#[derive(Debug, Default, PartialEq, Eq)]
pub struct RecycleReport {
    pub moved: Vec<PathBuf>,
    pub failed: Vec<(PathBuf, RecycleError)>,
}

/// Whether items on the drive holding `path` can go to the Recycle Bin. The UI offers
/// Move to Recycle Bin only when this is true.
pub fn has_recycle_bin(path: &Path) -> bool {
    sys::has_recycle_bin(path)
}

/// Bytes already in the Recycle Bin of the drive holding `path`.
pub fn recycle_bin_size(path: &Path) -> io::Result<u64> {
    sys::recycle_bin_size(path)
}

/// Moves each path to the Recycle Bin, one at a time so a file in use does not stop the
/// rest. Undo stays possible from the Recycle Bin.
pub fn move_to_recycle_bin(paths: &[PathBuf]) -> RecycleReport {
    let mut report = RecycleReport::default();
    for p in paths {
        match recycle_one(p) {
            Ok(()) => report.moved.push(p.clone()),
            Err(e) => report.failed.push((p.clone(), e)),
        }
    }
    report
}

fn recycle_one(p: &Path) -> Result<(), RecycleError> {
    let s = p.as_os_str().to_string_lossy();
    // The shell expands wildcards in its source list, and the verbatim prefix is not
    // understood by SHFileOperationW, so either would make it act on the wrong thing.
    if !p.is_absolute() || p.parent().is_none() || s.contains(['*', '?']) || s.starts_with(r"\\?\") {
        return Err(RecycleError::InvalidPath);
    }
    if crate::rules::holds_system_folder(p) {
        return Err(RecycleError::Refused);
    }
    match p.symlink_metadata() {
        Err(e) if e.kind() == io::ErrorKind::NotFound => return Err(RecycleError::NotFound),
        Err(e) if e.kind() == io::ErrorKind::PermissionDenied => return Err(RecycleError::AccessDenied),
        _ => {}
    }
    if !sys::has_recycle_bin(p) {
        return Err(RecycleError::NoRecycleBin);
    }
    os_recycle(p)?;
    // The shell can report success while leaving the item in place (for example a file
    // locked by another process); trust only what the disk says.
    if p.symlink_metadata().is_ok() {
        return Err(RecycleError::InUse);
    }
    Ok(())
}

/// The Recycle Bin through `SHFileOperationW` with `FOF_ALLOWUNDO`.
#[cfg(windows)]
fn os_recycle(p: &Path) -> Result<(), RecycleError> {
    let (code, aborted) = sys::shell_recycle(p);
    if aborted {
        return Err(RecycleError::Aborted);
    }
    if code != 0 {
        return Err(match code {
            2 | 3 | 0x7C => RecycleError::NotFound,
            5 | 0x78 => RecycleError::AccessDenied,
            32 | 33 => RecycleError::InUse,
            c => RecycleError::Shell(c),
        });
    }
    Ok(())
}

/// The user's Trash through NSFileManager (macOS). Never a permanent delete.
#[cfg(not(windows))]
fn os_recycle(p: &Path) -> Result<(), RecycleError> {
    sys::trash(p)
}
