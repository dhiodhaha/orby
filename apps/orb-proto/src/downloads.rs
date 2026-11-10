//! Downloads on app_downloads: the clipboard offer, the queue, the yt-dlp sidecar, the orb's
//! progress ring, the done and error pills, the Downloads panel and the small format menu.
//!
//! Each running download reports from its runner thread through one channel and a posted
//! WM_APP_DL; the UI thread drains the channel, feeds the queue and redraws once. Nothing ticks
//! at rest. yt-dlp lives in `%LOCALAPPDATA%\orb-proto\bin` and is fetched only when the user
//! presses Fetch. downloads_model.rs decides the words, downloads_panel.rs draws.

use std::collections::{HashMap, HashSet};
use std::io::{Read, Write};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc::{channel, Receiver, Sender};
use std::sync::Arc;
use std::time::{Duration, Instant, SystemTime};

use app_downloads::{
    self as dl, Download, ErrorKind, Format, FormatPrefs, Job, JobId, JobState, MediaKind,
    MediaLink, Queue, Ring, Sidecar, Tone, Update,
};
use app_habits::held::Priority;
use windows::core::{w, HSTRING, PCWSTR};
use windows::Win32::Foundation::*;
use windows::Win32::Graphics::Direct2D::ID2D1DeviceContext;
use windows::Win32::Graphics::Gdi::*;
use windows::Win32::Storage::FileSystem::GetDiskFreeSpaceExW;
use windows::Win32::System::Com::*;
use windows::Win32::System::DataExchange::AddClipboardFormatListener;
use windows::Win32::UI::Input::KeyboardAndMouse::*;
use windows::Win32::UI::Shell::*;
use windows::Win32::UI::WindowsAndMessaging::*;

use crate::downloads_model::{self as dm, Btn, Field};
use crate::downloads_panel::{self as dp, EngineCard, Hit, View};
use crate::gfx::Surface;
use crate::hook::{self, unpack};
use crate::notices::{self, Source};
use crate::orb::{self, work_area, Side};
use crate::panels::{self, Kind};
use crate::pill::dismiss_pill;
use crate::ui::{self, Icon, Pill, PillAction};
use crate::{
    habits, log_line, px, start_anim, store, with_app, App, MSG_MOUSELEAVE, TIMER_PILL, WM_APP_DL,
};

const GITHUB: &str = "https://github.com/yt-dlp/yt-dlp/releases/latest/download/yt-dlp.exe";
/// Some apps set the clipboard twice for one copy.
const SAME_COPY: Duration = Duration::from_millis(800);

pub enum Engine {
    Ready(Sidecar),
    Missing,
    Fetching { done: u64, total: Option<u64> },
    Failed(String),
}

enum Msg {
    Job(JobId, Update),
    Fetch(u64, Option<u64>),
    Fetched(Result<PathBuf, String>),
    /// Change folder moved the job's half-done files; these are their new paths.
    Moved(JobId, Vec<PathBuf>),
    /// The pause after a job's first unnamed failure is over: start it again.
    Restart(JobId),
}

/// How long a job waits before its one silent retry.
const RETRY_PAUSE: Duration = Duration::from_secs(3);

#[derive(Clone)]
enum MenuFor {
    Offer(MediaLink),
    Job(JobId),
}

struct Menu {
    target: MenuFor,
    chosen: Format,
    hover: Option<usize>,
    tracking: bool,
}

pub struct Downloads {
    pub queue: Queue,
    running: HashMap<JobId, Download>,
    /// A cancelled job's files, deleted once its process has exited.
    to_delete: HashMap<JobId, Vec<PathBuf>>,
    /// Disk-full jobs whose half-done files are moving to the folder just picked.
    moving: HashSet<JobId>,
    tx: Sender<Msg>,
    rx: Receiver<Msg>,
    /// A WM_APP_DL is already on its way. yt-dlp prints many lines a second; one message per
    /// batch keeps clicks from queueing behind them.
    posted: Arc<AtomicBool>,
    engine: Engine,
    bin: PathBuf,
    prefs: FormatPrefs,
    folder: PathBuf,
    field: Field,
    scroll: f32,
    /// A failure that needs you: the ember badge on the drop until the panel is opened.
    pub attention: bool,
    /// The hover pill has named the ring since the pointer came close.
    hover_named: bool,
    /// Done pills merge while one is on screen: (files, bytes).
    last_copy: Option<(String, Instant)>,
    last_ring: Ring,
    spin_from: Instant,
    /// The disk-full job waiting for the folder picker.
    folder_for: Option<JobId>,
    menu: Option<Menu>,
    menu_hwnd: HWND,
    menu_surface: Option<Surface>,
}

fn bin_dir() -> PathBuf {
    match std::env::var_os("ORB_PROTO_DATA") {
        // A test drive keeps its engine next to its data.
        Some(d) => PathBuf::from(d).join("bin"),
        None => std::env::var_os("LOCALAPPDATA")
            .map(PathBuf::from)
            .unwrap_or_else(crate::exe_dir)
            .join("orb-proto")
            .join("bin"),
    }
}

fn env_path(key: &str) -> Option<PathBuf> {
    std::env::var_os(key)
        .filter(|v| !v.is_empty())
        .map(PathBuf::from)
}

fn find_yt_dlp(bin: &Path) -> Option<PathBuf> {
    match env_path("ORB_YTDLP") {
        Some(p) => p.is_file().then_some(p),
        None => Some(bin.join("yt-dlp.exe")).filter(|p| p.is_file()),
    }
}

/// ffmpeg merges video and sound. Ours, then one on PATH; a missing one fails a merge with
/// "needs ffmpeg" (fetching it is not in this unit).
fn find_ffmpeg(bin: &Path) -> PathBuf {
    if let Some(p) = env_path("ORB_FFMPEG") {
        return p;
    }
    let ours = bin.join("ffmpeg.exe");
    if ours.is_file() {
        return ours;
    }
    std::env::var_os("PATH")
        .and_then(|p| {
            std::env::split_paths(&p)
                .map(|d| d.join("ffmpeg.exe"))
                .find(|f| f.is_file())
        })
        .unwrap_or(ours)
}

/// yt-dlp with its ffmpeg and, for YouTube, a JavaScript runtime (deno next to yt-dlp, else
/// deno, node or bun on PATH).
fn sidecar(yt_dlp: PathBuf, bin: &Path) -> Sidecar {
    Sidecar {
        yt_dlp,
        ffmpeg: find_ffmpeg(bin),
        js: dl::find_js_runtime(bin, std::env::var_os("PATH").as_deref()),
    }
}

fn downloads_known() -> Option<PathBuf> {
    unsafe {
        let p = SHGetKnownFolderPath(&FOLDERID_Downloads, KF_FLAG_DEFAULT, None).ok()?;
        let s = p.to_string().ok();
        CoTaskMemFree(Some(p.0 as *const _));
        s.map(PathBuf::from)
    }
}

fn default_folder() -> PathBuf {
    env_path("ORB_DOWNLOADS").unwrap_or_else(|| {
        downloads_known()
            .or_else(|| env_path("USERPROFILE").map(|h| h.join("Downloads")))
            .unwrap_or_else(crate::exe_dir)
            .join("orb-proto")
    })
}

/// "Downloads\orb-proto" when it is inside the Downloads folder, else the whole path.
fn folder_label(folder: &Path) -> String {
    match downloads_known().and_then(|d| folder.strip_prefix(&d).ok().map(Path::to_path_buf)) {
        Some(rest) if rest.as_os_str().is_empty() => "Downloads".into(),
        Some(rest) => format!("Downloads\\{}", rest.display()),
        None => folder.display().to_string(),
    }
}

/// "38.2 GB free on C:", read from the folder or the nearest folder above it that exists.
fn free_label(folder: &Path) -> String {
    let Some(existing) = folder.ancestors().find(|p| p.is_dir()) else {
        return String::new();
    };
    let mut free = 0u64;
    let ok = unsafe {
        GetDiskFreeSpaceExW(
            &HSTRING::from(existing.as_os_str()),
            Some(&mut free),
            None,
            None,
        )
    }
    .is_ok();
    if ok {
        format!("{} free on {}", dm::size(free), dm::drive(folder))
    } else {
        String::new()
    }
}

impl Downloads {
    pub fn menu_hwnd(&self) -> HWND {
        self.menu_hwnd
    }

    pub fn load(menu_hwnd: HWND) -> Downloads {
        let bin = bin_dir();
        let engine = match find_yt_dlp(&bin) {
            Some(yt_dlp) => Engine::Ready(sidecar(yt_dlp, &bin)),
            None => Engine::Missing,
        };
        let (tx, rx) = channel();
        Downloads {
            queue: Queue::new(),
            running: HashMap::new(),
            to_delete: HashMap::new(),
            moving: HashSet::new(),
            tx,
            rx,
            posted: Arc::new(AtomicBool::new(false)),
            engine,
            bin,
            prefs: FormatPrefs::default(),
            folder: default_folder(),
            field: Field::Empty,
            scroll: 0.0,
            attention: false,
            hover_named: false,
            last_copy: None,
            last_ring: Ring::Hidden,
            spin_from: Instant::now(),
            folder_for: None,
            menu: None,
            menu_hwnd,
            menu_surface: None,
        }
    }
}

/// Sends a report to the UI thread, posting WM_APP_DL only when none is pending.
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
                    WM_APP_DL,
                    WPARAM(0),
                    LPARAM(0),
                );
            }
        }
    }
}

fn reporter(app: &App) -> Reporter {
    Reporter {
        tx: app.dl.tx.clone(),
        posted: Arc::clone(&app.dl.posted),
        hwnd: app.hwnd.0 as isize,
    }
}

/// At launch: the clipboard listener, and where the engine and the folder are.
pub fn start(app: &mut App) {
    let listening = unsafe { AddClipboardFormatListener(app.hwnd) }.is_ok();
    let engine = match &app.dl.engine {
        Engine::Ready(s) => format!(
            "yt-dlp {} (ffmpeg {}, js runtime {})",
            s.yt_dlp.display(),
            if s.ffmpeg.is_file() {
                "found"
            } else {
                "missing"
            },
            s.js.as_ref()
                .map_or("none".to_string(), |j| j.path.display().to_string())
        ),
        _ => format!("yt-dlp missing in {}", app.dl.bin.display()),
    };
    let folder = app.dl.folder.display().to_string();
    log_line(
        app,
        &format!("downloads: clipboard listener {listening}, {engine}, saving to {folder}"),
    );
}

// ---- the clipboard offer ----

/// WM_CLIPBOARDUPDATE: the text is read only to match a media link; nothing is fetched.
pub fn on_clipboard(app: &mut App) {
    let Some(text) = crate::sys::read_clipboard(app) else {
        return;
    };
    let text = text.trim().to_string();
    if app
        .dl
        .last_copy
        .as_ref()
        .is_some_and(|(t, at)| *t == text && at.elapsed() < SAME_COPY)
    {
        return;
    }
    app.dl.last_copy = Some((text.clone(), Instant::now()));
    let Some(link) = MediaLink::parse(&text) else {
        return;
    };
    // Full screen, focus, a pause or the hidden orb drop the offer: a stale offer is noise.
    let why_not = if let Some(why) = notices::offer_blocked(app) {
        Some(why)
    } else if panel_shown(app) {
        Some("the Downloads panel is open, Paste is there")
    } else if dm::in_queue(&app.dl.queue, &link) {
        Some("it is already in the queue")
    } else {
        None
    };
    let short = dm::short_link(&link);
    if let Some(why) = why_not {
        log_line(app, &format!("downloads: copied {short}, no offer ({why})"));
        return;
    }
    let pill = match app.dl.queue.saved(&link) {
        Some(job) => {
            let JobState::Done { path, .. } = &job.state else {
                unreachable!("saved() returns done jobs")
            };
            Pill {
                icon: Icon::Tick,
                dot: false,
                title: "Already saved".into(),
                value: dm::pill_title(&dm::title(job)),
                action: Some(("Open".into(), PillAction::OpenFile(path.clone()))),
                closable: true,
                ttl_ms: 6000,
            }
        }
        None => offer_pill(&link, app.dl.prefs.for_link(&link)),
    };
    log_line(
        app,
        &format!(
            "downloads: copied {short}, pill \"{} {}\"",
            pill.title, pill.value
        ),
    );
    notices::post(app, notices::offer(pill));
}

fn offer_pill(link: &MediaLink, format: Format) -> Pill {
    let (title, value) = dm::offer_words(format);
    let icon = if format.kind() == MediaKind::Audio {
        Icon::Music
    } else {
        Icon::Film
    };
    Pill {
        icon,
        dot: false,
        title: title.into(),
        value,
        action: Some(("Save".into(), PillAction::SaveLink(link.clone(), format))),
        closable: true,
        ttl_ms: 6000,
    }
}

/// Save on the offer pill, a format menu row, or Paste: the link joins the queue.
pub fn save(app: &mut App, link: MediaLink, format: Format) {
    close_menu(app);
    let short = dm::short_link(&link);
    drop_offers(app, &link);
    if dm::in_queue(&app.dl.queue, &link) {
        // Two jobs for one link would write the same part file at once.
        log_line(app, &format!("downloads: {short} is already in the queue"));
        return;
    }
    let folder = app.dl.folder.clone();
    app.dl.queue.add(link, format, folder);
    log_line(
        app,
        &format!("downloads: saving {short} as {}", format.label()),
    );
    if !matches!(app.dl.engine, Engine::Ready(_)) && !panel_shown(app) {
        // The first-run card explains the wait and holds the Fetch button.
        open(app);
    }
    pump(app);
}

/// The link joined the queue: an offer for it on screen or waiting its turn is out of date.
fn drop_offers(app: &mut App, link: &MediaLink) {
    let offers = |p: &Pill| matches!(&p.action, Some((_, PillAction::SaveLink(l, _))) if l == link);
    notices::drop_offers(app, offers);
    if app.pill.as_ref().is_some_and(offers) {
        dismiss_pill(app);
    }
}

// ---- running the queue ----

/// Starts what may start, then redraws what shows the queue.
fn pump(app: &mut App) {
    if let Engine::Ready(sidecar) = &app.dl.engine {
        let sidecar = sidecar.clone();
        for (id, req) in app.dl.queue.start_ready() {
            launch(app, &sidecar, id, &req, "started");
        }
    }
    after(app);
}

/// Spawns yt-dlp for a job the queue has made active.
fn launch(app: &mut App, sidecar: &Sidecar, id: JobId, req: &dl::Request, verb: &str) {
    let _ = std::fs::create_dir_all(&req.folder);
    let r = reporter(app);
    match dl::start(sidecar, req, move |u| r.send(Msg::Job(id, u))) {
        Ok(d) => {
            app.dl.running.insert(id, d);
            log_line(
                app,
                &format!(
                    "downloads: {verb} {} ({})",
                    dm::short_link(&req.link),
                    req.format.label()
                ),
            );
        }
        Err(e) => {
            log_line(app, &format!("downloads: could not start yt-dlp ({e})"));
            // Back to waiting, so it starts once the engine is there.
            let _ = app.dl.queue.pause(id);
            let _ = app.dl.queue.resume(id);
            app.dl.engine = Engine::Missing;
        }
    }
}

/// The silent retry: only if the job still waits for it (not paused, cancelled or already
/// started again by a resume in between).
fn restart(app: &mut App, id: JobId) {
    if app.dl.running.contains_key(&id) {
        return;
    }
    let Some(req) = app
        .dl
        .queue
        .get(id)
        .filter(|j| j.state.is_active())
        .map(|j| j.request.clone())
    else {
        return;
    };
    match &app.dl.engine {
        Engine::Ready(sidecar) => {
            let sidecar = sidecar.clone();
            launch(app, &sidecar, id, &req, "trying again");
        }
        // Back to waiting, so it starts once the engine is there.
        _ => {
            let _ = app.dl.queue.pause(id);
            let _ = app.dl.queue.resume(id);
        }
    }
}

fn ring_changed(a: Ring, b: Ring) -> bool {
    match (a, b) {
        // About one degree of the ring.
        (
            Ring::Progress {
                fraction: f,
                tone: t,
            },
            Ring::Progress {
                fraction: g,
                tone: u,
            },
        ) => t != u || (f - g).abs() >= 1.0 / 360.0,
        _ => a != b,
    }
}

fn after(app: &mut App) {
    let ring = app.dl.queue.ring();
    if ring_changed(app.dl.last_ring, ring) {
        if app.dl.last_ring != Ring::Reading && ring == Ring::Reading {
            app.dl.spin_from = Instant::now();
        }
        app.dl.last_ring = ring;
        orb::render_orb(app);
        if spinning(app) {
            start_anim(app);
        }
    }
    if panel_shown(app) {
        panels::redraw(app);
    }
}

/// WM_APP_DL: runner and fetch reports, drained at once so one redraw covers them.
pub fn on_msg(app: &mut App) {
    app.dl.posted.store(false, Ordering::Release);
    let msgs: Vec<Msg> = app.dl.rx.try_iter().collect();
    if msgs.is_empty() {
        return;
    }
    let now = SystemTime::now();
    for m in msgs {
        match m {
            Msg::Job(id, u) => {
                let finished = matches!(u, Update::Finished(_));
                if finished {
                    app.dl.running.remove(&id);
                    if let Some(files) = app.dl.to_delete.remove(&id) {
                        delete_leftovers(&files);
                    }
                }
                let was_active = app.dl.queue.get(id).is_some_and(|j| j.state.is_active());
                if app.dl.queue.apply(id, u, now) {
                    let why = app
                        .dl
                        .queue
                        .get(id)
                        .map(|j| {
                            format!(
                                "{}: {}",
                                dm::short_link(&j.request.link),
                                j.error_text.as_deref().unwrap_or("no error text")
                            )
                        })
                        .unwrap_or_default();
                    log_line(
                        app,
                        &format!(
                            "downloads: {why}; trying again in {} s",
                            RETRY_PAUSE.as_secs()
                        ),
                    );
                    let r = reporter(app);
                    std::thread::spawn(move || {
                        std::thread::sleep(RETRY_PAUSE);
                        r.send(Msg::Restart(id));
                    });
                } else if finished && was_active {
                    if let Some(job) = app.dl.queue.get(id).cloned() {
                        on_finished(app, &job);
                    }
                }
            }
            Msg::Fetch(done, total) => {
                if let Engine::Fetching { .. } = app.dl.engine {
                    app.dl.engine = Engine::Fetching { done, total };
                }
            }
            Msg::Fetched(Ok(yt_dlp)) => {
                log_line(
                    app,
                    &format!("downloads: yt-dlp is ready at {}", yt_dlp.display()),
                );
                app.dl.engine = Engine::Ready(sidecar(yt_dlp, &app.dl.bin));
            }
            Msg::Fetched(Err(why)) => {
                log_line(app, &format!("downloads: fetching yt-dlp failed: {why}"));
                app.dl.engine = Engine::Failed(why);
            }
            Msg::Restart(id) => restart(app, id),
            Msg::Moved(id, moved) => {
                app.dl.moving.remove(&id);
                if app.dl.queue.get(id).is_some() {
                    log_line(
                        app,
                        &format!(
                            "downloads: moved {} half-done file(s), going on",
                            moved.len()
                        ),
                    );
                    let _ = app.dl.queue.retry(id);
                } else {
                    // Removed while its files were moving.
                    for f in &moved {
                        let _ = std::fs::remove_file(f);
                    }
                }
            }
        }
    }
    pump(app);
}

/// The open Downloads panel already shows what a pill would say.
fn panel_shown(app: &App) -> bool {
    app.panel_open && app.panel_kind == Kind::Downloads
}

fn on_finished(app: &mut App, job: &Job) {
    let name = dm::title(job);
    // While the orb is hidden they wait on the badge; the open panel already shows them.
    let pills = !panel_shown(app);
    match &job.state {
        JobState::Done { path, size, .. } => {
            log_line(
                app,
                &format!("downloads: saved {} ({})", path.display(), dm::size(*size)),
            );
            let p = Pill {
                icon: Icon::Tick,
                dot: false,
                title: "Saved".into(),
                value: dm::pill_title(&name),
                action: Some(("Open".into(), PillAction::OpenFile(path.clone()))),
                closable: true,
                ttl_ms: 6000,
            };
            // A second one while the first shows or waits merges: "2 downloads done Videos".
            let folder = job
                .request
                .folder
                .file_name()
                .map(|f| f.to_string_lossy().to_string())
                .unwrap_or_default();
            let n = notices::merging(
                notices::Notice::new(Source::Downloads, Priority::Normal, p),
                "done",
                "downloads done",
                &folder,
                Some(("Show".into(), PillAction::OpenDownloads)),
            );
            if pills {
                notices::post(app, n);
            }
        }
        JobState::Failed { kind, .. } => {
            log_line(
                app,
                &format!(
                    "downloads: {} failed ({kind:?}): {}",
                    dm::short_link(&job.request.link),
                    job.error_text.as_deref().unwrap_or("no error text")
                ),
            );
            let (title, mut value, action) = dm::error_words(*kind, &job.request.folder);
            if *kind == ErrorKind::Offline {
                let n = dm::offline_count(&app.dl.queue);
                value = if n == 1 {
                    "1 download paused".into()
                } else {
                    format!("{n} downloads paused")
                };
            } else if !panel_shown(app) {
                app.dl.attention = true;
                orb::render_orb(app);
            }
            let icon = match kind {
                ErrorKind::Private => Icon::Private,
                ErrorKind::DiskFull => Icon::Drive,
                ErrorKind::Offline => Icon::CloudOff,
                _ => Icon::Alert,
            };
            let action = action.map(|label| {
                let a = if *kind == ErrorKind::DiskFull {
                    PillAction::ChangeFolder(job.id)
                } else {
                    PillAction::RetryDownload(job.id)
                };
                (label.to_string(), a)
            });
            let p = Pill {
                icon,
                dot: false,
                title: title.into(),
                value,
                action,
                closable: true,
                ttl_ms: 8000,
            };
            if pills {
                notices::notify(app, Source::Downloads, p);
            }
        }
        _ => {}
    }
}

/// The ring for the orb (the focus ring wins while a session runs).
pub fn ring(app: &App) -> Ring {
    app.dl.queue.ring()
}

pub fn reduced_motion() -> bool {
    let mut on: i32 = 1;
    let ok = unsafe {
        SystemParametersInfoW(
            SPI_GETCLIENTAREAANIMATION,
            0,
            Some(&mut on as *mut i32 as *mut _),
            SYSTEM_PARAMETERS_INFO_UPDATE_FLAGS(0),
        )
    }
    .is_ok();
    ok && on == 0
}

/// Reading a link: the short arc travels round the drop (held still under reduced motion).
pub fn spinning(app: &App) -> bool {
    app.visible
        && app.dl.queue.ring() == Ring::Reading
        && habits::ring(app).is_none()
        && !reduced_motion()
}

/// Where the travelling arc starts, 0..1 of a turn.
pub fn spin_start(app: &App) -> f32 {
    if reduced_motion() {
        0.0
    } else {
        (app.dl.spin_from.elapsed().as_secs_f32() * 0.8).fract()
    }
}

pub fn ring_color(app: &App, tone: Tone) -> [f32; 3] {
    let c = match tone {
        Tone::Ink => app.theme.text,
        Tone::Grey => app.theme.passed,
        Tone::Ember => app.theme.ember,
    };
    [c.r, c.g, c.b]
}

fn is_hover_pill(p: &Pill) -> bool {
    p.action.is_none() && p.title == "Downloading"
}

/// The pointer left the orb: the pill that named the ring goes with it.
pub fn on_orb_gone(app: &mut App) {
    app.dl.hover_named = false;
    if !app.pill_closing && app.pill.as_ref().is_some_and(is_hover_pill) {
        dismiss_pill(app);
    }
}

/// The pointer came close to the drop while saving: a pill names the ring once, and stays
/// while the pointer does.
pub fn on_orb_near(app: &mut App, close: bool) {
    if !close {
        app.dl.hover_named = false;
        return;
    }
    let ring = app.dl.queue.ring();
    if app.dl.hover_named
        || app.pill.is_some()
        || app.dial_open
        || ring == Ring::Hidden
        || habits::ring(app).is_some()
    {
        return;
    }
    let value = if ring == Ring::Reading {
        "reading the link".to_string()
    } else {
        dm::overall_words(app.dl.queue.overall())
    };
    notices::reply(
        app,
        Pill {
            icon: Icon::Download,
            dot: false,
            title: "Downloading".into(),
            value,
            action: None,
            closable: false,
            ttl_ms: 6000,
        },
    );
    app.dl.hover_named = true;
}

pub fn dial_value(app: &App) -> String {
    let q = &app.dl.queue;
    dm::dial_value(q.ring(), q.counts(), dm::offline_count(q))
}

pub fn dial_children(app: &App) -> (Vec<ui::DialChild>, usize) {
    let c = app.dl.queue.counts();
    let value = if c == app_downloads::Counts::default() {
        String::new()
    } else {
        dm::count_label(c, dm::offline_count(&app.dl.queue))
    };
    (
        vec![ui::DialChild {
            text: String::new(),
            icon: Some(Icon::Download),
            name: "Downloads".into(),
            value,
        }],
        0,
    )
}

// ---- row and pill actions ----

fn pause(app: &mut App, id: JobId) {
    if app.dl.queue.pause(id).is_ok() {
        if let Some(d) = app.dl.running.get(&id) {
            let _ = d.stop();
        }
        log_line(app, "downloads: paused");
    }
    after(app);
}

fn resume(app: &mut App, id: JobId) {
    if app.dl.queue.resume(id).is_ok() {
        log_line(app, "downloads: resumed");
    }
    pump(app);
}

/// Cancel (and Remove on a failed row): the job leaves, its part files are deleted.
fn cancel(app: &mut App, id: JobId) {
    if let Ok(c) = app.dl.queue.cancel(id) {
        match app.dl.running.get(&id).filter(|_| c.stop_process) {
            Some(d) => {
                let _ = d.stop();
                app.dl.to_delete.insert(id, c.delete);
            }
            None => delete_leftovers(&c.delete),
        }
        log_line(app, "downloads: cancelled");
    }
    pump(app);
}

/// Only what the job left half done; missing files are fine.
fn delete_leftovers(files: &[PathBuf]) {
    for f in dm::leftovers(files) {
        let _ = std::fs::remove_file(f);
    }
}

pub fn retry(app: &mut App, id: JobId) {
    // A job whose files are moving goes on by itself once they are there.
    if !app.dl.moving.contains(&id) && app.dl.queue.retry(id).is_ok() {
        log_line(app, "downloads: retry");
    }
    pump(app);
}

/// The folder picker, run from the orb window's message with the app not borrowed: its modal
/// loop keeps dispatching our messages, and they need the app.
pub fn pick_folder(owner: HWND, start: Option<&Path>, title: &str) -> Option<PathBuf> {
    unsafe {
        let _ = CoInitializeEx(None, COINIT_APARTMENTTHREADED);
        let dlg: IFileOpenDialog =
            CoCreateInstance(&FileOpenDialog, None, CLSCTX_INPROC_SERVER).ok()?;
        let opts = dlg.GetOptions().ok()?;
        dlg.SetOptions(opts | FOS_PICKFOLDERS | FOS_FORCEFILESYSTEM)
            .ok()?;
        dlg.SetTitle(&HSTRING::from(title)).ok()?;
        // It opens where downloads go now (or the nearest folder above it that exists).
        if let Some(dir) = start.and_then(|p| p.ancestors().find(|a| a.is_dir())) {
            if let Ok(item) = SHCreateItemFromParsingName::<_, _, IShellItem>(
                &HSTRING::from(dir.as_os_str()),
                None,
            ) {
                let _ = dlg.SetFolder(&item);
            }
        }
        dlg.Show(Some(owner)).ok()?;
        let item = dlg.GetResult().ok()?;
        let p = item.GetDisplayName(SIGDN_FILESYSPATH).ok()?;
        let s = p.to_string().ok();
        CoTaskMemFree(Some(p.0 as *const _));
        s.map(PathBuf::from)
    }
}

/// Disk full: pick another folder (WM_APP_DL with PICK_FOLDER opens the picker).
pub fn change_folder(app: &mut App, id: JobId) {
    if app.dl.moving.contains(&id) {
        return;
    }
    app.dl.folder_for = Some(id);
    log_line(app, "downloads: change folder");
    unsafe {
        let _ = PostMessageW(Some(app.hwnd), WM_APP_DL, WPARAM(PICK_FOLDER), LPARAM(0));
    }
}

pub const PICK_FOLDER: usize = 1;

/// Where the folder picker opens.
pub fn save_folder(app: &mut App) -> PathBuf {
    app.dl.folder.clone()
}

/// The job goes on in the picked folder, and so do later ones. Its half-done files move there
/// first, on a thread (another drive means a copy), so it goes on from where it stopped.
pub fn on_folder_picked(app: &mut App, folder: Option<PathBuf>) {
    let Some(id) = app.dl.folder_for.take() else {
        return;
    };
    let Some(folder) = folder else {
        log_line(app, "downloads: folder unchanged");
        return;
    };
    log_line(
        app,
        &format!("downloads: folder changed to {}", folder.display()),
    );
    app.dl.folder = folder.clone();
    let moves = app
        .dl
        .queue
        .get(id)
        .map(|j| dm::moves(&j.files, &folder))
        .unwrap_or_default();
    if app.dl.queue.set_folder(id, &folder).is_ok() {
        if moves.is_empty() {
            let _ = app.dl.queue.retry(id);
        } else {
            app.dl.moving.insert(id);
            let rep = reporter(app);
            std::thread::spawn(move || {
                let moved = moves
                    .into_iter()
                    .filter_map(|(src, dst)| move_file(&src, &dst).then_some(dst))
                    .collect();
                rep.send(Msg::Moved(id, moved));
            });
        }
    }
    pump(app);
}

/// Moves one half-done file; true when it is at `dst`. The old copy goes either way: it sits on
/// the disk that is full, and the job now lives in the new folder.
fn move_file(src: &Path, dst: &Path) -> bool {
    if !src.is_file() {
        return false;
    }
    if dst.exists() {
        // Never overwrite; yt-dlp goes on from the copy already there.
        let _ = std::fs::remove_file(src);
        return false;
    }
    if std::fs::rename(src, dst).is_ok() {
        return true;
    }
    let copied = std::fs::copy(src, dst).is_ok();
    if !copied {
        let _ = std::fs::remove_file(dst);
    }
    let _ = std::fs::remove_file(src);
    copied
}

/// Quit: a yt-dlp left running would go on writing with nobody to finish it. Stopping it keeps
/// the part file, as Pause does, so the same link later goes on from there.
pub fn shutdown(app: &mut App) {
    if app.dl.running.is_empty() {
        return;
    }
    for d in app.dl.running.values() {
        let _ = d.stop();
    }
    let n = app.dl.running.len();
    app.dl.running.clear();
    log_line(app, &format!("downloads: stopped {n} running at quit"));
}

fn shell_open(file: &str, params: Option<&str>) {
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

pub fn open_file(app: &mut App, path: &Path) {
    log_line(app, &format!("downloads: open {}", path.display()));
    shell_open(&path.to_string_lossy(), None);
}

fn show_in_folder(app: &mut App, path: &Path) {
    log_line(
        app,
        &format!("downloads: show in folder {}", path.display()),
    );
    shell_open(
        "explorer.exe",
        Some(&format!("/select,\"{}\"", path.display())),
    );
}

fn done_path(app: &App, id: JobId) -> Option<PathBuf> {
    match &app.dl.queue.get(id)?.state {
        JobState::Done { path, .. } => Some(path.clone()),
        _ => None,
    }
}

/// Paste in the panel, or tray "Paste a link into Downloads".
pub fn paste(app: &mut App) {
    let text = crate::sys::read_clipboard(app).unwrap_or_default();
    match dm::paste_verdict(&text) {
        Ok(link) => {
            app.dl.field = Field::Empty;
            if let Some(path) = done_path_of(app, &link) {
                log_line(app, "downloads: pasted a link that is already saved");
                show_in_folder(app, &path);
            } else {
                let f = app.dl.prefs.for_link(&link);
                save(app, link, f);
            }
        }
        Err(field) => {
            log_line(app, &format!("downloads: paste refused ({field:?})"));
            app.dl.field = field;
        }
    }
    if !panel_shown(app) {
        open(app);
    }
    after(app);
}

fn done_path_of(app: &App, link: &MediaLink) -> Option<PathBuf> {
    app.dl.queue.saved(link).and_then(|j| done_path(app, j.id))
}

// ---- the panel ----

pub fn open(app: &mut App) {
    app.dl.scroll = 0.0;
    if app.dl.attention {
        app.dl.attention = false;
        orb::render_orb(app);
    }
    panels::open_panel(app, Kind::Downloads);
}

fn unix(t: SystemTime) -> i64 {
    t.duration_since(SystemTime::UNIX_EPOCH)
        .map_or(0, |d| d.as_secs() as i64)
}

fn view(app: &App) -> View {
    let q = &app.dl.queue;
    let rows = dm::rows(q, &|t| habits::hhmm(store::local(unix(t))));
    let waiting = q.jobs().iter().any(|j| j.state == JobState::Waiting);
    let engine = match &app.dl.engine {
        Engine::Ready(_) => EngineCard::None,
        Engine::Missing => EngineCard::Missing { waiting },
        Engine::Fetching { done, total } => EngineCard::Fetching {
            done: *done,
            total: *total,
            waiting,
        },
        Engine::Failed(why) => EngineCard::Failed(why.clone()),
    };
    let mut v = View {
        count: dm::count_label(q.counts(), dm::offline_count(q)),
        engine,
        field: app.dl.field.clone(),
        rows,
        folder: folder_label(&app.dl.folder),
        free: free_label(&app.dl.folder),
        scroll: 0.0,
    };
    v.scroll = app.dl.scroll.clamp(0.0, dp::max_scroll(&v));
    v
}

pub fn height(app: &App) -> f32 {
    dp::height(&view(app))
}

pub fn draw_panel(app: &App, dc: &ID2D1DeviceContext, hover: Option<Hit>) {
    dp::draw(
        &mut crate::gfx::D2dCanvas::new(&app.g, dc),
        &app.theme,
        &app.fonts,
        &view(app),
        hover,
    );
}

pub fn hit(app: &App, x: f32, y: f32) -> Option<Hit> {
    dp::hit(&app.fonts, &view(app), x, y)
}

pub fn click(app: &mut App, h: Hit) {
    match h {
        Hit::Paste => paste(app),
        Hit::Fetch => fetch_engine(app),
        Hit::ClearList => {
            app.dl.queue.clear_done();
            after(app);
        }
        Hit::Folder => {
            let f = app.dl.folder.clone();
            let _ = std::fs::create_dir_all(&f);
            log_line(app, "downloads: open the folder");
            shell_open(&f.to_string_lossy(), None);
        }
        Hit::Row(_) => {}
        Hit::Btn(i, b) => {
            let Some(id) = view(app).rows.get(i).map(|r| r.id) else {
                return;
            };
            match b {
                Btn::Pause => pause(app, id),
                Btn::Resume => resume(app, id),
                Btn::Cancel | Btn::Remove => cancel(app, id),
                Btn::Retry => retry(app, id),
                Btn::ChangeFolder => change_folder(app, id),
                Btn::Format => open_job_menu(app, i, id),
                Btn::Open => {
                    if let Some(p) = done_path(app, id) {
                        open_file(app, &p);
                    }
                }
                Btn::Folder => {
                    if let Some(p) = done_path(app, id) {
                        show_in_folder(app, &p);
                    }
                }
            }
        }
    }
}

pub fn on_wheel(app: &mut App, delta: i16) {
    let v = view(app);
    let s = (v.scroll - delta as f32 / 120.0 * 48.0).clamp(0.0, dp::max_scroll(&v));
    if s != app.dl.scroll {
        app.dl.scroll = s;
        panels::redraw(app);
    }
}

// ---- fetching yt-dlp (only when the user presses Fetch) ----

fn fetch(url: &str, bin: &Path, progress: impl Fn(u64, Option<u64>)) -> Result<PathBuf, String> {
    std::fs::create_dir_all(bin)
        .map_err(|e| format!("Couldn't make the folder {} ({})", bin.display(), e.kind()))?;
    let tls = native_tls::TlsConnector::new()
        .map_err(|_| "Secure connections are not available".to_string())?;
    let agent = ureq::AgentBuilder::new()
        .tls_connector(Arc::new(tls))
        .timeout_connect(Duration::from_secs(15))
        .timeout_read(Duration::from_secs(30))
        .user_agent("orb-proto")
        .build();
    let resp = agent.get(url).call().map_err(|e| match e {
        ureq::Error::Status(code, _) => format!("The download answered {code}"),
        _ => "Couldn't reach GitHub".to_string(),
    })?;
    let total: Option<u64> = resp.header("Content-Length").and_then(|v| v.parse().ok());
    let part = bin.join("yt-dlp.exe.part");
    let mut out = std::fs::File::create(&part)
        .map_err(|e| format!("Couldn't write in {} ({})", bin.display(), e.kind()))?;
    let mut reader = resp.into_reader();
    let mut buf = vec![0u8; 64 * 1024];
    let (mut done, mut told) = (0u64, 0u64);
    let result = loop {
        let n = match reader.read(&mut buf) {
            Ok(0) => break Ok(()),
            Ok(n) => n,
            Err(_) => break Err("The download stopped halfway".to_string()),
        };
        if done == 0 && !buf[..n].starts_with(b"MZ") {
            break Err("The file was not a Windows program".to_string());
        }
        if out.write_all(&buf[..n]).is_err() {
            break Err("Couldn't write the file".to_string());
        }
        done += n as u64;
        if done - told >= 256 * 1024 {
            told = done;
            progress(done, total);
        }
    };
    drop(out);
    let result = result.and_then(|_| {
        if total.is_some_and(|t| t != done) {
            Err("The download stopped halfway".to_string())
        } else {
            Ok(())
        }
    });
    if let Err(e) = result {
        let _ = std::fs::remove_file(&part);
        return Err(e);
    }
    let dest = bin.join("yt-dlp.exe");
    std::fs::rename(&part, &dest)
        .map_err(|e| format!("Couldn't put yt-dlp in place ({})", e.kind()))?;
    Ok(dest)
}

pub fn fetch_engine(app: &mut App) {
    if matches!(app.dl.engine, Engine::Fetching { .. } | Engine::Ready(_)) {
        return;
    }
    app.dl.engine = Engine::Fetching {
        done: 0,
        total: None,
    };
    let url = std::env::var("ORB_YTDLP_URL")
        .ok()
        .filter(|u| !u.is_empty())
        .unwrap_or_else(|| GITHUB.to_string());
    let host = url.split('/').nth(2).unwrap_or("?").to_string();
    log_line(app, &format!("downloads: fetching yt-dlp from {host}"));
    let (bin, rep) = (app.dl.bin.clone(), reporter(app));
    std::thread::spawn(move || {
        let r = fetch(&url, &bin, |d, t| rep.send(Msg::Fetch(d, t)));
        rep.send(Msg::Fetched(r));
    });
    after(app);
}

// ---- the format menu window ----

pub fn menu_open(app: &App) -> bool {
    app.dl.menu.is_some()
}

fn render_menu(app: &mut App) {
    let Some(m) = &app.dl.menu else { return };
    let (chosen, hover) = (m.chosen, m.hover);
    let (w, h) = (
        px(app, dp::MENU_W).ceil() as u32,
        px(app, dp::MENU_H).ceil() as u32,
    );
    if app.dl.menu_surface.is_none() {
        app.dl.menu_surface = Surface::new(&app.g, app.dl.menu_hwnd, w, h, app.scale).ok();
    }
    let Some(s) = app.dl.menu_surface.as_ref() else {
        return;
    };
    let dc = s.begin(&app.g);
    dp::draw_menu(
        &mut crate::gfx::D2dCanvas::new(&app.g, dc),
        &app.theme,
        &app.fonts,
        chosen,
        hover,
    );
    s.end(&app.g);
}

fn menu_size(app: &App) -> (f32, f32) {
    (px(app, dp::MENU_W).ceil(), px(app, dp::MENU_H).ceil())
}

/// Shows the menu window at (x, y) in screen pixels.
fn show_menu(app: &mut App, menu: Menu, x: f32, y: f32) {
    let (w, h) = menu_size(app);
    app.dl.menu = Some(menu);
    unsafe {
        let _ = SetWindowPos(
            app.dl.menu_hwnd,
            Some(HWND_TOPMOST),
            x.round() as i32,
            y.round() as i32,
            w as i32,
            h as i32,
            SWP_NOACTIVATE | SWP_NOOWNERZORDER,
        );
        let r = px(app, 24.0).round() as i32;
        let rgn = CreateRoundRectRgn(0, 0, w as i32 + 1, h as i32 + 1, r, r);
        let _ = SetWindowRgn(app.dl.menu_hwnd, Some(rgn), true);
        let _ = ShowWindow(app.dl.menu_hwnd, SW_SHOWNOACTIVATE);
    }
    render_menu(app);
    hook::update_hook(app);
    hook::set_watch_clicks(true);
}

/// The offer pill's chevron or a right-click on it. The pill stays while the menu is open.
pub fn open_offer_menu(app: &mut App) {
    let Some(Some((_, PillAction::SaveLink(link, chosen)))) =
        app.pill.as_ref().map(|p| p.action.clone())
    else {
        return;
    };
    let mut r = RECT::default();
    unsafe {
        let _ = GetWindowRect(app.pill_hwnd, &mut r);
        let _ = KillTimer(Some(app.pill_hwnd), TIMER_PILL);
    }
    let pad = px(app, ui::PILL_PAD);
    let (w, h) = menu_size(app);
    // Above the pill (below it without room), on the pill's edge away from the drop.
    let x = if app.side == Side::Right {
        r.right as f32 - pad - w
    } else {
        r.left as f32 + pad
    };
    let (top, bottom, gap) = (r.top as f32 + pad, r.bottom as f32 - pad, px(app, 8.0));
    let y = if top - gap - h >= work_area(app.hwnd).top as f32 {
        top - gap - h
    } else {
        bottom + gap
    };
    log_line(app, "downloads: format menu (offer)");
    show_menu(
        app,
        Menu {
            target: MenuFor::Offer(link),
            chosen,
            hover: None,
            tracking: false,
        },
        x,
        y,
    );
}

/// A waiting row's format chip: the menu opens under it.
fn open_job_menu(app: &mut App, row: usize, id: JobId) {
    let v = view(app);
    let (Some(chip), Some(job)) = (
        dp::btn_rect(&app.fonts, &v, row, Btn::Format),
        app.dl.queue.get(id),
    ) else {
        return;
    };
    let chosen = job.request.format;
    let mut r = RECT::default();
    unsafe {
        let _ = GetWindowRect(app.panel_hwnd, &mut r);
    }
    let (ox, oy) = (r.left as f32, r.top as f32);
    let (t, rr, b) = (
        oy + px(app, chip.1),
        ox + px(app, chip.2),
        oy + px(app, chip.3),
    );
    let (w, h) = menu_size(app);
    // Under the chip and right-aligned to it, or above it near the screen's foot.
    let gap = px(app, 4.0);
    let y = if b + gap + h <= work_area(app.hwnd).bottom as f32 {
        b + gap
    } else {
        t - gap - h
    };
    log_line(app, "downloads: format menu (row)");
    show_menu(
        app,
        Menu {
            target: MenuFor::Job(id),
            chosen,
            hover: None,
            tracking: false,
        },
        rr - w,
        y,
    );
}

pub fn close_menu(app: &mut App) {
    let Some(m) = app.dl.menu.take() else { return };
    unsafe {
        let _ = ShowWindow(app.dl.menu_hwnd, SW_HIDE);
    }
    hook::set_watch_clicks(app.dial_open || app.panel_open);
    // The offer pill's auto-dismiss waited while its menu was open.
    if let (MenuFor::Offer(_), Some(p)) = (m.target, app.pill.as_ref()) {
        if !app.pill_closing && !app.pill_tracking {
            unsafe {
                SetTimer(Some(app.pill_hwnd), TIMER_PILL, p.ttl_ms, None);
            }
        }
    }
}

/// The offer pill went away: its menu goes too.
pub fn on_pill_dismissed(app: &mut App) {
    if matches!(
        app.dl.menu.as_ref().map(|m| &m.target),
        Some(MenuFor::Offer(_))
    ) {
        close_menu(app);
    }
}

fn menu_pick(app: &mut App, i: usize) {
    let Some(m) = &app.dl.menu else { return };
    let target = m.target.clone();
    let Some((format, ..)) = dm::menu_rows().into_iter().nth(i) else {
        return;
    };
    close_menu(app);
    match target {
        MenuFor::Offer(link) => {
            dismiss_pill(app);
            save(app, link, format);
        }
        MenuFor::Job(id) => {
            if app.dl.queue.set_format(id, format).is_ok() {
                log_line(
                    app,
                    &format!("downloads: format changed to {}", format.label()),
                );
            }
            after(app);
        }
    }
}

pub unsafe extern "system" fn menu_proc(
    hwnd: HWND,
    msg: u32,
    wparam: WPARAM,
    lparam: LPARAM,
) -> LRESULT {
    match msg {
        WM_MOUSEACTIVATE => return LRESULT(MA_NOACTIVATE as isize),
        WM_MOUSEMOVE => {
            with_app(|app| {
                let (x, y) = unpack(lparam);
                let row = dp::menu_hit(x / app.scale, y / app.scale);
                let Some(m) = app.dl.menu.as_mut() else {
                    return;
                };
                if !m.tracking {
                    let mut tme = TRACKMOUSEEVENT {
                        cbSize: std::mem::size_of::<TRACKMOUSEEVENT>() as u32,
                        dwFlags: TME_LEAVE,
                        hwndTrack: hwnd,
                        dwHoverTime: 0,
                    };
                    let _ = TrackMouseEvent(&mut tme);
                    m.tracking = true;
                }
                if m.hover != row {
                    m.hover = row;
                    render_menu(app);
                }
            });
            return LRESULT(0);
        }
        MSG_MOUSELEAVE => {
            with_app(|app| {
                if let Some(m) = app.dl.menu.as_mut() {
                    m.tracking = false;
                    m.hover = None;
                    render_menu(app);
                }
            });
            return LRESULT(0);
        }
        WM_LBUTTONUP => {
            with_app(|app| {
                let (x, y) = unpack(lparam);
                if let Some(i) = dp::menu_hit(x / app.scale, y / app.scale) {
                    menu_pick(app, i);
                }
            });
            return LRESULT(0);
        }
        _ => {}
    }
    DefWindowProcW(hwnd, msg, wparam, lparam)
}

#[cfg(test)]
mod tests {
    use super::move_file;

    #[test]
    fn a_moved_part_file_leaves_the_full_disk_and_never_overwrites() {
        let root = std::env::temp_dir().join(format!("orb-proto-move-{}", std::process::id()));
        let (old, new) = (root.join("full"), root.join("new"));
        std::fs::create_dir_all(&old).unwrap();
        std::fs::create_dir_all(&new).unwrap();
        let (src, dst) = (
            old.join("Clip [abc].mp4.part"),
            new.join("Clip [abc].mp4.part"),
        );
        std::fs::write(&src, b"25 KB so far").unwrap();
        assert!(move_file(&src, &dst));
        assert!(!src.exists(), "the full disk gets its space back");
        assert_eq!(std::fs::read(&dst).unwrap(), b"25 KB so far");

        std::fs::write(&src, b"older").unwrap();
        assert!(!move_file(&src, &dst), "a file already there wins");
        assert_eq!(std::fs::read(&dst).unwrap(), b"25 KB so far");
        assert!(!move_file(
            &old.join("missing.part"),
            &new.join("missing.part")
        ));
        let _ = std::fs::remove_dir_all(&root);
    }
}
