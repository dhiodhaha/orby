//! The multi-threaded walk. Workers only read folders; the calling thread owns the
//! [`Tree`], inserts each listing as it arrives and hands out jobs for new subfolders.
//! That keeps the tree lock-free and lets the progress callback read it directly.

use crate::sys;
use crate::tree::{NewEntry, NodeId, NodeKind, ScanStatus, SkipReason, Skipped, Tree, F_NOT_LOCAL, F_REPARSE, F_SYSTEM};
use std::ffi::OsString;
use std::fs::{self, DirEntry};
use std::io;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{mpsc, Arc, Mutex};
use std::time::{Duration, Instant};

#[derive(Clone, Debug)]
pub struct ScanOptions {
    /// Worker threads. Folder reads are I/O bound, so more than the core count helps.
    pub threads: usize,
    /// Call the progress callback after this many folders have been added.
    pub progress_every: usize,
    /// Folders not entered, listed in [`Tree::skipped`] as [`SkipReason::Protected`]. On macOS
    /// an app without Full Disk Access is stopped by a consent prompt when it opens some of its
    /// own user's folders; the read waits until someone answers. See [`protected_folders`].
    pub protected: Vec<PathBuf>,
}

impl Default for ScanOptions {
    fn default() -> Self {
        let cores = std::thread::available_parallelism().map_or(4, |n| n.get());
        ScanOptions { threads: (cores * 2).clamp(2, 64), progress_every: 64, protected: Vec::new() }
    }
}

/// Handed to the progress callback. The tree is live: totals, `children_by_size` and
/// `largest_files` all work on what has been counted so far.
pub struct Progress<'a> {
    pub tree: &'a Tree,
    /// The folder that was just added.
    pub current: &'a Path,
}

#[derive(Debug)]
pub enum ScanError {
    NotFound,
    /// The drive is not plugged in, or is locked (BitLocker) or has no media.
    NotReady,
    AccessDenied,
    NotADirectory,
    Io(io::Error),
}

impl From<io::Error> for ScanError {
    fn from(e: io::Error) -> Self {
        const ERROR_NOT_READY: i32 = 21;
        const ERROR_INVALID_DRIVE: i32 = 15;
        match (e.raw_os_error(), e.kind()) {
            (Some(ERROR_NOT_READY), _) | (Some(ERROR_INVALID_DRIVE), _) => ScanError::NotReady,
            (_, io::ErrorKind::NotFound) => ScanError::NotFound,
            (_, io::ErrorKind::PermissionDenied) => ScanError::AccessDenied,
            _ => ScanError::Io(e),
        }
    }
}

impl std::fmt::Display for ScanError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            ScanError::NotFound => f.write_str("folder not found"),
            ScanError::NotReady => f.write_str("drive is not ready"),
            ScanError::AccessDenied => f.write_str("access denied"),
            ScanError::NotADirectory => f.write_str("not a folder"),
            ScanError::Io(e) => write!(f, "{e}"),
        }
    }
}

impl std::error::Error for ScanError {}

/// What every worker shares about the scanned volume.
struct Volume {
    cluster: Option<u64>,
    mounts: sys::Mounts,
}

struct Job {
    dir: NodeId,
    path: PathBuf,
}

enum Read {
    Done(Vec<NewEntry>),
    /// Cancel arrived mid-folder: what was read is kept.
    Partial(Vec<NewEntry>),
    /// Cancel arrived before the folder was opened.
    NotStarted,
    Failed(io::Error),
}

struct Listing {
    dir: NodeId,
    path: PathBuf,
    read: Read,
    /// Entries inside the folder that could not be read.
    entry_errors: Vec<(OsString, io::Error)>,
}

/// Scans `root` and returns the tree. Folders the user may not read are listed in
/// [`Tree::skipped`] and the scan carries on. Setting `cancel` stops promptly; the
/// returned tree then holds what was counted and has status [`ScanStatus::Stopped`].
/// Junctions, symbolic links and mount points are never followed.
pub fn scan(root: &Path, options: &ScanOptions, mut progress: impl FnMut(Progress<'_>), cancel: &AtomicBool) -> Result<Tree, ScanError> {
    let root = std::path::absolute(root)?;
    let md = fs::metadata(&root)?;
    if !md.is_dir() {
        return Err(ScanError::NotADirectory);
    }
    let vol = Arc::new(Volume { cluster: sys::cluster_size(&root), mounts: sys::Mounts::new() });
    let mut tree = Tree::new(root.clone(), md.modified().ok());
    let threads = options.threads.max(1);
    let every = options.progress_every.max(1);
    let mut incomplete = false;
    let mut root_error = None;

    let (job_tx, job_rx) = mpsc::channel::<Job>();
    let job_rx = Arc::new(Mutex::new(job_rx));
    // The workers are not scoped: one the system holds inside a folder (a consent prompt, a
    // dead network share) must not hold up the end of the scan. They see `stop` instead of the
    // caller's flag, which this thread passes on.
    let stop = Arc::new(AtomicBool::new(false));
    let (res_tx, res_rx) = mpsc::channel::<Listing>();
    for _ in 0..threads {
        let (res_tx, job_rx, vol, stop) = (res_tx.clone(), Arc::clone(&job_rx), Arc::clone(&vol), Arc::clone(&stop));
        std::thread::spawn(move || worker(&job_rx, res_tx, &stop, &vol));
    }
    drop(res_tx);

    // Every job yields exactly one listing, so `pending` reaching zero means done.
    let _ = job_tx.send(Job { dir: tree.root(), path: root.clone() });
    let mut pending = 1usize;
    let mut added = 0usize;
    let mut last = Instant::now();
    while pending > 0 {
        if cancel.load(Ordering::Relaxed) {
            stop.store(true, Ordering::Relaxed);
        }
        let listing = match res_rx.recv_timeout(Duration::from_millis(250)) {
            Ok(l) => l,
            Err(mpsc::RecvTimeoutError::Timeout) => {
                // Nothing for a while, and every folder still out has been held that long: the
                // system is waiting on someone. Those folders are left out and the scan ends.
                if last.elapsed() >= HELD {
                    let held = in_flight_since(&root, HELD);
                    if !held.is_empty() && held.len() >= pending.min(threads) {
                        // Folders still queued behind the held ones were never read.
                        incomplete |= held.len() < pending;
                        for p in held {
                            tree.skip(Skipped { path: p, reason: SkipReason::Protected });
                        }
                        break;
                    }
                }
                continue;
            }
            Err(mpsc::RecvTimeoutError::Disconnected) => break,
        };
        last = Instant::now();
        pending -= 1;
        for (name, e) in listing.entry_errors {
            // No name: reading the folder's listing failed part way, so the folder is what
            // was skipped (joining "" would add a trailing separator).
            let path = if name.is_empty() { listing.path.clone() } else { listing.path.join(name) };
            tree.skip(Skipped { path, reason: SkipReason::of(&e) });
        }
        let entries = match listing.read {
            Read::Done(v) => v,
            Read::Partial(v) => {
                incomplete = true;
                v
            }
            Read::NotStarted => {
                incomplete = true;
                continue;
            }
            Read::Failed(e) => {
                if listing.dir == tree.root() {
                    root_error = Some(e);
                } else {
                    tree.skip(Skipped { path: listing.path, reason: SkipReason::of(&e) });
                }
                continue;
            }
        };
        let subdirs = tree.add_listing(listing.dir, entries);
        if cancel.load(Ordering::Relaxed) {
            incomplete |= !subdirs.is_empty();
        } else {
            for d in subdirs {
                let path = listing.path.join(tree.node(d).name());
                if options.protected.contains(&path) {
                    tree.skip(Skipped { path, reason: SkipReason::Protected });
                    continue;
                }
                let _ = job_tx.send(Job { dir: d, path });
                pending += 1;
            }
        }
        added += 1;
        if added.is_multiple_of(every) {
            progress(Progress { tree: &tree, current: &listing.path });
        }
    }
    // Idle workers end when the job channel closes; a held one ends when the system lets go.
    stop.store(true, Ordering::Relaxed);
    drop(job_tx);

    if let Some(e) = root_error {
        return Err(e.into());
    }
    tree.finish(if incomplete { ScanStatus::Stopped } else { ScanStatus::Complete });
    Ok(tree)
}

fn worker(jobs: &Mutex<mpsc::Receiver<Job>>, results: mpsc::Sender<Listing>, cancel: &AtomicBool, vol: &Volume) {
    // This thread is ours: listing a folder must never download a cloud placeholder.
    let _local_only = sys::NoMaterialize::new();
    loop {
        let job = jobs.lock().unwrap_or_else(|e| e.into_inner()).recv();
        let Ok(job) = job else { return };
        let listing = if cancel.load(Ordering::Relaxed) {
            Listing { dir: job.dir, path: job.path, read: Read::NotStarted, entry_errors: Vec::new() }
        } else {
            read_dir(job, cancel, vol)
        };
        if results.send(listing).is_err() {
            return;
        }
    }
}

/// The folders being listed right now, with when each started: a folder that takes long is
/// usually one the system is holding (a consent prompt, a slow network share).
static IN_FLIGHT: Mutex<Vec<(PathBuf, std::time::Instant)>> = Mutex::new(Vec::new());

/// How long the walk waits on folders the system holds before it leaves them out.
const HELD: Duration = Duration::from_secs(4);

/// This scan's folders (under `root`) still being listed since at least `at_least` ago.
fn in_flight_since(root: &Path, at_least: Duration) -> Vec<PathBuf> {
    IN_FLIGHT
        .lock()
        .map(|l| l.iter().filter(|(p, t)| p.starts_with(root) && t.elapsed() >= at_least).map(|(p, _)| p.clone()).collect())
        .unwrap_or_default()
}

/// Folders any scan has been listing for longer than `at_least`, longest first.
pub fn scan_waiting(at_least: std::time::Duration) -> Vec<(PathBuf, std::time::Duration)> {
    let mut v: Vec<(PathBuf, std::time::Duration)> = IN_FLIGHT
        .lock()
        .map(|l| l.iter().map(|(p, t)| (p.clone(), t.elapsed())).filter(|(_, d)| *d >= at_least).collect())
        .unwrap_or_default();
    v.sort_by_key(|x| std::cmp::Reverse(x.1));
    v
}

fn read_dir(job: Job, cancel: &AtomicBool, vol: &Volume) -> Listing {
    if let Ok(mut l) = IN_FLIGHT.lock() {
        l.push((job.path.clone(), std::time::Instant::now()));
    }
    let listing = read_dir_inner(job, cancel, vol);
    if let Ok(mut l) = IN_FLIGHT.lock() {
        if let Some(i) = l.iter().position(|(p, _)| *p == listing.path) {
            l.swap_remove(i);
        }
    }
    listing
}

fn read_dir_inner(job: Job, cancel: &AtomicBool, vol: &Volume) -> Listing {
    let dev = vol.mounts.device(&job.path);
    let mut entry_errors = Vec::new();
    let read = match read_dir_retrying(&job.path) {
        Err(e) => Read::Failed(e),
        Ok(iter) => {
            let mut entries = Vec::new();
            let mut stopped = false;
            for item in iter {
                if cancel.load(Ordering::Relaxed) {
                    stopped = true;
                    break;
                }
                match item {
                    Err(e) => entry_errors.push((OsString::new(), e)),
                    Ok(de) => match entry(&de, vol, dev) {
                        Ok(n) => entries.push(n),
                        Err(e) => entry_errors.push((de.file_name(), e)),
                    },
                }
            }
            if stopped {
                Read::Partial(entries)
            } else {
                Read::Done(entries)
            }
        }
    };
    Listing { dir: job.dir, path: job.path, read, entry_errors }
}

/// `fs::read_dir`, tried again when a signal interrupts it: macOS sometimes answers `EINTR`
/// for a sandboxed app container's folder that lists fine a moment later.
fn read_dir_retrying(p: &Path) -> io::Result<fs::ReadDir> {
    let mut tries = 0;
    loop {
        match fs::read_dir(p) {
            Err(e) if e.kind() == io::ErrorKind::Interrupted && tries < 3 => tries += 1,
            r => return r,
        }
    }
}

/// Builds a node from directory-listing data only. On Windows that data comes from the
/// listing itself, so no file is opened: cloud placeholders are never downloaded.
fn entry(e: &DirEntry, vol: &Volume, parent_dev: Option<u64>) -> io::Result<NewEntry> {
    let cluster = vol.cluster;
    let md = e.metadata()?;
    let ft = md.file_type();
    let facts = sys::facts(&md);
    // std reports name-surrogate reparse points (junctions, symlinks, mount points) as
    // symlinks. Other reparse points, such as OneDrive folders, are real folders here. On macOS
    // a folder on another device than its parent is a mount point and is a link too.
    let mount = vol.mounts.crosses(parent_dev, e, &md);
    let kind = if ft.is_symlink() || mount {
        NodeKind::Link
    } else if ft.is_dir() {
        NodeKind::Dir
    } else {
        NodeKind::File
    };
    let size = if kind == NodeKind::File { md.len() } else { 0 };
    let allocated = if kind != NodeKind::File || facts.not_local {
        0
    } else if facts.packed {
        sys::compressed_size(&e.path()).map_or_else(|| round_up(size, cluster), |c| round_up(c, cluster))
    } else {
        allocated_unix(&md).unwrap_or_else(|| round_up(size, cluster))
    };
    let name = e.file_name();
    Ok(NewEntry {
        hidden: facts.hidden || (cfg!(not(windows)) && name.as_encoded_bytes().first() == Some(&b'.')),
        name: name.into_boxed_os_str(),
        kind,
        size,
        allocated,
        modified: md.modified().ok(),
        flags: if facts.system { F_SYSTEM } else { 0 }
            | if facts.not_local { F_NOT_LOCAL } else { 0 }
            | if facts.reparse || ft.is_symlink() || mount { F_REPARSE } else { 0 },
    })
}

fn round_up(n: u64, cluster: Option<u64>) -> u64 {
    match cluster {
        Some(c) if c > 0 => n.div_ceil(c) * c,
        _ => n,
    }
}

#[cfg(unix)]
fn allocated_unix(md: &fs::Metadata) -> Option<u64> {
    use std::os::unix::fs::MetadataExt;
    Some(md.blocks() * 512)
}

#[cfg(not(unix))]
fn allocated_unix(_md: &fs::Metadata) -> Option<u64> {
    None
}

/// Whether this process has Full Disk Access (macOS): it can open the user's privacy database,
/// which only Full Disk Access allows, and which never asks. None elsewhere.
pub fn full_disk_access() -> Option<bool> {
    #[cfg(target_os = "macos")]
    {
        let home = std::env::var_os("HOME").map(PathBuf::from)?;
        Some(fs::File::open(home.join("Library/Application Support/com.apple.TCC/TCC.db")).is_ok())
    }
    #[cfg(not(target_os = "macos"))]
    None
}

/// The folders an app scan should leave out so it never stops at a consent prompt: on macOS
/// without Full Disk Access, the home folder's Desktop, Documents and Downloads, iCloud Drive
/// and cloud storage, other apps' containers, and the Photos, Photo Booth, Music and TV
/// libraries. Nothing elsewhere, or with Full Disk Access. A folder missing here is still left
/// out after a few seconds of waiting (the walk's watchdog), only later.
pub fn protected_folders() -> Vec<PathBuf> {
    if full_disk_access() != Some(false) {
        return Vec::new();
    }
    let Some(home) = std::env::var_os("HOME").map(PathBuf::from).filter(|h| h.is_absolute()) else {
        return Vec::new();
    };
    [
        "Desktop",
        "Documents",
        "Downloads",
        "Library/Mobile Documents",
        "Library/CloudStorage",
        "Library/Containers",
        "Library/Group Containers",
        "Library/Photos",
        "Pictures/Photos Library.photoslibrary",
        "Pictures/Photo Booth Library",
        "Music/Music",
        "Movies/TV",
    ]
        .iter()
        .map(|p| home.join(p))
        .collect()
}

#[cfg(test)]
mod tests {
    use super::round_up;

    #[test]
    fn allocation_rounds_to_whole_clusters() {
        assert_eq!(round_up(0, Some(4096)), 0);
        assert_eq!(round_up(1, Some(4096)), 4096);
        assert_eq!(round_up(4096, Some(4096)), 4096);
        assert_eq!(round_up(4097, Some(4096)), 8192);
        assert_eq!(round_up(4097, None), 4097);
    }
}
