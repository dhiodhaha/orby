//! The only module that talks to the OS beyond `std::fs`. Everything here is a thin,
//! safe wrapper over one Win32 call, or on macOS over one BSD or Foundation call, with a
//! fallback elsewhere so the crate still builds.

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
    /// Windows marks it as a system file.
    pub system: bool,
    /// Any reparse point, not only junctions and links (OneDrive files carry one too).
    pub reparse: bool,
}

/// What an open handle says about a file, read without touching its data.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) struct Probe {
    /// Volume serial and file index: two names with the same pair are one file (a hard link).
    pub id: (u32, u64),
    pub size: u64,
    /// Last write, in 100 ns ticks since 1601 (FILETIME).
    pub written: u64,
    /// Placeholder, offline, system, reparse point or folder: never read for its contents.
    pub off_limits: bool,
}

#[cfg(windows)]
mod imp {
    use super::EntryFacts;
    use super::Probe;
    use std::ffi::OsString;
    use std::fs::Metadata;
    use std::io;
    use std::os::windows::ffi::{OsStrExt, OsStringExt};
    use std::os::windows::fs::MetadataExt;
    use std::path::{Path, PathBuf};
    use windows_sys::Win32::Foundation::{CloseHandle, INVALID_HANDLE_VALUE};
    use windows_sys::Win32::Storage::FileSystem::{
        CreateFileW, GetCompressedFileSizeW, GetDiskFreeSpaceExW, GetDiskFreeSpaceW, GetDriveTypeW, GetFileInformationByHandle,
        GetVolumeNameForVolumeMountPointW, GetVolumePathNameW, BY_HANDLE_FILE_INFORMATION, FILE_ATTRIBUTE_COMPRESSED,
        FILE_ATTRIBUTE_DIRECTORY, FILE_ATTRIBUTE_HIDDEN, FILE_ATTRIBUTE_OFFLINE, FILE_ATTRIBUTE_RECALL_ON_DATA_ACCESS,
        FILE_ATTRIBUTE_RECALL_ON_OPEN, FILE_ATTRIBUTE_REPARSE_POINT, FILE_ATTRIBUTE_SPARSE_FILE, FILE_ATTRIBUTE_SYSTEM,
        FILE_FLAG_BACKUP_SEMANTICS, FILE_FLAG_OPEN_NO_RECALL, FILE_FLAG_OPEN_REPARSE_POINT, FILE_FLAG_SEQUENTIAL_SCAN,
        FILE_READ_ATTRIBUTES, FILE_SHARE_DELETE, FILE_SHARE_READ, FILE_SHARE_WRITE, INVALID_FILE_SIZE, OPEN_EXISTING,
    };
    use windows_sys::Win32::System::Com::CoTaskMemFree;
    use windows_sys::Win32::System::Ioctl::{
        PropertyStandardQuery, StorageDeviceSeekPenaltyProperty, DEVICE_SEEK_PENALTY_DESCRIPTOR, IOCTL_STORAGE_QUERY_PROPERTY,
        STORAGE_PROPERTY_QUERY,
    };
    use windows_sys::Win32::System::WindowsProgramming::DRIVE_FIXED;
    use windows_sys::Win32::System::IO::DeviceIoControl;
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
            system: a & FILE_ATTRIBUTE_SYSTEM != 0,
            reparse: a & FILE_ATTRIBUTE_REPARSE_POINT != 0,
        }
    }

    /// Opens the file for its attributes only, on the reparse point itself and with no recall, so
    /// a cloud placeholder is never downloaded, and reads its id, size, write time and attributes.
    pub fn probe(p: &Path) -> io::Result<Probe> {
        let path = wide_long(p);
        // SAFETY: `path` is NUL terminated; a null security pointer and template are allowed.
        let h = unsafe {
            CreateFileW(
                path.as_ptr(),
                FILE_READ_ATTRIBUTES,
                FILE_SHARE_READ | FILE_SHARE_WRITE | FILE_SHARE_DELETE,
                std::ptr::null(),
                OPEN_EXISTING,
                FILE_FLAG_BACKUP_SEMANTICS | FILE_FLAG_OPEN_REPARSE_POINT | FILE_FLAG_OPEN_NO_RECALL,
                std::ptr::null_mut(),
            )
        };
        if h == INVALID_HANDLE_VALUE {
            return Err(io::Error::last_os_error());
        }
        // SAFETY: all-zero is a valid BY_HANDLE_FILE_INFORMATION.
        let mut info: BY_HANDLE_FILE_INFORMATION = unsafe { std::mem::zeroed() };
        // SAFETY: `h` is an open handle and `info` a valid out pointer.
        let ok = unsafe { GetFileInformationByHandle(h, &mut info) };
        let err = io::Error::last_os_error();
        // SAFETY: `h` came from CreateFileW and is closed once.
        unsafe { CloseHandle(h) };
        if ok == 0 {
            return Err(err);
        }
        let a = info.dwFileAttributes;
        let off = FILE_ATTRIBUTE_RECALL_ON_DATA_ACCESS
            | FILE_ATTRIBUTE_RECALL_ON_OPEN
            | FILE_ATTRIBUTE_OFFLINE
            | FILE_ATTRIBUTE_SYSTEM
            | FILE_ATTRIBUTE_REPARSE_POINT
            | FILE_ATTRIBUTE_DIRECTORY;
        let ft = info.ftLastWriteTime;
        Ok(Probe {
            id: (info.dwVolumeSerialNumber, (u64::from(info.nFileIndexHigh) << 32) | u64::from(info.nFileIndexLow)),
            size: (u64::from(info.nFileSizeHigh) << 32) | u64::from(info.nFileSizeLow),
            written: (u64::from(ft.dwHighDateTime) << 32) | u64::from(ft.dwLowDateTime),
            off_limits: a & off != 0,
        })
    }

    /// Whether the drive holding `p` has a seek penalty (a spinning disk), from
    /// IOCTL_STORAGE_QUERY_PROPERTY with StorageDeviceSeekPenaltyProperty. None when the volume
    /// cannot be opened or does not answer.
    pub fn seek_penalty(p: &Path) -> Option<bool> {
        let root = wide(&volume_root(p).ok()?);
        let mut name = [0u16; 64];
        // SAFETY: `root` is NUL terminated and `name` holds `name.len()` u16s.
        if unsafe { GetVolumeNameForVolumeMountPointW(root.as_ptr(), name.as_mut_ptr(), name.len() as u32) } == 0 {
            return None;
        }
        // "\\?\Volume{...}\" opens the volume once the trailing backslash is gone.
        let mut dev: Vec<u16> = name.iter().copied().take_while(|&c| c != 0).collect();
        if dev.last() == Some(&u16::from(b'\\')) {
            dev.pop();
        }
        dev.push(0);
        // SAFETY: `dev` is NUL terminated. No access rights are asked for: the query needs none.
        let h = unsafe {
            CreateFileW(dev.as_ptr(), 0, FILE_SHARE_READ | FILE_SHARE_WRITE, std::ptr::null(), OPEN_EXISTING, 0, std::ptr::null_mut())
        };
        if h == INVALID_HANDLE_VALUE {
            return None;
        }
        let query = STORAGE_PROPERTY_QUERY {
            PropertyId: StorageDeviceSeekPenaltyProperty,
            QueryType: PropertyStandardQuery,
            AdditionalParameters: [0],
        };
        // SAFETY: all-zero is a valid descriptor.
        let mut out: DEVICE_SEEK_PENALTY_DESCRIPTOR = unsafe { std::mem::zeroed() };
        let mut got = 0u32;
        // SAFETY: both buffers are valid for the sizes passed; the call is synchronous.
        let ok = unsafe {
            DeviceIoControl(
                h,
                IOCTL_STORAGE_QUERY_PROPERTY,
                &query as *const STORAGE_PROPERTY_QUERY as *const _,
                std::mem::size_of::<STORAGE_PROPERTY_QUERY>() as u32,
                &mut out as *mut DEVICE_SEEK_PENALTY_DESCRIPTOR as *mut _,
                std::mem::size_of::<DEVICE_SEEK_PENALTY_DESCRIPTOR>() as u32,
                &mut got,
                std::ptr::null_mut(),
            )
        };
        // SAFETY: `h` came from CreateFileW and is closed once.
        unsafe { CloseHandle(h) };
        (ok != 0 && got as usize >= std::mem::size_of::<DEVICE_SEEK_PENALTY_DESCRIPTOR>()).then_some(out.IncursSeekPenalty != 0)
    }

    /// Flags for reading file data: sequential, and never recalled from remote storage.
    pub const READ_FLAGS: u32 = FILE_FLAG_SEQUENTIAL_SCAN | FILE_FLAG_OPEN_NO_RECALL;

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

#[cfg(target_os = "macos")]
mod imp {
    use super::EntryFacts;
    use super::Probe;
    use crate::recycle::RecycleError;
    use std::collections::HashSet;
    use std::ffi::{CStr, CString};
    use std::fs::Metadata;
    use std::io;
    use std::os::macos::fs::MetadataExt as _;
    use std::os::unix::ffi::OsStrExt;
    use std::os::unix::fs::MetadataExt;
    use std::path::{Path, PathBuf};

    // st_flags bits from <sys/stat.h>.
    /// Hidden in the Finder (chflags hidden).
    const UF_HIDDEN: u32 = 0x0000_8000;
    /// System Integrity Protection guards it: only Apple's installers may change it.
    const SF_RESTRICTED: u32 = 0x0008_0000;
    /// An iCloud or File Provider placeholder: the data (or a folder's listing) is not on this
    /// Mac, and opening it would download it.
    pub const SF_DATALESS: u32 = 0x4000_0000;

    // <sys/resource.h>: a thread-scoped switch that makes a dataless file fail to open or
    // read (EDEADLK) instead of downloading it, like FILE_FLAG_OPEN_NO_RECALL on Windows.
    const IOPOL_TYPE_VFS_MATERIALIZE_DATALESS_FILES: libc::c_int = 3;
    const IOPOL_SCOPE_THREAD: libc::c_int = 1;
    const IOPOL_MATERIALIZE_DATALESS_FILES_OFF: libc::c_int = 1;

    extern "C" {
        fn getiopolicy_np(iotype: libc::c_int, scope: libc::c_int) -> libc::c_int;
        fn setiopolicy_np(iotype: libc::c_int, scope: libc::c_int, policy: libc::c_int) -> libc::c_int;
    }

    /// The listing facts for one entry, from its `st_flags`. Pure, for tests.
    pub fn facts_from(flags: u32, symlink: bool) -> EntryFacts {
        EntryFacts {
            hidden: flags & UF_HIDDEN != 0,
            not_local: flags & SF_DATALESS != 0,
            // st_blocks already counts what a compressed file takes, so no special case.
            packed: false,
            system: flags & SF_RESTRICTED != 0,
            reparse: symlink,
        }
    }

    /// Whether the duplicate finder must leave a file alone: not a plain file, a placeholder,
    /// protected by SIP, or a hard link (moving one name would free nothing). Pure, for tests.
    pub fn off_limits(flags: u32, plain_file: bool, links: u64) -> bool {
        !plain_file || flags & (SF_DATALESS | SF_RESTRICTED) != 0 || links > 1
    }

    pub fn facts(md: &Metadata) -> EntryFacts {
        facts_from(md.st_flags(), md.file_type().is_symlink())
    }

    /// lstat only: never follows a link, never opens the file, so a placeholder stays in the cloud.
    pub fn probe(p: &Path) -> io::Result<Probe> {
        let md = std::fs::symlink_metadata(p)?;
        let written =
            md.modified().ok().and_then(|t| t.duration_since(std::time::UNIX_EPOCH).ok()).map_or(0, |d| (d.as_nanos() / 100) as u64);
        Ok(Probe {
            id: (md.dev() as u32, md.ino()),
            size: md.len(),
            written,
            off_limits: off_limits(md.st_flags(), md.file_type().is_file(), md.nlink()),
        })
    }

    pub fn hard_linked(md: &Metadata) -> bool {
        md.is_file() && md.nlink() > 1
    }

    fn statfs(p: &Path) -> io::Result<libc::statfs> {
        let c = CString::new(p.as_os_str().as_bytes()).map_err(|_| io::Error::from(io::ErrorKind::InvalidInput))?;
        // SAFETY: all-zero is a valid statfs.
        let mut out: libc::statfs = unsafe { std::mem::zeroed() };
        // SAFETY: `c` is NUL terminated and `out` is a valid out pointer.
        if unsafe { libc::statfs(c.as_ptr(), &mut out) } != 0 {
            return Err(io::Error::last_os_error());
        }
        Ok(out)
    }

    fn mounted_on(s: &libc::statfs) -> PathBuf {
        // SAFETY: the kernel NUL terminates f_mntonname.
        let name = unsafe { CStr::from_ptr(s.f_mntonname.as_ptr()) };
        PathBuf::from(std::ffi::OsStr::from_bytes(name.to_bytes()))
    }

    /// Apple Silicon Macs boot from their internal SSD, so the boot volume and its APFS siblings
    /// (`/` and `/System/Volumes/...`) are an SSD. Anything else (external disks, network shares,
    /// an Intel Mac that may have a hard drive or Fusion Drive) does not say, without IOKit.
    pub fn seek_penalty(p: &Path) -> Option<bool> {
        if !cfg!(target_arch = "aarch64") {
            return None;
        }
        let s = statfs(p).ok()?;
        let on = mounted_on(&s);
        // SAFETY: the kernel NUL terminates f_fstypename.
        let apfs = unsafe { CStr::from_ptr(s.f_fstypename.as_ptr()) }.to_bytes() == b"apfs";
        let boot = on == Path::new("/") || on.starts_with("/System/Volumes");
        (apfs && boot && s.f_flags & libc::MNT_LOCAL as u32 != 0).then_some(false)
    }

    /// O_NOFOLLOW: a file swapped for a symbolic link since the probe fails to open.
    pub const READ_FLAGS: u32 = libc::O_NOFOLLOW as u32;

    pub fn is_reparse(md: &Metadata) -> bool {
        md.file_type().is_symlink()
    }

    pub fn cluster_size(p: &Path) -> Option<u64> {
        statfs(p).ok().map(|s| u64::from(s.f_bsize)).filter(|&b| b > 0)
    }

    pub fn compressed_size(_p: &Path) -> Option<u64> {
        None
    }

    /// (total bytes, free bytes available to this user). Purgeable space (local snapshots,
    /// iCloud copies the system may evict) is not counted as free, unlike the Finder.
    pub fn space(p: &Path) -> io::Result<(u64, u64)> {
        let s = statfs(p)?;
        let b = u64::from(s.f_bsize);
        Ok((s.f_blocks.saturating_mul(b), s.f_bavail.saturating_mul(b)))
    }

    /// Local, writable volumes have a Trash (`~/.Trash`, or `.Trashes` on other disks). Network
    /// shares and read-only disks do not: nothing is attempted there.
    pub fn has_recycle_bin(p: &Path) -> bool {
        statfs(p).is_ok_and(|s| s.f_flags & libc::MNT_LOCAL as u32 != 0 && s.f_flags & libc::MNT_RDONLY as u32 == 0)
    }

    fn home() -> Option<PathBuf> {
        std::env::var_os("HOME").map(PathBuf::from).filter(|p| p.is_absolute())
    }

    /// The Trash folder for the volume holding `p`: `~/.Trash` on the home volume,
    /// `<volume>/.Trashes/<uid>` elsewhere.
    fn trash_dir(p: &Path) -> io::Result<PathBuf> {
        let here = statfs(p)?;
        if let Some(h) = home() {
            if statfs(&h).is_ok_and(|s| mounted_on(&s) == mounted_on(&here)) {
                return Ok(h.join(".Trash"));
            }
        }
        // SAFETY: getuid cannot fail.
        let uid = unsafe { libc::getuid() };
        Ok(mounted_on(&here).join(".Trashes").join(uid.to_string()))
    }

    /// Bytes already in the Trash of the volume holding `p`. Reading `~/.Trash` needs Full Disk
    /// Access on recent macOS; without it this is an error, not zero.
    pub fn recycle_bin_size(p: &Path) -> io::Result<u64> {
        let dir = trash_dir(p)?;
        match std::fs::read_dir(&dir) {
            // Another volume's Trash appears with the first item moved there.
            Err(e) if e.kind() == io::ErrorKind::NotFound => return Ok(0),
            Err(e) => return Err(e),
            Ok(_) => {}
        }
        let mut total = 0u64;
        let mut stack = vec![dir];
        while let Some(d) = stack.pop() {
            let Ok(rd) = std::fs::read_dir(&d) else { continue };
            for e in rd.flatten() {
                let Ok(md) = e.metadata() else { continue };
                if md.is_dir() {
                    stack.push(e.path());
                } else if md.is_file() {
                    total += md.blocks() * 512;
                }
            }
        }
        Ok(total)
    }

    // NSCocoaErrorDomain codes from <Foundation/FoundationErrors.h>.
    const NS_FILE_NO_SUCH_FILE: isize = 4;
    const NS_FILE_LOCKING: isize = 255;
    const NS_FILE_READ_NO_SUCH_FILE: isize = 260;
    const NS_FILE_READ_NO_PERMISSION: isize = 257;
    const NS_FILE_WRITE_NO_PERMISSION: isize = 513;
    const NS_FILE_WRITE_VOLUME_READ_ONLY: isize = 642;
    const NS_FEATURE_UNSUPPORTED: isize = 3328;

    /// What an NSError from trashItemAtURL means here. Pure, for tests.
    pub fn trash_error(cocoa: bool, code: isize) -> RecycleError {
        match (cocoa, code) {
            (true, NS_FILE_NO_SUCH_FILE | NS_FILE_READ_NO_SUCH_FILE) => RecycleError::NotFound,
            (true, NS_FILE_READ_NO_PERMISSION | NS_FILE_WRITE_NO_PERMISSION) => RecycleError::AccessDenied,
            (true, NS_FILE_LOCKING) => RecycleError::InUse,
            // The volume has no Trash: NSFileManager refuses rather than deleting.
            (true, NS_FILE_WRITE_VOLUME_READ_ONLY | NS_FEATURE_UNSUPPORTED) => RecycleError::NoRecycleBin,
            (false, c) if c == libc::ENOENT as isize => RecycleError::NotFound,
            (false, c) if c == libc::EACCES as isize || c == libc::EPERM as isize => RecycleError::AccessDenied,
            (false, c) if c == libc::EBUSY as isize => RecycleError::InUse,
            (_, c) => RecycleError::Shell(i32::try_from(c).unwrap_or(i32::MIN)),
        }
    }

    /// NSFileManager trashItemAtURL:resultingItemURL:error:, the Finder's Move to Trash. It
    /// only ever moves into the user's Trash; it never deletes, and it fails where there is no
    /// Trash. Put Back works from the Finder.
    pub fn trash(p: &Path) -> Result<Option<PathBuf>, RecycleError> {
        use objc2_foundation::{NSFileManager, NSString, NSURL};
        let Some(s) = p.to_str() else { return Err(RecycleError::InvalidPath) };
        objc2::rc::autoreleasepool(|_| {
            let url = NSURL::fileURLWithPath(&NSString::from_str(s));
            let fm = NSFileManager::defaultManager();
            let mut out: Option<objc2::rc::Retained<NSURL>> = None;
            fm.trashItemAtURL_resultingItemURL_error(&url, Some(&mut out)).map_err(|e| {
                let cocoa = e.domain().to_string() == "NSCocoaErrorDomain";
                trash_error(cocoa, e.code())
            })?;
            // Where the item now is in the Trash, for Put Back.
            Ok(out.and_then(|u| u.path()).map(|s| PathBuf::from(s.to_string())))
        })
    }

    // <sys/attr.h>: the extended common attributes, asked for in `forkattr` with
    // FSOPT_ATTR_CMN_EXTENDED.
    const ATTR_BIT_MAP_COUNT: u16 = 5;
    const ATTR_CMN_RETURNED_ATTRS: u32 = 0x8000_0000;
    const ATTR_CMNEXT_PRIVATESIZE: u32 = 0x0000_0008;
    const FSOPT_NOFOLLOW: u32 = 0x0000_0001;
    const FSOPT_ATTR_CMN_EXTENDED: u32 = 0x0000_0020;

    #[repr(C)]
    struct AttrList {
        bitmapcount: u16,
        reserved: u16,
        commonattr: u32,
        volattr: u32,
        dirattr: u32,
        fileattr: u32,
        forkattr: u32,
    }

    extern "C" {
        fn getattrlist(path: *const libc::c_char, list: *mut AttrList, buf: *mut libc::c_void, size: libc::size_t, options: u32) -> libc::c_int;
    }

    /// The bytes moving this file would free at once: on APFS, its blocks that no clone and no
    /// snapshot shares (`ATTR_CMNEXT_PRIVATESIZE`, getattrlist(2)). A clone of another file frees
    /// close to nothing. None when the volume does not say.
    pub fn frees(p: &Path) -> Option<u64> {
        let c = CString::new(p.as_os_str().as_bytes()).ok()?;
        let mut list = AttrList {
            bitmapcount: ATTR_BIT_MAP_COUNT,
            reserved: 0,
            commonattr: ATTR_CMN_RETURNED_ATTRS,
            volattr: 0,
            dirattr: 0,
            fileattr: 0,
            forkattr: ATTR_CMNEXT_PRIVATESIZE,
        };
        // u32 length, attribute_set_t (five u32), then the off_t.
        let mut buf = [0u8; 64];
        // SAFETY: `c` is NUL terminated, `list` and `buf` are valid for the call.
        let r = unsafe {
            getattrlist(c.as_ptr(), &mut list, buf.as_mut_ptr().cast(), buf.len(), FSOPT_NOFOLLOW | FSOPT_ATTR_CMN_EXTENDED)
        };
        if r != 0 {
            return None;
        }
        let u32_at = |o: usize| u32::from_ne_bytes(buf[o..o + 4].try_into().unwrap_or_default());
        // The returned set's fork word says whether the volume answered.
        if u32_at(4 + 16) & ATTR_CMNEXT_PRIVATESIZE == 0 {
            return None;
        }
        let v = i64::from_ne_bytes(buf[24..32].try_into().ok()?);
        Some(v.max(0) as u64)
    }

    pub fn downloads_dir() -> Option<PathBuf> {
        home().map(|h| h.join("Downloads"))
    }

    /// Turns off dataless-file downloads on this thread while it lives, then restores the
    /// previous policy. Our own worker threads hold one for their whole life.
    pub struct NoMaterialize(Option<libc::c_int>, std::marker::PhantomData<*const ()>);

    impl NoMaterialize {
        pub fn new() -> Self {
            // SAFETY: plain syscalls on the calling thread's I/O policy.
            let old = unsafe { getiopolicy_np(IOPOL_TYPE_VFS_MATERIALIZE_DATALESS_FILES, IOPOL_SCOPE_THREAD) };
            // SAFETY: as above.
            let set = unsafe {
                setiopolicy_np(IOPOL_TYPE_VFS_MATERIALIZE_DATALESS_FILES, IOPOL_SCOPE_THREAD, IOPOL_MATERIALIZE_DATALESS_FILES_OFF)
            };
            NoMaterialize((set == 0 && old >= 0).then_some(old), std::marker::PhantomData)
        }
    }

    impl Drop for NoMaterialize {
        fn drop(&mut self) {
            if let Some(old) = self.0 {
                // SAFETY: restores the value read in `new` on the same thread (the guard is !Send).
                unsafe { setiopolicy_np(IOPOL_TYPE_VFS_MATERIALIZE_DATALESS_FILES, IOPOL_SCOPE_THREAD, old) };
            }
        }
    }

    /// Where the scan stops: a mount point (an external disk under `/Volumes`,
    /// `/System/Volumes/Data` seen again from `/`) is listed as a link and never entered. A
    /// folder is a mount point when the mount table names it, or when it is on another device
    /// than its parent (a disk mounted after the scan started). The table matters: the system and
    /// data volumes of an APFS volume group report the same `st_dev`, so `/System/Volumes/Data`
    /// does not look like another device. Firmlinks (`/Users`, `/Applications`, ...) cross from
    /// the system volume to the data volume but are the one real copy, so they are entered.
    pub struct Mounts {
        firmlinks: HashSet<PathBuf>,
        mounted: HashSet<PathBuf>,
    }

    /// Every mount point but `/`, from `getfsstat` (no waiting on slow or network mounts).
    fn mount_points() -> HashSet<PathBuf> {
        // SAFETY: a null buffer asks for the count only.
        let n = unsafe { libc::getfsstat(std::ptr::null_mut(), 0, libc::MNT_NOWAIT) };
        let Ok(n) = usize::try_from(n) else { return HashSet::new() };
        // Room for a few mounts that appear between the two calls.
        // SAFETY: all-zero is a valid statfs.
        let mut buf: Vec<libc::statfs> = vec![unsafe { std::mem::zeroed() }; n + 8];
        let bytes = libc::c_int::try_from(buf.len() * std::mem::size_of::<libc::statfs>()).unwrap_or(libc::c_int::MAX);
        // SAFETY: `buf` holds `bytes` bytes of statfs records.
        let got = unsafe { libc::getfsstat(buf.as_mut_ptr(), bytes, libc::MNT_NOWAIT) };
        buf.truncate(usize::try_from(got).unwrap_or(0));
        buf.iter().map(mounted_on).filter(|p| p != Path::new("/")).collect()
    }

    /// The firmlinks Apple ships, in case `/usr/share/firmlinks` cannot be read.
    const FIRMLINKS: &[&str] = &[
        "/AppleInternal",
        "/Applications",
        "/Library",
        "/System/Library/Caches",
        "/System/Library/Assets",
        "/System/Library/PreinstalledAssets",
        "/System/Library/AssetsV2",
        "/System/Library/PreinstalledAssetsV2",
        "/System/Library/CoreServices/CoreTypes.bundle/Contents/Library",
        "/System/Library/Speech",
        "/Users",
        "/Volumes",
        "/cores",
        "/opt",
        "/private",
        "/usr/local",
        "/usr/libexec/cups",
        "/usr/share/snmp",
    ];

    impl Mounts {
        pub fn new() -> Self {
            let mut firmlinks: HashSet<PathBuf> = FIRMLINKS.iter().map(PathBuf::from).collect();
            if let Ok(text) = std::fs::read_to_string("/usr/share/firmlinks") {
                firmlinks.extend(text.lines().filter_map(|l| l.split('\t').next()).filter(|p| p.starts_with('/')).map(PathBuf::from));
            }
            Mounts { firmlinks, mounted: mount_points() }
        }

        pub fn device(&self, dir: &Path) -> Option<u64> {
            std::fs::symlink_metadata(dir).ok().map(|m| m.dev())
        }

        pub fn crosses(&self, parent_dev: Option<u64>, child: &std::fs::DirEntry, md: &Metadata) -> bool {
            if !md.is_dir() {
                return false;
            }
            let path = child.path();
            !self.firmlinks.contains(&path) && (self.mounted.contains(&path) || parent_dev.is_some_and(|d| d != md.dev()))
        }
    }
}

#[cfg(not(any(windows, target_os = "macos")))]
mod imp {
    use super::EntryFacts;
    use std::fs::Metadata;
    use std::io;
    use std::path::{Path, PathBuf};

    fn unsupported<T>() -> io::Result<T> {
        Err(io::Error::new(io::ErrorKind::Unsupported, "not available on this platform yet"))
    }

    pub fn facts(md: &Metadata) -> EntryFacts {
        EntryFacts { hidden: false, not_local: false, packed: false, system: false, reparse: md.file_type().is_symlink() }
    }
    pub fn probe(p: &Path) -> io::Result<super::Probe> {
        use std::os::unix::fs::MetadataExt;
        let md = std::fs::symlink_metadata(p)?;
        let written =
            md.modified().ok().and_then(|t| t.duration_since(std::time::UNIX_EPOCH).ok()).map_or(0, |d| (d.as_nanos() / 100) as u64);
        Ok(super::Probe { id: (md.dev() as u32, md.ino()), size: md.len(), written, off_limits: !md.file_type().is_file() })
    }
    pub fn seek_penalty(_p: &Path) -> Option<bool> {
        None
    }
    pub const READ_FLAGS: u32 = 0;
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
    pub fn trash(_p: &Path) -> Result<Option<PathBuf>, crate::recycle::RecycleError> {
        Err(crate::recycle::RecycleError::NoRecycleBin)
    }
    pub fn downloads_dir() -> Option<PathBuf> {
        std::env::var_os("HOME").map(|h| PathBuf::from(h).join("Downloads"))
    }
}

/// Elsewhere, file reads never download anything by themselves (Windows uses
/// FILE_FLAG_OPEN_NO_RECALL per handle instead), so this does nothing.
#[cfg(not(target_os = "macos"))]
pub(crate) struct NoMaterialize;

#[cfg(not(target_os = "macos"))]
impl NoMaterialize {
    pub fn new() -> Self {
        NoMaterialize
    }
}

/// Elsewhere mount points are reparse points or links, which the scan already lists as links.
#[cfg(not(target_os = "macos"))]
pub(crate) struct Mounts;

#[cfg(not(target_os = "macos"))]
impl Mounts {
    pub fn new() -> Self {
        Mounts
    }
    pub fn device(&self, _dir: &Path) -> Option<u64> {
        None
    }
    pub fn crosses(&self, _parent_dev: Option<u64>, _child: &std::fs::DirEntry, _md: &Metadata) -> bool {
        false
    }
}

/// Mount-point detection for the scan (macOS).
#[cfg(target_os = "macos")]
pub(crate) use imp::Mounts;
/// Turns off iCloud and File Provider downloads on the calling thread while held (macOS).
#[cfg(target_os = "macos")]
pub(crate) use imp::NoMaterialize;

/// A plain file with more than one name. Only macOS reads this; on Windows hard links are
/// found by file id in the duplicate finder and counted once.
pub(crate) fn hard_linked(_md: &Metadata) -> bool {
    #[cfg(target_os = "macos")]
    return imp::hard_linked(_md);
    #[cfg(not(target_os = "macos"))]
    false
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
#[cfg(windows)]
pub(crate) fn shell_recycle(p: &Path) -> (i32, bool) {
    imp::shell_recycle(p)
}
/// Moves one item to the user's Trash; never deletes. Gives where it now is, when known.
#[cfg(not(windows))]
pub(crate) fn trash(p: &Path) -> Result<Option<PathBuf>, crate::recycle::RecycleError> {
    imp::trash(p)
}
/// The bytes moving this file would free at once (APFS: blocks no clone or snapshot shares).
/// None where the file system does not say (Windows, other volumes).
pub(crate) fn frees(_p: &Path) -> Option<u64> {
    #[cfg(target_os = "macos")]
    return imp::frees(_p);
    #[cfg(not(target_os = "macos"))]
    None
}
pub(crate) fn downloads_dir() -> Option<PathBuf> {
    imp::downloads_dir()
}
pub(crate) fn probe(p: &Path) -> io::Result<Probe> {
    imp::probe(p)
}
pub(crate) fn seek_penalty(p: &Path) -> Option<bool> {
    imp::seek_penalty(p)
}
pub(crate) const READ_FLAGS: u32 = imp::READ_FLAGS;

#[cfg(all(test, target_os = "macos"))]
mod mac_tests {
    use super::imp::{facts_from, off_limits, trash_error, Mounts, SF_DATALESS};
    use crate::recycle::RecycleError;

    #[test]
    fn a_dataless_placeholder_is_not_local_and_never_read() {
        let f = facts_from(SF_DATALESS, false);
        assert!(f.not_local && !f.reparse && !f.system);
        assert!(off_limits(SF_DATALESS, true, 1), "a placeholder is never opened for its bytes");
        assert!(!off_limits(0, true, 1), "a plain local file is read");
    }

    #[test]
    fn hidden_protected_links_and_hard_links_are_flagged() {
        assert!(facts_from(0x8000, false).hidden);
        assert!(facts_from(0x0008_0000, false).system, "SIP-restricted");
        assert!(facts_from(0, true).reparse, "a symbolic link");
        assert!(off_limits(0x0008_0000, true, 1));
        assert!(off_limits(0, false, 1), "not a plain file");
        assert!(off_limits(0, true, 2), "a hard link frees nothing when one name moves");
    }

    /// Reads only the listing of `/System/Volumes`. The data volume shares `st_dev` with the
    /// sealed system volume, so only the mount table tells the scan not to enter it again.
    #[test]
    fn the_data_volume_seen_from_the_system_volume_is_a_mount_point() {
        let data = std::path::Path::new("/System/Volumes/Data");
        if !data.is_dir() {
            return;
        }
        let m = Mounts::new();
        let e = std::fs::read_dir("/System/Volumes").unwrap().flatten().find(|e| e.path() == data).unwrap();
        let md = e.metadata().unwrap();
        assert!(m.crosses(m.device(std::path::Path::new("/System/Volumes")), &e, &md));
        let users = std::fs::read_dir("/").unwrap().flatten().find(|e| e.file_name() == "Users").unwrap();
        assert!(!m.crosses(m.device(std::path::Path::new("/")), &users, &users.metadata().unwrap()), "a firmlink is entered");
    }

    #[test]
    fn trash_errors_map_to_the_recycle_errors() {
        assert_eq!(trash_error(true, 4), RecycleError::NotFound);
        assert_eq!(trash_error(true, 513), RecycleError::AccessDenied);
        assert_eq!(trash_error(true, 3328), RecycleError::NoRecycleBin);
        assert_eq!(trash_error(false, libc::ENOENT as isize), RecycleError::NotFound);
        assert_eq!(trash_error(false, libc::EPERM as isize), RecycleError::AccessDenied);
        assert_eq!(trash_error(true, 99_999), RecycleError::Shell(99_999));
    }
}
