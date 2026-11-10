//! What Disk shows and decides, with no windows or threads: sizes and words, the
//! items moved to the Recycle Bin this session (subtracted from the scan), one folder level's
//! rows, files and types, the squarified treemap, and the confirm sheet's list.

use std::cmp::Reverse;
use std::collections::{BTreeMap, BTreeSet, BinaryHeap};
use std::path::{Path, PathBuf};
use std::time::{Duration, SystemTime};

use app_disk::{Category, DriveSpace, NodeId, NodeKind, ScanError, Tree, TypeShare};

pub type Rect = (f32, f32, f32, f32);

const KB: f64 = 1024.0;
const MB: f64 = KB * 1024.0;
const GB: f64 = MB * 1024.0;
const TB: f64 = GB * 1024.0;

/// "438 GB", "41.8 GB", "640 MB", "12 KB". Binary units, as Explorer counts them, so the numbers
/// match what Windows shows for the same drive.
pub fn size(bytes: u64) -> String {
    let b = bytes as f64;
    if b >= TB {
        format!("{:.1} TB", b / TB)
    } else if b >= GB {
        format!("{:.1} GB", b / GB)
    } else if b >= MB {
        let v = b / MB;
        if v >= 100.0 {
            format!("{} MB", v.round() as u64)
        } else {
            format!("{v:.1} MB")
        }
    } else if bytes == 0 {
        "0 KB".into()
    } else {
        format!("{} KB", (b / KB).ceil() as u64)
    }
}

/// Drive totals in whole numbers ("438 GB used", "38 GB free of 476 GB"), one decimal below 10.
pub fn size_round(bytes: u64) -> String {
    let b = bytes as f64;
    let (v, unit) = if b >= TB {
        (b / TB, "TB")
    } else if b >= GB {
        (b / GB, "GB")
    } else {
        return size(bytes);
    };
    if v >= 10.0 {
        format!("{} {unit}", v.round() as u64)
    } else {
        format!("{v:.1} {unit}")
    }
}

/// "1,204,318".
pub fn count(n: u64) -> String {
    let s = n.to_string();
    let mut out = String::new();
    for (i, c) in s.chars().enumerate() {
        if i > 0 && (s.len() - i).is_multiple_of(3) {
            out.push(',');
        }
        out.push(c);
    }
    out
}

/// "0:52", "12:04", "1:02:09".
pub fn clock(d: Duration) -> String {
    let s = d.as_secs();
    if s >= 3600 {
        format!("{}:{:02}:{:02}", s / 3600, s / 60 % 60, s % 60)
    } else {
        format!("{}:{:02}", s / 60, s % 60)
    }
}

pub fn plural(n: u64, one: &str, many: &str) -> String {
    if n == 1 {
        format!("1 {one}")
    } else {
        format!("{} {many}", count(n))
    }
}

/// The drive letter of an absolute Windows path.
pub fn drive_letter(p: &Path) -> Option<char> {
    let s = p.to_string_lossy();
    let b = s.as_bytes();
    (b.len() >= 2 && b[1] == b':' && b[0].is_ascii_alphabetic())
        .then(|| (b[0] as char).to_ascii_uppercase())
}

pub fn drive_label(p: &Path) -> String {
    drive_letter(p).map_or_else(|| "this drive".into(), |c| format!("{c}:"))
}

/// `C:\` or `C:`.
pub fn is_drive_root(p: &Path) -> bool {
    let s = p.to_string_lossy();
    drive_letter(p).is_some() && s.trim_end_matches(['\\', '/']).len() == 2
}

/// "C:" for a drive, else the folder's own name.
pub fn place_name(p: &Path) -> String {
    if is_drive_root(p) {
        return drive_label(p);
    }
    p.file_name().map_or_else(
        || p.display().to_string(),
        |n| n.to_string_lossy().into_owned(),
    )
}

/// A local date and minute, built by the caller from the clock (the model reads no clock).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Stamp {
    pub year: i32,
    pub month: u32,
    pub day: u32,
    pub minute: u32,
}

const MONTHS: [&str; 12] = [
    "Jan", "Feb", "Mar", "Apr", "May", "Jun", "Jul", "Aug", "Sep", "Oct", "Nov", "Dec",
];

/// "Mar 2025", the modified column.
pub fn month_year(s: Stamp) -> String {
    format!("{} {}", MONTHS[(s.month.clamp(1, 12) - 1) as usize], s.year)
}

/// "8 Aug".
pub fn day_month(s: Stamp) -> String {
    format!("{} {}", s.day, MONTHS[(s.month.clamp(1, 12) - 1) as usize])
}

fn hhmm(minute: u32) -> String {
    format!("{:02}:{:02}", minute / 60, minute % 60)
}

/// "today at 09:12", or "6 Nov at 09:12".
pub fn when(at: Stamp, today: Stamp) -> String {
    let day = if (at.year, at.month, at.day) == (today.year, today.month, today.day) {
        "today".to_string()
    } else {
        day_month(at)
    };
    format!("{day} at {}", hhmm(at.minute))
}

// ---- the system's own words ----

/// What the bin, the file viewer and the system are called where Orby runs.
pub mod os {
    #[cfg(windows)]
    mod w {
        pub const BIN: &str = "Recycle Bin";
        pub const THE_BIN: &str = "the Recycle Bin";
        pub const VIEWER: &str = "Explorer";
        pub const OPEN_IN: &str = "Open in Explorer";
        pub const MOVE: &str = "Move to Recycle Bin";
        pub const SYSTEM: &str = "Windows";
        pub const PUT_IT_BACK: &str =
            "You can put it back from the Recycle Bin until it's emptied. Orby never empties it for you.";
        pub const PUT_THEM_BACK: &str =
            "You can put them back from the Recycle Bin until it's emptied. Orby never empties it for you.";
        pub const TEMP_FILES: &str = "Windows temp files";
        pub const TEMP_BODY: &str = "Windows makes new ones when needed. Files in use are skipped.";
        pub const BROWSER_BODY: &str =
            "Edge and Chrome load sites a bit slower for a day. You stay signed in.";
        pub const SKIPPED_BODY: &str =
            "Windows keeps them private, so what is in them is not counted. The scan went on past them.";
        pub const SYSTEM_BODY: &str =
            "Update leftovers and restore points. Windows manages these itself.";
    }
    #[cfg(not(windows))]
    mod w {
        pub const BIN: &str = "Trash";
        pub const THE_BIN: &str = "the Trash";
        pub const VIEWER: &str = "Finder";
        pub const OPEN_IN: &str = "Show in Finder";
        pub const MOVE: &str = "Move to Trash";
        pub const SYSTEM: &str = "macOS";
        pub const PUT_IT_BACK: &str =
            "You can put it back from the Trash until it's emptied. Orby never empties it for you.";
        pub const PUT_THEM_BACK: &str =
            "You can put them back from the Trash until it's emptied. Orby never empties it for you.";
        pub const TEMP_FILES: &str = "Temporary files";
        pub const TEMP_BODY: &str = "macOS makes new ones when needed. Files in use are skipped.";
        pub const BROWSER_BODY: &str =
            "Chrome and Edge load sites a bit slower for a day. You stay signed in.";
        pub const SKIPPED_BODY: &str =
            "macOS keeps them private until Orby has Full Disk Access, so what is in them is not counted. The scan went on past them.";
        pub const SYSTEM_BODY: &str = "Snapshots, caches and updates. macOS manages these itself.";
    }
    pub use w::*;
}

// ---- pills ----

/// The low-space pill: "C:" then "has 38 GB left".
pub fn low_space_words(drive: char, space: DriveSpace) -> (String, String) {
    (
        format!("{drive}:"),
        format!("has {} left", size_round(space.free)),
    )
}

/// How far a drive scan has got: bytes found on disk (allocated, so cloud placeholders add
/// nothing) over bytes in use. Hard links still count twice, so a scan still running never
/// claims 100%.
pub fn drive_percent(space: DriveSpace, on_disk: u64, running: bool) -> f64 {
    let p = space.scan_percent(on_disk);
    if running {
        p.min(99.0)
    } else {
        p
    }
}

/// The hover pill while scanning: "41% · 512,340 files", or the size so far for a folder.
pub fn scanning_value(percent: Option<f64>, counted: u64, files: u64) -> String {
    match percent {
        Some(p) => format!("{}% · {}", p.floor() as u64, plural(files, "file", "files")),
        None => format!("{} · {}", size(counted), plural(files, "file", "files")),
    }
}

/// Why a scan could not start, in the error pill's two tones ("D:" "is not ready") and the
/// window card's sentence.
pub fn scan_error_words(root: &Path, e: &ScanError) -> (String, String, &'static str) {
    let place = place_name(root);
    match e {
        // A drive letter that is not there is a drive that is unplugged or locked.
        ScanError::NotReady | ScanError::NotFound if is_drive_root(root) => (
            place,
            "is not ready".into(),
            if cfg!(windows) {
                "Plug the drive back in, or unlock it in Explorer, then scan again."
            } else {
                "Plug the disk back in, or unlock it in the Finder, then scan again."
            },
        ),
        ScanError::NotFound => (
            place,
            "is not there".into(),
            "The folder was moved or deleted. Pick it again, then scan.",
        ),
        ScanError::AccessDenied => (
            place,
            "can't be read".into(),
            if cfg!(windows) {
                "Windows keeps this folder private, so nothing in it was counted."
            } else {
                "macOS keeps this folder private, so nothing in it was counted. Full Disk Access lets Orby read it."
            },
        ),
        ScanError::NotADirectory => (
            place,
            "is not a folder".into(),
            "Pick a folder or a drive to scan.",
        ),
        ScanError::NotReady => (
            place,
            "is not ready".into(),
            if cfg!(windows) {
                "Plug the drive back in, or unlock it in Explorer, then scan again."
            } else {
                "Plug the disk back in, or unlock it in the Finder, then scan again."
            },
        ),
        ScanError::Io(_) => (
            place,
            "couldn't be read".into(),
            if cfg!(windows) {
                "Windows would not list it. Try again in a moment."
            } else {
                "macOS would not list it. Try again in a moment."
            },
        ),
    }
}

// ---- what moved to the Recycle Bin this session ----

/// Items moved out of the scanned tree. The tree itself never changes after a scan, so the window
/// subtracts these. Entries never overlap: a folder replaces anything moved from inside it.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Removed {
    items: BTreeMap<NodeId, ()>,
}

fn is_under(tree: &Tree, id: NodeId, ancestor: NodeId) -> bool {
    let mut at = tree.node(id).parent();
    while let Some(p) = at {
        if p == ancestor {
            return true;
        }
        at = tree.node(p).parent();
    }
    false
}

impl Removed {
    pub fn add(&mut self, tree: &Tree, id: NodeId) {
        if self.is_gone(tree, id) {
            return;
        }
        self.items.retain(|&r, _| !is_under(tree, r, id));
        self.items.insert(id, ());
    }

    /// Undo: the item is back where it was.
    pub fn restore(&mut self, id: NodeId) {
        self.items.remove(&id);
    }

    /// Moved, or inside something that was.
    pub fn is_gone(&self, tree: &Tree, id: NodeId) -> bool {
        self.items.contains_key(&id) || self.items.keys().any(|&r| is_under(tree, id, r))
    }

    fn under<'a>(&'a self, tree: &'a Tree, id: NodeId) -> impl Iterator<Item = NodeId> + 'a {
        self.items
            .keys()
            .copied()
            .filter(move |&r| is_under(tree, r, id))
    }

    /// The node's size with whatever moved from inside it taken off.
    pub fn size(&self, tree: &Tree, id: NodeId) -> u64 {
        let gone: u64 = self.under(tree, id).map(|r| tree.node(r).size()).sum();
        tree.node(id).size().saturating_sub(gone)
    }
}

// ---- one folder level ----

#[derive(Clone, Debug, PartialEq)]
pub struct Row {
    pub id: NodeId,
    pub name: String,
    pub size: u64,
    /// Share of the parent, 0 to 100.
    pub percent: f64,
    pub dir: bool,
    pub hidden: bool,
}

/// The children of `at`, biggest first, without what moved, with sizes after the move.
pub fn rows(tree: &Tree, removed: &Removed, at: NodeId) -> Vec<Row> {
    let total = removed.size(tree, at);
    let mut out: Vec<Row> = tree
        .children(at)
        .iter()
        .filter(|&&c| !removed.is_gone(tree, c))
        .map(|&c| {
            let n = tree.node(c);
            let size = removed.size(tree, c);
            Row {
                id: c,
                name: n.name().to_string_lossy().into_owned(),
                size,
                percent: if total == 0 {
                    0.0
                } else {
                    size as f64 * 100.0 / total as f64
                },
                dir: n.kind() == NodeKind::Dir,
                hidden: n.hidden(),
            }
        })
        .collect();
    out.sort_by(|a, b| {
        b.size
            .cmp(&a.size)
            .then_with(|| a.name.to_lowercase().cmp(&b.name.to_lowercase()))
    });
    out
}

/// The files and types under a folder, from one walk of its subtree.
#[derive(Clone, Debug, PartialEq)]
pub struct Summary {
    /// Biggest first.
    pub top: Vec<NodeId>,
    pub files: u64,
    pub bytes: u64,
    /// The five named categories biggest first, then Other last, as the engine orders them.
    pub types: Vec<TypeShare>,
}

pub fn summarize(tree: &Tree, removed: &Removed, at: NodeId, n: usize) -> Summary {
    let mut heap = BinaryHeap::with_capacity(n + 1);
    let mut types: Vec<TypeShare> = Category::ALL
        .iter()
        .map(|&category| TypeShare {
            category,
            bytes: 0,
            files: 0,
        })
        .collect();
    let (mut files, mut bytes) = (0u64, 0u64);
    let mut stack = vec![at];
    while let Some(id) = stack.pop() {
        if removed.items.contains_key(&id) {
            continue;
        }
        let node = tree.node(id);
        match node.kind() {
            NodeKind::Dir => stack.extend_from_slice(tree.children(id)),
            NodeKind::Link => {}
            NodeKind::File => {
                files += 1;
                bytes += node.size();
                let s = &mut types[Category::of(node.name()) as usize];
                s.bytes += node.size();
                s.files += 1;
                if n > 0 {
                    heap.push(Reverse((node.size(), Reverse(id))));
                    if heap.len() > n {
                        heap.pop();
                    }
                }
            }
        }
    }
    let mut top: Vec<(u64, NodeId)> = heap
        .into_iter()
        .map(|Reverse((s, Reverse(f)))| (s, f))
        .collect();
    top.sort_by(|a, b| b.0.cmp(&a.0).then(a.1.cmp(&b.1)));
    types.sort_by(|a, b| {
        (a.category == Category::Other)
            .cmp(&(b.category == Category::Other))
            .then(b.bytes.cmp(&a.bytes))
            .then((a.category as usize).cmp(&(b.category as usize)))
    });
    Summary {
        top: top.into_iter().map(|(_, f)| f).collect(),
        files,
        bytes,
        types,
    }
}

/// A file's folder below the scanned root, for the muted line in the Files tab ("AppData\Local").
pub fn folder_below(tree: &Tree, at: NodeId, file: NodeId) -> String {
    let chain = tree.ancestry(file);
    let from = chain.iter().position(|&n| n == at).map_or(0, |i| i + 1);
    let parts: Vec<String> = chain[from..chain.len().saturating_sub(1)]
        .iter()
        .map(|&n| tree.node(n).name().to_string_lossy().into_owned())
        .collect();
    parts.join("\\")
}

// ---- the treemap ----

/// Squarified layout (Bruls, Huizing, van Wijk): `values` biggest first, rects in the same order,
/// areas proportional to the values and filling `r`.
pub fn squarify(values: &[f64], r: Rect) -> Vec<Rect> {
    let mut out = vec![(r.0, r.1, r.0, r.1); values.len()];
    let total: f64 = values.iter().filter(|v| **v > 0.0).sum();
    let (w, h) = ((r.2 - r.0) as f64, (r.3 - r.1) as f64);
    if total <= 0.0 || w <= 0.0 || h <= 0.0 {
        return out;
    }
    let scale = w * h / total;
    let areas: Vec<f64> = values.iter().map(|v| v.max(0.0) * scale).collect();
    let mut rest = (r.0 as f64, r.1 as f64, r.2 as f64, r.3 as f64);
    let worst = |row: &[f64], side: f64| -> f64 {
        let s: f64 = row.iter().sum();
        let (mx, mn) = row
            .iter()
            .fold((0.0f64, f64::MAX), |(a, b), &v| (a.max(v), b.min(v)));
        if s <= 0.0 || mn <= 0.0 {
            return f64::MAX;
        }
        (side * side * mx / (s * s)).max(s * s / (side * side * mn))
    };
    let mut i = 0;
    let n = areas.iter().rposition(|a| *a > 0.0).map_or(0, |p| p + 1);
    while i < n {
        let side = (rest.2 - rest.0).min(rest.3 - rest.1);
        let mut j = i + 1;
        while j < n && worst(&areas[i..=j], side) <= worst(&areas[i..j], side) {
            j += 1;
        }
        let row = &areas[i..j];
        let s: f64 = row.iter().sum();
        let (rw, rh) = (rest.2 - rest.0, rest.3 - rest.1);
        if rw >= rh {
            // A column on the left.
            let cw = if j == n { rw } else { s / rh };
            let mut y = rest.1;
            for (k, a) in row.iter().enumerate() {
                let hh = if k + 1 == row.len() {
                    rest.3 - y
                } else {
                    a / cw
                };
                out[i + k] = (
                    rest.0 as f32,
                    y as f32,
                    (rest.0 + cw) as f32,
                    (y + hh) as f32,
                );
                y += hh;
            }
            rest.0 += cw;
        } else {
            // A row along the top.
            let rhh = if j == n { rh } else { s / rw };
            let mut x = rest.0;
            for (k, a) in row.iter().enumerate() {
                let ww = if k + 1 == row.len() {
                    rest.2 - x
                } else {
                    a / rhh
                };
                out[i + k] = (
                    x as f32,
                    rest.1 as f32,
                    (x + ww) as f32,
                    (rest.1 + rhh) as f32,
                );
                x += ww;
            }
            rest.1 += rhh;
        }
        i = j;
    }
    out
}

#[derive(Clone, Debug, PartialEq)]
pub struct Tile {
    /// The row it shows, or None for the "6 more" tile.
    pub row: Option<usize>,
    pub rect: Rect,
    /// Inner tiles for the folder's own children (one level), with the inner names and sizes.
    pub inner: Vec<(Rect, String, u64)>,
    /// How many rows the "more" tile holds.
    pub more: usize,
}

/// At most this many tiles, then one "N more" tile for the rest.
pub const MAX_TILES: usize = 9;
/// Tiles smaller than this share of the level go into "N more".
const MIN_SHARE: f64 = 0.012;

/// The tiles for one level. `inner_of` gives a folder's children (name, size), biggest first.
pub fn treemap(
    rows: &[Row],
    r: Rect,
    inner_of: &dyn Fn(NodeId) -> Vec<(String, u64)>,
) -> Vec<Tile> {
    let total: u64 = rows.iter().map(|x| x.size).sum();
    if total == 0 {
        return Vec::new();
    }
    let mut shown = rows
        .iter()
        .take(MAX_TILES)
        .take_while(|x| x.size as f64 / total as f64 >= MIN_SHARE)
        .count();
    if shown == 0 {
        shown = 1;
    }
    let rest: u64 = rows[shown..].iter().map(|x| x.size).sum();
    let mut values: Vec<f64> = rows[..shown].iter().map(|x| x.size as f64).collect();
    if rest > 0 {
        values.push(rest as f64);
    }
    let rects = squarify(&values, r);
    rects
        .into_iter()
        .enumerate()
        .map(|(i, rect)| {
            if i >= shown {
                return Tile {
                    row: None,
                    rect,
                    inner: Vec::new(),
                    more: rows.len() - shown,
                };
            }
            let row = &rows[i];
            // Room for the name and size on top, then nested tiles under it.
            let (w, h) = (rect.2 - rect.0, rect.3 - rect.1);
            let inner = if row.dir && w >= 88.0 && h >= 96.0 {
                let kids: Vec<(String, u64)> = inner_of(row.id)
                    .into_iter()
                    .filter(|k| k.1 > 0)
                    .take(6)
                    .collect();
                let area = (rect.0 + 6.0, rect.1 + 42.0, rect.2 - 6.0, rect.3 - 6.0);
                let vals: Vec<f64> = kids.iter().map(|k| k.1 as f64).collect();
                // Scale to the inner area by the folder's whole size, so small leftovers stay empty.
                let sum: f64 = vals.iter().sum();
                let frac = if row.size > 0 {
                    (sum / row.size as f64).min(1.0)
                } else {
                    0.0
                };
                let ih = (area.3 - area.1) * frac as f32;
                let area = (area.0, area.1, area.2, area.1 + ih);
                squarify(&vals, area)
                    .into_iter()
                    .zip(kids)
                    .filter(|(r, _)| r.2 - r.0 >= 4.0 && r.3 - r.1 >= 4.0)
                    .map(|(r, (n, s))| (r, n, s))
                    .collect()
            } else {
                Vec::new()
            };
            Tile {
                row: Some(i),
                rect,
                inner,
                more: 0,
            }
        })
        .collect()
}

/// Tile fill strength, darker for bigger: 0.10 for the biggest down to 0.035.
pub fn tile_alpha(size: u64, biggest: u64) -> f32 {
    if biggest == 0 {
        return 0.035;
    }
    0.035 + 0.065 * (size as f64 / biggest as f64).sqrt() as f32
}

// ---- the confirm sheet ----

/// Up to five items are listed; more collapse to "and 12 more".
pub const CONFIRM_LIST: usize = 5;

pub fn confirm_title(n: usize) -> String {
    if n == 1 {
        format!("Move 1 item to {}?", os::THE_BIN)
    } else {
        format!("Move {n} items to {}?", os::THE_BIN)
    }
}

pub fn confirm_more(n: usize) -> Option<String> {
    (n > CONFIRM_LIST).then(|| format!("and {} more", n - CONFIRM_LIST))
}

/// "You can put it back" for one item, "them" for more.
pub fn put_back_words(n: usize) -> &'static str {
    match (n == 1, cfg!(windows)) {
        (true, true) => "You can put it back from the Recycle Bin until it is emptied.",
        (false, true) => "You can put them back from the Recycle Bin until it is emptied.",
        (true, false) => "You can put it back from the Trash until it is emptied.",
        (false, false) => "You can put them back from the Trash until it is emptied.",
    }
}

pub fn confirm_honest(total: u64) -> String {
    format!("The {} comes back only when {} is emptied. Orby never empties it for you, and has no permanent delete.", size(total), os::THE_BIN)
}

/// What a move reported, in the card's words. `in_use` names the first file that stayed.
pub fn moved_words(
    moved: usize,
    in_use: &[PathBuf],
    other_failed: usize,
) -> Option<(String, String)> {
    if in_use.is_empty() && other_failed == 0 {
        return None;
    }
    let name = |p: &PathBuf| {
        p.file_name().map_or_else(
            || p.display().to_string(),
            |n| n.to_string_lossy().into_owned(),
        )
    };
    let title = if in_use.is_empty() {
        plural(other_failed as u64, "item stayed", "items stayed")
    } else {
        plural(in_use.len() as u64, "file is in use", "files are in use")
    };
    let first = in_use
        .first()
        .map(|p| format!("{} is open in another app.", name(p)))
        .unwrap_or_else(|| format!("{} would not move it.", os::SYSTEM));
    let tail = match moved {
        0 => String::new(),
        1 => " The other item was moved.".into(),
        n => format!(" The other {n} items were moved."),
    };
    Some((title, format!("{first}{tail}")))
}

/// "In Recycle Bin" and "2 items".
pub fn in_bin_words(n: usize) -> (String, String) {
    (format!("In {}", os::BIN), plural(n as u64, "item", "items"))
}

#[allow(unused_imports)] // Until the Disk window redraw uses them (see `mod cleaning`).
pub use cleaning::*;

/// Unfinished-download labels and the cleaning-rules screen (see the comment inside).
// The Disk window redraw wires these in; until it does, only the tests call them.
#[allow(dead_code)]
mod cleaning {
    use super::{confirm_honest, confirm_title, count, size};
    use std::path::PathBuf;

    // ---- large files ----

    /// The muted label a Files row shows instead of its folder, when its name says it is an
    /// unfinished download (`.crdownload`, `.part`, `.tmp`): "Unfinished download".
    pub fn file_label(name: &std::ffi::OsStr) -> Option<&'static str> {
        app_disk::is_unfinished_download(name).then_some(app_disk::UNFINISHED_DOWNLOAD)
    }

    // ---- cleaning rules (Developer caches, App caches, Browser caches, AI models, Crash reports) ----
    //
    // How the window uses this:
    //
    // 1. Measure off the UI thread: `app_disk::RuleMeasure::start(app_disk::RuleEnv::from_process())`,
    //    poll `try_take()` each frame (or `cancel()` / drop it to stop). It yields
    //    `app_disk::Measured { found, complete }`.
    // 2. `CleanList::new(measured.found)` builds the groups. Only rules whose folders exist and hold
    //    bytes are present; nothing is selected.
    // 3. Filter chips: `list.chips()` gives ("All 12", "Safe to remove 8", "Check first 4") with
    //    their `TierFilter`; `list.groups_for(filter)` gives the groups to draw, empty groups dropped.
    //    Each `CleanItem` row: `name` (500), `detail` (the plain explanation, 12/16 muted),
    //    `tier.label()` as the honesty tag ("Safe to remove" filled, "Check first" outline),
    //    `size(item.size)` on the right, a checkbox from `selected`, and `paths` for Open in Explorer.
    // 4. Space or a click: `list.toggle(id)`. "Select all safe" is
    //    `list.select_tier(TierFilter::Safe, true)`; Check first rows are only ever selected one by one.
    // 5. Selection bar: `list.selection_words()` -> ("3 selected", "11.3 GB"), or
    //    `list.selection_line()` -> "3 selected · 11.3 GB". `None` when nothing is selected.
    // 6. Confirm sheet: `list.confirm()` -> `CleanConfirm` with `title`, `sections` (Check first first,
    //    then Safe to remove, each with its items and bytes), `bytes`, and `honest`.
    // 7. Move: `app_disk::recycle_rule_items(&env, &list.removal())` on a worker thread. It checks every
    //    path again (never-touch list, inside its rule, no junctions) and only ever recycles.
    //    Then `list.forget(&report.moved)` drops what went.

    pub use app_disk::{Group, Tier};

    /// Which rows the filter chips show.
    #[derive(Clone, Copy, Debug, PartialEq, Eq)]
    pub enum TierFilter {
        All,
        Safe,
        CheckFirst,
    }

    impl TierFilter {
        pub fn admits(self, t: Tier) -> bool {
            match self {
                TierFilter::All => true,
                TierFilter::Safe => t == Tier::SafeToRemove,
                TierFilter::CheckFirst => t == Tier::CheckFirst,
            }
        }

        pub fn label(self) -> &'static str {
            match self {
                TierFilter::All => "All",
                TierFilter::Safe => Tier::SafeToRemove.label(),
                TierFilter::CheckFirst => Tier::CheckFirst.label(),
            }
        }
    }

    /// One row: one cleaning rule with what it found.
    #[derive(Clone, Debug, PartialEq)]
    pub struct CleanItem {
        /// The rule id ("npm-cache"); stable for selection.
        pub id: &'static str,
        pub rule: &'static app_disk::Rule,
        /// "npm cache".
        pub name: &'static str,
        /// The plain explanation: what removing it costs and how it comes back.
        pub detail: &'static str,
        pub tier: Tier,
        /// Bytes across `paths`.
        pub size: u64,
        /// The folders that would move, biggest first. Usually one.
        pub paths: Vec<PathBuf>,
        /// Bytes of each of `paths`, in the same order.
        pub path_sizes: Vec<u64>,
        pub selected: bool,
    }

    /// One group heading ("Developer caches") and its rows, biggest first.
    #[derive(Clone, Debug, PartialEq)]
    pub struct CleanGroup {
        pub group: Group,
        /// "Developer caches".
        pub title: &'static str,
        /// Bytes of the rows in this group (of the rows shown, for `groups_for`).
        pub size: u64,
        pub items: Vec<CleanItem>,
    }

    /// Rows per tier, for the filter chips.
    #[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
    pub struct TierCounts {
        pub all: usize,
        pub safe: usize,
        pub check_first: usize,
    }

    /// One block of the confirm sheet.
    #[derive(Clone, Debug, PartialEq)]
    pub struct ConfirmSection {
        pub tier: Tier,
        /// "Check first" or "Safe to remove".
        pub title: &'static str,
        /// (name, bytes), biggest first.
        pub items: Vec<(&'static str, u64)>,
        pub bytes: u64,
    }

    /// The confirm sheet for the selected rows.
    #[derive(Clone, Debug, PartialEq)]
    pub struct CleanConfirm {
        /// "Move 3 items to the Recycle Bin?"
        pub title: String,
        /// Check first first, then Safe to remove; empty sections are left out.
        pub sections: Vec<ConfirmSection>,
        pub count: usize,
        pub bytes: u64,
        /// The honest muted line about when the space comes back.
        pub honest: String,
    }

    /// Everything the cleaning-rules screen shows.
    #[derive(Clone, Debug, Default, PartialEq)]
    pub struct CleanList {
        /// In `Group::ALL` order, only groups with rows.
        pub groups: Vec<CleanGroup>,
    }

    impl CleanList {
        /// From the engine's measure. Rows with no bytes are dropped; nothing is selected.
        pub fn new(found: Vec<app_disk::RuleFound>) -> Self {
            let mut groups: Vec<CleanGroup> = Vec::new();
            for g in Group::ALL {
                let mut items: Vec<CleanItem> = found
                    .iter()
                    .filter(|f| f.rule.group == g && f.total() > 0)
                    .map(|f| CleanItem {
                        id: f.rule.id,
                        rule: f.rule,
                        name: f.rule.name,
                        detail: f.rule.explanation,
                        tier: f.rule.tier,
                        size: f.total(),
                        paths: f.items.iter().map(|c| c.path.clone()).collect(),
                        path_sizes: f.items.iter().map(|c| c.size).collect(),
                        selected: false,
                    })
                    .collect();
                if items.is_empty() {
                    continue;
                }
                items.sort_by(|a, b| b.size.cmp(&a.size).then_with(|| a.name.cmp(b.name)));
                groups.push(CleanGroup {
                    group: g,
                    title: g.label(),
                    size: items.iter().map(|i| i.size).sum(),
                    items,
                });
            }
            CleanList { groups }
        }

        pub fn items(&self) -> impl Iterator<Item = &CleanItem> {
            self.groups.iter().flat_map(|g| g.items.iter())
        }

        fn items_mut(&mut self) -> impl Iterator<Item = &mut CleanItem> {
            self.groups.iter_mut().flat_map(|g| g.items.iter_mut())
        }

        pub fn is_empty(&self) -> bool {
            self.groups.is_empty()
        }

        /// Bytes of every row.
        pub fn total(&self) -> u64 {
            self.items().map(|i| i.size).sum()
        }

        pub fn counts(&self) -> TierCounts {
            let mut c = TierCounts::default();
            for i in self.items() {
                c.all += 1;
                match i.tier {
                    Tier::SafeToRemove => c.safe += 1,
                    Tier::CheckFirst => c.check_first += 1,
                }
            }
            c
        }

        /// The three chips with their words: "All 12", "Safe to remove 8", "Check first 4".
        pub fn chips(&self) -> [(TierFilter, String); 3] {
            let c = self.counts();
            [
                (TierFilter::All, format!("All {}", c.all)),
                (
                    TierFilter::Safe,
                    format!("{} {}", TierFilter::Safe.label(), c.safe),
                ),
                (
                    TierFilter::CheckFirst,
                    format!("{} {}", TierFilter::CheckFirst.label(), c.check_first),
                ),
            ]
        }

        /// The groups to draw under a chip; groups left with no rows are dropped.
        pub fn groups_for(&self, filter: TierFilter) -> Vec<CleanGroup> {
            self.groups
                .iter()
                .filter_map(|g| {
                    let items: Vec<CleanItem> = g
                        .items
                        .iter()
                        .filter(|i| filter.admits(i.tier))
                        .cloned()
                        .collect();
                    (!items.is_empty()).then(|| CleanGroup {
                        size: items.iter().map(|i| i.size).sum(),
                        items,
                        ..g.clone()
                    })
                })
                .collect()
        }

        /// Flips one row. Returns the new state, or None for an unknown id.
        pub fn toggle(&mut self, id: &str) -> Option<bool> {
            let item = self.items_mut().find(|i| i.id == id)?;
            item.selected = !item.selected;
            Some(item.selected)
        }

        pub fn set_selected(&mut self, id: &str, on: bool) {
            if let Some(item) = self.items_mut().find(|i| i.id == id) {
                item.selected = on;
            }
        }

        /// Selects or clears every Safe to remove row (`TierFilter::Safe`), or clears rows under any
        /// filter. Check first rows are never selected in bulk.
        pub fn select_tier(&mut self, filter: TierFilter, on: bool) {
            for i in self.items_mut().filter(|i| filter.admits(i.tier)) {
                if !on || i.tier == Tier::SafeToRemove {
                    i.selected = on;
                }
            }
        }

        /// (rows selected, their bytes).
        pub fn selection(&self) -> (usize, u64) {
            self.items()
                .filter(|i| i.selected)
                .fold((0, 0), |(n, b), i| (n + 1, b + i.size))
        }

        /// ("3 selected", "11.3 GB"), the selection bar's two tones.
        pub fn selection_words(&self) -> Option<(String, String)> {
            let (n, b) = self.selection();
            (n > 0).then(|| (format!("{} selected", count(n as u64)), size(b)))
        }

        /// "3 selected · 11.3 GB".
        pub fn selection_line(&self) -> Option<String> {
            self.selection_words().map(|(a, b)| format!("{a} · {b}"))
        }

        /// The confirm sheet: Check first first, then Safe to remove.
        pub fn confirm(&self) -> CleanConfirm {
            let sections: Vec<ConfirmSection> = [Tier::CheckFirst, Tier::SafeToRemove]
                .into_iter()
                .filter_map(|tier| {
                    let mut items: Vec<(&'static str, u64)> = self
                        .items()
                        .filter(|i| i.selected && i.tier == tier)
                        .map(|i| (i.name, i.size))
                        .collect();
                    items.sort_by(|a, b| b.1.cmp(&a.1).then_with(|| a.0.cmp(b.0)));
                    (!items.is_empty()).then(|| ConfirmSection {
                        tier,
                        title: tier.label(),
                        bytes: items.iter().map(|i| i.1).sum(),
                        items,
                    })
                })
                .collect();
            let count: usize = sections.iter().map(|s| s.items.len()).sum();
            let bytes: u64 = sections.iter().map(|s| s.bytes).sum();
            CleanConfirm {
                title: confirm_title(count),
                sections,
                count,
                bytes,
                honest: confirm_honest(bytes),
            }
        }

        /// What to hand `app_disk::recycle_rule_items`: each selected row's folders with its rule.
        pub fn removal(&self) -> Vec<(&'static app_disk::Rule, PathBuf)> {
            self.items()
                .filter(|i| i.selected)
                .flat_map(|i| i.paths.iter().map(move |p| (i.rule, p.clone())))
                .collect()
        }

        /// After a move: drops the folders that went and their bytes. A row whose folders all went
        /// is dropped; one with folders left keeps them, still selected, so Try again works.
        pub fn forget(&mut self, moved: &[PathBuf]) {
            for g in &mut self.groups {
                for i in &mut g.items {
                    let kept: Vec<(PathBuf, u64)> = i
                        .paths
                        .drain(..)
                        .zip(i.path_sizes.drain(..))
                        .filter(|(p, _)| !moved.contains(p))
                        .collect();
                    i.size = kept.iter().map(|k| k.1).sum();
                    (i.paths, i.path_sizes) = kept.into_iter().unzip();
                }
                g.items.retain(|i| !i.paths.is_empty());
                g.size = g.items.iter().map(|i| i.size).sum();
            }
            self.groups.retain(|g| !g.items.is_empty());
        }
    }
}

// ---- duplicates ----

/// Folders whose copy is never preferred as the keeper while another copy lives elsewhere.
const PASSING_THROUGH: [&str; 3] = ["Downloads", "Temp", "Desktop"];

/// In Downloads, Temp or Desktop (any folder of that name on the way).
pub fn passing_through(p: &Path) -> bool {
    p.components().any(|c| {
        let n = c.as_os_str().to_string_lossy();
        PASSING_THROUGH.iter().any(|t| t.eq_ignore_ascii_case(&n))
    })
}

/// One copy of a duplicated file. `gone` once it was moved to the Recycle Bin (from here or
/// from Results), until Undo puts it back.
#[derive(Clone, Debug, PartialEq)]
pub struct DupeCopy {
    pub path: PathBuf,
    pub modified: Option<SystemTime>,
    pub gone: bool,
    /// What moving it frees, when the file system says (APFS: a clone frees close to nothing).
    pub frees: Option<u64>,
}

/// One group of identical files with its keeper and what is picked for removal. The keeper is
/// always a copy that is still there and is never picked, so a group can never have every copy
/// picked: every way to change the picks goes through the methods below.
#[derive(Clone, Debug)]
pub struct DupeSet {
    pub size: u64,
    pub copies: Vec<DupeCopy>,
    keep: usize,
    picked: BTreeSet<usize>,
}

/// The copy to keep: not in Downloads, Temp or Desktop when another copy lives elsewhere, then
/// the oldest, then the first by path. Only copies still there count.
pub fn default_keeper(copies: &[DupeCopy]) -> usize {
    (0..copies.len())
        .filter(|&i| !copies[i].gone)
        .min_by(|&a, &b| {
            let (x, y) = (&copies[a], &copies[b]);
            passing_through(&x.path)
                .cmp(&passing_through(&y.path))
                .then_with(|| match (x.modified, y.modified) {
                    (Some(p), Some(q)) => p.cmp(&q),
                    (Some(_), None) => std::cmp::Ordering::Less,
                    (None, Some(_)) => std::cmp::Ordering::Greater,
                    (None, None) => std::cmp::Ordering::Equal,
                })
                .then_with(|| x.path.cmp(&y.path))
        })
        .unwrap_or(0)
}

impl DupeSet {
    pub fn new(size: u64, copies: Vec<DupeCopy>) -> DupeSet {
        let keep = default_keeper(&copies);
        DupeSet {
            size,
            copies,
            keep,
            picked: BTreeSet::new(),
        }
    }

    pub fn live(&self) -> usize {
        self.copies.iter().filter(|c| !c.gone).count()
    }

    /// Two or more copies still there: the group shows.
    pub fn shown(&self) -> bool {
        self.live() >= 2
    }

    pub fn keeper(&self) -> usize {
        self.keep
    }

    pub fn is_picked(&self, i: usize) -> bool {
        self.picked.contains(&i)
    }

    pub fn picked(&self) -> impl Iterator<Item = usize> + '_ {
        self.picked.iter().copied()
    }

    /// What one copy frees: what the file system says (APFS), else its size.
    pub fn frees(&self, i: usize) -> u64 {
        self.copies[i].frees.unwrap_or(self.size)
    }

    /// What moving every live copy but the keeper frees: (live - 1) × size, or on APFS what
    /// those copies say they free (a clone of the keeper frees close to nothing).
    pub fn reclaimable(&self) -> u64 {
        if self.live() < 2 {
            return 0;
        }
        (0..self.copies.len())
            .filter(|&i| i != self.keep && !self.copies[i].gone)
            .map(|i| self.frees(i))
            .sum()
    }

    /// Some copies share their blocks with another file (APFS clones, or a snapshot holds them).
    pub fn shares_space(&self) -> bool {
        self.copies
            .iter()
            .any(|c| !c.gone && c.frees.is_some_and(|b| b < self.size / 2))
    }

    /// One click: this copy stays. A copy that is gone cannot be the keeper.
    pub fn set_keeper(&mut self, i: usize) -> bool {
        if self.copies.get(i).is_none_or(|c| c.gone) {
            return false;
        }
        self.keep = i;
        self.picked.remove(&i);
        true
    }

    /// Picks or unpicks a copy. The keeper and copies that are gone never toggle.
    pub fn toggle(&mut self, i: usize) -> bool {
        if i == self.keep || self.copies.get(i).is_none_or(|c| c.gone) {
            return false;
        }
        if !self.picked.remove(&i) {
            self.picked.insert(i);
        }
        true
    }

    /// Every copy but the keeper, or none.
    pub fn pick_extras(&mut self, on: bool) {
        self.picked = if on && self.shown() {
            (0..self.copies.len())
                .filter(|&i| i != self.keep && !self.copies[i].gone)
                .collect()
        } else {
            BTreeSet::new()
        };
    }

    pub fn all_extras_picked(&self) -> bool {
        self.shown() && self.picked.len() + 1 == self.live()
    }

    /// Copies moved to the Recycle Bin (`gone`) or put back, on their own or with a folder that
    /// holds them. A keeper that went gets a new one by the rule; picks never outlive their copy.
    pub fn mark(&mut self, paths: &[PathBuf], gone: bool) {
        for c in &mut self.copies {
            if paths.iter().any(|p| c.path.starts_with(p)) {
                c.gone = gone;
            }
        }
        let copies = &self.copies;
        self.picked.retain(|&i| !copies[i].gone);
        if self.copies[self.keep].gone {
            self.keep = default_keeper(&self.copies);
            self.picked.remove(&self.keep);
        }
        if !self.shown() {
            self.picked.clear();
        }
    }
}

/// The groups a check found, most to reclaim first (the check's order, kept while copies go).
#[derive(Clone, Debug, Default)]
pub struct DupeList {
    pub sets: Vec<DupeSet>,
}

impl DupeList {
    pub fn new(groups: &[app_disk::DupeGroup]) -> DupeList {
        DupeList {
            sets: groups
                .iter()
                .map(|g| {
                    let copies = g
                        .files
                        .iter()
                        .map(|f| DupeCopy {
                            path: f.path.clone(),
                            modified: f.modified,
                            gone: false,
                            frees: f.frees,
                        })
                        .collect();
                    DupeSet::new(g.size, copies)
                })
                .collect(),
        }
    }

    /// Indexes of the groups that show (two or more copies still there).
    pub fn shown(&self) -> Vec<usize> {
        (0..self.sets.len())
            .filter(|&i| self.sets[i].shown())
            .collect()
    }

    pub fn reclaimable(&self) -> u64 {
        self.sets.iter().map(DupeSet::reclaimable).sum()
    }

    /// Picked copies and their bytes.
    pub fn selection(&self) -> (usize, u64) {
        self.sets.iter().fold((0, 0), |(n, b), s| {
            let bytes: u64 = s.picked.iter().map(|&i| s.frees(i)).sum();
            (n + s.picked.len(), b + bytes)
        })
    }

    pub fn pick_extras(&mut self, on: bool) {
        for s in &mut self.sets {
            s.pick_extras(on);
        }
    }

    pub fn all_extras_picked(&self) -> bool {
        let shown = self.shown();
        !shown.is_empty() && shown.iter().all(|&i| self.sets[i].all_extras_picked())
    }

    /// What the confirm moves: (copy, size, the copy that stays).
    pub fn removals(&self) -> Vec<(PathBuf, u64, PathBuf)> {
        self.sets
            .iter()
            .flat_map(|s| {
                let keep = s.copies[s.keep].path.clone();
                s.picked()
                    .map(move |i| (s.copies[i].path.clone(), s.frees(i), keep.clone()))
            })
            .collect()
    }

    pub fn mark(&mut self, paths: &[PathBuf], gone: bool) {
        for s in &mut self.sets {
            s.mark(paths, gone);
        }
    }

    pub fn clear(&mut self) {
        self.pick_extras(false);
    }
}

/// The running check's stage, as a verb with an ellipsis.
pub fn dupe_stage_words(stage: app_disk::DupeStage) -> &'static str {
    match stage {
        app_disk::DupeStage::Sizes => "Matching sizes…",
        app_disk::DupeStage::Ends => "Comparing the start and end of each file…",
        app_disk::DupeStage::Contents => "Comparing whole files…",
    }
}

/// "1,204 of 3,400 files · 1.2 GB of 4.5 GB", and how far along the bar is.
pub fn dupe_progress_words(p: &app_disk::DupeProgress) -> (String, f32) {
    let files = format!(
        "{} of {}",
        count(p.files_done),
        plural(p.files_total, "file", "files")
    );
    let frac = if p.bytes_total > 0 {
        p.bytes_done as f32 / p.bytes_total as f32
    } else if p.files_total > 0 {
        p.files_done as f32 / p.files_total as f32
    } else {
        0.0
    };
    if p.bytes_total > 0 {
        (
            format!(
                "{files} · {} of {}",
                short_size(p.bytes_done),
                short_size(p.bytes_total)
            ),
            frac.clamp(0.0, 1.0),
        )
    } else {
        (files, frac.clamp(0.0, 1.0))
    }
}

/// One decimal with ".0" dropped, as the Disk window shows sizes.
fn short_size(bytes: u64) -> String {
    size(bytes).replacen(".0 ", " ", 1)
}

/// "3 copies · 1.2 GB to reclaim".
pub fn dupe_group_words(copies: usize, reclaimable: u64, shares: bool) -> String {
    format!(
        "{} · {} to reclaim{}",
        plural(copies as u64, "copy", "copies"),
        short_size(reclaimable),
        if shares {
            " · shares space (APFS clone)"
        } else {
            ""
        }
    )
}

/// What a check costs, honestly and roughly: by drive type, not a timer.
pub fn dupe_cost_words(kind: Option<app_disk::DriveKind>) -> &'static str {
    match kind {
        Some(app_disk::DriveKind::Ssd) => {
            "Reads files to compare them. On this SSD it usually takes under a minute."
        }
        Some(app_disk::DriveKind::Hdd) => {
            "Reads files to compare them. On this hard drive it can take minutes."
        }
        _ => "Reads files to compare them. SSD: usually under a minute · hard drive: can take minutes.",
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn sizes_read_like_explorer() {
        assert_eq!(size_round(476 * 1024 * 1024 * 1024), "476 GB");
        assert_eq!(size((182.4 * GB) as u64), "182.4 GB");
        assert_eq!(size((41.8 * GB) as u64), "41.8 GB");
        assert_eq!(size((5.9 * GB) as u64), "5.9 GB");
        assert_eq!(size(640 * 1024 * 1024), "640 MB");
        assert_eq!(size(1), "1 KB");
        assert_eq!(size(0), "0 KB");
        assert_eq!(count(1_204_318), "1,204,318");
        assert_eq!(count(999), "999");
        assert_eq!(clock(Duration::from_secs(52)), "0:52");
        assert_eq!(clock(Duration::from_secs(3729)), "1:02:09");
    }

    #[test]
    fn drives_and_folders_are_named_for_people() {
        assert!(is_drive_root(Path::new(r"C:\")));
        assert!(is_drive_root(Path::new("d:")));
        assert!(!is_drive_root(Path::new(r"C:\Users")));
        assert_eq!(place_name(Path::new(r"C:\")), "C:");
        assert_eq!(
            place_name(Path::new(r"C:\Users\dopamine\dev\clippy")),
            "clippy"
        );
        assert_eq!(drive_label(Path::new(r"e:\x")), "E:");
        let (a, b) = low_space_words(
            'C',
            DriveSpace {
                total: 476 * 1024 * 1024 * 1024,
                free: 38 * 1024 * 1024 * 1024,
            },
        );
        assert_eq!((a.as_str(), b.as_str()), ("C:", "has 38 GB left"));
    }

    #[test]
    fn a_running_drive_scan_never_reads_100_percent() {
        let s = DriveSpace {
            total: 1000 << 30,
            free: 80 << 30,
        };
        assert_eq!(drive_percent(s, 460 << 30, true).round(), 50.0);
        // More counted than in use (hard links): 99 while running, 100 once it has ended.
        assert_eq!(drive_percent(s, 990 << 30, true), 99.0);
        assert_eq!(drive_percent(s, 990 << 30, false), 100.0);
    }

    #[test]
    fn a_missing_drive_is_not_ready_but_a_missing_folder_is_not_there() {
        let (place, value, _) = scan_error_words(Path::new(r"Q:\"), &ScanError::NotFound);
        assert_eq!((place.as_str(), value.as_str()), ("Q:", "is not ready"));
        let (place, value, _) = scan_error_words(Path::new(r"C:\gone"), &ScanError::NotFound);
        assert_eq!((place.as_str(), value.as_str()), ("gone", "is not there"));
    }

    #[test]
    fn the_scan_stamp_says_today_or_the_date() {
        let t = Stamp {
            year: 2026,
            month: 11,
            day: 7,
            minute: 9 * 60 + 12,
        };
        assert_eq!(when(t, t), "today at 09:12");
        assert_eq!(when(Stamp { day: 6, ..t }, t), "6 Nov at 09:12");
        assert_eq!(
            month_year(Stamp {
                month: 3,
                year: 2025,
                ..t
            }),
            "Mar 2025"
        );
    }

    fn area(r: &Rect) -> f64 {
        ((r.2 - r.0) * (r.3 - r.1)) as f64
    }

    #[test]
    fn squarified_tiles_fill_the_box_in_proportion() {
        let vals = [64.2, 41.8, 33.5, 18.3, 12.6, 6.4, 5.6];
        let r = (0.0, 0.0, 336.0, 252.0);
        let rects = squarify(&vals, r);
        let total: f64 = vals.iter().sum();
        let all: f64 = rects.iter().map(area).sum();
        assert!(
            (all - 336.0 * 252.0).abs() < 1.0,
            "the tiles fill the box: {all}"
        );
        for (v, rc) in vals.iter().zip(&rects) {
            let want = v / total * 336.0 * 252.0;
            assert!(
                (area(rc) - want).abs() / want < 0.01,
                "{v}: {} vs {want}",
                area(rc)
            );
            assert!(rc.0 >= -0.01 && rc.1 >= -0.01 && rc.2 <= 336.01 && rc.3 <= 252.01);
        }
        // Squarified: no sliver tiles for this mix.
        for rc in &rects {
            let (w, h) = (rc.2 - rc.0, rc.3 - rc.1);
            assert!(w.max(h) / w.min(h) < 4.0, "{rc:?}");
        }
    }

    #[test]
    fn empty_and_zero_values_give_empty_tiles() {
        assert!(squarify(&[], (0.0, 0.0, 10.0, 10.0)).is_empty());
        let r = squarify(&[5.0, 0.0], (0.0, 0.0, 10.0, 10.0));
        assert_eq!(area(&r[0]), 100.0);
        assert_eq!(area(&r[1]), 0.0);
    }

    #[test]
    fn the_confirm_list_collapses_after_five() {
        assert_eq!(confirm_title(1), "Move 1 item to the Recycle Bin?");
        assert_eq!(confirm_title(2), "Move 2 items to the Recycle Bin?");
        assert_eq!(
            put_back_words(1),
            "You can put it back from the Recycle Bin until it is emptied."
        );
        assert!(put_back_words(3).starts_with("You can put them back"));
        assert_eq!(confirm_more(5), None);
        assert_eq!(confirm_more(17).as_deref(), Some("and 12 more"));
    }

    #[test]
    fn a_partial_move_names_the_file_in_use_and_what_did_move() {
        let busy = vec![PathBuf::from(r"C:\D\Win11_24H2_x64.iso")];
        let (t, b) = moved_words(1, &busy, 0).unwrap();
        assert_eq!(t, "1 file is in use");
        assert_eq!(
            b,
            "Win11_24H2_x64.iso is open in another app. The other item was moved."
        );
        assert_eq!(moved_words(3, &[], 0), None);
        assert_eq!(in_bin_words(2), ("In Recycle Bin".into(), "2 items".into()));
    }

    /// A small real tree under the temp folder, scanned by the engine.
    fn scanned(name: &str) -> (Tree, PathBuf) {
        let root =
            std::env::temp_dir().join(format!("orb-proto-model-{name}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&root);
        for (rel, bytes) in [
            ("Videos/a.mp4", 9000usize),
            ("Videos/b.mkv", 3000),
            ("Downloads/old/setup.exe", 4000),
            ("Downloads/x.zip", 2000),
            ("notes.txt", 500),
        ] {
            let p = root.join(rel);
            std::fs::create_dir_all(p.parent().unwrap()).unwrap();
            std::fs::write(&p, vec![1u8; bytes]).unwrap();
        }
        let tree = app_disk::scan(
            &root,
            &app_disk::ScanOptions::default(),
            |_| {},
            &std::sync::atomic::AtomicBool::new(false),
        )
        .unwrap();
        (tree, root)
    }

    #[test]
    fn a_level_lists_children_biggest_first_with_their_share() {
        let (tree, root) = scanned("rows");
        let r = rows(&tree, &Removed::default(), tree.root());
        let names: Vec<&str> = r.iter().map(|x| x.name.as_str()).collect();
        assert_eq!(names, ["Videos", "Downloads", "notes.txt"]);
        assert_eq!(r[0].size, 12000);
        assert!((r[0].percent - 12000.0 * 100.0 / 18500.0).abs() < 1e-9);
        assert!(r[0].dir && !r[2].dir);
        let s = summarize(&tree, &Removed::default(), tree.root(), 2);
        assert_eq!(s.files, 5);
        assert_eq!(s.bytes, 18500);
        let top: Vec<String> = s
            .top
            .iter()
            .map(|&id| tree.node(id).name().to_string_lossy().into_owned())
            .collect();
        assert_eq!(top, ["a.mp4", "setup.exe"]);
        assert_eq!(s.types[0].category, Category::Video);
        assert_eq!(s.types[0].bytes, 12000);
        assert_eq!(s.types.last().unwrap().category, Category::Other);
        let setup = tree.find(&root.join("Downloads/old/setup.exe")).unwrap();
        assert_eq!(folder_below(&tree, tree.root(), setup), "Downloads\\old");
        let _ = std::fs::remove_dir_all(&root);
    }

    #[test]
    fn moved_items_leave_the_level_and_their_bytes_leave_every_parent() {
        let (tree, root) = scanned("removed");
        let mut gone = Removed::default();
        let exe = tree.find(&root.join("Downloads/old/setup.exe")).unwrap();
        let downloads = tree.find(&root.join("Downloads")).unwrap();
        gone.add(&tree, exe);
        assert_eq!(gone.size(&tree, tree.root()), 14500);
        assert_eq!(gone.size(&tree, downloads), 2000);
        assert!(gone.is_gone(&tree, exe));
        let s = summarize(&tree, &gone, tree.root(), 10);
        assert_eq!((s.files, s.bytes), (4, 14500));
        // The whole folder replaces the file inside it, so nothing is subtracted twice.
        gone.add(&tree, downloads);
        assert_eq!(gone.size(&tree, tree.root()), 12500);
        let names: Vec<String> = rows(&tree, &gone, tree.root())
            .into_iter()
            .map(|r| r.name)
            .collect();
        assert_eq!(names, ["Videos", "notes.txt"]);
        // Undo brings it back.
        gone.restore(downloads);
        assert_eq!(gone.size(&tree, tree.root()), 18500);
        let _ = std::fs::remove_dir_all(&root);
    }

    #[test]
    fn the_treemap_shows_each_big_child_once_and_folds_the_tiny_ones() {
        let (tree, root) = scanned("map");
        let gone = Removed::default();
        let r = rows(&tree, &gone, tree.root());
        let inner = |id: NodeId| {
            rows(&tree, &gone, id)
                .into_iter()
                .map(|x| (x.name, x.size))
                .collect()
        };
        let tiles = treemap(&r, (0.0, 0.0, 336.0, 252.0), &inner);
        assert_eq!(
            tiles.iter().filter_map(|t| t.row).collect::<Vec<_>>(),
            [0, 1, 2]
        );
        let videos = &tiles[0];
        assert_eq!(
            videos.inner.len(),
            2,
            "Videos is big enough to show a.mp4 and b.mkv inside"
        );
        assert!(videos
            .inner
            .iter()
            .all(|(ir, _, _)| ir.1 >= videos.rect.1 + 42.0 && ir.3 <= videos.rect.3));
        assert!(tile_alpha(12000, 12000) > tile_alpha(500, 12000));
        let many: Vec<Row> = (0..20)
            .map(|i| Row {
                id: r[0].id,
                name: format!("f{i}"),
                size: 1000 - i * 10,
                percent: 0.0,
                dir: false,
                hidden: false,
            })
            .collect();
        let tiles = treemap(&many, (0.0, 0.0, 336.0, 252.0), &|_| Vec::new());
        assert_eq!(tiles.len(), MAX_TILES + 1);
        assert_eq!(tiles.last().unwrap().more, 20 - MAX_TILES);
        let _ = std::fs::remove_dir_all(&root);
    }

    #[test]
    fn the_hover_pill_counts_files_and_percent() {
        assert_eq!(
            scanning_value(Some(41.7), 0, 512_340),
            "41% · 512,340 files"
        );
        assert_eq!(
            scanning_value(None, 1024 * 1024 * 1024 + 1, 1),
            "1.0 GB · 1 file"
        );
    }

    // ---- cleaning rules ----

    fn rule(id: &str) -> &'static app_disk::Rule {
        app_disk::RULES.iter().find(|r| r.id == id).unwrap()
    }

    fn found(id: &str, parts: &[(&str, u64)]) -> app_disk::RuleFound {
        app_disk::RuleFound {
            rule: rule(id),
            items: parts
                .iter()
                .map(|(p, s)| app_disk::Candidate {
                    path: PathBuf::from(p),
                    size: *s,
                    newest: None,
                })
                .collect(),
        }
    }

    fn sample() -> CleanList {
        CleanList::new(vec![
            found(
                "npm-cache",
                &[
                    (r"C:\L\npm-cache\_cacache", 3 << 30),
                    (r"C:\L\npm-cache\_npx", 1 << 30),
                ],
            ),
            found(
                "cargo-registry",
                &[(r"C:\H\.cargo\registry\cache", 2 << 30)],
            ),
            found("gradle-caches", &[(r"C:\H\.gradle\caches", 5 << 30)]),
            found("discord-cache", &[(r"C:\R\discord\Cache", 300 << 20)]),
            found("ollama-models", &[(r"C:\H\.ollama\models", 7 << 30)]),
            found("crash-dumps", &[(r"C:\L\CrashDumps", 0)]),
        ])
    }

    #[test]
    fn rules_land_in_their_groups_biggest_first_with_nothing_selected() {
        let l = sample();
        let titles: Vec<&str> = l.groups.iter().map(|g| g.title).collect();
        assert_eq!(
            titles,
            ["Developer caches", "App caches", "AI models"],
            "empty rows and groups are dropped"
        );
        let dev: Vec<&str> = l.groups[0].items.iter().map(|i| i.id).collect();
        assert_eq!(dev, ["gradle-caches", "npm-cache", "cargo-registry"]);
        assert_eq!(l.groups[0].size, 11 << 30);
        let npm = &l.groups[0].items[1];
        assert_eq!(npm.size, 4 << 30);
        assert_eq!(npm.paths.len(), 2);
        assert_eq!(npm.detail, rule("npm-cache").explanation);
        assert!(l.items().all(|i| !i.selected));
        assert_eq!(l.selection_line(), None);
    }

    #[test]
    fn chips_count_rows_per_tier_and_filter_the_groups() {
        let l = sample();
        assert_eq!(
            l.counts(),
            TierCounts {
                all: 5,
                safe: 3,
                check_first: 2
            }
        );
        let chips: Vec<String> = l.chips().into_iter().map(|c| c.1).collect();
        assert_eq!(chips, ["All 5", "Safe to remove 3", "Check first 2"]);
        let check = l.groups_for(TierFilter::CheckFirst);
        let titles: Vec<&str> = check.iter().map(|g| g.title).collect();
        assert_eq!(titles, ["Developer caches", "AI models"]);
        assert_eq!(check[0].size, 5 << 30);
        assert!(l
            .groups_for(TierFilter::Safe)
            .iter()
            .all(|g| g.items.iter().all(|i| i.tier == Tier::SafeToRemove)));
        assert_eq!(l.groups_for(TierFilter::All), l.groups);
    }

    #[test]
    fn bulk_select_takes_safe_rows_only_and_the_confirm_puts_check_first_first() {
        let mut l = sample();
        l.select_tier(TierFilter::All, true);
        assert_eq!(l.selection().0, 3, "Check first is never selected in bulk");
        assert_eq!(l.toggle("ollama-models"), Some(true));
        assert_eq!(l.toggle("nope"), None);
        let (n, b) = l.selection();
        assert_eq!(
            (n, b),
            (
                4,
                (3u64 << 30) + (1 << 30) + (2 << 30) + (300 << 20) + (7 << 30)
            )
        );
        assert_eq!(
            l.selection_words(),
            Some(("4 selected".to_string(), size(b)))
        );
        assert_eq!(
            l.selection_line().unwrap(),
            format!("4 selected · {}", size(b))
        );
        let c = l.confirm();
        assert_eq!(c.title, "Move 4 items to the Recycle Bin?");
        assert_eq!(c.count, 4);
        assert_eq!(c.bytes, b);
        let tiers: Vec<&str> = c.sections.iter().map(|s| s.title).collect();
        assert_eq!(tiers, ["Check first", "Safe to remove"]);
        assert_eq!(c.sections[0].items, [("Ollama models", 7u64 << 30)]);
        assert_eq!(c.sections[0].bytes, 7 << 30);
        let safe: Vec<&str> = c.sections[1].items.iter().map(|i| i.0).collect();
        assert_eq!(safe, ["npm cache", "Cargo registry", "Discord cache"]);
        assert_eq!(c.sections.iter().map(|s| s.bytes).sum::<u64>(), c.bytes);
        assert!(c.honest.contains(&size(b)));
        // Only Safe rows: the Check first section is left out.
        l.set_selected("ollama-models", false);
        assert_eq!(l.confirm().sections.len(), 1);
        l.select_tier(TierFilter::Safe, false);
        assert_eq!(l.selection(), (0, 0));
        assert!(l.confirm().sections.is_empty());
    }

    #[test]
    fn removal_hands_every_folder_with_its_rule_and_forget_drops_what_moved() {
        let mut l = sample();
        l.set_selected("npm-cache", true);
        l.set_selected("discord-cache", true);
        let ids: Vec<(&str, PathBuf)> = l
            .removal()
            .into_iter()
            .map(|(rule, p)| (rule.id, p))
            .collect();
        assert_eq!(
            ids,
            [
                ("npm-cache", PathBuf::from(r"C:\L\npm-cache\_cacache")),
                ("npm-cache", PathBuf::from(r"C:\L\npm-cache\_npx")),
                ("discord-cache", PathBuf::from(r"C:\R\discord\Cache")),
            ]
        );
        // _npx was in use and stayed.
        l.forget(&[
            PathBuf::from(r"C:\L\npm-cache\_cacache"),
            PathBuf::from(r"C:\R\discord\Cache"),
        ]);
        let npm = l.items().find(|i| i.id == "npm-cache").unwrap();
        assert_eq!(
            (
                npm.size,
                npm.paths.len(),
                npm.path_sizes.len(),
                npm.selected
            ),
            (1 << 30, 1, 1, true)
        );
        assert!(l.items().all(|i| i.id != "discord-cache"));
        assert!(l.groups.iter().all(|g| g.title != "App caches"));
        assert_eq!(l.counts().all, 4);
    }

    #[test]
    fn unfinished_downloads_get_their_label() {
        use std::ffi::OsStr;
        assert_eq!(
            file_label(OsStr::new("Win11.iso.crdownload")),
            Some("Unfinished download")
        );
        assert_eq!(
            file_label(OsStr::new("show.mkv.part")),
            Some("Unfinished download")
        );
        assert_eq!(file_label(OsStr::new("x.TMP")), Some("Unfinished download"));
        assert_eq!(file_label(OsStr::new("Win11.iso")), None);
    }

    fn copy(path: &str, days_old: u64) -> DupeCopy {
        DupeCopy {
            path: PathBuf::from(path),
            modified: Some(
                std::time::UNIX_EPOCH + Duration::from_secs(1_800_000_000 - days_old * 86_400),
            ),
            gone: false,
            frees: None,
        }
    }

    #[test]
    fn the_keeper_is_the_oldest_copy_outside_downloads_temp_and_desktop() {
        let s = DupeSet::new(
            10,
            vec![
                copy(r"C:\Users\me\Downloads\movie.mp4", 900),
                copy(r"C:\Users\me\Videos\movie.mp4", 30),
                copy(r"D:\Backup\movie.mp4", 200),
            ],
        );
        assert_eq!(s.keeper(), 2, "the oldest copy outside Downloads");
        let s = DupeSet::new(
            10,
            vec![
                copy(r"C:\Users\me\Desktop\a.zip", 5),
                copy(r"C:\Users\me\AppData\Local\Temp\a.zip", 9),
            ],
        );
        assert_eq!(s.keeper(), 1, "all passing through: the oldest");
        assert!(passing_through(Path::new(r"C:\Users\me\downloads\x")));
        assert!(!passing_through(Path::new(r"C:\Users\me\Downloads2\x")));
    }

    #[test]
    fn nothing_is_picked_until_you_pick_and_the_keeper_never_is() {
        let mut s = DupeSet::new(
            10,
            vec![copy(r"C:\a", 3), copy(r"C:\b", 2), copy(r"C:\c", 1)],
        );
        assert_eq!(s.picked().count(), 0);
        assert_eq!(s.keeper(), 0);
        assert!(!s.toggle(0), "the keeper does not tick");
        s.pick_extras(true);
        assert_eq!(s.picked().collect::<Vec<_>>(), [1, 2]);
        assert!(s.all_extras_picked());
        // One click on another copy makes it the keeper and unpicks it.
        assert!(s.set_keeper(2));
        assert_eq!(s.picked().collect::<Vec<_>>(), [1]);
        assert_eq!(s.reclaimable(), 20);
    }

    #[test]
    fn a_group_can_never_have_every_copy_picked() {
        // Every operation in every order, on groups of two to four copies, some going and coming
        // back: the keeper is always a live copy and never picked.
        let mut seed = 7u64;
        let mut next = |n: usize| {
            seed = seed
                .wrapping_mul(6_364_136_223_846_793_005)
                .wrapping_add(1_442_695_040_888_963_407);
            (seed >> 33) as usize % n
        };
        for size in 2..=4 {
            let paths: Vec<PathBuf> = (0..size)
                .map(|i| PathBuf::from(format!(r"C:\f{i}")))
                .collect();
            let mut s = DupeSet::new(
                1,
                paths
                    .iter()
                    .map(|p| DupeCopy {
                        path: p.clone(),
                        modified: None,
                        gone: false,
                        frees: None,
                    })
                    .collect(),
            );
            for _ in 0..2_000 {
                let i = next(size);
                match next(6) {
                    0 => {
                        s.toggle(i);
                    }
                    1 => {
                        s.set_keeper(i);
                    }
                    2 => s.pick_extras(true),
                    3 => s.pick_extras(false),
                    4 => s.mark(&[paths[i].clone()], true),
                    _ => s.mark(&[paths[i].clone()], false),
                }
                let live = s.live();
                assert!(!s.is_picked(s.keeper()), "the keeper is never picked");
                assert!(
                    live == 0 || !s.copies[s.keeper()].gone,
                    "the keeper is still there"
                );
                assert!(
                    s.picked().count() < live.max(1),
                    "at least one copy stays: {} of {live}",
                    s.picked().count()
                );
                assert!(s.picked().all(|k| !s.copies[k].gone));
            }
        }
    }

    #[test]
    fn removals_name_the_copy_that_stays_and_moved_copies_leave_the_group() {
        let groups = vec![app_disk::DupeGroup {
            size: 100,
            files: ["a", "b", "c"]
                .iter()
                .map(|n| app_disk::DupeFile {
                    path: PathBuf::from(format!(r"C:\{n}")),
                    size: 100,
                    modified: None,
                    frees: None,
                })
                .collect(),
        }];
        let mut l = DupeList::new(&groups);
        assert_eq!(l.selection(), (0, 0));
        l.pick_extras(true);
        assert_eq!(l.selection(), (2, 200));
        assert!(l
            .removals()
            .iter()
            .all(|(p, _, keep)| keep == &PathBuf::from(r"C:\a") && p != keep));
        l.mark(&[PathBuf::from(r"C:\b")], true);
        assert_eq!(l.reclaimable(), 100);
        assert_eq!(l.selection(), (1, 100));
        l.mark(&[PathBuf::from(r"C:\c")], true);
        assert!(l.shown().is_empty(), "one copy left is not a duplicate");
        assert_eq!(l.selection(), (0, 0));
        l.mark(&[PathBuf::from(r"C:\b")], false);
        assert_eq!(l.shown(), [0], "Undo brings the group back");
        // A folder moved from Results takes the copies inside it along.
        let mut l = DupeList::new(&[app_disk::DupeGroup {
            size: 5,
            files: [r"C:\Videos\x.mp4", r"D:\Backup\x.mp4"]
                .iter()
                .map(|p| app_disk::DupeFile {
                    path: PathBuf::from(p),
                    size: 5,
                    modified: None,
                    frees: None,
                })
                .collect(),
        }]);
        l.mark(&[PathBuf::from(r"C:\Videos")], true);
        assert!(l.shown().is_empty());
    }

    #[test]
    fn duplicate_words_read_plainly() {
        assert_eq!(
            dupe_group_words(3, 1288490188, false),
            "3 copies · 1.2 GB to reclaim"
        );
        assert_eq!(
            dupe_group_words(2, 3 << 30, false),
            "2 copies · 3 GB to reclaim"
        );
        assert_eq!(
            dupe_group_words(2, 0, true),
            "2 copies · 0 KB to reclaim · shares space (APFS clone)"
        );
        let (line, frac) = dupe_progress_words(&app_disk::DupeProgress {
            stage: app_disk::DupeStage::Contents,
            files_done: 1204,
            files_total: 3400,
            bytes_done: 1 << 30,
            bytes_total: 4 << 30,
        });
        assert_eq!(line, "1,204 of 3,400 files · 1 GB of 4 GB");
        assert!((frac - 0.25).abs() < 1e-6);
        assert!(dupe_stage_words(app_disk::DupeStage::Ends).ends_with('…'));
    }
}
