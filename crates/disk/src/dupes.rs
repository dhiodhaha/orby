//! Duplicate files: groups of files with the same bytes, found among the files a scan listed.
//!
//! - Stage 1 ([`candidates`]) groups the scan's files by size. No file is opened, so the
//!   "possible duplicates" figure it gives is an upper bound.
//! - [`find_duplicates`] opens each same-size file for its attributes only, drops cloud
//!   placeholders, offline, system files and reparse points, and counts hard links (same volume
//!   serial and file index) once. Stage 2 hashes the first and last 64 KB. Stage 3 hashes whole
//!   files with BLAKE3. A full BLAKE3 match counts as identical: there is no byte compare.
//! - An SSD is read by one thread per core, a hard drive (or a drive that does not say) by one
//!   thread with 1 MB reads. Hashes are cached by path, size and write time, so a rescan only
//!   reads files that changed.
//!
//! Nothing here deletes. [`recycle_duplicates`] moves extra copies to the Recycle Bin after
//! checking each one again, and only while its kept copy is still there.

use crate::recycle::{move_to_recycle_bin, RecycleError, RecycleReport};
use crate::rules::{self, never_touch, Refusal, RuleEnv};
use crate::sys::{self, Probe};
use crate::tree::{NodeKind, Tree};
use serde::{Deserialize, Serialize};
use std::collections::{HashMap, HashSet};
use std::fs::{self, File};
use std::io::{self, Read, Seek, SeekFrom};
use std::path::{Component, Path, PathBuf};
use std::sync::atomic::{AtomicBool, AtomicU64, AtomicUsize, Ordering};
use std::sync::mpsc;
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

/// Files smaller than this are left out by default.
pub const DUPE_MIN_SIZE: u64 = 1 << 20;
/// Stage 2 reads this much from each end of a file.
pub const DUPE_EDGE: u64 = 64 * 1024;
/// Folders with these names are never looked into (case does not matter).
pub const DUPE_SKIPPED_NAMES: &[&str] = &[".git", "node_modules"];
/// One read, for both stages and both kinds of drive.
const BUF: usize = 1 << 20;
/// The cache keeps at most this many files, the most recently seen first...
const CACHE_MAX: usize = 100_000;
/// ...and forgets a file it has not seen for this long.
const CACHE_DAYS: u64 = 180;
const CACHE_VERSION: u32 = 1;
const PROGRESS_EVERY: Duration = Duration::from_millis(100);

/// One file, as the scan listed it (or, after [`find_duplicates`], as its handle reported it).
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct DupeFile {
    pub path: PathBuf,
    pub size: u64,
    pub modified: Option<SystemTime>,
}

#[derive(Clone, Debug)]
pub struct DupeOptions {
    /// Smaller files are left out. Zero-byte files are always left out.
    pub min_size: u64,
    /// For the never-touch list ([`never_touch`]).
    pub env: RuleEnv,
    /// Windows, Program Files, ProgramData: skipped with everything inside them.
    pub system_dirs: Vec<PathBuf>,
    /// The hash cache (JSON). None: no cache.
    pub cache: Option<PathBuf>,
    /// None: one per core on an SSD, one on a hard drive or a drive that does not say.
    pub threads: Option<usize>,
    /// Stamps cache entries; the crate reads no clock.
    pub now: SystemTime,
}

impl DupeOptions {
    /// The defaults: 1 MB and up, the user's never-touch list and system folders from the
    /// environment, threads by drive type.
    pub fn for_current_user(cache: Option<PathBuf>, now: SystemTime) -> Self {
        DupeOptions { min_size: DUPE_MIN_SIZE, env: RuleEnv::from_process(), system_dirs: rules::system_dirs(), cache, threads: None, now }
    }
}

/// Stage 1: the files that share their size with at least one other.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Candidates {
    /// The scanned folder or drive.
    pub root: PathBuf,
    /// Biggest size first, then by path.
    pub files: Vec<DupeFile>,
    /// The sum over same-size groups of (n - 1) × size: the most a full check could find. An
    /// upper bound, not a promise.
    pub possible: u64,
    /// Same-size groups.
    pub groups: usize,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub enum DupeStage {
    /// Opening each file for its id and attributes (no data is read).
    Sizes,
    /// The first and last 64 KB.
    Ends,
    /// Whole files.
    Contents,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct DupeProgress {
    pub stage: DupeStage,
    pub files_done: u64,
    pub files_total: u64,
    /// Bytes read in this stage, and the most it will read.
    pub bytes_done: u64,
    pub bytes_total: u64,
}

/// Identical files: same size, same BLAKE3 hash of the whole file.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct DupeGroup {
    pub size: u64,
    /// At least two, by path.
    pub files: Vec<DupeFile>,
}

impl DupeGroup {
    /// (n - 1) × size: what moving every copy but one would free.
    pub fn reclaimable(&self) -> u64 {
        self.size * (self.files.len() as u64).saturating_sub(1)
    }
}

/// Whether the drive seeks: decides the threads and the read pattern.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum DriveKind {
    Ssd,
    Hdd,
    /// The drive did not answer; read it like a hard drive.
    #[default]
    Unknown,
}

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct DupeReport {
    /// Most bytes to reclaim first.
    pub groups: Vec<DupeGroup>,
    /// Files that could not be opened or read (in use, access denied, changed while read).
    pub skipped: u64,
    /// Stop was pressed: `groups` is empty, because a half-checked group proves nothing.
    pub stopped: bool,
    pub drive: DriveKind,
    pub threads: usize,
    /// Bytes read from disk in stages 2 and 3.
    pub bytes_read: u64,
    /// Files whose hashes came from the cache.
    pub from_cache: u64,
}

impl DupeReport {
    pub fn reclaimable(&self) -> u64 {
        self.groups.iter().map(DupeGroup::reclaimable).sum()
    }
}

/// IOCTL_STORAGE_QUERY_PROPERTY, StorageDeviceSeekPenaltyProperty: a seek penalty is a hard
/// drive, none is an SSD, no answer is [`DriveKind::Unknown`].
pub fn drive_kind(path: &Path) -> DriveKind {
    match sys::seek_penalty(path) {
        Some(false) => DriveKind::Ssd,
        Some(true) => DriveKind::Hdd,
        None => DriveKind::Unknown,
    }
}

fn threads_for(kind: DriveKind) -> usize {
    match kind {
        DriveKind::Ssd => std::thread::available_parallelism().map_or(4, |n| n.get()),
        DriveKind::Hdd | DriveKind::Unknown => 1,
    }
}

fn skipped_name(name: &std::ffi::OsStr) -> bool {
    let n = name.to_string_lossy();
    DUPE_SKIPPED_NAMES.iter().any(|s| s.eq_ignore_ascii_case(&n))
}

/// Stage 1 over a scan's tree. Leaves out `.git` and `node_modules`, the system folders, links
/// and other reparse points, system files, cloud placeholders, files under `min_size`, and
/// anything on the never-touch list. Opens no file.
pub fn candidates(tree: &Tree, opts: &DupeOptions) -> Candidates {
    let root = tree.root_path().to_path_buf();
    let mut files = Vec::new();
    let min = opts.min_size.max(1);
    // A system folder is pruned where the walk meets it, at its own depth.
    let sys: Vec<(usize, &Path)> = opts.system_dirs.iter().map(|d| (d.components().count(), d.as_path())).collect();
    let inside_sys = sys.iter().any(|(_, d)| rules::is_within(&root, d));
    if !inside_sys {
        let mut stack = vec![(tree.root(), root.clone(), root.components().count())];
        while let Some((id, path, depth)) = stack.pop() {
            for &c in tree.children(id) {
                let n = tree.node(c);
                match n.kind() {
                    NodeKind::Dir => {
                        if n.reparse() || skipped_name(n.name()) {
                            continue;
                        }
                        let p = path.join(n.name());
                        if sys.iter().any(|(k, d)| *k == depth + 1 && rules::same(&p, d)) {
                            continue;
                        }
                        stack.push((c, p, depth + 1));
                    }
                    NodeKind::File => {
                        if n.size() >= min && !n.system() && !n.not_local() && !n.reparse() {
                            files.push(DupeFile { path: path.join(n.name()), size: n.size(), modified: n.modified() });
                        }
                    }
                    NodeKind::Link => {}
                }
            }
        }
    }
    let mut groups = by_size(files);
    // The never-touch lists are expanded once; only files that share a size are checked.
    let never = rules::NeverTouch::new(&opts.env);
    for g in &mut groups {
        g.retain(|f| !never.holds(&f.path));
    }
    groups.retain(|g| g.len() >= 2);
    let possible = groups.iter().map(|g| g[0].size * (g.len() as u64 - 1)).sum();
    let n = groups.len();
    Candidates { root, files: groups.into_iter().flatten().collect(), possible, groups: n }
}

/// Same-size groups of two or more, biggest size first, each by path.
fn by_size(files: Vec<DupeFile>) -> Vec<Vec<DupeFile>> {
    let mut map: HashMap<u64, Vec<DupeFile>> = HashMap::new();
    for f in files {
        map.entry(f.size).or_default().push(f);
    }
    let mut groups: Vec<Vec<DupeFile>> = map.into_values().filter(|g| g.len() >= 2).collect();
    for g in &mut groups {
        g.sort_by(|a, b| a.path.cmp(&b.path));
    }
    groups.sort_by(|a, b| b[0].size.cmp(&a[0].size).then_with(|| a[0].path.cmp(&b[0].path)));
    groups
}

type Hash = [u8; 32];

struct Item {
    file: DupeFile,
    written: u64,
    ends: Option<Hash>,
    full: Option<Hash>,
}

/// Calls `progress` at most every 100 ms, and always when `force` is set.
struct Pace<'a, F: FnMut(DupeProgress)> {
    f: &'a mut F,
    last: Option<Instant>,
}

impl<F: FnMut(DupeProgress)> Pace<'_, F> {
    fn send(&mut self, p: DupeProgress, force: bool) {
        if force || self.last.is_none_or(|t| t.elapsed() >= PROGRESS_EVERY) {
            (self.f)(p);
            self.last = Some(Instant::now());
        }
    }
}

/// Stages 2 and 3 (and the attribute and hard-link pass before them) over stage 1's files.
/// Blocks; the caller runs it on a thread. Setting `cancel` stops within one 1 MB read per
/// thread; the report then has `stopped` and no groups. Files that cannot be read are counted in
/// `skipped` and left out, never fatal.
pub fn find_duplicates(c: &Candidates, opts: &DupeOptions, mut progress: impl FnMut(DupeProgress), cancel: &AtomicBool) -> DupeReport {
    let drive = c.files.first().map_or(DriveKind::Unknown, |f| drive_kind(&f.path));
    let threads = opts.threads.unwrap_or_else(|| threads_for(drive)).max(1);
    let mut report = DupeReport { drive, threads, ..DupeReport::default() };
    let mut pace = Pace { f: &mut progress, last: None };
    let mut cache = Cache::load(opts.cache.as_deref());
    let now = unix_secs(opts.now);

    // Attributes and ids: placeholders and the like are dropped, hard links count once.
    let total = c.files.len() as u64;
    let at =
        |stage, files_done, files_total, bytes_done, bytes_total| DupeProgress { stage, files_done, files_total, bytes_done, bytes_total };
    pace.send(at(DupeStage::Sizes, 0, total, 0, 0), true);
    let mut probes: Vec<Option<Probe>> = vec![None; c.files.len()];
    let mut done = 0u64;
    pool(
        &c.files,
        threads,
        cancel,
        |f| sys::probe(&f.path),
        |r| {
            if let Some((i, r)) = r {
                done += 1;
                match r {
                    Ok(p) => probes[i] = Some(p),
                    Err(_) => report.skipped += 1,
                }
            }
            pace.send(at(DupeStage::Sizes, done, total, 0, 0), false);
        },
    );
    if cancel.load(Ordering::Relaxed) {
        report.stopped = true;
        return report;
    }
    let mut seen_ids = HashSet::new();
    let mut items: Vec<Item> = Vec::new();
    for (f, p) in c.files.iter().zip(probes) {
        let Some(p) = p else { continue };
        // Changed since the scan, or not plain local data: never read.
        if p.off_limits || p.size != f.size || !seen_ids.insert(p.id) {
            continue;
        }
        let (ends, full) = cache.get(&f.path, p.size, p.written);
        if ends.is_some() {
            report.from_cache += 1;
        }
        let file = DupeFile { path: f.path.clone(), size: f.size, modified: filetime(p.written).or(f.modified) };
        items.push(Item { file, written: p.written, ends, full });
    }
    items = regroup(items, |it| Some((it.file.size, [0; 32])));

    // Stage 2: the first and last 64 KB.
    let need: Vec<usize> = (0..items.len()).filter(|&i| items[i].ends.is_none()).collect();
    let bytes_total: u64 = need.iter().map(|&i| items[i].file.size.min(2 * DUPE_EDGE)).sum();
    let total = items.len() as u64;
    let mut done = total - need.len() as u64;
    let read = AtomicU64::new(0);
    pace.send(at(DupeStage::Ends, done, total, 0, bytes_total), true);
    let mut got: Vec<Option<Hash>> = vec![None; need.len()];
    let jobs: Vec<(&Path, u64)> = need.iter().map(|&i| (items[i].file.path.as_path(), items[i].file.size)).collect();
    pool(
        &jobs,
        threads,
        cancel,
        |&(path, size)| read_ends(path, size, &read),
        |r| {
            if let Some((k, r)) = r {
                done += 1;
                match r {
                    Ok(h) => got[k] = Some(h),
                    Err(_) => report.skipped += 1,
                }
            }
            pace.send(at(DupeStage::Ends, done, total, read.load(Ordering::Relaxed), bytes_total), false);
        },
    );
    for (k, h) in got.into_iter().enumerate() {
        if h.is_some() {
            items[need[k]].ends = h;
        }
    }
    report.bytes_read += read.load(Ordering::Relaxed);
    for it in &items {
        if it.ends.is_some() {
            cache.put(&it.file.path, it.file.size, it.written, it.ends, it.full, now);
        }
    }
    if cancel.load(Ordering::Relaxed) {
        report.stopped = true;
        cache.save(opts.cache.as_deref(), &c.root, None, now);
        return report;
    }
    items = regroup(items, |it| it.ends.map(|h| (it.file.size, h)));

    // Stage 3: whole files. Up to 128 KB the ends already covered every byte.
    for it in &mut items {
        if it.file.size <= 2 * DUPE_EDGE {
            it.full = it.ends;
        }
    }
    let need: Vec<usize> = (0..items.len()).filter(|&i| items[i].full.is_none()).collect();
    let bytes_total: u64 = need.iter().map(|&i| items[i].file.size).sum();
    let total = items.len() as u64;
    let mut done = total - need.len() as u64;
    let read = AtomicU64::new(0);
    pace.send(at(DupeStage::Contents, done, total, 0, bytes_total), true);
    let mut got: Vec<Option<Hash>> = vec![None; need.len()];
    let jobs: Vec<(&Path, u64)> = need.iter().map(|&i| (items[i].file.path.as_path(), items[i].file.size)).collect();
    pool(
        &jobs,
        threads,
        cancel,
        |&(path, size)| read_full(path, size, &read, cancel),
        |r| {
            if let Some((k, r)) = r {
                match r {
                    Ok(Some(h)) => {
                        done += 1;
                        got[k] = Some(h);
                    }
                    Ok(None) => {}
                    Err(_) => {
                        done += 1;
                        report.skipped += 1;
                    }
                }
            }
            pace.send(at(DupeStage::Contents, done, total, read.load(Ordering::Relaxed), bytes_total), false);
        },
    );
    for (k, h) in got.into_iter().enumerate() {
        if h.is_some() {
            items[need[k]].full = h;
        }
    }
    report.bytes_read += read.load(Ordering::Relaxed);
    for it in &items {
        cache.put(&it.file.path, it.file.size, it.written, it.ends, it.full, now);
    }
    if cancel.load(Ordering::Relaxed) {
        report.stopped = true;
        cache.save(opts.cache.as_deref(), &c.root, None, now);
        return report;
    }
    pace.send(at(DupeStage::Contents, total, total, bytes_total, bytes_total), true);

    let mut map: HashMap<(u64, Hash), Vec<DupeFile>> = HashMap::new();
    for it in items {
        if let Some(h) = it.full {
            map.entry((it.file.size, h)).or_default().push(it.file);
        }
    }
    let mut groups: Vec<DupeGroup> = map
        .into_iter()
        .filter(|(_, v)| v.len() >= 2)
        .map(|((size, _), mut files)| {
            files.sort_by(|a, b| a.path.cmp(&b.path));
            DupeGroup { size, files }
        })
        .collect();
    groups.sort_by(|a, b| {
        b.reclaimable().cmp(&a.reclaimable()).then(b.size.cmp(&a.size)).then_with(|| a.files[0].path.cmp(&b.files[0].path))
    });
    report.groups = groups;
    let seen: HashSet<String> = c.files.iter().filter_map(|f| f.path.to_str().map(str::to_owned)).collect();
    cache.save(opts.cache.as_deref(), &c.root, Some(&seen), now);
    report
}

/// Keeps the items whose key is shared by at least one other item.
fn regroup(items: Vec<Item>, key: impl Fn(&Item) -> Option<(u64, Hash)>) -> Vec<Item> {
    let mut count: HashMap<(u64, Hash), usize> = HashMap::new();
    for it in &items {
        if let Some(k) = key(it) {
            *count.entry(k).or_default() += 1;
        }
    }
    items.into_iter().filter(|it| key(it).is_some_and(|k| count[&k] >= 2)).collect()
}

/// Runs `work` over `items` on `threads` threads. `on` gets each result on the calling thread,
/// and None every 100 ms while it waits, so progress can move during a long read.
fn pool<T: Sync, R: Send>(
    items: &[T],
    threads: usize,
    cancel: &AtomicBool,
    work: impl Fn(&T) -> R + Sync,
    mut on: impl FnMut(Option<(usize, R)>),
) {
    if items.is_empty() {
        return;
    }
    let next = AtomicUsize::new(0);
    std::thread::scope(|s| {
        let (tx, rx) = mpsc::channel();
        for _ in 0..threads.clamp(1, items.len()) {
            let (tx, next, work) = (tx.clone(), &next, &work);
            s.spawn(move || loop {
                if cancel.load(Ordering::Relaxed) {
                    return;
                }
                let i = next.fetch_add(1, Ordering::Relaxed);
                let Some(item) = items.get(i) else { return };
                if tx.send((i, work(item))).is_err() {
                    return;
                }
            });
        }
        drop(tx);
        loop {
            match rx.recv_timeout(PROGRESS_EVERY) {
                Ok(r) => on(Some(r)),
                Err(mpsc::RecvTimeoutError::Timeout) => on(None),
                Err(mpsc::RecvTimeoutError::Disconnected) => break,
            }
        }
    });
}

fn open_read(p: &Path) -> io::Result<File> {
    let mut o = fs::OpenOptions::new();
    o.read(true);
    #[cfg(windows)]
    {
        use std::os::windows::fs::OpenOptionsExt;
        o.custom_flags(sys::READ_FLAGS);
    }
    o.open(p)
}

/// BLAKE3 of the size, the first 64 KB and the last 64 KB (the whole file up to 128 KB).
fn read_ends(p: &Path, size: u64, read: &AtomicU64) -> io::Result<Hash> {
    let mut f = open_read(p)?;
    let mut h = blake3::Hasher::new();
    h.update(&size.to_le_bytes());
    if size <= 2 * DUPE_EDGE {
        let mut buf = Vec::with_capacity(size as usize);
        f.read_to_end(&mut buf)?;
        if buf.len() as u64 != size {
            return Err(io::ErrorKind::InvalidData.into());
        }
        h.update(&buf);
        read.fetch_add(size, Ordering::Relaxed);
    } else {
        let mut buf = vec![0u8; DUPE_EDGE as usize];
        f.read_exact(&mut buf)?;
        h.update(&buf);
        f.seek(SeekFrom::Start(size - DUPE_EDGE))?;
        f.read_exact(&mut buf)?;
        h.update(&buf);
        read.fetch_add(2 * DUPE_EDGE, Ordering::Relaxed);
    }
    Ok(*h.finalize().as_bytes())
}

/// BLAKE3 of the whole file, read in 1 MB pieces. Ok(None) when cancelled.
fn read_full(p: &Path, size: u64, read: &AtomicU64, cancel: &AtomicBool) -> io::Result<Option<Hash>> {
    let mut f = open_read(p)?;
    let mut h = blake3::Hasher::new();
    let mut buf = vec![0u8; BUF];
    let mut got = 0u64;
    loop {
        if cancel.load(Ordering::Relaxed) {
            return Ok(None);
        }
        let n = match f.read(&mut buf) {
            Ok(0) => break,
            Ok(n) => n,
            Err(e) if e.kind() == io::ErrorKind::Interrupted => continue,
            Err(e) => return Err(e),
        };
        h.update(&buf[..n]);
        got += n as u64;
        read.fetch_add(n as u64, Ordering::Relaxed);
    }
    if got != size {
        return Err(io::ErrorKind::InvalidData.into());
    }
    Ok(Some(*h.finalize().as_bytes()))
}

/// FILETIME ticks (100 ns since 1601) to a SystemTime.
fn filetime(ticks: u64) -> Option<SystemTime> {
    const TO_UNIX: u64 = 116_444_736_000_000_000;
    if cfg!(windows) {
        let t = ticks.checked_sub(TO_UNIX)?;
        Some(UNIX_EPOCH + Duration::from_nanos(t.saturating_mul(100)))
    } else {
        Some(UNIX_EPOCH + Duration::from_nanos(ticks.saturating_mul(100)))
    }
}

fn unix_secs(t: SystemTime) -> u64 {
    t.duration_since(UNIX_EPOCH).map_or(0, |d| d.as_secs())
}

// ---- the hash cache ----

#[derive(Serialize, Deserialize, Clone, Debug, PartialEq)]
struct CacheEntry {
    size: u64,
    /// FILETIME ticks.
    written: u64,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    ends: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    full: Option<String>,
    /// Unix seconds when a check last used it.
    seen: u64,
}

#[derive(Serialize, Deserialize, Default)]
struct CacheFile {
    version: u32,
    files: HashMap<String, CacheEntry>,
}

/// Hashes by path, valid only while the size and write time match.
#[derive(Default)]
struct Cache {
    files: HashMap<String, CacheEntry>,
}

fn hex(h: &Hash) -> String {
    blake3::Hash::from_bytes(*h).to_hex().to_string()
}

fn unhex(s: &str) -> Option<Hash> {
    blake3::Hash::from_hex(s).ok().map(|h| *h.as_bytes())
}

impl Cache {
    fn load(path: Option<&Path>) -> Cache {
        let Some(text) = path.and_then(|p| fs::read_to_string(p).ok()) else { return Cache::default() };
        match serde_json::from_str::<CacheFile>(&text) {
            Ok(f) if f.version == CACHE_VERSION => Cache { files: f.files },
            _ => Cache::default(),
        }
    }

    fn get(&self, path: &Path, size: u64, written: u64) -> (Option<Hash>, Option<Hash>) {
        match path.to_str().and_then(|k| self.files.get(k)) {
            Some(e) if e.size == size && e.written == written => (e.ends.as_deref().and_then(unhex), e.full.as_deref().and_then(unhex)),
            _ => (None, None),
        }
    }

    fn put(&mut self, path: &Path, size: u64, written: u64, ends: Option<Hash>, full: Option<Hash>, now: u64) {
        // A name that is not valid Unicode has no exact key, so it is never cached.
        let Some(k) = path.to_str() else { return };
        let e = CacheEntry { size, written, ends: ends.as_ref().map(hex), full: full.as_ref().map(hex), seen: now };
        self.files.insert(k.to_owned(), e);
    }

    /// Forgets old entries and, after a full check of `root`, files under it that the check did
    /// not see (moved, deleted or no longer sharing a size); keeps the newest `CACHE_MAX`; then
    /// writes the file whole (write, then rename). A write that fails just leaves no cache.
    fn save(mut self, path: Option<&Path>, root: &Path, seen: Option<&HashSet<String>>, now: u64) {
        let Some(path) = path else { return };
        let old = now.saturating_sub(CACHE_DAYS * 86_400);
        self.files.retain(|k, e| e.seen >= old && seen.is_none_or(|s| s.contains(k) || !rules::is_within(Path::new(k), root)));
        if self.files.len() > CACHE_MAX {
            let mut all: Vec<(String, CacheEntry)> = self.files.drain().collect();
            all.sort_by_key(|a| std::cmp::Reverse(a.1.seen));
            all.truncate(CACHE_MAX);
            self.files = all.into_iter().collect();
        }
        let file = CacheFile { version: CACHE_VERSION, files: self.files };
        let Ok(text) = serde_json::to_string(&file) else { return };
        if let Some(dir) = path.parent() {
            let _ = fs::create_dir_all(dir);
        }
        let tmp = path.with_extension("json.tmp");
        if fs::write(&tmp, text).is_ok() {
            let _ = fs::rename(&tmp, path);
        }
    }
}

// ---- moving extra copies ----

/// One extra copy to move, with the copy that stays.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct DupeRemoval {
    pub path: PathBuf,
    /// The size the check compared.
    pub size: u64,
    pub keep: PathBuf,
}

fn plain_file(p: &Path, size: u64) -> bool {
    p.symlink_metadata().is_ok_and(|m| {
        let f = sys::facts(&m);
        m.is_file() && m.len() == size && !f.reparse && !f.system && !f.not_local
    })
}

/// Whether a folder between the drive root and `p` is a junction, link or other reparse point.
/// A folder that cannot be read counts as one.
fn reparse_on_the_way(p: &Path) -> bool {
    let Some(parent) = p.parent() else { return true };
    let mut at = PathBuf::new();
    for c in parent.components() {
        at.push(c.as_os_str());
        if matches!(c, Component::Prefix(_) | Component::RootDir) {
            continue;
        }
        match at.symlink_metadata() {
            Ok(m) if m.file_type().is_symlink() || sys::is_reparse(&m) => return true,
            Ok(_) => {}
            Err(_) => return true,
        }
    }
    false
}

/// Whether `item` may go to the Recycle Bin, checked just before the move: a plain path that
/// is not on the never-touch list or in a system, `.git` or `node_modules` folder, no reparse
/// point on the way, still the same plain file, and its kept copy is still there, the same size,
/// and not in `batch`.
pub fn check_duplicate_removal(env: &RuleEnv, item: &DupeRemoval, batch: &[DupeRemoval]) -> Result<(), Refusal> {
    let p = &item.path;
    if !rules::is_normal(p) {
        return Err(Refusal::NotNormal);
    }
    if never_touch(env, p)
        || rules::system_dirs().iter().any(|d| rules::is_within(p, d))
        || p.components().any(|c| skipped_name(c.as_os_str()))
    {
        return Err(Refusal::NeverTouch);
    }
    if reparse_on_the_way(p) {
        return Err(Refusal::ReparsePoint);
    }
    if !plain_file(p, item.size) {
        return Err(Refusal::Changed);
    }
    if rules::same(p, &item.keep) || batch.iter().any(|b| rules::same(&b.path, &item.keep)) || !plain_file(&item.keep, item.size) {
        return Err(Refusal::NoCopyLeft);
    }
    Ok(())
}

/// Moves extra copies to the Recycle Bin. Each is checked with [`check_duplicate_removal`]
/// first; a refused one is reported as [`RecycleError::Refused`] and never reaches the shell.
pub fn recycle_duplicates(env: &RuleEnv, items: &[DupeRemoval]) -> RecycleReport {
    let mut refused = Vec::new();
    let mut allowed = Vec::new();
    for it in items {
        match check_duplicate_removal(env, it, items) {
            Ok(()) => allowed.push(it.path.clone()),
            Err(_) => refused.push((it.path.clone(), RecycleError::Refused)),
        }
    }
    let mut report = if allowed.is_empty() { RecycleReport::default() } else { move_to_recycle_bin(&allowed) };
    report.failed.extend(refused);
    report
}

#[cfg(test)]
mod tests {
    use super::*;

    fn f(path: &str, size: u64) -> DupeFile {
        DupeFile { path: PathBuf::from(path), size, modified: None }
    }

    #[test]
    fn size_groups_need_two_and_put_the_biggest_first() {
        let g = by_size(vec![f(r"C:\b", 5), f(r"C:\a", 5), f(r"C:\c", 9), f(r"C:\d", 9), f(r"C:\e", 9), f(r"C:\lonely", 7)]);
        assert_eq!(g.len(), 2);
        assert_eq!(g[0].iter().map(|x| x.size).collect::<Vec<_>>(), [9, 9, 9]);
        assert_eq!(g[1][0].path, PathBuf::from(r"C:\a"), "each group by path");
    }

    #[test]
    fn reclaimable_is_every_copy_but_one() {
        let g = DupeGroup { size: 100, files: vec![f("a", 100), f("b", 100), f("c", 100)] };
        assert_eq!(g.reclaimable(), 200);
    }

    #[test]
    fn the_cache_matches_only_the_same_size_and_write_time() {
        let mut c = Cache::default();
        let p = Path::new(r"C:\x\movie.mp4");
        c.put(p, 10, 77, Some([1; 32]), Some([2; 32]), 5);
        assert_eq!(c.get(p, 10, 77), (Some([1; 32]), Some([2; 32])));
        assert_eq!(c.get(p, 10, 78), (None, None), "written again");
        assert_eq!(c.get(p, 11, 77), (None, None), "another size");
        assert_eq!(unhex(&hex(&[9; 32])), Some([9; 32]));
    }

    #[test]
    fn filetime_reads_as_unix_time() {
        if cfg!(windows) {
            assert_eq!(filetime(116_444_736_000_000_000), Some(UNIX_EPOCH));
            assert_eq!(filetime(116_444_736_000_000_000 + 10_000_000), Some(UNIX_EPOCH + Duration::from_secs(1)));
            assert_eq!(filetime(5), None);
        }
    }
}
