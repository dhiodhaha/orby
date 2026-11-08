//! The only module that talks to the OS beyond `std::fs`. Everything here is a thin,
//! safe wrapper over one Win32 call, with a `not(windows)` fallback so the crate still
//! builds on macOS (where these features arrive later).

use std::fs::Metadata;
use std::io;
use std::path::{Path, PathBuf};

/// What the walker needs from one directory entry's metadata, without opening the file.
pub(crate) struct EntryFacts {
    pub hidden: bool,
    /// File data is not on this disk (cloud placeholder or offline), so it takes no clusters.
    pub not_local: bool,
    /// Compressed or sparse, where cluster rounding of the logical size would be wrong.
    pub packed: bool,
}

#[cfg(windows)]
mod imp {
    use super::EntryFacts;
    use std::ffi::OsString;
    use std::fs::Metadata;
    use std::io;
    use std::os::windows::ffi::{OsStrExt, OsStringExt};
    use std::os::windows::fs::MetadataExt;
    use std::path::{Path, PathBuf};
    use windows_sys::Win32::Storage::FileSystem::{
        GetCompressedFileSizeW, GetDiskFreeSpaceExW, GetDiskFreeSpaceW, GetDriveTypeW, GetVolumePathNameW, FILE_ATTRIBUTE_COMPRESSED,
        FILE_ATTRIBUTE_HIDDEN, FILE_ATTRIBUTE_OFFLINE, FILE_ATTRIBUTE_RECALL_ON_DATA_ACCESS, FILE_ATTRIBUTE_RECALL_ON_OPEN,
        FILE_ATTRIBUTE_REPARSE_POINT, FILE_ATTRIBUTE_SPARSE_FILE, INVALID_FILE_SIZE,
    };
    use windows_sys::Win32::System::Com::CoTaskMemFree;
    use windows_sys::Win32::System::WindowsProgramming::DRIVE_FIXED;
    use windows_sys::Win32::UI::Shell::{
        FOLDERID_Downloads, SHFileOperationW, SHGetKnownFolderPath, SHQueryRecycleBinW, FOF_ALLOWUNDO, FOF_NOCONFIRMATION, FOF_NOERRORUI,
        FOF_SILENT, FOF_WANTNUKEWARNING, FO_DELETE, SHFILEOPSTRUCTW, SHQUERYRBINFO,
    };

    fn wide(p: &Path) -> Vec<u16> {
        p.as_os_str().encode_wide().chain(Some(0)).collect()
    }

    /// `\\?\` form so calls work past MAX_PATH. Only for APIs that accept it.
    fn wide_long(p: &Path) -> Vec<u16> {
        let s: Vec<u16> = p.as_os_str().encode_wide().collect();
        let starts = |pre: &str| {
            let pre: Vec<u16> = pre.encode_utf16().collect();
            s.starts_with(&pre)
        };
        let mut out: Vec<u16> = if starts(r"\\?\") || starts(r"\\.\") {
            Vec::new()
        } else if starts(r"\\") {
            let mut v: Vec<u16> = r"\\?\UNC".encode_utf16().collect();
            v.extend_from_slice(&s[1..]);
            v.push(0);
            return v;
        } else {
            r"\\?\".encode_utf16().collect()
        };
        out.extend_from_slice(&s);
        out.push(0);
        out
    }

    pub fn facts(md: &Metadata) -> EntryFacts {
        let a = md.file_attributes();
        EntryFacts {
            hidden: a & FILE_ATTRIBUTE_HIDDEN != 0,
            not_local: a & (FILE_ATTRIBUTE_RECALL_ON_DATA_ACCESS | FILE_ATTRIBUTE_RECALL_ON_OPEN | FILE_ATTRIBUTE_OFFLINE) != 0,
            packed: a & (FILE_ATTRIBUTE_COMPRESSED | FILE_ATTRIBUTE_SPARSE_FILE) != 0,
        }
    }

    pub fn is_reparse(md: &Metadata) -> bool {
        md.file_attributes() & FILE_ATTRIBUTE_REPARSE_POINT != 0
    }

    pub fn volume_root(p: &Path) -> io::Result<PathBuf> {
        let path = wide(p);
        let mut buf = vec![0u16; 1024];
        // SAFETY: `path` is NUL terminated and `buf` holds `buf.len()` u16s.
        let ok = unsafe { GetVolumePathNameW(path.as_ptr(), buf.as_mut_ptr(), buf.len() as u32) };
        if ok == 0 {
            return Err(io::Error::last_os_error());
        }
        let len = buf.iter().position(|&c| c == 0).unwrap_or(buf.len());
        Ok(PathBuf::from(OsString::from_wide(&buf[..len])))
    }

    pub fn cluster_size(p: &Path) -> Option<u64> {
        let root = wide(&volume_root(p).ok()?);
        let (mut spc, mut bps, mut free, mut total) = (0u32, 0u32, 0u32, 0u32);
        // SAFETY: `root` is NUL terminated; out pointers are valid locals.
        let ok = unsafe { GetDiskFreeSpaceW(root.as_ptr(), &mut spc, &mut bps, &mut free, &mut total) };
        (ok != 0 && spc > 0 && bps > 0).then(|| u64::from(spc) * u64::from(bps))
    }

    pub fn compressed_size(p: &Path) -> Option<u64> {
        let path = wide_long(p);
        let mut high = 0u32;
        // SAFETY: `path` is NUL terminated; `high` is a valid out pointer.
        let low = unsafe { GetCompressedFileSizeW(path.as_ptr(), &mut high) };
        if low == INVALID_FILE_SIZE && io::Error::last_os_error().raw_os_error() != Some(0) {
            return None;
        }
        Some((u64::from(high) << 32) | u64::from(low))
    }

    /// (total bytes, free bytes available to this user).
    pub fn space(p: &Path) -> io::Result<(u64, u64)> {
        let path = wide(p);
        let (mut avail, mut total, mut free) = (0u64, 0u64, 0u64);
        // SAFETY: `path` is NUL terminated; out pointers are valid locals.
        let ok = unsafe { GetDiskFreeSpaceExW(path.as_ptr(), &mut avail, &mut total, &mut free) };
        if ok == 0 {
            return Err(io::Error::last_os_error());
        }
        Ok((total, avail))
    }

    pub fn has_recycle_bin(p: &Path) -> bool {
        // Removable and network drives have no Recycle Bin: the shell would delete for good.
        let Ok(root) = volume_root(p) else { return false };
        let root = wide(&root);
        // SAFETY: `root` is NUL terminated.
        unsafe { GetDriveTypeW(root.as_ptr()) == DRIVE_FIXED }
    }

    pub fn recycle_bin_size(p: &Path) -> io::Result<u64> {
        let root = wide(&volume_root(p)?);
        let mut info = SHQUERYRBINFO { cbSize: std::mem::size_of::<SHQUERYRBINFO>() as u32, i64Size: 0, i64NumItems: 0 };
        // SAFETY: `root` is NUL terminated; `info` has cbSize set as the API requires.
        let hr = unsafe { SHQueryRecycleBinW(root.as_ptr(), &mut info) };
        if hr < 0 {
            return Err(io::Error::from_raw_os_error(hr));
        }
        Ok(info.i64Size.max(0) as u64)
    }

    /// Result code of SHFileOperationW, and whether the user aborted (for example at a
    /// "delete permanently?" prompt raised by FOF_WANTNUKEWARNING).
    pub fn shell_recycle(p: &Path) -> (i32, bool) {
        // pFrom is a list of NUL terminated names ended by an extra NUL.
        let mut from = wide(p);
        from.push(0);
        let mut op = SHFILEOPSTRUCTW {
            hwnd: std::ptr::null_mut(),
            wFunc: FO_DELETE,
            pFrom: from.as_ptr(),
            pTo: std::ptr::null(),
            // FOF_ALLOWUNDO makes delete mean recycle. FOF_WANTNUKEWARNING makes the shell
            // ask instead of silently destroying an item it cannot recycle (too big for the bin).
            fFlags: (FOF_ALLOWUNDO | FOF_NOCONFIRMATION | FOF_SILENT | FOF_NOERRORUI | FOF_WANTNUKEWARNING) as u16,
            fAnyOperationsAborted: 0,
            hNameMappings: std::ptr::null_mut(),
            lpszProgressTitle: std::ptr::null(),
        };
        // SAFETY: `op` is fully initialised and `from` outlives the call.
        let code = unsafe { SHFileOperationW(&mut op) };
        (code, op.fAnyOperationsAborted != 0)
    }

    pub fn downloads_dir() -> Option<PathBuf> {
        let mut out: *mut u16 = std::ptr::null_mut();
        // SAFETY: valid GUID pointer and out pointer; a null token means the current user.
        let hr = unsafe { SHGetKnownFolderPath(&FOLDERID_Downloads, 0, std::ptr::null_mut(), &mut out) };
        let path = (hr >= 0 && !out.is_null()).then(|| {
            // SAFETY: on success `out` is a NUL terminated wide string owned by us.
            let len = (0..).take_while(|&i| unsafe { *out.add(i) } != 0).count();
            PathBuf::from(OsString::from_wide(unsafe { std::slice::from_raw_parts(out, len) }))
        });
        // SAFETY: the API requires freeing `out` with CoTaskMemFree even on failure.
        unsafe { CoTaskMemFree(out as *const _) };
        path
    }
}

#[cfg(not(windows))]
mod imp {
    use super::EntryFacts;
    use std::fs::Metadata;
    use std::io;
    use std::path::{Path, PathBuf};

    fn unsupported<T>() -> io::Result<T> {
        Err(io::Error::new(io::ErrorKind::Unsupported, "not available on this platform yet"))
    }

    pub fn facts(_md: &Metadata) -> EntryFacts {
        EntryFacts { hidden: false, not_local: false, packed: false }
    }
    pub fn is_reparse(md: &Metadata) -> bool {
        md.file_type().is_symlink()
    }
    pub fn cluster_size(_p: &Path) -> Option<u64> {
        None
    }
    pub fn compressed_size(_p: &Path) -> Option<u64> {
        None
    }
    pub fn space(_p: &Path) -> io::Result<(u64, u64)> {
        unsupported()
    }
    pub fn has_recycle_bin(_p: &Path) -> bool {
        false
    }
    pub fn recycle_bin_size(_p: &Path) -> io::Result<u64> {
        unsupported()
    }
    pub fn shell_recycle(_p: &Path) -> (i32, bool) {
        (-1, false)
    }
    pub fn downloads_dir() -> Option<PathBuf> {
        std::env::var_os("HOME").map(|h| PathBuf::from(h).join("Downloads"))
    }
}

pub(crate) fn facts(md: &Metadata) -> EntryFacts {
    imp::facts(md)
}
/// Junction, symbolic link, mount point, or any other reparse point (OneDrive folders too).
pub(crate) fn is_reparse(md: &Metadata) -> bool {
    imp::is_reparse(md)
}
pub(crate) fn cluster_size(p: &Path) -> Option<u64> {
    imp::cluster_size(p)
}
pub(crate) fn compressed_size(p: &Path) -> Option<u64> {
    imp::compressed_size(p)
}
pub(crate) fn space(p: &Path) -> io::Result<(u64, u64)> {
    imp::space(p)
}
pub(crate) fn has_recycle_bin(p: &Path) -> bool {
    imp::has_recycle_bin(p)
}
pub(crate) fn recycle_bin_size(p: &Path) -> io::Result<u64> {
    imp::recycle_bin_size(p)
}
pub(crate) fn shell_recycle(p: &Path) -> (i32, bool) {
    imp::shell_recycle(p)
}
pub(crate) fn downloads_dir() -> Option<PathBuf> {
    imp::downloads_dir()
}
