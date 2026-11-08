//! Disk on app_disk: the low-space pill, scans on a worker thread with the orb's ring and the
//! Scanning screen, Results (treemap, folders, files, types), Safe to clean with Review, the
//! confirm sheet, Move to Recycle Bin on a worker thread, and Undo. It all shows in the Disk
//! window (disk_window.rs), a real window like Settings.
//!
//! Nothing here deletes. The only remove action is `app_disk::move_to_recycle_bin`, and it runs
//! only from Move to Recycle Bin on the confirm sheet. Undo puts the items back (disk_restore.rs).
//! disk_model.rs decides the words and numbers, disk_panel.rs draws.

use std::collections::{BTreeMap, BTreeSet, HashMap};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc::{channel, Receiver, Sender};
use std::sync::Arc;
use std::time::{Duration, Instant, SystemTime};

use app_disk::{
    self as ad, Candidate, Category, CleanPlaces, DriveSpace, LowSpaceWatch, NodeId, RecycleError,
    RecycleReport, ScanError, ScanOptions, ScanStatus, SuggestionKind, Tree,
};
use app_downloads::{Ring, Tone};
use app_habits::held::Priority;
use serde::{Deserialize, Serialize};
use windows::core::{w, HSTRING, PCWSTR};
use windows::Win32::Foundation::*;
use windows::Win32::Graphics::Direct2D::ID2D1DeviceContext;
use windows::Win32::Storage::FileSystem::{GetDriveTypeW, GetLogicalDrives, GetVolumeInformationW};
use windows::Win32::UI::Input::KeyboardAndMouse::*;
use windows::Win32::UI::Shell::ShellExecuteW;
use windows::Win32::UI::WindowsAndMessaging::*;

use crate::disk_model::{self as m, Rect, Removed, Row, Stamp, Summary};
use crate::disk_panel::{self as dp, Card, Filter, Focus, Hit, Screen, Tab, View};
use crate::disk_restore;
use crate::disk_window::{self, Win};
use crate::notices;
use crate::pill::dismiss_pill;
use crate::ui::{self, Icon, Pill, PillAction};
use crate::{
    downloads, habits, log_line, memory_mb, orb, start_anim, store, App, TIMER_DISK,
    TIMER_DISK_RELEASE, WM_APP_DISK,
};

/// The low-space check, as ORB_SPEC asks: every 10 minutes.
const CHECK_MS: u32 = 10 * 60 * 1000;
/// The live numbers reach the UI this often while a scan runs.
const LIVE_EVERY: Duration = Duration::from_millis(150);
/// Files listed in the Files tab.
const TOP_FILES: usize = 200;
/// Downloads older than this are suggested (and marked Check first).
const OLD_DAYS: u32 = 90;
const DRIVE_FIXED: u32 = 3;
pub const PICK_FOLDER: usize = 1;
/// The low-space pill's cooldown kind in the notice queue.
const LOW_SPACE: &str = "low-space";

/// One child of the scanned place while the scan runs: name, bytes so far, folder, hidden.
#[derive(Clone, Debug, Default, PartialEq)]
struct LiveRow {
    name: String,
    size: u64,
    dir: bool,
    hidden: bool,
}

#[derive(Clone, Debug, Default)]
struct Live {
    counted: u64,
    /// Allocated bytes, the measure for a drive's percentage.
    on_disk: u64,
    files: u64,
    current: String,
    /// The scanned place's children so far, biggest first.
    rows: Vec<LiveRow>,
}

/// At most this many children of the scanned place travel with each live report.
const LIVE_ROWS: usize = 200;

struct Done {
    tree: Tree,
    space: Option<DriveSpace>,
    at: SystemTime,
    volume: String,
    bin: bool,
}

enum Scan {
    Idle,
    Running {
        id: u64,
        root: PathBuf,
        user: bool,
        started: Instant,
        cancel: Arc<AtomicBool>,
        space: Option<DriveSpace>,
        volume: String,
        live: Live,
    },
    Done(Box<Done>),
    Failed {
        root: PathBuf,
        error: ScanError,
    },
    /// The tree was dropped to give its memory back; opening the window scans the place again.
    Released {
        root: PathBuf,
    },
}

#[derive(Clone, Copy, PartialEq, Debug)]
enum Place {
    Start,
    Scanning,
    Results,
    Clean,
    Review,
}

#[derive(Clone, Copy, PartialEq, Debug)]
enum Source {
    Tree,
    Clean(SuggestionKind),
    /// Cleaning-rule folders (with or without suggestions in the same move).
    Rules,
}

enum Msg {
    Live(u64, Live),
    Scanned(u64, Result<Tree, ScanError>, Duration),
    Suggestions(Vec<ad::Suggestion>, Option<u64>),
    /// The cleaning rules' folders, measured (crates/disk/src/rules.rs).
    Rules(ad::Measured),
    Recycled(RecycleReport, SystemTime, Source),
}

/// Rule rows share Safe to clean's index space with the suggestions, from here up.
const RULE_BASE: usize = 1000;

fn rule_index(id: &str) -> Option<usize> {
    ad::RULES
        .iter()
        .position(|r| r.id == id)
        .map(|i| RULE_BASE + i)
}

fn rule_id(i: usize) -> Option<&'static str> {
    i.checked_sub(RULE_BASE)
        .and_then(|k| ad::RULES.get(k))
        .map(|r| r.id)
}

enum Clean {
    NotRead,
    Reading,
    Ready {
        list: Vec<ad::Suggestion>,
        bin: Option<u64>,
    },
}

struct Review {
    kind: SuggestionKind,
    items: Vec<Candidate>,
    checked: BTreeSet<usize>,
    first: usize,
}

struct Confirm {
    items: Vec<(PathBuf, u64)>,
    /// What the sheet lists: Check first or not, name, the muted line, bytes. One line can stand
    /// for many items (a whole suggestion).
    lines: Vec<(bool, String, String, u64)>,
    /// The cleaning-rule folders among `items`, each with its rule.
    rules: Vec<(&'static ad::Rule, PathBuf)>,
    source: Source,
    on_move: bool,
}

/// What the last action did, shown as a card in the open window (a pill when it is closed).
enum Note {
    Moved(usize),
    /// Some items stayed (in use); Try again asks again for just those.
    Partial {
        title: String,
        body: String,
        moved: usize,
        stayed: Vec<(PathBuf, u64)>,
        source: Source,
    },
    NoBin(String, PathBuf),
    Restored {
        back: usize,
        stayed: usize,
    },
}

struct LastMove {
    paths: Vec<PathBuf>,
    since: SystemTime,
    source: Source,
    /// The Undo pill was shown on the way out of the window; it comes once per move.
    offered: bool,
}

/// One folder level of the results, computed when the place or the moved items change.
struct Level {
    at: NodeId,
    removed_gen: u64,
    rows: Vec<Row>,
    summary: Summary,
    /// The treemap's rectangle the tiles were laid out in (it follows the window's room).
    map: Rect,
    tiles: Vec<dp::MapTile>,
    files: Vec<(NodeId, dp::FileRow)>,
}

/// A value easing from one number to another (the veil, the hover fade, the suffix).
#[derive(Clone, Copy, Debug)]
struct Fade {
    from: f32,
    to: f32,
    at: Instant,
    ms: f32,
}

impl Fade {
    fn new(v: f32) -> Fade {
        Fade {
            from: v,
            to: v,
            at: Instant::now(),
            ms: 1.0,
        }
    }

    fn progress(&self, now: Instant) -> f32 {
        (now.saturating_duration_since(self.at).as_secs_f32() * 1000.0 / self.ms).clamp(0.0, 1.0)
    }

    /// Eased out (`fade`).
    fn value(&self, now: Instant) -> f32 {
        let p = self.progress(now);
        let e = 1.0 - (1.0 - p) * (1.0 - p);
        self.from + (self.to - self.from) * e
    }

    /// A critically damped spring (`switch`, k 700, c 53: 90% at ~150 ms, at rest by `ms`).
    fn spring(&self, now: Instant) -> f32 {
        let t = now.saturating_duration_since(self.at).as_secs_f32();
        if t * 1000.0 >= self.ms {
            return self.to;
        }
        let w = 26.5;
        let e = 1.0 - (1.0 + w * t) * (-w * t).exp();
        self.from + (self.to - self.from) * e
    }

    fn moving(&self, now: Instant) -> bool {
        self.from != self.to && self.progress(now) < 1.0
    }

    /// Head for `to` from wherever it is now; snaps under reduced motion.
    fn go(&mut self, to: f32, ms: f32, now: Instant) {
        if (to - self.to).abs() < f32::EPSILON {
            return;
        }
        let from = if downloads::reduced_motion() {
            to
        } else {
            self.value(now)
        };
        *self = Fade {
            from,
            to,
            at: now,
            ms,
        };
    }
}

/// A number heading for a new value on Foundation's "roll" spring (critically damped, ω 7.5, at
/// rest after 0.9 s). The value eases rather than each digit rolling in its own column, because
/// `motion::rolling_text` keys numbers by a `&'static str` and these are keyed by folder name
/// (a row must keep its own number when the list reorders during a scan).
#[derive(Clone, Copy, Debug)]
struct Roll {
    from: f64,
    to: f64,
    at: Instant,
}

const ROLL_S: f32 = 0.9;

impl Roll {
    fn value(&self, now: Instant) -> f64 {
        let t = now.saturating_duration_since(self.at).as_secs_f32();
        let p = crate::motion::critical(7.5, t, ROLL_S) as f64;
        self.from + (self.to - self.from) * p
    }
}

/// The window's moving parts. Everything here settles, so an idle window draws nothing.
struct Anim {
    /// The veil over a scan in progress.
    veil: Fade,
    /// The veil pill's words (kept while it fades after the scan ends).
    veil_text: Option<(String, String, Option<f32>, String)>,
    /// When the counted total last changed (the veil lifts once it has held 1.5 s).
    total_at: Instant,
    total: u64,
    /// The hover fade and what stays lit.
    dim: Fade,
    focus: Option<Focus>,
    /// Treemap tiles by key: where they were drawn when the layout last changed, where they go.
    map_from: HashMap<String, Rect>,
    map_to: Vec<(String, Rect)>,
    map_at: Instant,
    /// Rows entering.
    ripple_at: Option<Instant>,
    /// The primary button's size suffix: width spring and opacity.
    suffix_w: Fade,
    suffix_a: Fade,
    suffix_text: String,
    rolls: HashMap<String, Roll>,
    shimmer_from: Instant,
}

impl Default for Anim {
    fn default() -> Self {
        let now = Instant::now();
        Anim {
            veil: Fade::new(0.0),
            veil_text: None,
            total_at: now,
            total: 0,
            dim: Fade::new(0.0),
            focus: None,
            map_from: HashMap::new(),
            map_to: Vec::new(),
            map_at: now,
            ripple_at: None,
            suffix_w: Fade::new(0.0),
            suffix_a: Fade::new(0.0),
            suffix_text: String::new(),
            rolls: HashMap::new(),
            shimmer_from: now,
        }
    }
}

const REFLOW_MS: f32 = 700.0;
const VEIL_MS: f32 = 400.0;
const VEIL_HOLD: Duration = Duration::from_millis(1500);
const RIPPLE_MS: f32 = 8.0 * 30.0 + 200.0;
const SHIMMER_S: f32 = 1.4;
const HOVER_MS: f32 = 150.0;

#[derive(Serialize, Deserialize, Default)]
struct Saved {
    /// Free percent at the last low-space pill, per drive letter.
    #[serde(default)]
    low_space: BTreeMap<String, f64>,
}

pub struct Disk {
    file: PathBuf,
    watch: BTreeMap<char, LowSpaceWatch>,
    scan: Scan,
    next_id: u64,
    place: Place,
    /// Where Safe to clean goes back to.
    clean_from: Place,
    tab: Tab,
    at: Option<NodeId>,
    sel: Option<usize>,
    first: usize,
    files_first: usize,
    checked: BTreeSet<NodeId>,
    cursor: Option<usize>,
    kb: bool,
    removed: Removed,
    removed_gen: u64,
    level: Option<Level>,
    clean: Clean,
    review: Option<Review>,
    confirm: Option<Confirm>,
    /// The items of the move in flight, for the sizes of any that stay.
    asked: Vec<(PathBuf, u64)>,
    /// Try again after a partial move: Undo then covers both moves.
    merge_next: bool,
    note: Option<Note>,
    last_move: Option<LastMove>,
    moving: bool,
    tx: Sender<Msg>,
    rx: Receiver<Msg>,
    posted: Arc<AtomicBool>,
    hover_named: bool,
    /// A scan you started failed: the ember badge on the drop until the window is opened.
    pub attention: bool,
    spin_from: Instant,
    last_ring: Option<Ring>,
    drives: Vec<(PathBuf, String, DriveSpace)>,
    /// Safe to clean: the suggestions ticked, the filter, the list's first entry.
    clean_checked: BTreeSet<usize>,
    /// The cleaning rules with something to offer, and which are ticked (disk_model::CleanList).
    rules: Option<m::CleanList>,
    filter: Filter,
    clean_first: usize,
    /// While a scan runs: each top-level folder's bytes and when they last changed.
    live_seen: HashMap<String, (u64, Instant)>,
    anim: Anim,
    pub win: Win,
}

fn data_file() -> PathBuf {
    store::data_dir().join("disk.json")
}

impl Disk {
    pub fn load(hwnd: HWND) -> Disk {
        let mut d = Disk::load_from(data_file());
        d.win.hwnd = hwnd;
        d
    }

    fn load_from(file: PathBuf) -> Disk {
        let saved: Saved = std::fs::read_to_string(&file)
            .ok()
            .and_then(|s| serde_json::from_str(&s).ok())
            .unwrap_or_default();
        let watch = saved
            .low_space
            .iter()
            .filter_map(|(k, v)| {
                k.chars().next().map(|c| {
                    (
                        c,
                        LowSpaceWatch::Shown {
                            at_free_percent: *v,
                        },
                    )
                })
            })
            .collect();
        let (tx, rx) = channel();
        Disk {
            file,
            watch,
            scan: Scan::Idle,
            next_id: 1,
            place: Place::Start,
            clean_from: Place::Start,
            tab: Tab::Folders,
            at: None,
            sel: None,
            first: 0,
            files_first: 0,
            checked: BTreeSet::new(),
            cursor: None,
            kb: false,
            removed: Removed::default(),
            removed_gen: 0,
            level: None,
            clean: Clean::NotRead,
            review: None,
            confirm: None,
            asked: Vec::new(),
            merge_next: false,
            note: None,
            last_move: None,
            moving: false,
            tx,
            rx,
            posted: Arc::new(AtomicBool::new(false)),
            hover_named: false,
            attention: false,
            spin_from: Instant::now(),
            last_ring: None,
            drives: Vec::new(),
            clean_checked: BTreeSet::new(),
            rules: None,
            filter: Filter::All,
            clean_first: 0,
            live_seen: HashMap::new(),
            anim: Anim::default(),
            win: Win::default(),
        }
    }

    fn save(&self) {
        let low_space = self
            .watch
            .iter()
            .filter_map(|(c, w)| match w {
                LowSpaceWatch::Shown { at_free_percent } => Some((c.to_string(), *at_free_percent)),
                LowSpaceWatch::Armed => None,
            })
            .collect();
        if let Ok(s) = serde_json::to_string_pretty(&Saved { low_space }) {
            let _ = std::fs::create_dir_all(self.file.parent().unwrap_or(Path::new(".")));
            let tmp = self.file.with_extension("json.tmp");
            if std::fs::write(&tmp, s).is_ok() {
                let _ = std::fs::rename(&tmp, &self.file);
            }
        }
    }

    fn done(&self) -> Option<&Done> {
        match &self.scan {
            Scan::Done(d) => Some(d),
            _ => None,
        }
    }
}

/// Sends a report to the UI thread, posting WM_APP_DISK only when none is pending.
#[derive(Clone)]
struct Reporter {
    tx: Sender<Msg>,
    posted: Arc<AtomicBool>,
    hwnd: isize,
}

impl Reporter {
    fn send(&self, m: Msg) {
        if self.tx.send(m).is_ok() && !self.posted.swap(true, Ordering::AcqRel) {
            unsafe {
                let _ = PostMessageW(
                    Some(HWND(self.hwnd as *mut _)),
                    WM_APP_DISK,
                    WPARAM(0),
                    LPARAM(0),
                );
            }
        }
    }
}

fn reporter(app: &App) -> Reporter {
    Reporter {
        tx: app.disk.tx.clone(),
        posted: Arc::clone(&app.disk.posted),
        hwnd: app.hwnd.0 as isize,
    }
}

// ---- drives and the low-space pill ----

fn fixed_drives() -> Vec<PathBuf> {
    let mask = unsafe { GetLogicalDrives() };
    (0..26u8)
        .filter(|i| mask & (1 << i) != 0)
        .map(|i| PathBuf::from(format!("{}:\\", (b'A' + i) as char)))
        .filter(|p| unsafe { GetDriveTypeW(&HSTRING::from(p.as_os_str())) } == DRIVE_FIXED)
        .collect()
}

/// "Windows", or "Local Disk" when the volume has no name.
fn volume_label(root: &Path) -> String {
    let mut name = [0u16; 261];
    let ok = unsafe {
        GetVolumeInformationW(
            &HSTRING::from(root.as_os_str()),
            Some(&mut name),
            None,
            None,
            None,
            None,
        )
    }
    .is_ok();
    let n = name.iter().position(|&c| c == 0).unwrap_or(0);
    let s = String::from_utf16_lossy(&name[..n]);
    if ok && !s.is_empty() {
        s
    } else {
        "Local Disk".into()
    }
}

/// At launch: the saved low-space state, the first check, and the 10 min timer.
pub fn start(app: &mut App) {
    unsafe {
        SetTimer(Some(app.hwnd), TIMER_DISK, CHECK_MS, None);
    }
    let shown: Vec<String> = app
        .disk
        .watch
        .iter()
        .map(|(c, w)| format!("{c}: {w:?}"))
        .collect();
    log_line(
        app,
        &format!("disk: low-space check every 10 min, saved {shown:?}"),
    );
    check_space(app);
}

pub fn on_timer(app: &mut App) {
    check_space(app);
}

/// Reads each fixed drive and runs the pill rule on it.
pub fn check_space(app: &mut App) {
    let readings: Vec<(char, DriveSpace)> = fixed_drives()
        .into_iter()
        .filter_map(|r| Some((m::drive_letter(&r)?, ad::drive_space(&r).ok()?)))
        .collect();
    consider(app, &readings);
}

/// One reading per drive. At most one pill per check, for the drive with the least room; any
/// other drive that would fire is left for the next check, so pills never pile up. While the orb
/// cannot show a pill the readings are not fed to the rule, so the pill comes later, not never.
fn consider(app: &mut App, readings: &[(char, DriveSpace)]) {
    if notices::quiet(app).holds() {
        log_line(
            app,
            &format!(
                "disk: {} drive(s) not checked (orb hidden, full screen, focus or a pause)",
                readings.len()
            ),
        );
        return;
    }
    let fires = |w: &LowSpaceWatch, s: DriveSpace| {
        let mut w = *w;
        w.check(s)
    };
    let pick = readings
        .iter()
        .filter(|(c, s)| fires(&app.disk.watch.get(c).copied().unwrap_or_default(), *s))
        .min_by(|a, b| a.1.free_percent().total_cmp(&b.1.free_percent()))
        .map(|(c, _)| *c);
    let mut changed = false;
    for &(letter, space) in readings {
        let w = app.disk.watch.entry(letter).or_default();
        let before = *w;
        if fires(w, space) && Some(letter) != pick {
            continue;
        }
        w.check(space);
        changed |= before != *w;
    }
    if changed {
        app.disk.save();
    }
    let low: Vec<String> = readings
        .iter()
        .filter(|(_, s)| s.free_percent() < ad::LOW_SPACE_PERCENT)
        .map(|(c, s)| format!("{c}: {:.1}%", s.free_percent()))
        .collect();
    let Some((letter, space)) = readings.iter().find(|(c, _)| Some(*c) == pick).copied() else {
        if !low.is_empty() {
            log_line(
                app,
                &format!(
                    "disk: low {low:?}, no pill (each was shown and has not dropped 5 more points)"
                ),
            );
        }
        return;
    };
    let (title, value) = m::low_space_words(letter, space);
    log_line(app, &format!("disk: low {low:?}, pill \"{title} {value}\""));
    let p = Pill {
        icon: Icon::HardDrive,
        dot: false,
        title,
        value,
        action: Some((
            "Find big folders".into(),
            PillAction::FindBigFolders(letter),
        )),
        closable: true,
        ttl_ms: 6000,
    };
    // "Disk space: once a day" (docs/design/notices.md): a second one waits out the day.
    notices::post(
        app,
        notices::Notice::new(notices::Source::Disk, Priority::Normal, p)
            .cooldown(LOW_SPACE, Duration::from_secs(86_400)),
    );
}

/// Test: the rule fed a made-up reading of C: with this much free. resh forgets the last C:
/// pill first, as if free space had come back above 10%.
pub fn test_low_space(app: &mut App, free_percent: f64, fresh: bool) {
    if fresh {
        app.disk.watch.insert('C', LowSpaceWatch::Armed);
        notices::forget_cooldown(app, LOW_SPACE);
    }
    let total = ad::drive_space(Path::new("C:\\")).map_or(476 << 30, |s| s.total);
    let space = DriveSpace {
        total,
        free: (total as f64 * free_percent / 100.0) as u64,
    };
    log_line(app, &format!("disk: test reading C: {free_percent}% free"));
    consider(app, &[('C', space)]);
}

// ---- scanning ----

/// The low-space pill's action: scan the drive and show it scanning.
pub fn find_big_folders(app: &mut App, letter: char) {
    scan_path(app, PathBuf::from(format!("{letter}:\\")), true);
    open(app);
}

/// Scans `root` on a worker thread. A scan already running is stopped first.
pub fn scan_path(app: &mut App, root: PathBuf, user: bool) {
    if let Scan::Running { cancel, .. } = &app.disk.scan {
        cancel.store(true, Ordering::Relaxed);
    }
    let id = app.disk.next_id;
    app.disk.next_id += 1;
    let space = if m::is_drive_root(&root) {
        ad::drive_space(&root).ok()
    } else {
        None
    };
    let cancel = Arc::new(AtomicBool::new(false));
    let rep = reporter(app);
    let (r, c) = (root.clone(), Arc::clone(&cancel));
    std::thread::spawn(move || {
        let t0 = Instant::now();
        let mut last = Instant::now() - LIVE_EVERY;
        let opts = ScanOptions::default();
        let res = ad::scan(
            &r,
            &opts,
            |p| {
                if last.elapsed() >= LIVE_EVERY {
                    last = Instant::now();
                    let t = p.tree;
                    let root = t.node(t.root());
                    let rows = t
                        .children_by_size(t.root())
                        .into_iter()
                        .take(LIVE_ROWS)
                        .map(|c| {
                            let n = t.node(c.id);
                            LiveRow {
                                name: n.name().to_string_lossy().into_owned(),
                                size: c.size,
                                dir: n.is_dir(),
                                hidden: n.hidden(),
                            }
                        })
                        .collect();
                    rep.send(Msg::Live(
                        id,
                        Live {
                            counted: root.size(),
                            on_disk: root.allocated(),
                            files: root.files(),
                            current: p.current.display().to_string(),
                            rows,
                        },
                    ));
                }
            },
            &c,
        );
        rep.send(Msg::Scanned(id, res, t0.elapsed()));
    });
    log_line(app, &format!("disk: scanning {}", root.display()));
    app.disk.attention = false;
    let volume = if m::is_drive_root(&root) {
        volume_label(&root)
    } else {
        String::new()
    };
    app.disk.scan = Scan::Running {
        id,
        root,
        user,
        started: Instant::now(),
        cancel,
        space,
        volume,
        live: Live::default(),
    };
    app.disk.spin_from = Instant::now();
    app.disk.place = Place::Scanning;
    app.disk.tab = Tab::Folders;
    app.disk.note = None;
    app.disk.confirm = None;
    app.disk.live_seen.clear();
    let now = Instant::now();
    let a = &mut app.disk.anim;
    a.veil = Fade::new(1.0);
    a.total = 0;
    a.total_at = now;
    a.shimmer_from = now;
    a.map_from.clear();
    a.map_to.clear();
    a.rolls.clear();
    a.ripple_at = Some(now);
    after(app);
}

/// Stop keeps what was counted.
fn stop(app: &mut App) {
    if let Scan::Running { cancel, .. } = &app.disk.scan {
        cancel.store(true, Ordering::Relaxed);
        log_line(app, "disk: stop pressed");
    }
}

fn scan_again(app: &mut App) {
    let root = match &app.disk.scan {
        Scan::Done(d) => d.tree.root_path().to_path_buf(),
        Scan::Failed { root, .. } | Scan::Running { root, .. } | Scan::Released { root } => {
            root.clone()
        }
        Scan::Idle => PathBuf::from("C:\\"),
    };
    scan_path(app, root, true);
}

pub fn retry_scan(app: &mut App) {
    scan_again(app);
    open(app);
}

/// WM_APP_DISK: scan progress, results, suggestions and moves, drained at once.
pub fn on_msg(app: &mut App) {
    app.disk.posted.store(false, Ordering::Release);
    let msgs: Vec<Msg> = app.disk.rx.try_iter().collect();
    for msg in msgs {
        match msg {
            Msg::Live(id, live) => {
                if let Scan::Running {
                    id: cur, live: l, ..
                } = &mut app.disk.scan
                {
                    if *cur == id {
                        let now = Instant::now();
                        for r in live.rows.iter().filter(|r| r.dir) {
                            let e = app
                                .disk
                                .live_seen
                                .entry(r.name.clone())
                                .or_insert((r.size, now));
                            if e.0 != r.size {
                                *e = (r.size, now);
                            }
                        }
                        if live.counted != app.disk.anim.total {
                            app.disk.anim.total = live.counted;
                            app.disk.anim.total_at = now;
                        }
                        *l = live;
                    }
                }
            }
            Msg::Scanned(id, res, took) => on_scanned(app, id, res, took),
            Msg::Suggestions(list, bin) => {
                let total: u64 = list.iter().map(|s| s.total()).sum();
                log_line(
                    app,
                    &format!("disk: safe to clean read, {} to review", m::size(total)),
                );
                app.disk.clean = Clean::Ready { list, bin };
                app.disk.clean_checked.clear();
                if let Some(r) = &mut app.disk.review {
                    if let Clean::Ready { list, .. } = &app.disk.clean {
                        r.items = list
                            .iter()
                            .find(|s| s.kind == r.kind)
                            .map(|s| s.items.clone())
                            .unwrap_or_default();
                        r.checked.clear();
                        r.first = 0;
                    }
                }
            }
            Msg::Rules(measured) => {
                let list = m::CleanList::new(measured.found);
                log_line(
                    app,
                    &format!(
                        "disk: cleaning rules measured, {} to review",
                        m::size(list.total())
                    ),
                );
                app.disk.rules = Some(list);
            }
            Msg::Recycled(report, since, source) => on_recycled(app, report, since, source),
        }
    }
    after(app);
}

fn on_scanned(app: &mut App, id: u64, res: Result<Tree, ScanError>, took: Duration) {
    let Scan::Running {
        id: cur,
        root,
        user,
        space,
        ..
    } = &app.disk.scan
    else {
        return;
    };
    if *cur != id {
        return;
    }
    let (root, user, space) = (root.clone(), *user, *space);
    // The veil's pill keeps its last words while it fades over the results.
    app.disk.anim.veil_text = live_text(app);
    let shown = window_shown(app);
    match res {
        Ok(tree) => {
            let n = tree.node(tree.root());
            let status = tree.status();
            log_line(
                app,
                &format!(
                    "disk: scanned {} in {:.2}s: {} files, {} folders, {} ({status:?}, {} skipped)",
                    root.display(),
                    took.as_secs_f32(),
                    n.files(),
                    n.dirs(),
                    m::size(n.size()),
                    tree.skipped().len()
                ),
            );
            let volume = if m::is_drive_root(&root) {
                volume_label(&root)
            } else {
                String::new()
            };
            let bin = ad::has_recycle_bin(&root);
            let space = space.or_else(|| ad::drive_space(&root).ok());
            let words = done_words(&root, n.size(), space);
            app.disk.scan = Scan::Done(Box::new(Done {
                tree,
                space,
                at: SystemTime::now(),
                volume,
                bin,
            }));
            app.disk.removed = Removed::default();
            app.disk.removed_gen += 1;
            app.disk.level = None;
            app.disk.checked.clear();
            app.disk.last_move = None;
            let root_id = app.disk.done().map(|d| d.tree.root());
            // The live tiles reflow into the final ones and the numbers roll on: going to the
            // root here is not a new level.
            let a = &mut app.disk.anim;
            let keep = (
                std::mem::take(&mut a.map_from),
                std::mem::take(&mut a.map_to),
                std::mem::take(&mut a.rolls),
                a.ripple_at,
            );
            go_to(app, root_id);
            let a = &mut app.disk.anim;
            (a.map_from, a.map_to, a.rolls, a.ripple_at) = keep;
            if app.disk.place == Place::Scanning {
                app.disk.place = Place::Results;
            }
            if !shown && status == ScanStatus::Complete {
                let p = Pill {
                    icon: Icon::Tick,
                    dot: false,
                    title: "Scan done".into(),
                    value: words,
                    action: Some(("Open".into(), PillAction::OpenDisk)),
                    closable: true,
                    ttl_ms: 6000,
                };
                notices::notify(app, notices::Source::Disk, p);
            }
        }
        Err(error) => {
            log_line(
                app,
                &format!("disk: scan of {} failed ({error})", root.display()),
            );
            let (place, value, _) = m::scan_error_words(&root, &error);
            if app.disk.place == Place::Scanning {
                to_start(app);
            }
            if user && !shown {
                app.disk.attention = true;
                let p = Pill {
                    icon: Icon::Alert,
                    dot: false,
                    title: place,
                    value,
                    action: Some(("Retry".into(), PillAction::RetryScan)),
                    closable: true,
                    ttl_ms: 8000,
                };
                notices::notify(app, notices::Source::Disk, p);
            }
            app.disk.scan = Scan::Failed { root, error };
            app.disk.anim.veil = Fade::new(0.0);
        }
    }
    orb::render_orb(app);
}

/// "C: 438 GB used", or "clippy 1.5 GB" for a folder.
fn done_words(root: &Path, counted: u64, space: Option<DriveSpace>) -> String {
    match space.filter(|_| m::is_drive_root(root)) {
        Some(s) => format!("{} {} used", m::place_name(root), m::size_round(s.used())),
        None => format!("{} {}", m::place_name(root), m::size(counted)),
    }
}

/// The ring for the orb while a scan runs: bytes counted over bytes in use on a drive, or the
/// travelling arc for a folder (its size is not known until it is counted).
pub fn ring(app: &App) -> Option<Ring> {
    match &app.disk.scan {
        Scan::Running {
            space: Some(s),
            live,
            ..
        } => Some(Ring::Progress {
            fraction: m::drive_percent(*s, live.on_disk, true) / 100.0,
            tone: Tone::Ink,
        }),
        Scan::Running { space: None, .. } => Some(Ring::Reading),
        _ => None,
    }
}

pub fn spinning(app: &App) -> bool {
    app.visible
        && ring(app) == Some(Ring::Reading)
        && habits::ring(app).is_none()
        && !downloads::reduced_motion()
}

pub fn spin_start(app: &App) -> f32 {
    if downloads::reduced_motion() {
        0.0
    } else {
        (app.disk.spin_from.elapsed().as_secs_f32() * 0.8).fract()
    }
}

fn ring_moved(a: Option<Ring>, b: Option<Ring>) -> bool {
    match (a, b) {
        (Some(Ring::Progress { fraction: f, .. }), Some(Ring::Progress { fraction: g, .. })) => {
            (f - g).abs() >= 1.0 / 360.0
        }
        _ => a != b,
    }
}

/// After any change: the orb's ring, the hover pill's numbers, and the open window.
fn after(app: &mut App) {
    level(app);
    let r = ring(app);
    if ring_moved(app.disk.last_ring, r) {
        app.disk.last_ring = r;
        orb::render_orb(app);
        if spinning(app) {
            start_anim(app);
        }
    }
    if app.pill.as_ref().is_some_and(is_hover_pill) && !app.pill_closing {
        match hover_pill(app) {
            Some(p) => notices::live(app, p),
            None => dismiss_pill(app),
        }
    }
    disk_window::refresh(app);
}

fn is_hover_pill(p: &Pill) -> bool {
    p.action.is_none() && p.title.starts_with("Scanning ")
}

fn hover_pill(app: &App) -> Option<Pill> {
    let Scan::Running {
        root, space, live, ..
    } = &app.disk.scan
    else {
        return None;
    };
    let value = m::scanning_value(
        space.map(|s| m::drive_percent(s, live.on_disk, true)),
        live.counted,
        live.files,
    );
    Some(Pill {
        icon: Icon::Search,
        dot: false,
        title: format!("Scanning {}", m::place_name(root)),
        value,
        action: None,
        closable: false,
        ttl_ms: 6000,
    })
}

/// The pointer came close while scanning: the pill names the ring and keeps counting.
pub fn on_orb_near(app: &mut App, close: bool) {
    if !close {
        app.disk.hover_named = false;
        return;
    }
    if app.disk.hover_named || app.pill.is_some() || app.dial_open || habits::ring(app).is_some() {
        return;
    }
    if let Some(p) = hover_pill(app) {
        notices::reply(app, p);
        app.disk.hover_named = true;
    }
}

pub fn on_orb_gone(app: &mut App) {
    app.disk.hover_named = false;
    if !app.pill_closing && app.pill.as_ref().is_some_and(is_hover_pill) {
        dismiss_pill(app);
    }
}

pub fn dial_child(app: &App) -> ui::DialChild {
    let value = match &app.disk.scan {
        Scan::Running {
            space: Some(s),
            live,
            ..
        } => format!(
            "scanning {}%",
            m::drive_percent(*s, live.on_disk, true).floor() as u64
        ),
        Scan::Running { .. } => "scanning".into(),
        _ => ad::drive_space(Path::new("C:\\"))
            .map(|s| format!("C: {} free", m::size_round(s.free)))
            .unwrap_or_default(),
    };
    ui::DialChild {
        text: String::new(),
        icon: Some(Icon::HardDrive),
        name: "Disk".into(),
        value,
    }
}

pub fn dial_value(app: &App) -> Option<String> {
    match &app.disk.scan {
        Scan::Running {
            space: Some(s),
            live,
            ..
        } => Some(format!(
            "scanning {}%",
            m::drive_percent(*s, live.on_disk, true).floor() as u64
        )),
        Scan::Running { .. } => Some("scanning".into()),
        _ => None,
    }
}

// ---- moving to the Recycle Bin, and Undo ----

/// One sheet line per path: its name, and for Check first its folder under it.
fn path_lines(items: &[(PathBuf, u64)], check_first: bool) -> Vec<(bool, String, String, u64)> {
    items
        .iter()
        .map(|(p, s)| {
            let name = p.file_name().map_or_else(
                || p.display().to_string(),
                |n| n.to_string_lossy().into_owned(),
            );
            let sub = if check_first {
                p.parent()
                    .map(|q| q.display().to_string())
                    .unwrap_or_default()
            } else {
                String::new()
            };
            (check_first, name, sub, *s)
        })
        .collect()
}

fn ask(
    app: &mut App,
    items: Vec<(PathBuf, u64)>,
    lines: Vec<(bool, String, String, u64)>,
    source: Source,
) {
    if items.is_empty() || app.disk.moving {
        return;
    }
    if !ad::has_recycle_bin(&items[0].0) {
        let drive = m::drive_label(&items[0].0);
        log_line(
            app,
            &format!("disk: {drive} has no Recycle Bin, only Open in Explorer"),
        );
        app.disk.note = Some(Note::NoBin(drive, items[0].0.clone()));
        return;
    }
    log_line(app, &format!("disk: confirm {} item(s)", items.len()));
    app.disk.confirm = Some(Confirm {
        items,
        lines,
        rules: Vec::new(),
        source,
        on_move: false,
    });
}

/// Move to Recycle Bin on the confirm sheet: the only path to `move_to_recycle_bin`.
fn confirm_move(app: &mut App) {
    let Some(c) = app.disk.confirm.take() else {
        return;
    };
    app.disk.moving = true;
    // Cleaning-rule folders go through `recycle_rule_items`, which checks each path again
    // (never-touch list, inside its rule, no junctions); everything else goes straight to the bin.
    let rules = c.rules;
    let paths: Vec<PathBuf> = c
        .items
        .iter()
        .map(|(p, _)| p.clone())
        .filter(|p| !rules.iter().any(|(_, q)| q == p))
        .collect();
    app.disk.asked = c.items;
    log_line(
        app,
        &format!(
            "disk: moving {} item(s) and {} rule folder(s) to the Recycle Bin",
            paths.len(),
            rules.len()
        ),
    );
    let (rep, source) = (reporter(app), c.source);
    std::thread::spawn(move || {
        let since = SystemTime::now();
        let mut report = if paths.is_empty() {
            RecycleReport::default()
        } else {
            ad::move_to_recycle_bin(&paths)
        };
        if !rules.is_empty() {
            let r = ad::recycle_rule_items(&ad::RuleEnv::from_process(), &rules);
            report.moved.extend(r.moved);
            report.failed.extend(r.failed);
        }
        rep.send(Msg::Recycled(report, since, source));
    });
}

fn on_recycled(app: &mut App, report: RecycleReport, since: SystemTime, source: Source) {
    app.disk.moving = false;
    let failed: Vec<String> = report
        .failed
        .iter()
        .map(|(p, e)| format!("{} ({e:?})", p.display()))
        .collect();
    log_line(
        app,
        &format!(
            "disk: moved {} to the Recycle Bin, {} stayed {failed:?}",
            report.moved.len(),
            report.failed.len()
        ),
    );
    mark_moved(app, &report.moved);
    let in_use: Vec<PathBuf> = report
        .failed
        .iter()
        .filter(|(_, e)| *e == RecycleError::InUse)
        .map(|(p, _)| p.clone())
        .collect();
    let other = report.failed.len() - in_use.len();
    let sizes: BTreeMap<PathBuf, u64> = app.disk.asked.drain(..).collect();
    let stayed = report
        .failed
        .iter()
        .map(|(p, _)| (p.clone(), sizes.get(p).copied().unwrap_or(0)))
        .collect();
    app.disk.note = match m::moved_words(report.moved.len(), &in_use, other) {
        Some((title, body)) => Some(Note::Partial {
            title,
            body,
            moved: report.moved.len(),
            stayed,
            source,
        }),
        None if !report.moved.is_empty() => Some(Note::Moved(report.moved.len())),
        None => None,
    };
    if report.moved.is_empty() {
        return;
    }
    let merge = std::mem::take(&mut app.disk.merge_next);
    let n = report.moved.len();
    match app.disk.last_move.as_mut().filter(|_| merge) {
        Some(last) => {
            last.paths.extend(report.moved);
            last.offered = false;
        }
        None => {
            app.disk.last_move = Some(LastMove {
                paths: report.moved,
                since,
                source,
                offered: false,
            })
        }
    }
    let all = app.disk.last_move.as_ref().map_or(n, |l| l.paths.len());
    if let Some(Note::Moved(k)) = &mut app.disk.note {
        *k = all;
    }
    if !window_shown(app) {
        let (title, value) = m::in_bin_words(all);
        notices::reply(
            app,
            Pill {
                icon: Icon::Trash,
                dot: false,
                title,
                value,
                action: Some(("Undo".into(), PillAction::UndoRecycle)),
                closable: true,
                ttl_ms: 6000,
            },
        );
    }
}

/// Takes moved items out of the results and the suggestions.
fn mark_moved(app: &mut App, moved: &[PathBuf]) {
    if let Scan::Done(d) = &app.disk.scan {
        let ids: Vec<NodeId> = moved.iter().filter_map(|p| d.tree.find(p)).collect();
        for id in ids {
            let tree = &d.tree;
            app.disk.removed.add(tree, id);
            app.disk.checked.remove(&id);
        }
        app.disk.removed_gen += 1;
    }
    if let Clean::Ready { list, .. } = &mut app.disk.clean {
        for s in list.iter_mut() {
            s.items.retain(|c| !moved.contains(&c.path));
        }
    }
    if let Some(r) = &mut app.disk.review {
        r.items.retain(|c| !moved.contains(&c.path));
        r.checked.clear();
        r.first = r.first.min(r.items.len().saturating_sub(1));
    }
    app.disk.clean_checked.clear();
    if let Some(r) = &mut app.disk.rules {
        r.forget(moved);
        r.select_tier(m::TierFilter::All, false);
    }
    app.disk.sel = None;
}

/// Undo on the pill or the card: what the last move sent to the bin goes back.
pub fn undo(app: &mut App) {
    let Some(last) = app.disk.last_move.take() else {
        return;
    };
    let results = disk_restore::restore(&last.paths, last.since);
    let back: Vec<PathBuf> = results
        .iter()
        .filter(|(_, r)| r.is_ok())
        .map(|(p, _)| p.clone())
        .collect();
    let stayed: Vec<String> = results
        .iter()
        .filter_map(|(p, r)| r.err().map(|e| format!("{} ({e:?})", p.display())))
        .collect();
    log_line(
        app,
        &format!(
            "disk: undo put back {}, {} stayed {stayed:?}",
            back.len(),
            stayed.len()
        ),
    );
    if let Scan::Done(d) = &app.disk.scan {
        for p in &back {
            if let Some(id) = d.tree.find(p) {
                app.disk.removed.restore(id);
            }
        }
        app.disk.removed_gen += 1;
    }
    if matches!(last.source, Source::Clean(_) | Source::Rules)
        || matches!(app.disk.clean, Clean::Ready { .. })
    {
        read_suggestions(app);
    }
    app.disk.note = Some(Note::Restored {
        back: back.len(),
        stayed: stayed.len(),
    });
    after(app);
}

// ---- Safe to clean ----

fn clean_places() -> CleanPlaces {
    match std::env::var_os("ORB_DISK_CLEAN")
        .filter(|v| !v.is_empty())
        .map(PathBuf::from)
    {
        // A test drive points the suggestions at a tree it made, never at real files.
        Some(root) => CleanPlaces {
            downloads: Some(root.join("Downloads")),
            temp: vec![root.join("Temp")],
            browser_user_data: vec![root.join("Browser").join("User Data")],
        },
        None => CleanPlaces::for_current_user(),
    }
}

fn read_suggestions(app: &mut App) {
    if matches!(app.disk.clean, Clean::Reading) {
        return;
    }
    app.disk.clean = Clean::Reading;
    let rep = reporter(app);
    let test_tree = clean_test_root().is_some();
    std::thread::spawn(move || {
        let places = clean_places();
        let list = ad::safe_to_clean(&places, SystemTime::now(), OLD_DAYS);
        let bin = places
            .downloads
            .as_deref()
            .or(places.temp.first().map(|p| p.as_path()))
            .and_then(|p| ad::recycle_bin_size(p).ok());
        rep.send(Msg::Suggestions(list, bin));
        // A test drive (ORB_DISK_CLEAN) points at a made-up tree, so it never measures the real
        // tool caches.
        let measured = if test_tree {
            ad::Measured {
                found: Vec::new(),
                complete: true,
            }
        } else {
            ad::measure_rules(
                &ad::RuleEnv::from_process(),
                ad::RULES,
                &AtomicBool::new(false),
            )
        };
        rep.send(Msg::Rules(measured));
    });
}

fn clean_test_root() -> Option<PathBuf> {
    std::env::var_os("ORB_DISK_CLEAN")
        .filter(|v| !v.is_empty())
        .map(PathBuf::from)
}

fn open_clean(app: &mut App) {
    if app.disk.place != Place::Clean && app.disk.place != Place::Review {
        app.disk.clean_from = app.disk.place;
    }
    app.disk.place = Place::Clean;
    app.disk.cursor = None;
    app.disk.note = None;
    app.disk.clean_first = 0;
    enter_rows(app);
    read_suggestions(app);
    log_line(app, "disk: safe to clean");
}

/// Rows enter with the ripple (a new level, tab, list or filter).
fn enter_rows(app: &mut App) {
    app.disk.anim.ripple_at = (!downloads::reduced_motion()).then(Instant::now);
    app.disk
        .anim
        .rolls
        .retain(|k, _| !k.starts_with("row:") && !k.starts_with("pct:"));
}

fn tier_filter(f: Filter) -> m::TierFilter {
    match f {
        Filter::All => m::TierFilter::All,
        Filter::Safe => m::TierFilter::Safe,
        Filter::Check => m::TierFilter::CheckFirst,
    }
}

/// The suggestions Safe to clean lists under the filter (by index), then the cleaning rules'
/// rows (from `RULE_BASE`), in the order the screen shows them.
fn clean_order(app: &App) -> Vec<usize> {
    let Clean::Ready { list, .. } = &app.disk.clean else {
        return Vec::new();
    };
    let filter = tier_filter(app.disk.filter);
    let mut out: Vec<usize> = list
        .iter()
        .enumerate()
        .filter(|(_, s)| !s.items.is_empty() && filter.admits(s.kind.tag()))
        .map(|(i, _)| i)
        .collect();
    if let Some(rules) = &app.disk.rules {
        for g in rules.groups_for(filter) {
            out.extend(g.items.iter().filter_map(|it| rule_index(it.id)));
        }
    }
    out
}

/// The three classic suggestions sit under this heading; the rules bring their own groups.
const SUGGESTED: &str = "Suggested";

/// A ticked or listed rule row, by its index.
fn rule_item(app: &App, i: usize) -> Option<&m::CleanItem> {
    let id = rule_id(i)?;
    app.disk.rules.as_ref()?.items().find(|it| it.id == id)
}

/// The cleaning-rule folders among `paths`, each with the rule it belongs to (for a retry, so
/// rule folders always go through `recycle_rule_items` and its checks).
fn rule_pairs(app: &App, paths: &[PathBuf]) -> Vec<(&'static ad::Rule, PathBuf)> {
    let Some(rules) = &app.disk.rules else {
        return Vec::new();
    };
    paths
        .iter()
        .filter_map(|p| {
            rules
                .items()
                .find(|it| it.paths.contains(p))
                .map(|it| (it.rule, p.clone()))
        })
        .collect()
}

/// A rule group's chip. Ui.rs has no package, code or chat glyph yet, so these are the nearest.
fn group_icon(g: ad::Group) -> Icon {
    match g {
        ad::Group::DeveloperCaches => Icon::Archive,
        ad::Group::AppCaches => Icon::Layers,
        ad::Group::BrowserCaches => Icon::Globe,
        ad::Group::AiModels => Icon::Grid,
        ad::Group::CrashReports => Icon::File,
    }
}

fn open_review(app: &mut App, i: usize) {
    let Clean::Ready { list, .. } = &app.disk.clean else {
        return;
    };
    let Some(s) = list.get(i).filter(|s| !s.items.is_empty()) else {
        return;
    };
    app.disk.review = Some(Review {
        kind: s.kind,
        items: s.items.clone(),
        checked: BTreeSet::new(),
        first: 0,
    });
    app.disk.place = Place::Review;
    app.disk.cursor = None;
    app.disk.note = None;
    let kind = s.kind;
    enter_rows(app);
    log_line(app, &format!("disk: review {kind:?}"));
}

// ---- the window ----

fn window_shown(app: &App) -> bool {
    app.disk.win.open
}

pub fn open(app: &mut App) {
    unsafe {
        let _ = KillTimer(Some(app.hwnd), TIMER_DISK_RELEASE);
    }
    if let Scan::Released { root } = &app.disk.scan {
        let root = root.clone();
        log_line(app, "disk: scanning again, the last tree was let go");
        scan_path(app, root, true);
    }
    if matches!(app.disk.scan, Scan::Done(_)) && !window_shown(app) {
        // A scan that finished while the window was closed opens straight on its results.
        app.disk.anim.veil = Fade::new(0.0);
        app.disk.anim.veil_text = None;
    }
    if app.disk.attention {
        app.disk.attention = false;
        orb::render_orb(app);
    }
    let offers_undo = match &app.disk.note {
        Some(Note::Moved(_)) => true,
        Some(Note::Partial { moved, .. }) => *moved > 0,
        _ => false,
    };
    if offers_undo && !undo_fresh(app) {
        app.disk.note = None;
    }
    app.disk.place = match (&app.disk.scan, app.disk.place) {
        (Scan::Running { .. }, _) => Place::Scanning,
        (Scan::Done(_), Place::Clean | Place::Review) => app.disk.place,
        (Scan::Done(_), _) => Place::Results,
        (_, Place::Clean | Place::Review) => app.disk.place,
        _ => Place::Start,
    };
    if app.disk.place == Place::Start {
        to_start(app);
    }
    app.disk.kb = false;
    disk_window::show(app);
}

/// Every way into the drive list goes through here, so the list is never stale or empty.
fn to_start(app: &mut App) {
    app.disk.place = Place::Start;
    app.disk.drives = fixed_drives()
        .into_iter()
        .filter_map(|r| {
            ad::drive_space(&r)
                .ok()
                .map(|s| (r.clone(), volume_label(&r), s))
        })
        .collect();
}

fn go_to(app: &mut App, at: Option<NodeId>) {
    app.disk.at = at;
    app.disk.sel = None;
    app.disk.first = 0;
    app.disk.files_first = 0;
    app.disk.cursor = None;
    app.disk.level = None;
    app.disk.anim.map_from.clear();
    app.disk.anim.map_to.clear();
    enter_rows(app);
}

/// The cached level for the place the results show.
fn level(app: &mut App) {
    if !matches!(app.disk.scan, Scan::Done(_)) {
        return;
    }
    let map = dp::map_rect(&app.g, &app.fonts, app.disk.win.room, &cards(app));
    let Scan::Done(d) = &app.disk.scan else {
        return;
    };
    let at = app.disk.at.unwrap_or(d.tree.root());
    if app
        .disk
        .level
        .as_ref()
        .is_some_and(|l| l.at == at && l.removed_gen == app.disk.removed_gen && l.map == map)
    {
        return;
    }
    let tree = &d.tree;
    let removed = &app.disk.removed;
    let rows = m::rows(tree, removed, at);
    let summary = m::summarize(tree, removed, at, TOP_FILES);
    let inner_of = |id: NodeId| {
        m::rows(tree, removed, id)
            .into_iter()
            .take(6)
            .map(|r| (r.name, r.size))
            .collect()
    };
    let inner = |i: usize, rect: Rect| -> Vec<(Rect, String, u64)> {
        if !rows[i].dir {
            return Vec::new();
        }
        m::treemap(&rows[i..=i], rect, &inner_of)
            .into_iter()
            .next()
            .map(|t| t.inner)
            .unwrap_or_default()
    };
    let sizes: Vec<u64> = rows.iter().map(|r| r.size).collect();
    let tiles = fit_tiles(&sizes, map, 0, &inner);
    let files = summary
        .top
        .iter()
        .map(|&id| {
            let n = tree.node(id);
            let name = n.name().to_string_lossy().into_owned();
            let month = n
                .modified()
                .map(|t| m::month_year(stamp(t)))
                .unwrap_or_default();
            let row = dp::FileRow {
                icon: file_icon(n.name()),
                // An unfinished download says so instead of naming its folder.
                sub: m::file_label(n.name())
                    .map(String::from)
                    .unwrap_or_else(|| m::folder_below(tree, at, id)),
                month,
                size: m::size(n.size()),
                name,
                checked: false,
            };
            (id, row)
        })
        .collect();
    app.disk.level = Some(Level {
        at,
        removed_gen: app.disk.removed_gen,
        rows,
        summary,
        map,
        tiles,
        files,
    });
}

/// The smallest tile that still gets a place of its own; smaller rows roll into "N smaller".
const TILE_MIN: (f32, f32) = (80.0, 36.0);

/// The treemap tiles: as many of the biggest rows as each get at least 80 × 36 (at most nine),
/// one "N smaller" tile for the rest, and while scanning a dotted tile for what is not counted.
fn fit_tiles(
    sizes: &[u64],
    r: Rect,
    skeleton: u64,
    inner: &dyn Fn(usize, Rect) -> Vec<(Rect, String, u64)>,
) -> Vec<dp::MapTile> {
    let n = sizes.iter().take_while(|s| **s > 0).count();
    if n == 0 && skeleton == 0 {
        return Vec::new();
    }
    let mut shown = n.min(m::MAX_TILES);
    loop {
        let rest: u64 = sizes[shown..].iter().sum();
        let mut vals: Vec<f64> = sizes[..shown].iter().map(|&s| s as f64).collect();
        if rest > 0 {
            vals.push(rest as f64);
        }
        if skeleton > 0 {
            vals.push(skeleton as f64);
        }
        let rects = m::squarify(&vals, r);
        let fits = rects[..shown]
            .iter()
            .all(|q| q.2 - q.0 >= TILE_MIN.0 && q.3 - q.1 >= TILE_MIN.1);
        if fits || shown <= 1 {
            let mut out: Vec<dp::MapTile> = (0..shown)
                .map(|i| dp::MapTile {
                    row: Some(i),
                    rect: rects[i],
                    inner: inner(i, rects[i]),
                    more: 0,
                    more_bytes: 0,
                    skeleton: false,
                })
                .collect();
            let mut k = shown;
            if rest > 0 {
                out.push(dp::MapTile {
                    row: None,
                    rect: rects[k],
                    inner: Vec::new(),
                    more: sizes.len() - shown,
                    more_bytes: rest,
                    skeleton: false,
                });
                k += 1;
            }
            if skeleton > 0 {
                out.push(dp::MapTile {
                    row: None,
                    rect: rects[k],
                    inner: Vec::new(),
                    more: 0,
                    more_bytes: 0,
                    skeleton: true,
                });
            }
            return out;
        }
        shown -= 1;
    }
}

/// A tile's key for the reflow: the row's name, "more" or "uncounted".
fn tile_key(t: &dp::MapTile, name: impl Fn(usize) -> String) -> String {
    match t.row {
        Some(i) => format!("row:{}", name(i)),
        None if t.skeleton => "uncounted".into(),
        None => "more".into(),
    }
}

fn file_icon(name: &std::ffi::OsStr) -> Icon {
    match Category::of(name) {
        Category::Video => Icon::Film,
        Category::Archives => Icon::Archive,
        _ => Icon::File,
    }
}

fn stamp(t: SystemTime) -> Stamp {
    let secs = t
        .duration_since(SystemTime::UNIX_EPOCH)
        .map_or(0, |d| d.as_secs() as i64);
    let mo = store::local(secs);
    let d = mo.date();
    Stamp {
        year: d.year(),
        month: d.month(),
        day: d.day(),
        minute: mo.minute_of_day(),
    }
}

fn now_stamp() -> Stamp {
    stamp(SystemTime::now())
}

fn selected_words(n: usize, bytes: u64) -> Option<(String, String)> {
    (n > 0).then(|| (format!("{n} selected"), dp::size_label(bytes)))
}

/// The ticked tree items, without any inside another ticked one (the folder takes them along).
fn top_checked(app: &App) -> Vec<NodeId> {
    let Some(d) = app.disk.done() else {
        return Vec::new();
    };
    let set = &app.disk.checked;
    set.iter()
        .copied()
        .filter(|&id| {
            let chain = d.tree.ancestry(id);
            !chain[..chain.len().saturating_sub(1)]
                .iter()
                .any(|a| set.contains(a))
        })
        .collect()
}

/// What is ticked on the screen showing: how many, and their bytes.
fn selection(app: &App) -> Option<(usize, u64)> {
    let (n, bytes) = match app.disk.place {
        Place::Results => {
            let d = app.disk.done()?;
            let ids = top_checked(app);
            let bytes = ids
                .iter()
                .map(|&id| app.disk.removed.size(&d.tree, id))
                .sum();
            (ids.len(), bytes)
        }
        Place::Clean => {
            let Clean::Ready { list, .. } = &app.disk.clean else {
                return None;
            };
            let bytes: u64 = app
                .disk
                .clean_checked
                .iter()
                .filter_map(|&i| list.get(i))
                .map(|s| s.total())
                .sum();
            let (rn, rb) = app.disk.rules.as_ref().map_or((0, 0), |r| r.selection());
            (app.disk.clean_checked.len() + rn, bytes + rb)
        }
        Place::Review => {
            let r = app.disk.review.as_ref()?;
            let bytes = r
                .checked
                .iter()
                .filter_map(|&i| r.items.get(i))
                .map(|c| c.size)
                .sum();
            (r.checked.len(), bytes)
        }
        _ => return None,
    };
    (n > 0).then_some((n, bytes))
}

/// The value a number shows this frame: rolling toward `target` when it has a roll.
fn rolled(app: &App, key: &str, target: f64, now: Instant) -> f64 {
    match app.disk.anim.rolls.get(key) {
        Some(r) if (r.to - target).abs() < 1e-9 => r.value(now),
        _ => target,
    }
}

/// Windows' own files on a drive's top level, shown muted as "system".
fn system_file(name: &str) -> bool {
    [
        "pagefile.sys",
        "hiberfil.sys",
        "swapfile.sys",
        "DumpStack.log.tmp",
    ]
    .iter()
    .any(|s| s.eq_ignore_ascii_case(name))
}

/// "· scanned 09:12", or "· scanned 6 Nov" for an older scan.
fn scanned_words(at: SystemTime) -> String {
    let (s, today) = (stamp(at), now_stamp());
    if (s.year, s.month, s.day) == (today.year, today.month, today.day) {
        format!("· scanned {:02}:{:02}", s.minute / 60, s.minute % 60)
    } else {
        format!("· scanned {}", m::day_month(s))
    }
}

fn chip_words(root: &Path, volume: &str) -> String {
    if m::is_drive_root(root) {
        format!("({}) {}", m::drive_label(root), volume)
    } else {
        m::place_name(root)
    }
}

/// The capacity bar: the four biggest top-level items, System files (in use but not counted),
/// Other, then free space. A folder scan has no free space or system share.
fn segments(
    top: &[(String, u64)],
    space: Option<DriveSpace>,
    counted: u64,
    uncounted: Option<u64>,
    linked: bool,
    counting: Option<&str>,
) -> Vec<dp::Seg> {
    let named = if space.is_some() { 4 } else { 5 };
    let mut out: Vec<dp::Seg> = top
        .iter()
        .take(named)
        .filter(|(_, b)| *b > 0)
        .enumerate()
        .map(|(i, (name, b))| dp::Seg {
            label: if counting == Some(name.as_str()) {
                format!("{name}, so far")
            } else {
                name.clone()
            },
            bytes: *b,
            kind: dp::SegKind::Grey(i),
            row: linked.then_some(i),
        })
        .collect();
    let rest: u64 = top.iter().skip(named).map(|x| x.1).sum();
    match (space, uncounted) {
        (Some(_), Some(left)) => {
            if rest > 0 {
                out.push(dp::Seg {
                    label: "Other".into(),
                    bytes: rest,
                    kind: dp::SegKind::Grey(5),
                    row: None,
                });
            }
            if left > 0 {
                out.push(dp::Seg {
                    label: "Not counted yet".into(),
                    bytes: left,
                    kind: dp::SegKind::Dotted,
                    row: None,
                });
            }
        }
        (Some(s), None) => {
            let system = s.used().saturating_sub(counted);
            // Only a share worth a segment: restore points, the page file's slack, metadata.
            if system > s.total / 200 {
                out.push(dp::Seg {
                    label: "System files".into(),
                    bytes: system,
                    kind: dp::SegKind::Grey(4),
                    row: None,
                });
            }
            if rest > 0 {
                out.push(dp::Seg {
                    label: "Other".into(),
                    bytes: rest,
                    kind: dp::SegKind::Grey(5),
                    row: None,
                });
            }
        }
        (None, _) => {
            if rest > 0 {
                out.push(dp::Seg {
                    label: "Other".into(),
                    bytes: rest,
                    kind: dp::SegKind::Grey(5),
                    row: None,
                });
            }
        }
    }
    if let Some(s) = space {
        out.push(dp::Seg {
            label: "Free".into(),
            bytes: s.free,
            kind: dp::SegKind::Free,
            row: None,
        });
    }
    out
}

/// The capacity bar for what the window shows now.
fn current_segs(app: &App) -> Vec<dp::Seg> {
    match &app.disk.scan {
        Scan::Running {
            root, space, live, ..
        } => {
            let top: Vec<(String, u64)> =
                live.rows.iter().map(|r| (r.name.clone(), r.size)).collect();
            let counting = counting_folder(root, live);
            let left = space.map(|s| s.used().saturating_sub(live.on_disk));
            segments(&top, *space, live.counted, left, true, counting.as_deref())
        }
        Scan::Done(d) => {
            let tree = &d.tree;
            let root = tree.root();
            let space = d.space.filter(|_| m::is_drive_root(tree.root_path()));
            let top: Vec<(String, u64)> = m::rows(tree, &app.disk.removed, root)
                .into_iter()
                .map(|r| (r.name, r.size))
                .collect();
            let at_root = app.disk.at.is_none_or(|a| a == root);
            let counted = app.disk.removed.size(tree, root);
            segments(&top, space, counted, None, at_root, None)
        }
        _ => Vec::new(),
    }
}

/// The top-level folder the scan is inside now.
fn counting_folder(root: &Path, live: &Live) -> Option<String> {
    let rest = Path::new(&live.current).strip_prefix(root).ok()?;
    rest.components()
        .next()
        .map(|c| c.as_os_str().to_string_lossy().into_owned())
}

/// Top-level folders whose bytes have held 1.5 s and the scan is not inside: (counted, all).
fn folders_counted(app: &App, root: &Path, live: &Live, now: Instant) -> (usize, usize) {
    let inside = counting_folder(root, live);
    let dirs: Vec<&LiveRow> = live.rows.iter().filter(|r| r.dir).collect();
    let done = dirs
        .iter()
        .filter(|r| Some(&r.name) != inside.as_ref())
        .filter(|r| {
            app.disk
                .live_seen
                .get(&r.name)
                .is_some_and(|(_, at)| now.saturating_duration_since(*at) >= VEIL_HOLD)
        })
        .count();
    (done, dirs.len())
}

/// The veil pill's words while a scan runs: label, path, progress, footer.
fn live_text(app: &App) -> Option<(String, String, Option<f32>, String)> {
    let Scan::Running {
        root,
        started,
        space,
        live,
        ..
    } = &app.disk.scan
    else {
        return None;
    };
    let place = counting_folder(root, live).unwrap_or_else(|| m::place_name(root));
    let frac = space.map(|s| (m::drive_percent(s, live.on_disk, true) / 100.0) as f32);
    let path = if live.current.is_empty() {
        root.display().to_string()
    } else {
        live.current.clone()
    };
    let foot = format!(
        "{} so far · {}",
        m::plural(live.files, "file", "files"),
        m::clock(started.elapsed())
    );
    Some((format!("Scanning {place}"), path, frac, foot))
}

/// The folder rows, the tiles and the headline numbers the targets of the rolls come from.
fn number_targets(app: &App) -> Vec<(String, f64)> {
    let mut out = Vec::new();
    match &app.disk.scan {
        Scan::Running { live, .. } => {
            out.push(("big".to_string(), live.counted as f64));
            for r in &live.rows {
                out.push((format!("row:{}", r.name), r.size as f64));
                out.push((
                    format!("pct:{}", r.name),
                    if live.counted == 0 {
                        0.0
                    } else {
                        r.size as f64 * 100.0 / live.counted as f64
                    },
                ));
            }
        }
        Scan::Done(d) => {
            let drive = d.space.filter(|_| m::is_drive_root(d.tree.root_path()));
            match drive {
                Some(s) => {
                    out.push(("big".to_string(), s.used() as f64));
                    out.push(("free".to_string(), s.free as f64));
                }
                None => out.push((
                    "big".to_string(),
                    app.disk.removed.size(&d.tree, d.tree.root()) as f64,
                )),
            }
            if let Some(l) = &app.disk.level {
                for r in &l.rows {
                    out.push((format!("row:{}", r.name), r.size as f64));
                    out.push((format!("pct:{}", r.name), r.percent));
                }
            }
        }
        _ => {}
    }
    out
}

/// The row the pointer or the selection points at, for the strip and the lit tile.
fn focus_row(app: &App) -> Option<usize> {
    match app.disk.anim.focus {
        Some(Focus::Row(i)) => Some(i),
        _ => app.disk.sel,
    }
}

fn live_results(app: &App, now: Instant) -> Option<dp::ResultsView> {
    let Scan::Running {
        root,
        space,
        volume,
        live,
        ..
    } = &app.disk.scan
    else {
        return None;
    };
    let place = m::place_name(root);
    let counting = counting_folder(root, live);
    let counted = rolled(app, "big", live.counted as f64, now) as u64;
    let (big, of) = match space.filter(|_| m::is_drive_root(root)) {
        Some(s) => (
            format!("{} counted", m::size_round(counted)),
            format!("of {} used", m::size_round(s.used())),
        ),
        None => (
            format!("{} counted", dp::size_label(counted)),
            format!("in {place}"),
        ),
    };
    let (done, all) = folders_counted(app, root, live, now);
    let biggest = live.rows.first().map_or(1, |r| r.size.max(1));
    let rows: Vec<dp::ListRow> = live
        .rows
        .iter()
        .map(|r| {
            let size = rolled(app, &format!("row:{}", r.name), r.size as f64, now) as u64;
            let target = if live.counted == 0 {
                0.0
            } else {
                r.size as f64 * 100.0 / live.counted as f64
            };
            let pct = rolled(app, &format!("pct:{}", r.name), target, now);
            dp::ListRow {
                name: r.name.clone(),
                bytes: r.size,
                size: dp::size_label(size),
                pct: dp::pct_label(pct),
                pct_v: pct,
                bar: r.size as f32 / biggest as f32,
                dir: r.dir,
                tag: if r.hidden && system_file(&r.name) {
                    Some("system")
                } else if r.hidden {
                    Some("hidden")
                } else {
                    None
                },
                checked: false,
                counting: counting.as_deref() == Some(r.name.as_str()),
            }
        })
        .collect();
    let map = dp::map_rect(&app.g, &app.fonts, app.disk.win.room, &[]);
    let sizes: Vec<u64> = live.rows.iter().map(|r| r.size).collect();
    let skeleton = space.map_or(0, |s| s.used().saturating_sub(live.on_disk));
    let tiles = fit_tiles(&sizes, map, skeleton, &|_, _| Vec::new());
    let (label, path, frac, foot) = live_text(app)?;
    Some(dp::ResultsView {
        folder: !m::is_drive_root(root),
        chip: chip_words(root, volume),
        chip_sub: "· scanning".into(),
        big,
        of,
        right: if all > 0 {
            format!("{done} of {all} folders")
        } else {
            String::new()
        },
        segs: current_segs(app),
        crumbs: vec![place],
        tab: Tab::Folders,
        rows,
        first: 0,
        sel: None,
        tiles,
        types: Vec::new(),
        files: Vec::new(),
        files_first: 0,
        files_count: String::new(),
        selected: None,
        can_recycle: false,
        detail: None,
        scanning: true,
        live: Some(dp::LiveView {
            label,
            path,
            frac,
            foot,
        }),
    })
}

fn results_view(app: &App, now: Instant) -> Option<dp::ResultsView> {
    let d = app.disk.done()?;
    let l = app.disk.level.as_ref()?;
    let tree = &d.tree;
    let root = tree.root_path();
    let folder = !m::is_drive_root(root);
    let drive = d.space.filter(|_| !folder);
    let (big, of, right) = match drive {
        Some(s) => {
            let used = rolled(app, "big", s.used() as f64, now) as u64;
            let free = rolled(app, "free", s.free as f64, now) as u64;
            (
                format!("{} used", m::size_round(used)),
                format!("of {}", m::size_round(s.total)),
                format!("{} free", m::size_round(free)),
            )
        }
        None => {
            let total = app.disk.removed.size(tree, tree.root());
            let shown = rolled(app, "big", total as f64, now) as u64;
            (
                dp::size_label(shown),
                format!("in {}", m::place_name(root)),
                m::plural(tree.node(tree.root()).files(), "file", "files"),
            )
        }
    };
    let crumbs: Vec<String> = tree
        .ancestry(l.at)
        .into_iter()
        .map(|id| {
            if id == tree.root() {
                m::place_name(root)
            } else {
                tree.node(id).name().to_string_lossy().into_owned()
            }
        })
        .collect();
    let biggest = l.rows.first().map_or(1, |r| r.size.max(1));
    let rows: Vec<dp::ListRow> = l
        .rows
        .iter()
        .map(|r| {
            let size = rolled(app, &format!("row:{}", r.name), r.size as f64, now) as u64;
            let pct = rolled(app, &format!("pct:{}", r.name), r.percent, now);
            dp::ListRow {
                name: r.name.clone(),
                bytes: r.size,
                size: dp::size_label(size),
                pct: dp::pct_label(pct),
                pct_v: r.percent,
                bar: r.size as f32 / biggest as f32,
                dir: r.dir,
                tag: if r.hidden && !r.dir && system_file(&r.name) {
                    Some("system")
                } else if r.hidden {
                    Some("hidden")
                } else {
                    None
                },
                checked: app.disk.checked.contains(&r.id),
                counting: false,
            }
        })
        .collect();
    let types = l
        .summary
        .types
        .iter()
        .filter(|t| t.files > 0)
        .enumerate()
        .map(|(i, t)| dp::TypeRow {
            label: if t.category == Category::AppsAndGames {
                "Apps, games"
            } else {
                t.category.label()
            },
            bytes: t.bytes,
            size: dp::size_label(t.bytes),
            files: m::plural(t.files, "file", "files"),
            grey: if t.category == Category::Other { 5 } else { i },
        })
        .collect();
    let files: Vec<dp::FileRow> = l
        .files
        .iter()
        .map(|(id, r)| dp::FileRow {
            name: r.name.clone(),
            sub: r.sub.clone(),
            month: r.month.clone(),
            size: r.size.clone(),
            icon: r.icon,
            checked: app.disk.checked.contains(id),
        })
        .collect();
    let here_total = app.disk.removed.size(tree, tree.root()).max(1);
    let detail = match focus_row(app).and_then(|i| l.rows.get(i)) {
        Some(r) => dp::Detail {
            name: r.name.clone(),
            path: tree.path(r.id).display().to_string(),
            pct: dp::pct_label(r.percent),
            size: dp::size_label(r.size),
        },
        None => {
            let size = app.disk.removed.size(tree, l.at);
            dp::Detail {
                name: if l.at == tree.root() {
                    m::place_name(root)
                } else {
                    tree.node(l.at).name().to_string_lossy().into_owned()
                },
                path: tree.path(l.at).display().to_string(),
                pct: dp::pct_label(size as f64 * 100.0 / here_total as f64),
                size: dp::size_label(size),
            }
        }
    };
    let (n, bytes) = selection(app).unwrap_or((0, 0));
    let veil = app
        .disk
        .anim
        .veil_text
        .clone()
        .filter(|_| app.disk.anim.veil.value(now) > 0.0);
    Some(dp::ResultsView {
        folder,
        chip: chip_words(root, &d.volume),
        chip_sub: scanned_words(d.at),
        big,
        of,
        right,
        segs: current_segs(app),
        crumbs,
        tab: app.disk.tab,
        rows,
        first: app.disk.first,
        sel: app.disk.sel,
        tiles: l.tiles.clone(),
        types,
        files,
        files_first: app.disk.files_first,
        files_count: format!(
            "{} · {}",
            m::plural(l.summary.files, "file", "files"),
            dp::size_label(l.summary.bytes)
        ),
        selected: selected_words(n, bytes),
        can_recycle: d.bin,
        detail: Some(detail),
        scanning: false,
        live: veil.map(|(label, path, _, foot)| dp::LiveView {
            label,
            path,
            frac: Some(1.0),
            foot,
        }),
    })
}

fn kind_words(kind: SuggestionKind) -> (&'static str, Icon) {
    match kind {
        SuggestionKind::OldDownloads => ("Downloads older than 90 days", Icon::Download),
        SuggestionKind::TempFiles => ("Windows temp files", Icon::Layers),
        SuggestionKind::BrowserCaches => ("Browser caches", Icon::Globe),
    }
}

/// A suggestion's one line of plain truth.
fn kind_body(s: &ad::Suggestion) -> String {
    match s.kind {
        SuggestionKind::OldDownloads => {
            let cutoff = SystemTime::now()
                .checked_sub(Duration::from_secs(u64::from(OLD_DAYS) * 86_400))
                .map(|t| m::day_month(stamp(t)))
                .unwrap_or_default();
            let all_files = s.items.iter().all(|c| c.path.is_file());
            let what = if all_files {
                m::plural(s.items.len() as u64, "file", "files")
            } else {
                m::plural(s.items.len() as u64, "item", "items")
            };
            format!("{what} from before {cutoff}. They're yours, so look first.")
        }
        SuggestionKind::TempFiles => {
            "Windows makes new ones when needed. Files in use are skipped.".into()
        }
        SuggestionKind::BrowserCaches => {
            "Edge and Chrome load sites a bit slower for a day. You stay signed in.".into()
        }
    }
}

fn clean_view(app: &App) -> dp::CleanView {
    let drive = app
        .disk
        .done()
        .map_or("C:".to_string(), |d| m::drive_label(d.tree.root_path()));
    let place = match app.disk.done() {
        Some(d) => format!(
            "{drive}, from the {} scan",
            m::when(stamp(d.at), now_stamp())
                .rsplit(' ')
                .next()
                .unwrap_or_default()
        ),
        None => drive.clone(),
    };
    let (n, bytes) = selection(app).unwrap_or((0, 0));
    match &app.disk.clean {
        Clean::Ready { list, bin } => {
            let filter = tier_filter(app.disk.filter);
            let shown: Vec<&ad::Suggestion> = list.iter().filter(|s| !s.items.is_empty()).collect();
            let rule_counts = app
                .disk
                .rules
                .as_ref()
                .map(|r| r.counts())
                .unwrap_or_default();
            let check = shown
                .iter()
                .filter(|s| s.kind.tag() == ad::Tag::CheckFirst)
                .count()
                + rule_counts.check_first;
            let all = shown.len() + rule_counts.all;
            let total: u64 = list.iter().map(|s| s.total()).sum::<u64>()
                + app.disk.rules.as_ref().map_or(0, |r| r.total());
            // The three classic suggestions first, under "Suggested".
            let suggested: Vec<dp::CleanRow> = list
                .iter()
                .enumerate()
                .filter(|(_, s)| !s.items.is_empty() && filter.admits(s.kind.tag()))
                .map(|(i, s)| {
                    let (title, icon) = kind_words(s.kind);
                    dp::CleanRow {
                        index: i,
                        icon,
                        title: title.into(),
                        body: kind_body(s),
                        check_first: s.kind.tag() == ad::Tag::CheckFirst,
                        size: dp::size_label(s.total()),
                        checked: app.disk.clean_checked.contains(&i),
                        can_review: true,
                    }
                })
                .collect();
            let mut groups = Vec::new();
            if !suggested.is_empty() {
                let bytes: u64 = suggested.iter().map(|r| list[r.index].total()).sum();
                groups.push(dp::CleanGroup {
                    title: SUGGESTED.into(),
                    sub: format!("{} · {}", suggested.len(), dp::size_label(bytes)),
                    rows: suggested,
                });
            }
            // Then the cleaning rules' groups: Developer caches, App caches, and so on.
            if let Some(rules) = &app.disk.rules {
                for g in rules.groups_for(filter) {
                    let rows: Vec<dp::CleanRow> = g
                        .items
                        .iter()
                        .filter_map(|it| {
                            Some(dp::CleanRow {
                                index: rule_index(it.id)?,
                                icon: group_icon(g.group),
                                title: it.name.into(),
                                body: it.detail.into(),
                                check_first: it.tier == ad::Tier::CheckFirst,
                                size: dp::size_label(it.size),
                                checked: it.selected,
                                can_review: !it.paths.is_empty(),
                            })
                        })
                        .collect();
                    groups.push(dp::CleanGroup {
                        title: g.title.into(),
                        sub: format!("{} · {}", rows.len(), dp::size_label(g.size)),
                        rows,
                    });
                }
            }
            dp::CleanView {
                place,
                total: dp::size_label(total),
                filter: app.disk.filter,
                counts: [all, all - check, check],
                groups,
                first: app.disk.clean_first,
                bin: bin.map(dp::size_label).unwrap_or_default(),
                nothing: (total == 0).then(|| format!("Nothing to clean on {drive}")),
                selected: selected_words(n, bytes),
            }
        }
        _ => {
            let rows = [
                SuggestionKind::OldDownloads,
                SuggestionKind::TempFiles,
                SuggestionKind::BrowserCaches,
            ]
            .into_iter()
            .enumerate()
            .map(|(i, k)| {
                let (title, icon) = kind_words(k);
                dp::CleanRow {
                    index: i,
                    icon,
                    title: title.into(),
                    body: "Measuring what is there.".into(),
                    check_first: k.tag() == ad::Tag::CheckFirst,
                    size: "…".into(),
                    checked: false,
                    can_review: false,
                }
            })
            .collect();
            dp::CleanView {
                place,
                total: "Measuring".into(),
                filter: app.disk.filter,
                counts: [0, 0, 0],
                groups: vec![dp::CleanGroup {
                    title: SUGGESTED.into(),
                    sub: String::new(),
                    rows,
                }],
                first: 0,
                bin: String::new(),
                nothing: None,
                selected: None,
            }
        }
    }
}

fn review_view(app: &App) -> Option<dp::ReviewView> {
    let r = app.disk.review.as_ref()?;
    let (title, _) = kind_words(r.kind);
    let total: u64 = r.items.iter().map(|c| c.size).sum();
    let rows = r
        .items
        .iter()
        .enumerate()
        .map(|(i, c)| {
            let name = c.path.file_name().map_or_else(
                || c.path.display().to_string(),
                |n| n.to_string_lossy().into_owned(),
            );
            let dir = c.path.is_dir();
            let sub = match r.kind {
                SuggestionKind::OldDownloads => String::new(),
                _ => c
                    .path
                    .parent()
                    .map(|p| p.display().to_string())
                    .unwrap_or_default(),
            };
            let icon = if dir {
                Icon::Folder
            } else {
                file_icon(std::ffi::OsStr::new(&name))
            };
            dp::FileRow {
                name,
                sub,
                month: c
                    .newest
                    .map(|t| m::month_year(stamp(t)))
                    .unwrap_or_default(),
                size: dp::size_label(c.size),
                icon,
                checked: r.checked.contains(&i),
            }
        })
        .collect();
    let (n, bytes) = selection(app).unwrap_or((0, 0));
    let can = r
        .items
        .first()
        .is_some_and(|c| ad::has_recycle_bin(&c.path));
    Some(dp::ReviewView {
        title: title.into(),
        sub: format!(
            "{} · {}",
            m::plural(r.items.len() as u64, "item", "items"),
            dp::size_label(total)
        ),
        rows,
        first: r.first,
        selected: selected_words(n, bytes),
        can_recycle: can,
        all: !r.items.is_empty() && r.checked.len() == r.items.len(),
    })
}

fn cards(app: &App) -> Vec<Card> {
    let mut out = Vec::new();
    let card = |icon, ember, title: String, body: String, button: Option<&str>| Card {
        icon,
        ember,
        title,
        body,
        button: button.map(Into::into),
    };
    match app.disk.place {
        Place::Results => {
            if let Some(d) = app.disk.done() {
                let n = d.tree.node(d.tree.root());
                if d.tree.status() == ScanStatus::Stopped {
                    let at = match d.space.filter(|_| m::is_drive_root(d.tree.root_path())) {
                        Some(s) => format!(
                            "Scan stopped at {}%",
                            m::drive_percent(s, n.allocated(), false).floor() as u64
                        ),
                        None => "Scan stopped".into(),
                    };
                    out.push(card(Icon::Stop, false, at, format!("Showing the {} counted so far, so some folders look smaller than they are.", m::size(n.size())), Some("Scan again")));
                }
                let skipped = d.tree.skipped().len() as u64;
                if skipped > 0 {
                    out.push(card(Icon::Lock, false, format!("{} skipped", m::plural(skipped, "folder was", "folders were")), "Windows keeps them private, so what is in them is not counted. The scan went on past them.".into(), None));
                }
            }
        }
        Place::Start => {
            if let Scan::Failed { root, error, .. } = &app.disk.scan {
                let (place, value, body) = m::scan_error_words(root, error);
                out.push(card(
                    Icon::Alert,
                    true,
                    format!("{place} {value}"),
                    body.into(),
                    Some("Try again"),
                ));
            }
        }
        _ => {}
    }
    match &app.disk.note {
        Some(Note::Moved(n)) => {
            let (t, v) = m::in_bin_words(*n);
            out.push(card(
                Icon::Trash,
                false,
                format!("{t} · {v}"),
                format!(
                    "{} Undo puts {} back now.",
                    m::put_back_words(*n),
                    if *n == 1 { "it" } else { "them" }
                ),
                Some("Undo"),
            ));
        }
        Some(Note::Partial { title, body, .. }) => out.push(card(
            Icon::Lock,
            false,
            title.clone(),
            body.clone(),
            Some("Try again"),
        )),
        Some(Note::NoBin(drive, _)) => out.push(card(
            Icon::Shield,
            false,
            format!("{drive} has no Recycle Bin"),
            "Files on this drive would be gone for good, so Orby only opens them in Explorer."
                .into(),
            Some("Open in Explorer"),
        )),
        Some(Note::Restored { back, stayed }) => {
            let body = if *stayed == 0 && *back == 1 {
                "It is back where it was.".into()
            } else if *stayed == 0 {
                "They are back where they were.".into()
            } else {
                format!("{} stayed in the Recycle Bin: something new is at its old place, or it is no longer there. Nothing was overwritten.", m::plural(*stayed as u64, "item", "items"))
            };
            out.push(card(
                Icon::Refresh,
                false,
                format!("Put back · {}", m::plural(*back as u64, "item", "items")),
                body,
                None,
            ));
        }
        None => {}
    }
    out
}

fn confirm_view(app: &App) -> Option<dp::ConfirmView> {
    let c = app.disk.confirm.as_ref()?;
    let total: u64 = c.items.iter().map(|(_, s)| s).sum();
    let n = c.lines.len();
    // Check first comes first, so the lines that ask for a look are read before the rest.
    let groups = [true, false]
        .into_iter()
        .filter_map(|check| {
            let lines: Vec<&(bool, String, String, u64)> =
                c.lines.iter().filter(|l| l.0 == check).collect();
            if lines.is_empty() {
                return None;
            }
            let bytes: u64 = lines.iter().map(|l| l.3).sum();
            Some(dp::ConfirmGroup {
                check_first: check,
                count: format!("{} · {}", lines.len(), dp::size_label(bytes)),
                items: lines
                    .iter()
                    .take(m::CONFIRM_LIST)
                    .map(|l| (l.1.clone(), l.2.clone(), dp::size_label(l.3)))
                    .collect(),
                // Each group lists up to five lines; the rest fold into "and 12 more".
                more: m::confirm_more(lines.len()),
            })
        })
        .collect();
    Some(dp::ConfirmView {
        title: m::confirm_title(n),
        groups,
        note: if n == 1 {
            "You can put it back from the Recycle Bin until it's emptied. Orby never empties it for you."
        } else {
            "You can put them back from the Recycle Bin until it's emptied. Orby never empties it for you."
        },
        footer: format!("Moving {} to the Recycle Bin", dp::size_label(total)),
        action: format!("Move {n} to Recycle Bin"),
        on_move: c.on_move,
    })
}

/// The moving parts as this frame shows them.
fn motion(app: &App, now: Instant) -> dp::Motion {
    let a = &app.disk.anim;
    let calm = downloads::reduced_motion();
    let dim = a.dim.value(now);
    let reflow_p = if calm {
        1.0
    } else {
        (now.saturating_duration_since(a.map_at).as_secs_f32() * 1000.0 / REFLOW_MS).min(1.0)
    };
    let from = match screen_tiles(app) {
        Some((tiles, names)) if reflow_p < 1.0 => tiles
            .iter()
            .map(|t| a.map_from.get(&tile_key(t, |i| names[i].clone())).copied())
            .collect(),
        _ => Vec::new(),
    };
    let ripple = a.ripple_at.and_then(|at| {
        let ms = now.saturating_duration_since(at).as_secs_f32() * 1000.0;
        (ms < RIPPLE_MS).then_some(ms)
    });
    let scanning = matches!(app.disk.scan, Scan::Running { .. });
    let shimmer = (scanning && !calm)
        .then(|| (now.saturating_duration_since(a.shimmer_from).as_secs_f32() / SHIMMER_S).fract());
    dp::Motion {
        veil: a.veil.value(now),
        dim,
        focus: if dim > 0.0 || a.focus.is_some() {
            a.focus
        } else {
            None
        },
        reflow: reflow_ease(app, reflow_p),
        from,
        ripple,
        shimmer,
        suffix: a.suffix_w.spring(now),
        suffix_alpha: a.suffix_a.value(now),
        suffix_text: a.suffix_text.clone(),
        mouse: app.disk.win.mouse,
    }
}

/// The tile reflow curve. A finished layout eases in and out once. While scanning, every
/// update restarts the reflow, so it eases out instead: tiles head straight for their new
/// spot rather than starting slowly each time, lagging and drifting over each other.
fn reflow_ease(app: &App, p: f32) -> f32 {
    if matches!(app.disk.scan, Scan::Running { .. }) {
        crate::motion::cubic_bezier(0.23, 1.0, 0.32, 1.0, p)
    } else {
        crate::motion::cubic_bezier(0.7, 0.0, 0.3, 1.0, p)
    }
}

/// The tiles the window shows and their rows' names (live or from the level).
fn screen_tiles(app: &App) -> Option<(Vec<dp::MapTile>, Vec<String>)> {
    match &app.disk.scan {
        Scan::Running { space, live, .. } if app.disk.place == Place::Scanning => {
            let map = dp::map_rect(&app.g, &app.fonts, app.disk.win.room, &[]);
            let sizes: Vec<u64> = live.rows.iter().map(|r| r.size).collect();
            let skeleton = space.map_or(0, |s| s.used().saturating_sub(live.on_disk));
            Some((
                fit_tiles(&sizes, map, skeleton, &|_, _| Vec::new()),
                live.rows.iter().map(|r| r.name.clone()).collect(),
            ))
        }
        Scan::Done(_) if app.disk.place == Place::Results => {
            let l = app.disk.level.as_ref()?;
            Some((
                l.tiles.clone(),
                l.rows.iter().map(|r| r.name.clone()).collect(),
            ))
        }
        _ => None,
    }
}

/// Before a draw: point every moving part at what the data now says. A part already heading
/// there is left alone, so redraws in between do not restart anything.
fn sync_motion(app: &mut App) {
    let now = Instant::now();
    let calm = downloads::reduced_motion();
    // Tiles: a new layout starts a reflow from wherever each tile is drawn now.
    if let Some((tiles, names)) = screen_tiles(app) {
        let to: Vec<(String, Rect)> = tiles
            .iter()
            .map(|t| (tile_key(t, |i| names[i].clone()), t.rect))
            .collect();
        let p = (now.saturating_duration_since(app.disk.anim.map_at).as_secs_f32() * 1000.0
            / REFLOW_MS)
            .min(1.0);
        let e = reflow_ease(app, p);
        let a = &mut app.disk.anim;
        if to != a.map_to {
            let drawn: HashMap<String, Rect> = a
                .map_to
                .iter()
                .map(|(k, r)| {
                    let at = match a.map_from.get(k) {
                        Some(f) if e < 1.0 => (
                            f.0 + (r.0 - f.0) * e,
                            f.1 + (r.1 - f.1) * e,
                            f.2 + (r.2 - f.2) * e,
                            f.3 + (r.3 - f.3) * e,
                        ),
                        _ => *r,
                    };
                    (k.clone(), at)
                })
                .collect();
            a.map_from = if calm { HashMap::new() } else { drawn };
            a.map_to = to;
            a.map_at = now;
        }
    }
    // Numbers roll toward their new values; a number seen for the first time just shows.
    for (key, target) in number_targets(app) {
        let a = &mut app.disk.anim;
        match a.rolls.get_mut(&key) {
            Some(r) if (r.to - target).abs() > 1e-9 => {
                let v = if calm { target } else { r.value(now) };
                *r = Roll {
                    from: v,
                    to: target,
                    at: now,
                };
            }
            Some(_) => {}
            None => {
                a.rolls.insert(
                    key,
                    Roll {
                        from: target,
                        to: target,
                        at: now,
                    },
                );
            }
        }
    }
    // The veil lifts 400 ms once the scan is over and its totals have held for 1.5 s.
    let done = matches!(app.disk.scan, Scan::Done(_));
    let a = &mut app.disk.anim;
    if done && a.veil.to > 0.0 && now.saturating_duration_since(a.total_at) >= VEIL_HOLD {
        a.veil.go(0.0, VEIL_MS, now);
    }
    if !done && !matches!(app.disk.scan, Scan::Running { .. }) {
        a.veil = Fade::new(0.0);
    }
    if a.veil.to == 0.0 && !a.veil.moving(now) {
        a.veil_text = None;
    }
    // The primary button's size suffix appears and goes with the selection.
    let suffix = selection(app).map(|(_, b)| format!(" ({})", dp::size_label(b)));
    let a = &mut app.disk.anim;
    match suffix {
        Some(s) => {
            a.suffix_text = s;
            a.suffix_w.go(1.0, 250.0, now);
            a.suffix_a.go(1.0, 150.0, now);
        }
        None => {
            a.suffix_w.go(0.0, 250.0, now);
            a.suffix_a.go(0.0, 150.0, now);
            if !a.suffix_w.moving(now) && !a.suffix_a.moving(now) {
                a.suffix_text.clear();
            }
        }
    }
    if a.dim.to == 0.0 && !a.dim.moving(now) {
        a.focus = None;
    }
}

/// Whether anything is still moving (the window's frame timer runs only then).
pub fn animating(app: &App) -> bool {
    let now = Instant::now();
    let a = &app.disk.anim;
    let calm = downloads::reduced_motion();
    let scanning =
        matches!(app.disk.scan, Scan::Running { .. }) && app.disk.place == Place::Scanning;
    let veil_waits = matches!(app.disk.scan, Scan::Done(_)) && a.veil.to > 0.0;
    let reflow = !calm
        && !a.map_from.is_empty()
        && now.saturating_duration_since(a.map_at).as_secs_f32() * 1000.0 < REFLOW_MS;
    let ripple = a
        .ripple_at
        .is_some_and(|at| now.saturating_duration_since(at).as_secs_f32() * 1000.0 < RIPPLE_MS);
    let rolling = a
        .rolls
        .values()
        .any(|r| r.from != r.to && now.saturating_duration_since(r.at).as_secs_f32() < ROLL_S);
    (scanning && !calm)
        || veil_waits
        || a.veil.moving(now)
        || a.dim.moving(now)
        || a.suffix_w.moving(now)
        || a.suffix_a.moving(now)
        || reflow
        || ripple
        || rolling
}

/// The pointer moved onto something else: what stays lit, and the 150 ms fade of the rest.
pub fn on_hover(app: &mut App, h: Option<Hit>) {
    let focus = match h {
        Some(Hit::Tile(i) | Hit::Row(i) | Hit::RowCheck(i)) => Some(Focus::Row(i)),
        Some(Hit::Seg(i)) => Some(
            current_segs(app)
                .get(i)
                .and_then(|s| s.row)
                .map_or(Focus::Seg(i), Focus::Row),
        ),
        _ => None,
    };
    let now = Instant::now();
    let a = &mut app.disk.anim;
    match focus {
        Some(f) => {
            a.focus = Some(f);
            a.dim.go(1.0, HOVER_MS, now);
        }
        None => a.dim.go(0.0, HOVER_MS, now),
    }
}

fn view(app: &App) -> View {
    let now = Instant::now();
    let screen = match app.disk.place {
        Place::Scanning => live_results(app, now).map(Screen::Results),
        Place::Results => results_view(app, now).map(Screen::Results),
        Place::Clean => Some(Screen::Clean(clean_view(app))),
        Place::Review => review_view(app).map(Screen::Review),
        Place::Start => None,
    }
    .unwrap_or_else(|| {
        Screen::Start(
            app.disk
                .drives
                .iter()
                .map(|(r, label, s)| dp::DriveRow {
                    name: format!("{} {label}", m::drive_label(r)),
                    free: format!(
                        "{} free of {}",
                        m::size_round(s.free),
                        m::size_round(s.total)
                    ),
                    used: s.used_percent() as f32 / 100.0,
                })
                .collect(),
        )
    });
    View {
        screen,
        cards: cards(app),
        confirm: confirm_view(app),
        cursor: app.disk.cursor,
        kb: app.disk.kb,
        room: app.disk.win.room,
        motion: motion(app, now),
    }
}

pub fn size(app: &App) -> (f32, f32) {
    dp::size(&app.g, &app.fonts, &view(app))
}

/// Before a draw: the results level for where the window is (it is computed once per place),
/// and the moving parts pointed at it.
pub fn prepare(app: &mut App) {
    level(app);
    sync_motion(app);
}

pub fn draw(app: &App, dc: &ID2D1DeviceContext, hover: Option<Hit>) {
    dp::draw(&app.g, dc, &app.theme, &app.fonts, &view(app), hover);
}

pub fn hit(app: &App, x: f32, y: f32) -> Option<Hit> {
    dp::hit(&app.g, &app.fonts, &view(app), x, y)
}

/// Data behind the window changed: rebuild the level and redraw.
fn refresh(app: &mut App) {
    level(app);
    disk_window::refresh(app);
}

fn shell(file: &str, params: Option<&str>) {
    let params = params.map(HSTRING::from);
    unsafe {
        ShellExecuteW(
            None,
            w!("open"),
            &HSTRING::from(file),
            params
                .as_ref()
                .map_or(PCWSTR::null(), |p| PCWSTR(p.as_ptr())),
            PCWSTR::null(),
            SW_SHOWNORMAL,
        );
    }
}

/// A folder opens; a file is shown selected in its folder.
fn open_in_explorer(app: &mut App, p: &Path) {
    log_line(app, &format!("disk: open in Explorer {}", p.display()));
    if p.is_dir() {
        shell(&p.to_string_lossy(), None);
    } else {
        shell(
            "explorer.exe",
            Some(&format!("/select,\"{}\"", p.display())),
        );
    }
}

fn node_items(app: &App, ids: &[NodeId]) -> Vec<(PathBuf, u64)> {
    let Some(d) = app.disk.done() else {
        return Vec::new();
    };
    ids.iter()
        .map(|&id| (d.tree.path(id), app.disk.removed.size(&d.tree, id)))
        .collect()
}

fn row_id(app: &App, i: usize) -> Option<NodeId> {
    app.disk.level.as_ref()?.rows.get(i).map(|r| r.id)
}

fn file_id(app: &App, i: usize) -> Option<NodeId> {
    app.disk.level.as_ref()?.files.get(i).map(|(id, _)| *id)
}

fn open_row(app: &mut App, i: usize) {
    let Some(id) = row_id(app, i) else { return };
    let dir = app.disk.level.as_ref().is_some_and(|l| l.rows[i].dir);
    if dir {
        go_to(app, Some(id));
    } else if let Some(p) = node_items(app, &[id]).first().map(|x| x.0.clone()) {
        open_in_explorer(app, &p);
    }
}

fn up(app: &mut App) {
    let parent = app
        .disk
        .done()
        .and_then(|d| app.disk.at.and_then(|a| d.tree.node(a).parent()));
    match parent {
        Some(p) => {
            let from = app.disk.at;
            go_to(app, Some(p));
            level(app);
            // Land on the folder you came out of.
            app.disk.sel = app
                .disk
                .level
                .as_ref()
                .and_then(|l| l.rows.iter().position(|r| Some(r.id) == from));
            keep_visible(app);
        }
        None => {
            to_start(app);
            app.disk.cursor = None;
        }
    }
}

fn keep_visible(app: &mut App) {
    let fit = dp::fit(&app.g, &app.fonts, &view(app));
    if let Some(s) = app.disk.sel {
        if s < app.disk.first {
            app.disk.first = s;
        } else if s >= app.disk.first + fit.list {
            app.disk.first = s + 1 - fit.list;
        }
    }
    let Some(c) = app.disk.cursor else { return };
    match app.disk.place {
        Place::Results if app.disk.tab == Tab::Files => {
            if c < app.disk.files_first {
                app.disk.files_first = c;
            } else if c >= app.disk.files_first + fit.files {
                app.disk.files_first = c + 1 - fit.files;
            }
        }
        Place::Review => {
            if let Some(r) = &mut app.disk.review {
                if c < r.first {
                    r.first = c;
                } else if c >= r.first + fit.review {
                    r.first = c + 1 - fit.review;
                }
            }
        }
        Place::Clean => {
            // The cursor counts rows; the list scrolls by entries (group headers included).
            let entries = dp::clean_entries(&clean_view(app));
            let Some(at) = entries
                .iter()
                .enumerate()
                .filter(|(_, e)| matches!(e, dp::Entry::Row(..)))
                .nth(c)
                .map(|(k, _)| k)
            else {
                return;
            };
            let first = &mut app.disk.clean_first;
            if at <= *first {
                // Show the group's header above its first row.
                *first = at.saturating_sub(1);
            } else if at >= *first + fit.clean {
                *first = at + 1 - fit.clean;
            }
        }
        _ => {}
    }
}

fn toggle_file(app: &mut App, i: usize) {
    match app.disk.place {
        Place::Review => {
            if let Some(r) = &mut app.disk.review {
                if !r.checked.remove(&i) && i < r.items.len() {
                    r.checked.insert(i);
                }
            }
        }
        _ => {
            if let Some(id) = file_id(app, i) {
                if !app.disk.checked.remove(&id) {
                    app.disk.checked.insert(id);
                }
            }
        }
    }
}

fn toggle_row(app: &mut App, i: usize) {
    if let Some(id) = row_id(app, i) {
        if !app.disk.checked.remove(&id) {
            app.disk.checked.insert(id);
        }
    }
}

fn toggle_clean(app: &mut App, i: usize) {
    if let Some(id) = rule_id(i) {
        // Check first rows are ticked one by one, like every other row here.
        if let Some(r) = &mut app.disk.rules {
            r.toggle(id);
        }
        return;
    }
    let can = matches!(&app.disk.clean, Clean::Ready { list, .. } if list.get(i).is_some_and(|s| !s.items.is_empty()));
    if can && !app.disk.clean_checked.remove(&i) {
        app.disk.clean_checked.insert(i);
    }
}

/// Delete, the selection bar or a row: the confirm sheet for what is chosen.
fn ask_for_selection(app: &mut App) {
    match app.disk.place {
        Place::Review => {
            let Some(r) = &app.disk.review else { return };
            let mut pick: Vec<usize> = r.checked.iter().copied().collect();
            if pick.is_empty() {
                pick.extend(app.disk.cursor.filter(|&c| c < r.items.len()));
            }
            let items: Vec<(PathBuf, u64)> = pick
                .iter()
                .filter_map(|&i| r.items.get(i))
                .map(|c| (c.path.clone(), c.size))
                .collect();
            let kind = r.kind;
            let lines = path_lines(&items, kind.tag() == ad::Tag::CheckFirst);
            ask(app, items, lines, Source::Clean(kind));
        }
        Place::Clean => {
            let Clean::Ready { list, .. } = &app.disk.clean else {
                return;
            };
            let mut pick: Vec<usize> = app.disk.clean_checked.iter().copied().collect();
            let mut rule_rows: Vec<&m::CleanItem> = app
                .disk
                .rules
                .as_ref()
                .map(|r| r.items().filter(|it| it.selected).collect())
                .unwrap_or_default();
            if pick.is_empty() && rule_rows.is_empty() {
                // Nothing ticked: Delete takes the row under the keyboard.
                let at = app
                    .disk
                    .cursor
                    .and_then(|c| clean_order(app).get(c).copied());
                match at {
                    Some(i) if i >= RULE_BASE => rule_rows.extend(rule_item(app, i)),
                    Some(i) => pick.push(i),
                    None => {}
                }
            }
            let picked: Vec<&ad::Suggestion> = pick
                .iter()
                .filter_map(|&i| list.get(i))
                .filter(|s| !s.items.is_empty())
                .collect();
            let mut items: Vec<(PathBuf, u64)> = picked
                .iter()
                .flat_map(|s| s.items.iter().map(|c| (c.path.clone(), c.size)))
                .collect();
            // One line per row; a Check first one names its folder, or how many it holds.
            let sub = |check: bool, paths: Vec<&Path>| match (check, paths.as_slice()) {
                (false, _) => String::new(),
                (true, [one]) => one.display().to_string(),
                (true, many) => m::plural(many.len() as u64, "item", "items"),
            };
            let mut lines: Vec<(bool, String, String, u64)> = picked
                .iter()
                .map(|s| {
                    let check = s.kind.tag() == ad::Tag::CheckFirst;
                    let paths = s.items.iter().map(|c| c.path.as_path()).collect();
                    (
                        check,
                        kind_words(s.kind).0.to_string(),
                        sub(check, paths),
                        s.total(),
                    )
                })
                .collect();
            let mut rules = Vec::new();
            for it in &rule_rows {
                let check = it.tier == ad::Tier::CheckFirst;
                let paths = it.paths.iter().map(PathBuf::as_path).collect();
                lines.push((check, it.name.to_string(), sub(check, paths), it.size));
                for (p, s) in it.paths.iter().zip(&it.path_sizes) {
                    items.push((p.clone(), *s));
                    rules.push((it.rule, p.clone()));
                }
            }
            let source = picked
                .first()
                .map_or(Source::Rules, |s| Source::Clean(s.kind));
            ask(app, items, lines, source);
            if let Some(c) = &mut app.disk.confirm {
                c.rules = rules;
            }
        }
        Place::Results => {
            let mut ids = top_checked(app);
            if ids.is_empty() {
                let one = match app.disk.tab {
                    Tab::Files => app.disk.cursor.and_then(|c| file_id(app, c)),
                    Tab::Folders => app.disk.sel.and_then(|s| row_id(app, s)),
                    Tab::Types => None,
                };
                ids.extend(one);
            }
            // The user's own files: every line asks for a look.
            let items = node_items(app, &ids);
            let lines = path_lines(&items, true);
            ask(app, items, lines, Source::Tree);
        }
        _ => {}
    }
}

fn open_selection(app: &mut App) {
    let paths: Vec<PathBuf> = match app.disk.place {
        Place::Review => app
            .disk
            .review
            .as_ref()
            .map(|r| {
                r.checked
                    .iter()
                    .filter_map(|&i| r.items.get(i))
                    .map(|c| c.path.clone())
                    .collect()
            })
            .unwrap_or_default(),
        _ => {
            let ids = top_checked(app);
            node_items(app, &ids).into_iter().map(|(p, _)| p).collect()
        }
    };
    // One Explorer window per folder is plenty; the first item is shown selected.
    if let Some(p) = paths.first() {
        open_in_explorer(app, &p.clone());
    }
}

/// Unticks everything on the screen showing.
fn clear_selection(app: &mut App) {
    match app.disk.place {
        Place::Clean => {
            app.disk.clean_checked.clear();
            if let Some(r) = &mut app.disk.rules {
                r.select_tier(m::TierFilter::All, false);
            }
        }
        Place::Review => {
            if let Some(r) = &mut app.disk.review {
                r.checked.clear();
            }
        }
        _ => app.disk.checked.clear(),
    }
}

/// The detail strip's Open in Explorer: the row in focus, or the folder on show.
fn open_strip(app: &mut App) {
    let id = focus_row(app)
        .and_then(|i| row_id(app, i))
        .or(app.disk.level.as_ref().map(|l| l.at));
    if let Some(p) = id.and_then(|id| node_items(app, &[id]).first().map(|x| x.0.clone())) {
        open_in_explorer(app, &p);
    }
}

fn card_action(app: &mut App, i: usize) {
    let labels: Vec<Option<String>> = cards(app).into_iter().map(|c| c.button).collect();
    match labels.get(i).cloned().flatten().as_deref() {
        Some("Try again") if matches!(app.disk.note, Some(Note::Partial { .. })) => {
            if let Some(Note::Partial { stayed, source, .. }) = app.disk.note.take() {
                app.disk.merge_next = true;
                let lines = path_lines(&stayed, source == Source::Tree);
                // Rule folders that stayed go through the rule checks again.
                let paths: Vec<PathBuf> = stayed.iter().map(|(p, _)| p.clone()).collect();
                let rules = rule_pairs(app, &paths);
                ask(app, stayed, lines, source);
                if let Some(c) = &mut app.disk.confirm {
                    c.rules = rules;
                }
            }
        }
        Some("Scan again") | Some("Try again") => scan_again(app),
        Some("Undo") => undo(app),
        Some("Open in Explorer") => {
            if let Some(Note::NoBin(_, p)) = &app.disk.note {
                let p = p.clone();
                open_in_explorer(app, &p);
            }
        }
        _ => app.disk.note = None,
    }
}

fn set_filter(app: &mut App, f: Filter) {
    if app.disk.filter != f {
        app.disk.filter = f;
        app.disk.clean_first = 0;
        app.disk.cursor = None;
        enter_rows(app);
    }
}

pub fn click(app: &mut App, h: Hit) {
    app.disk.kb = false;
    match h {
        Hit::Cancel => {
            app.disk.confirm = None;
            app.disk.merge_next = false;
            log_line(app, "disk: confirm cancelled");
        }
        Hit::Move => confirm_move(app),
        Hit::Sheet => {}
        Hit::ScanDrive(i) => {
            if let Some(root) = app.disk.drives.get(i).map(|d| d.0.clone()) {
                scan_path(app, root, true);
            }
        }
        // The picker belongs to the Disk window, so it opens over it.
        Hit::ScanFolder => unsafe {
            let _ = PostMessageW(
                Some(app.disk.win.hwnd),
                WM_APP_DISK,
                WPARAM(PICK_FOLDER),
                LPARAM(0),
            );
        },
        Hit::OpenClean => open_clean(app),
        Hit::Stop => stop(app),
        Hit::ScanAgain => scan_again(app),
        Hit::Drive => {
            to_start(app);
            app.disk.cursor = None;
            enter_rows(app);
        }
        Hit::Back => match app.disk.place {
            Place::Clean if app.disk.clean_from == Place::Results && app.disk.done().is_some() => {
                app.disk.place = Place::Results;
                enter_rows(app);
            }
            Place::Clean => to_start(app),
            Place::Review => {
                app.disk.place = Place::Clean;
                enter_rows(app);
            }
            _ => up(app),
        },
        Hit::Crumb(i) => {
            let target = app.disk.done().and_then(|d| {
                d.tree
                    .ancestry(app.disk.at.unwrap_or(d.tree.root()))
                    .get(i)
                    .copied()
            });
            if target.is_some() && target != app.disk.at {
                go_to(app, target);
            }
        }
        Hit::Tab(t) => {
            if app.disk.tab != t {
                app.disk.tab = t;
                app.disk.cursor = None;
                enter_rows(app);
            }
        }
        Hit::Tile(i) | Hit::Row(i) => {
            if app.disk.sel == Some(i) {
                open_row(app, i);
            } else {
                app.disk.sel = Some(i);
            }
        }
        Hit::Seg(i) => {
            if let Some(row) = current_segs(app).get(i).and_then(|s| s.row) {
                app.disk.sel = Some(row);
                keep_visible(app);
            }
        }
        Hit::RowCheck(i) => toggle_row(app, i),
        Hit::StripOpen => open_strip(app),
        Hit::File(i) => {
            app.disk.cursor = Some(i);
            toggle_file(app, i);
        }
        Hit::SelOpen => open_selection(app),
        Hit::SelRecycle => ask_for_selection(app),
        Hit::SelClear => clear_selection(app),
        Hit::SelectAll => {
            if let Some(r) = &mut app.disk.review {
                if r.checked.len() == r.items.len() {
                    r.checked.clear();
                } else {
                    r.checked = (0..r.items.len()).collect();
                }
            }
        }
        Hit::Filter(f) => set_filter(app, f),
        // A suggestion opens its Review list; a rule row opens its folder in Explorer.
        Hit::Clean(i) => match rule_item(app, i).and_then(|it| it.paths.first().cloned()) {
            Some(p) => open_in_explorer(app, &p),
            None => open_review(app, i),
        },
        Hit::CleanCheck(i) => toggle_clean(app, i),
        Hit::BinOpen => {
            log_line(app, "disk: open the Recycle Bin");
            shell("shell:RecycleBinFolder", None);
        }
        Hit::Settings => {
            log_line(app, "disk: open Storage settings");
            shell("ms-settings:storagesense", None);
        }
        Hit::ReviewFolder => {
            let p = app.disk.review.as_ref().and_then(|r| {
                r.items
                    .first()
                    .and_then(|c| c.path.parent().map(Path::to_path_buf))
            });
            if let Some(p) = p {
                open_in_explorer(app, &p);
            }
        }
        Hit::Card(i) => card_action(app, i),
        Hit::Close => {
            disk_window::close(app);
            return;
        }
    }
    refresh(app);
}

/// The length of the list the keyboard moves in.
fn list_len(app: &App) -> usize {
    match app.disk.place {
        Place::Start => app.disk.drives.len(),
        Place::Results => match app.disk.tab {
            Tab::Folders => app.disk.level.as_ref().map_or(0, |l| l.rows.len()),
            Tab::Files => app.disk.level.as_ref().map_or(0, |l| l.files.len()),
            Tab::Types => 0,
        },
        Place::Clean => clean_order(app).len(),
        Place::Review => app.disk.review.as_ref().map_or(0, |r| r.items.len()),
        Place::Scanning => 0,
    }
}

/// WM_KEYDOWN in the Disk window (Esc goes to on_esc). Returns whether the key was used.
pub fn on_keydown(app: &mut App, vk: u16) -> bool {
    let key = VIRTUAL_KEY(vk);
    if app.disk.confirm.is_some() {
        match key {
            VK_TAB | VK_LEFT | VK_RIGHT => {
                if let Some(c) = &mut app.disk.confirm {
                    c.on_move = !c.on_move;
                }
            }
            VK_RETURN | VK_SPACE => {
                if app.disk.confirm.as_ref().is_some_and(|c| c.on_move) {
                    confirm_move(app);
                } else {
                    app.disk.confirm = None;
                    app.disk.merge_next = false;
                    log_line(app, "disk: confirm cancelled");
                }
            }
            _ => return false,
        }
        refresh(app);
        return true;
    }
    level(app);
    app.disk.kb = true;
    let folders = app.disk.place == Place::Results && app.disk.tab == Tab::Folders;
    let n = list_len(app);
    match key {
        VK_UP | VK_DOWN => {
            let down = key == VK_DOWN;
            let cur = if folders {
                app.disk.sel
            } else {
                app.disk.cursor
            };
            let next = match cur {
                None => 0,
                Some(c) if down => (c + 1).min(n.saturating_sub(1)),
                Some(c) => c.saturating_sub(1),
            };
            if n > 0 {
                if folders {
                    app.disk.sel = Some(next);
                } else {
                    app.disk.cursor = Some(next);
                }
                keep_visible(app);
            }
        }
        VK_LEFT | VK_RIGHT if app.disk.place == Place::Results => {
            let tabs = [Tab::Folders, Tab::Files, Tab::Types];
            let i = tabs.iter().position(|t| *t == app.disk.tab).unwrap_or(0);
            let j = if key == VK_RIGHT {
                (i + 1).min(2)
            } else {
                i.saturating_sub(1)
            };
            if tabs[j] != app.disk.tab {
                app.disk.tab = tabs[j];
                app.disk.cursor = None;
                enter_rows(app);
            }
        }
        VK_LEFT | VK_RIGHT if app.disk.place == Place::Clean => {
            let all = [Filter::All, Filter::Safe, Filter::Check];
            let i = all.iter().position(|f| *f == app.disk.filter).unwrap_or(0);
            let j = if key == VK_RIGHT {
                (i + 1).min(2)
            } else {
                i.saturating_sub(1)
            };
            set_filter(app, all[j]);
        }
        VK_SPACE => match app.disk.place {
            Place::Results if folders => {
                if let Some(s) = app.disk.sel {
                    toggle_row(app, s);
                }
            }
            Place::Clean => {
                if let Some(i) = app
                    .disk
                    .cursor
                    .and_then(|c| clean_order(app).get(c).copied())
                {
                    toggle_clean(app, i);
                }
            }
            _ => {
                if let Some(c) = app.disk.cursor {
                    toggle_file(app, c);
                }
            }
        },
        VK_RETURN => match app.disk.place {
            Place::Start => {
                if let Some(root) = app
                    .disk
                    .cursor
                    .and_then(|c| app.disk.drives.get(c))
                    .map(|d| d.0.clone())
                {
                    scan_path(app, root, true);
                }
            }
            Place::Results if folders => {
                if let Some(s) = app.disk.sel {
                    open_row(app, s);
                }
            }
            Place::Clean => {
                if let Some(i) = app
                    .disk
                    .cursor
                    .and_then(|c| clean_order(app).get(c).copied())
                {
                    open_review(app, i);
                }
            }
            _ => {}
        },
        VK_BACK => match app.disk.place {
            Place::Review => {
                app.disk.place = Place::Clean;
                enter_rows(app);
            }
            Place::Clean => click(app, Hit::Back),
            Place::Results => up(app),
            _ => {}
        },
        VK_DELETE => ask_for_selection(app),
        _ => return false,
    }
    refresh(app);
    true
}

/// Esc: the confirm sheet first (Cancel), then the window.
pub fn on_esc(app: &mut App) {
    if app.disk.confirm.take().is_some() {
        app.disk.merge_next = false;
        log_line(app, "disk: confirm cancelled");
        refresh(app);
    } else {
        disk_window::close(app);
    }
}

/// The wheel at (x, y), in window coordinates.
pub fn on_wheel(app: &mut App, x: f32, y: f32, delta: i16) {
    level(app);
    let v = view(app);
    let fit = dp::fit(&app.g, &app.fonts, &v);
    let target = dp::scroll_target(&app.g, &app.fonts, &v, x, y);
    let step = if delta > 0 { -1i64 } else { 1 };
    match target {
        Some(dp::Scroll::Folders) => {
            let n = app.disk.level.as_ref().map_or(0, |l| l.rows.len());
            app.disk.first = (app.disk.first as i64 + step)
                .clamp(0, n.saturating_sub(fit.list + 1) as i64)
                as usize;
        }
        Some(dp::Scroll::Files) => {
            let n = app.disk.level.as_ref().map_or(0, |l| l.files.len());
            app.disk.files_first = (app.disk.files_first as i64 + step)
                .clamp(0, n.saturating_sub(fit.files) as i64)
                as usize;
        }
        Some(dp::Scroll::Review) => {
            if let Some(r) = &mut app.disk.review {
                r.first = (r.first as i64 + step)
                    .clamp(0, r.items.len().saturating_sub(fit.review) as i64)
                    as usize;
            }
        }
        Some(dp::Scroll::Clean) => {
            let n = match &v.screen {
                Screen::Clean(c) => dp::clean_entries(c).len(),
                _ => 0,
            };
            app.disk.clean_first = (app.disk.clean_first as i64 + step)
                .clamp(0, n.saturating_sub(fit.clean) as i64)
                as usize;
        }
        None => return,
    }
    disk_window::refresh(app);
}

/// A scan's tree takes about 130 bytes per file and folder (a whole drive is hundreds of MB), so it
/// is let go this long after the window closes. A warm rescan takes a moment.
const RELEASE_MS: u32 = 3 * 60 * 1000;

pub fn on_release_timer(app: &mut App) {
    unsafe {
        let _ = KillTimer(Some(app.hwnd), TIMER_DISK_RELEASE);
    }
    if window_shown(app) {
        return;
    }
    let Some(root) = app.disk.done().map(|d| d.tree.root_path().to_path_buf()) else {
        return;
    };
    let (before, _) = memory_mb();
    app.disk.scan = Scan::Released { root };
    app.disk.level = None;
    app.disk.removed = Removed::default();
    app.disk.checked.clear();
    let (after, private) = memory_mb();
    log_line(
        app,
        &format!(
            "disk: let the scan tree go, ws {before:.1}MB -> {after:.1}MB (private {private:.1}MB)"
        ),
    );
}

/// How long after a move the orb keeps offering Undo: the pill once on closing the window, and
/// the card again when the window reopens.
const UNDO_WINDOW: Duration = Duration::from_secs(60);

fn undo_fresh(app: &App) -> bool {
    app.disk
        .last_move
        .as_ref()
        .is_some_and(|l| l.since.elapsed().unwrap_or_default() < UNDO_WINDOW)
}

/// The window closed (Esc, its close button, Alt+F4, the taskbar).
pub fn on_closed(app: &mut App) {
    app.disk.confirm = None;
    app.disk.kb = false;
    if app.disk.done().is_some() {
        unsafe {
            SetTimer(Some(app.hwnd), TIMER_DISK_RELEASE, RELEASE_MS, None);
        }
    }
    // While the window was open its card carried Undo; the orb offers it once more on the way out.
    let moved = match &app.disk.note {
        Some(Note::Moved(n)) => Some(*n),
        Some(Note::Partial { moved, .. }) if *moved > 0 => Some(*moved),
        _ => None,
    };
    let fresh = undo_fresh(app);
    if let (Some(n), Some(last)) = (moved, app.disk.last_move.as_mut()) {
        if fresh && !last.offered {
            last.offered = true;
            let (title, value) = m::in_bin_words(last.paths.len().max(n));
            notices::reply(
                app,
                Pill {
                    icon: Icon::Trash,
                    dot: false,
                    title,
                    value,
                    action: Some(("Undo".into(), PillAction::UndoRecycle)),
                    closable: true,
                    ttl_ms: 6000,
                },
            );
        }
    }
    if matches!(
        app.disk.note,
        Some(Note::Restored { .. }) | Some(Note::NoBin(..))
    ) {
        app.disk.note = None;
    }
}

/// Tray: scan the folder or drive whose path is on the clipboard, in the background: the ring
/// shows it, and the Scan done (or error) pill says when it ends.
pub fn scan_clipboard(app: &mut App) {
    let text = crate::tasks::read_clipboard(app.hwnd).unwrap_or_default();
    let p = PathBuf::from(text.trim().trim_matches('"'));
    if p.as_os_str().is_empty() || !p.is_absolute() {
        log_line(app, "disk: the clipboard holds no folder path");
        return;
    }
    scan_path(app, p, true);
}

/// The folder picker, run with the app not borrowed (its modal loop dispatches our messages).
pub fn on_folder_picked(app: &mut App, folder: Option<PathBuf>) {
    match folder {
        Some(f) => scan_path(app, f, true),
        None => log_line(app, "disk: no folder picked"),
    }
}

pub fn shutdown(app: &mut App) {
    if let Scan::Running { cancel, .. } = &app.disk.scan {
        cancel.store(true, Ordering::Relaxed);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_done_pill_names_the_drive_or_the_folder() {
        let s = DriveSpace {
            total: 476 << 30,
            free: 38 << 30,
        };
        assert_eq!(done_words(Path::new(r"C:\"), 1, Some(s)), "C: 438 GB used");
        assert_eq!(
            done_words(Path::new(r"C:\dev\clippy"), 1536 << 20, Some(s)),
            "clippy 1.5 GB"
        );
    }

    #[test]
    fn a_saved_pill_survives_a_restart_so_it_does_not_fire_again() {
        let dir = std::env::temp_dir().join(format!("orb-proto-disk-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        let mut d = Disk::load_from(dir.join("disk.json"));
        assert!(d.watch.is_empty(), "a first run has no saved pill");
        let low = DriveSpace {
            total: 100,
            free: 8,
        };
        assert!(d.watch.entry('C').or_default().check(low));
        d.save();
        let saved: Saved =
            serde_json::from_str(&std::fs::read_to_string(&d.file).unwrap()).unwrap();
        assert!(
            !Disk::load_from(d.file.clone())
                .watch
                .entry('C')
                .or_default()
                .check(low),
            "the saved pill is not repeated after a restart"
        );
        let mut w = LowSpaceWatch::Shown {
            at_free_percent: saved.low_space["C"],
        };
        assert!(
            !w.check(DriveSpace {
                total: 100,
                free: 7
            }),
            "still within five points"
        );
        assert!(w.check(DriveSpace {
            total: 100,
            free: 3
        }));
        let _ = std::fs::remove_dir_all(&dir);
    }
}
