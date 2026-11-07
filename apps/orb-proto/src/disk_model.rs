//! What the Disk panel shows and decides, with no windows or threads: sizes and words, the
//! items moved to the Recycle Bin this session (subtracted from the scan), one folder level's
//! rows, files and types, the squarified treemap, and the confirm sheet's list.

use std::cmp::Reverse;
use std::collections::{BTreeMap, BinaryHeap};
use std::path::{Path, PathBuf};
use std::time::Duration;

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
/// panel card's sentence.
pub fn scan_error_words(root: &Path, e: &ScanError) -> (String, String, &'static str) {
    let place = place_name(root);
    match e {
        // A drive letter that is not there is a drive that is unplugged or locked.
        ScanError::NotReady | ScanError::NotFound if is_drive_root(root) => (
            place,
            "is not ready".into(),
            "Plug the drive back in, or unlock it in Explorer, then scan again.",
        ),
        ScanError::NotFound => (
            place,
            "is not there".into(),
            "The folder was moved or deleted. Pick it again, then scan.",
        ),
        ScanError::AccessDenied => (
            place,
            "can't be read".into(),
            "Windows keeps this folder private, so nothing in it was counted.",
        ),
        ScanError::NotADirectory => (
            place,
            "is not a folder".into(),
            "Pick a folder or a drive to scan.",
        ),
        ScanError::NotReady => (
            place,
            "is not ready".into(),
            "Plug the drive back in, or unlock it in Explorer, then scan again.",
        ),
        ScanError::Io(_) => (
            place,
            "couldn't be read".into(),
            "Windows would not list it. Try again in a moment.",
        ),
    }
}

// ---- what moved to the Recycle Bin this session ----

/// Items moved out of the scanned tree. The tree itself never changes after a scan, so the panel
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
        "Move 1 item to the Recycle Bin?".into()
    } else {
        format!("Move {n} items to the Recycle Bin?")
    }
}

pub fn confirm_more(n: usize) -> Option<String> {
    (n > CONFIRM_LIST).then(|| format!("and {} more", n - CONFIRM_LIST))
}

/// "You can put it back" for one item, "them" for more.
pub fn put_back_words(n: usize) -> &'static str {
    if n == 1 {
        "You can put it back from the Recycle Bin until it is emptied."
    } else {
        "You can put them back from the Recycle Bin until it is emptied."
    }
}

pub fn confirm_honest(total: u64) -> String {
    format!("The {} comes back only when the Recycle Bin is emptied. [App] never empties it for you, and has no permanent delete.", size(total))
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
        .unwrap_or_else(|| "Windows would not move it.".into());
    let tail = match moved {
        0 => String::new(),
        1 => " The other item was moved.".into(),
        n => format!(" The other {n} items were moved."),
    };
    Some((title, format!("{first}{tail}")))
}

/// "In Recycle Bin" and "2 items".
pub fn in_bin_words(n: usize) -> (String, String) {
    ("In Recycle Bin".into(), plural(n as u64, "item", "items"))
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
}
