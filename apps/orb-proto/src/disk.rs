//! Disk on app_disk: the low-space pill, scans on a worker thread with the orb's ring and the
//! Scanning panel, Results (treemap, folders, files, types), Safe to clean with Review, the
//! confirm sheet, Move to Recycle Bin on a worker thread, and Undo.
//!
//! Nothing here deletes. The only remove action is `app_disk::move_to_recycle_bin`, and it runs
//! only from Move to Recycle Bin on the confirm sheet. Undo puts the items back (disk_restore.rs).
//! disk_model.rs decides the words and numbers, disk_panel.rs draws.

use std::collections::{BTreeMap, BTreeSet};
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

use crate::disk_model::{self as m, Removed, Row, Stamp, Summary, Tile};
use crate::disk_panel::{self as dp, Card, Hit, Screen, Tab, View};
use crate::disk_restore;
use crate::notices;
use crate::panels::{self, Kind};
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

#[derive(Clone, Debug, Default)]
struct Live {
    counted: u64,
    /// Allocated bytes, the measure for a drive's percentage.
    on_disk: u64,
    files: u64,
    dirs: u64,
    current: String,
    biggest: Vec<(String, u64)>,
}

struct Done {
    tree: Tree,
    space: Option<DriveSpace>,
    at: SystemTime,
    took: Duration,
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
        live: Live,
    },
    Done(Box<Done>),
    Failed {
        root: PathBuf,
        error: ScanError,
    },
    /// The tree was dropped to give its memory back; opening the panel scans the place again.
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
}

enum Msg {
    Live(u64, Live),
    Scanned(u64, Result<Tree, ScanError>, Duration),
    Suggestions(Vec<ad::Suggestion>, Option<u64>),
    Recycled(RecycleReport, SystemTime, Source),
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
    source: Source,
    on_move: bool,
}

/// What the last action did, shown as a card in the open panel (a pill when it is closed).
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
    /// The Undo pill was shown on the way out of the panel; it comes once per move.
    offered: bool,
}

/// One folder level of the results, computed when the place or the moved items change.
struct Level {
    at: NodeId,
    removed_gen: u64,
    rows: Vec<Row>,
    summary: Summary,
    tiles: Vec<Tile>,
    files: Vec<(NodeId, dp::FileRow)>,
}

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
    /// A scan you started failed: the ember badge on the drop until the panel is opened.
    pub attention: bool,
    spin_from: Instant,
    last_ring: Option<Ring>,
    drives: Vec<(PathBuf, String, DriveSpace)>,
}

fn data_file() -> PathBuf {
    store::data_dir().join("disk.json")
}

impl Disk {
    pub fn load() -> Disk {
        Disk::load_from(data_file())
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
                    let biggest = t
                        .children_by_size(t.root())
                        .into_iter()
                        .filter(|c| t.node(c.id).is_dir())
                        .take(3)
                        .map(|c| (t.node(c.id).name().to_string_lossy().into_owned(), c.size))
                        .collect();
                    rep.send(Msg::Live(
                        id,
                        Live {
                            counted: root.size(),
                            on_disk: root.allocated(),
                            files: root.files(),
                            dirs: root.dirs(),
                            current: p.current.display().to_string(),
                            biggest,
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
    app.disk.scan = Scan::Running {
        id,
        root,
        user,
        started: Instant::now(),
        cancel,
        space,
        live: Live::default(),
    };
    app.disk.spin_from = Instant::now();
    app.disk.place = Place::Scanning;
    app.disk.note = None;
    app.disk.confirm = None;
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
    let shown = panel_shown(app);
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
                took,
                volume,
                bin,
            }));
            app.disk.removed = Removed::default();
            app.disk.removed_gen += 1;
            app.disk.level = None;
            app.disk.checked.clear();
            app.disk.last_move = None;
            let root_id = app.disk.done().map(|d| d.tree.root());
            go_to(app, root_id);
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

/// After any change: the orb's ring, the hover pill's numbers, and the open panel.
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
    if panel_shown(app) {
        panels::refresh(app);
    }
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

fn ask(app: &mut App, items: Vec<(PathBuf, u64)>, source: Source) {
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
    let paths: Vec<PathBuf> = c.items.iter().map(|(p, _)| p.clone()).collect();
    app.disk.asked = c.items;
    log_line(
        app,
        &format!("disk: moving {} item(s) to the Recycle Bin", paths.len()),
    );
    let (rep, source) = (reporter(app), c.source);
    std::thread::spawn(move || {
        let since = SystemTime::now();
        let report = ad::move_to_recycle_bin(&paths);
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
    if !panel_shown(app) {
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
    if matches!(last.source, Source::Clean(_)) || matches!(app.disk.clean, Clean::Ready { .. }) {
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
    std::thread::spawn(move || {
        let places = clean_places();
        let list = ad::safe_to_clean(&places, SystemTime::now(), OLD_DAYS);
        let bin = places
            .downloads
            .as_deref()
            .or(places.temp.first().map(|p| p.as_path()))
            .and_then(|p| ad::recycle_bin_size(p).ok());
        rep.send(Msg::Suggestions(list, bin));
    });
}

fn open_clean(app: &mut App) {
    if app.disk.place != Place::Clean && app.disk.place != Place::Review {
        app.disk.clean_from = app.disk.place;
    }
    app.disk.place = Place::Clean;
    app.disk.cursor = None;
    app.disk.note = None;
    read_suggestions(app);
    log_line(app, "disk: safe to clean");
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
    log_line(app, &format!("disk: review {:?}", s.kind));
}

fn kind_words(kind: SuggestionKind) -> (&'static str, Icon) {
    match kind {
        SuggestionKind::OldDownloads => ("Downloads older than 90 days", Icon::Download),
        SuggestionKind::TempFiles => ("Windows temp files", Icon::Layers),
        SuggestionKind::BrowserCaches => ("Browser caches", Icon::Globe),
    }
}

// ---- the panel ----

fn panel_shown(app: &App) -> bool {
    app.panel_open && app.panel_kind == Kind::Disk
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
    panels::open_panel(app, Kind::Disk);
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
}

/// The cached level for the place the results show.
fn level(app: &mut App) {
    let Scan::Done(d) = &app.disk.scan else {
        return;
    };
    let at = app.disk.at.unwrap_or(d.tree.root());
    if app
        .disk
        .level
        .as_ref()
        .is_some_and(|l| l.at == at && l.removed_gen == app.disk.removed_gen)
    {
        return;
    }
    let tree = &d.tree;
    let removed = &app.disk.removed;
    let rows = m::rows(tree, removed, at);
    let summary = m::summarize(tree, removed, at, TOP_FILES);
    let inner = |id: NodeId| {
        m::rows(tree, removed, id)
            .into_iter()
            .take(6)
            .map(|r| (r.name, r.size))
            .collect()
    };
    let tiles = m::treemap(&rows, (16.0, 166.0, 352.0, 418.0), &inner);
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
                sub: m::folder_below(tree, at, id),
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
        tiles,
        files,
    });
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
    (n > 0).then(|| (format!("{n} selected"), m::size(bytes)))
}

fn results_view(app: &App) -> Option<dp::ResultsView> {
    let d = app.disk.done()?;
    let l = app.disk.level.as_ref()?;
    let tree = &d.tree;
    let root = tree.root_path();
    let folder = !m::is_drive_root(root);
    let rn = tree.node(tree.root());
    let title = if folder {
        m::place_name(root)
    } else {
        format!("{} {}", m::drive_label(root), d.volume)
    };
    let mut sub = format!(
        "Scanned {} · {} in {}",
        m::when(stamp(d.at), now_stamp()),
        m::plural(rn.files(), "file", "files"),
        m::clock(d.took)
    );
    if folder {
        sub = format!(
            "{} · {sub}",
            root.parent()
                .map(|p| p.display().to_string())
                .unwrap_or_default()
        );
    }
    let (used, used_text, used_pct, free_text) = match d.space {
        Some(s) => (
            s.used_percent() as f32 / 100.0,
            m::size_round(s.used()),
            format!("{}%", s.used_percent().round() as u64),
            format!(
                "{} free of {}",
                m::size_round(s.free),
                m::size_round(s.total)
            ),
        ),
        None => (0.0, String::new(), String::new(), String::new()),
    };
    let used_text = if folder {
        format!("{}: {used_text}", m::drive_letter(root).unwrap_or('C'))
    } else {
        used_text
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
    let rows = l
        .rows
        .iter()
        .map(|r| dp::ListRow {
            name: r.name.clone(),
            bytes: r.size,
            size: m::size(r.size),
            pct: r.percent,
            bar: r.size as f32 / biggest as f32,
            dir: r.dir,
            hidden: r.hidden,
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
            size: m::size(t.bytes),
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
    let sel_bytes: u64 = app
        .disk
        .checked
        .iter()
        .map(|&id| tree.node(id).size())
        .sum();
    Some(dp::ResultsView {
        folder,
        title,
        sub,
        used,
        used_text,
        used_pct,
        free_text,
        crumbs,
        here_size: m::size(app.disk.removed.size(tree, l.at)),
        here: if l.at == tree.root() {
            m::place_name(root)
        } else {
            tree.node(l.at).name().to_string_lossy().into_owned()
        },
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
            m::size(l.summary.bytes)
        ),
        selected: selected_words(app.disk.checked.len(), sel_bytes),
        can_recycle: d.bin,
    })
}

fn scanning_view(app: &App) -> Option<dp::ScanningView> {
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
    let place = m::place_name(root);
    let (big, of, frac) = match space {
        Some(s) => {
            let p = m::drive_percent(*s, live.on_disk, true);
            (
                format!("{}%", p.floor() as u64),
                format!("of the {} in use on {place}", m::size_round(s.used())),
                Some((p / 100.0) as f32),
            )
        }
        None => (
            m::size(live.counted),
            format!("counted so far in {place}"),
            None,
        ),
    };
    Some(dp::ScanningView {
        title: format!("Scanning {place}"),
        big,
        of,
        frac,
        files: m::count(live.files),
        folders: m::count(live.dirs),
        took: m::clock(started.elapsed()),
        current: if live.current.is_empty() {
            root.display().to_string()
        } else {
            live.current.clone()
        },
        biggest: live
            .biggest
            .iter()
            .map(|(n, b)| (n.clone(), *b, m::size(*b)))
            .collect(),
    })
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
    match &app.disk.clean {
        Clean::Ready { list, bin } => {
            let total: u64 = list.iter().map(|s| s.total()).sum();
            let rows = list
                .iter()
                .map(|s| {
                    let (title, icon) = kind_words(s.kind);
                    let body = match s.kind {
                        SuggestionKind::OldDownloads => {
                            let cutoff = SystemTime::now().checked_sub(Duration::from_secs(u64::from(OLD_DAYS) * 86_400)).map(|t| m::day_month(stamp(t))).unwrap_or_default();
                            let all_files = s.items.iter().all(|c| c.path.is_file());
                            let what = if all_files { m::plural(s.items.len() as u64, "file", "files") } else { m::plural(s.items.len() as u64, "item", "items") };
                            format!("{what} downloaded before {cutoff}. They are yours, so look first.")
                        }
                        SuggestionKind::TempFiles => "Windows makes new ones when it needs them. Files in use are skipped.".into(),
                        SuggestionKind::BrowserCaches => "Edge and Chrome. Sites load a bit slower for a day. You stay signed in.".into(),
                    };
                    dp::Suggestion { icon, title: title.into(), body, check_first: s.kind.tag() == ad::Tag::CheckFirst, size: m::size(s.total()), can_review: !s.items.is_empty() }
                })
                .collect();
            dp::CleanView {
                place,
                total: m::size(total),
                rows,
                bin: bin.map(m::size).unwrap_or_default(),
                nothing: (total == 0).then(|| format!("Nothing to clean on {drive}")),
            }
        }
        _ => {
            let rows = [
                SuggestionKind::OldDownloads,
                SuggestionKind::TempFiles,
                SuggestionKind::BrowserCaches,
            ]
            .into_iter()
            .map(|k| {
                let (title, icon) = kind_words(k);
                dp::Suggestion {
                    icon,
                    title: title.into(),
                    body: "Measuring what is there.".into(),
                    check_first: k.tag() == ad::Tag::CheckFirst,
                    size: "…".into(),
                    can_review: false,
                }
            })
            .collect();
            dp::CleanView {
                place,
                total: "Measuring".into(),
                rows,
                bin: String::new(),
                nothing: None,
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
                size: m::size(c.size),
                icon,
                checked: r.checked.contains(&i),
            }
        })
        .collect();
    let sel: u64 = r
        .checked
        .iter()
        .filter_map(|&i| r.items.get(i))
        .map(|c| c.size)
        .sum();
    let can = r
        .items
        .first()
        .is_some_and(|c| ad::has_recycle_bin(&c.path));
    Some(dp::ReviewView {
        title: title.into(),
        sub: format!(
            "{} · {}",
            m::plural(r.items.len() as u64, "item", "items"),
            m::size(total)
        ),
        rows,
        first: r.first,
        selected: selected_words(r.checked.len(), sel),
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
            "Files on this drive would be gone for good, so [App] only opens them in Explorer."
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
    let items = c
        .items
        .iter()
        .take(m::CONFIRM_LIST)
        .map(|(p, s)| {
            let name = p.file_name().map_or_else(
                || p.display().to_string(),
                |n| n.to_string_lossy().into_owned(),
            );
            let icon = if p.is_dir() {
                Icon::Folder
            } else {
                file_icon(std::ffi::OsStr::new(&name))
            };
            (icon, name, m::size(*s))
        })
        .collect();
    Some(dp::ConfirmView {
        title: m::confirm_title(c.items.len()),
        items,
        more: m::confirm_more(c.items.len()),
        body: m::put_back_words(c.items.len()),
        honest: m::confirm_honest(total),
        on_move: c.on_move,
    })
}

fn view(app: &App) -> View {
    let screen = match app.disk.place {
        Place::Scanning => scanning_view(app).map(Screen::Scanning),
        Place::Results => results_view(app).map(Screen::Results),
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
    }
}

pub fn size(app: &App) -> (f32, f32) {
    dp::size(&app.g, &app.fonts, &view(app))
}

/// Before a draw: the results level for where the panel is (it is computed once per place).
pub fn prepare(app: &mut App) {
    level(app);
}

pub fn draw_panel(app: &App, dc: &ID2D1DeviceContext, hover: Option<Hit>) {
    dp::draw(&app.g, dc, &app.theme, &app.fonts, &view(app), hover);
}

pub fn hit(app: &App, x: f32, y: f32) -> Option<Hit> {
    dp::hit(&app.g, &app.fonts, &view(app), x, y)
}

/// Data behind the panel changed: rebuild the level and redraw.
fn refresh(app: &mut App) {
    level(app);
    panels::refresh(app);
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
    if let Some(s) = app.disk.sel {
        if s < app.disk.first {
            app.disk.first = s;
        } else if s >= app.disk.first + dp::LIST_ROWS {
            app.disk.first = s + 1 - dp::LIST_ROWS;
        }
    }
    if let Some(c) = app.disk.cursor {
        match app.disk.place {
            Place::Results if app.disk.tab == Tab::Files => {
                if c < app.disk.files_first {
                    app.disk.files_first = c;
                } else if c >= app.disk.files_first + dp::FILES_ROWS {
                    app.disk.files_first = c + 1 - dp::FILES_ROWS;
                }
            }
            Place::Review => {
                if let Some(r) = &mut app.disk.review {
                    if c < r.first {
                        r.first = c;
                    } else if c >= r.first + dp::REVIEW_ROWS {
                        r.first = c + 1 - dp::REVIEW_ROWS;
                    }
                }
            }
            _ => {}
        }
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

/// Delete, the row's bin button, or the selection bar: the confirm sheet for what is chosen.
fn ask_for_selection(app: &mut App) {
    match app.disk.place {
        Place::Review => {
            let Some(r) = &app.disk.review else { return };
            let mut pick: Vec<usize> = r.checked.iter().copied().collect();
            if pick.is_empty() {
                pick.extend(app.disk.cursor.filter(|&c| c < r.items.len()));
            }
            let items = pick
                .iter()
                .filter_map(|&i| r.items.get(i))
                .map(|c| (c.path.clone(), c.size))
                .collect();
            let kind = r.kind;
            ask(app, items, Source::Clean(kind));
        }
        Place::Results if app.disk.tab == Tab::Files => {
            let mut ids: Vec<NodeId> = app.disk.checked.iter().copied().collect();
            if ids.is_empty() {
                ids.extend(app.disk.cursor.and_then(|c| file_id(app, c)));
            }
            let items = node_items(app, &ids);
            ask(app, items, Source::Tree);
        }
        Place::Results if app.disk.tab == Tab::Folders => {
            if let Some(id) = app.disk.sel.and_then(|s| row_id(app, s)) {
                let items = node_items(app, &[id]);
                ask(app, items, Source::Tree);
            }
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
            let ids: Vec<NodeId> = app.disk.checked.iter().copied().collect();
            node_items(app, &ids).into_iter().map(|(p, _)| p).collect()
        }
    };
    // One Explorer window per folder is plenty; the first item is shown selected.
    if let Some(p) = paths.first() {
        open_in_explorer(app, &p.clone());
    }
}

fn card_action(app: &mut App, i: usize) {
    let labels: Vec<Option<String>> = cards(app).into_iter().map(|c| c.button).collect();
    match labels.get(i).cloned().flatten().as_deref() {
        Some("Try again") if matches!(app.disk.note, Some(Note::Partial { .. })) => {
            if let Some(Note::Partial { stayed, source, .. }) = app.disk.note.take() {
                app.disk.merge_next = true;
                ask(app, stayed, source);
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
        Hit::ScanFolder => unsafe {
            let _ = PostMessageW(Some(app.hwnd), WM_APP_DISK, WPARAM(PICK_FOLDER), LPARAM(0));
        },
        Hit::OpenClean => open_clean(app),
        Hit::Stop => stop(app),
        Hit::ScanAgain => scan_again(app),
        Hit::Back => match app.disk.place {
            Place::Clean if app.disk.clean_from == Place::Results && app.disk.done().is_some() => {
                app.disk.place = Place::Results
            }
            Place::Clean => to_start(app),
            Place::Review => app.disk.place = Place::Clean,
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
            app.disk.tab = t;
            app.disk.cursor = None;
        }
        Hit::Tile(i) | Hit::Row(i) => {
            if app.disk.sel == Some(i) {
                open_row(app, i);
            } else {
                app.disk.sel = Some(i);
            }
        }
        Hit::RowOpen(i) => {
            if let Some(p) =
                row_id(app, i).and_then(|id| node_items(app, &[id]).first().map(|x| x.0.clone()))
            {
                open_in_explorer(app, &p);
            }
        }
        Hit::RowRecycle(i) => {
            app.disk.sel = Some(i);
            ask_for_selection(app);
        }
        Hit::File(i) => {
            app.disk.cursor = Some(i);
            toggle_file(app, i);
        }
        Hit::SelOpen => open_selection(app),
        Hit::SelRecycle => ask_for_selection(app),
        Hit::SelectAll => {
            if let Some(r) = &mut app.disk.review {
                if r.checked.len() == r.items.len() {
                    r.checked.clear();
                } else {
                    r.checked = (0..r.items.len()).collect();
                }
            }
        }
        Hit::Review(i) => open_review(app, i),
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
        Place::Clean => 3,
        Place::Review => app.disk.review.as_ref().map_or(0, |r| r.items.len()),
        Place::Scanning => 0,
    }
}

/// WM_KEYDOWN while the Disk panel has the keyboard. Esc comes as the hotkey (on_esc).
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
            app.disk.tab = tabs[j];
            app.disk.cursor = None;
        }
        VK_SPACE => {
            if let Some(c) = app.disk.cursor {
                toggle_file(app, c);
            }
        }
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
                if let Some(c) = app.disk.cursor {
                    open_review(app, c);
                }
            }
            _ => {}
        },
        VK_BACK => match app.disk.place {
            Place::Review => app.disk.place = Place::Clean,
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

/// Esc: the confirm sheet first (Cancel), then the panel.
pub fn on_esc(app: &mut App) {
    if app.disk.confirm.take().is_some() {
        app.disk.merge_next = false;
        log_line(app, "disk: confirm cancelled");
        refresh(app);
    } else {
        panels::close_panel(app);
    }
}

pub fn on_wheel(app: &mut App, x: f32, y: f32, delta: i16) {
    level(app);
    let v = view(app);
    let step = if delta > 0 { -1i64 } else { 1 };
    match dp::scroll_target(&v, x, y) {
        Some(dp::Scroll::Folders) => {
            let n = app.disk.level.as_ref().map_or(0, |l| l.rows.len());
            app.disk.first = (app.disk.first as i64 + step)
                .clamp(0, n.saturating_sub(dp::LIST_ROWS + 1) as i64)
                as usize;
        }
        Some(dp::Scroll::Files) => {
            let n = app.disk.level.as_ref().map_or(0, |l| l.files.len());
            app.disk.files_first = (app.disk.files_first as i64 + step)
                .clamp(0, n.saturating_sub(dp::FILES_ROWS) as i64)
                as usize;
        }
        Some(dp::Scroll::Review) => {
            if let Some(r) = &mut app.disk.review {
                r.first = (r.first as i64 + step)
                    .clamp(0, r.items.len().saturating_sub(dp::REVIEW_ROWS) as i64)
                    as usize;
            }
        }
        None => return,
    }
    panels::redraw(app);
}

/// A scan's tree takes about 130 bytes per file and folder (a whole drive is hundreds of MB), so it
/// is let go this long after the panel closes. A warm rescan takes a moment.
const RELEASE_MS: u32 = 3 * 60 * 1000;

pub fn on_release_timer(app: &mut App) {
    unsafe {
        let _ = KillTimer(Some(app.hwnd), TIMER_DISK_RELEASE);
    }
    if panel_shown(app) {
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

/// How long after a move the orb keeps offering Undo: the pill once on closing the panel, and
/// the card again when the panel reopens.
const UNDO_WINDOW: Duration = Duration::from_secs(60);

fn undo_fresh(app: &App) -> bool {
    app.disk
        .last_move
        .as_ref()
        .is_some_and(|l| l.since.elapsed().unwrap_or_default() < UNDO_WINDOW)
}

pub fn on_panel_closed(app: &mut App) {
    app.disk.confirm = None;
    app.disk.kb = false;
    if app.disk.done().is_some() {
        unsafe {
            SetTimer(Some(app.hwnd), TIMER_DISK_RELEASE, RELEASE_MS, None);
        }
    }
    // While the panel was open its card carried Undo; the orb offers it once more on the way out.
    let moved = match &app.disk.note {
        Some(Note::Moved(n)) => Some(*n),
        Some(Note::Partial { moved, .. }) if *moved > 0 => Some(*moved),
        _ => None,
    };
    // Not when the panel only switches to another one: the pill would sit on top of it.
    let fresh = undo_fresh(app);
    if let (Some(n), Some(last), false) = (moved, app.disk.last_move.as_mut(), app.panel_open) {
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
