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
use std::sync::{mpsc, Mutex};

#[derive(Clone, Debug)]
pub struct ScanOptions {
    /// Worker threads. Folder reads are I/O bound, so more than the core count helps.
    pub threads: usize,
    /// Call the progress callback after this many folders have been added.
    pub progress_every: usize,
}

impl Default for ScanOptions {
    fn default() -> Self {
        let cores = std::thread::available_parallelism().map_or(4, |n| n.get());
        ScanOptions { threads: (cores * 2).clamp(2, 64), progress_every: 64 }
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
    let cluster = sys::cluster_size(&root);
    let mut tree = Tree::new(root.clone(), md.modified().ok());
    let threads = options.threads.max(1);
    let every = options.progress_every.max(1);
    let mut incomplete = false;
    let mut root_error = None;

    let (job_tx, job_rx) = mpsc::channel::<Job>();
    let job_rx = Mutex::new(job_rx);
    std::thread::scope(|s| {
        let (res_tx, res_rx) = mpsc::channel::<Listing>();
        for _ in 0..threads {
            let (res_tx, job_rx) = (res_tx.clone(), &job_rx);
            s.spawn(move || worker(job_rx, res_tx, cancel, cluster));
        }
        drop(res_tx);

        // Every job yields exactly one listing, so `pending` reaching zero means done.
        let _ = job_tx.send(Job { dir: tree.root(), path: root.clone() });
        let mut pending = 1usize;
        let mut added = 0usize;
        while pending > 0 {
            let Ok(listing) = res_rx.recv() else { break };
            pending -= 1;
            for (name, e) in listing.entry_errors {
                tree.skip(Skipped { path: listing.path.join(name), reason: SkipReason::of(&e) });
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
                    let _ = job_tx.send(Job { dir: d, path });
                    pending += 1;
                }
            }
            added += 1;
            if added.is_multiple_of(every) {
                progress(Progress { tree: &tree, current: &listing.path });
            }
        }
        drop(job_tx);
    });

    if let Some(e) = root_error {
        return Err(e.into());
    }
    tree.finish(if incomplete { ScanStatus::Stopped } else { ScanStatus::Complete });
    Ok(tree)
}

fn worker(jobs: &Mutex<mpsc::Receiver<Job>>, results: mpsc::Sender<Listing>, cancel: &AtomicBool, cluster: Option<u64>) {
    loop {
        let job = jobs.lock().unwrap_or_else(|e| e.into_inner()).recv();
        let Ok(job) = job else { return };
        let listing = if cancel.load(Ordering::Relaxed) {
            Listing { dir: job.dir, path: job.path, read: Read::NotStarted, entry_errors: Vec::new() }
        } else {
            read_dir(job, cancel, cluster)
        };
        if results.send(listing).is_err() {
            return;
        }
    }
}

fn read_dir(job: Job, cancel: &AtomicBool, cluster: Option<u64>) -> Listing {
    let mut entry_errors = Vec::new();
    let read = match fs::read_dir(&job.path) {
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
                    Ok(de) => match entry(&de, cluster) {
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

/// Builds a node from directory-listing data only. On Windows that data comes from the
/// listing itself, so no file is opened: cloud placeholders are never downloaded.
fn entry(e: &DirEntry, cluster: Option<u64>) -> io::Result<NewEntry> {
    let md = e.metadata()?;
    let ft = md.file_type();
    let facts = sys::facts(&md);
    // std reports name-surrogate reparse points (junctions, symlinks, mount points) as
    // symlinks. Other reparse points, such as OneDrive folders, are real folders here.
    let kind = if ft.is_symlink() {
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
            | if facts.reparse || ft.is_symlink() { F_REPARSE } else { 0 },
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
