//! The calendar link and the Today panel. One ICS link (an https, webcal or file link, or a
//! path) is kept in `%APPDATA%\orb-proto\settings.json` and read every 10 min on a worker
//! thread with a small blocking HTTP client; the last good copy is cached in
//! `calendar-cache.ics`, so the day shows at launch and while offline. One exact timer runs to
//! the next event pill (10 min before, and at the start) or midnight. today_model.rs decides,
//! today_panel.rs draws.

use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::mpsc::{channel, Receiver};
use std::sync::Arc;
use std::time::Duration;

use app_calendar::{self as cal, Instant, Item, LocalDateTime, Occurrence, PrayerTime, TaskDue};
use app_core::prayer as pr;
use app_habits::held::Priority;
use app_store::{Task, TaskId};
use serde_json::Value;
use windows::core::{w, HSTRING};
use windows::Win32::Foundation::*;
use windows::Win32::Graphics::Direct2D::ID2D1DeviceContext;
use windows::Win32::UI::Shell::ShellExecuteW;
use windows::Win32::UI::WindowsAndMessaging::*;

use crate::notices;
use crate::panels::{self, Kind};
use crate::pill::info_pill;
use crate::tasks_model::{self as m, Selection};
use crate::today_model::{self as tm, Cue, Fail, Source, Sync};
use crate::today_panel::{self as tp, Card, Hit, Line, View};
use crate::ui::{Icon, Pill, PillAction};
use crate::{
    log_line, prayer, settings, store, tasks, App, TIMER_CAL_CUE, TIMER_CAL_READ, WM_APP_CAL,
};

const READ_EVERY_MS: u32 = 10 * 60 * 1000;
const CACHE: &str = "calendar-cache.ics";
const LINK_KEY: &str = "calendar_link";
const READ_AT_KEY: &str = "calendar_read_at";
/// Bigger than any real calendar export; a link that returns more is not one.
const MAX_BYTES: u64 = 10 * 1024 * 1024;

type ReadResult = Result<(String, cal::Calendar), (Fail, String)>;

pub struct Cal {
    dir: PathBuf,
    pub source: Option<Source>,
    pub sync: Sync,
    calendar: Option<cal::Calendar>,
    /// Occurrences from yesterday to the day after tomorrow, rebuilt after a read and at midnight.
    occ: Vec<Occurrence>,
    occ_day: Option<cal::Date>,
    /// The read in flight. Replacing it (a new link) orphans the old one's answer.
    rx: Option<Receiver<ReadResult>>,
    /// Event pills due up to here have had their turn (or were past at launch).
    fired_until: Instant,
    pub earlier_open: bool,
    pub scroll: f32,
    pub focus: Option<usize>,
}

/// What a click or Enter on a timeline line does.
enum Target {
    None,
    Earlier,
    Task(TaskId),
    Event(Option<String>),
}

/// Now, read once: the instant, the zone offset and the local day.
struct Now {
    at: Instant,
    ms: u64,
    offset: i32,
    today: cal::Date,
    day_start: Instant,
}

fn now() -> Now {
    let c = tasks::clock();
    let offset = (c.offset_secs / 60) as i32;
    let today = cal::Date::new(c.today.year(), c.today.month(), c.today.day()).expect("today");
    Now {
        at: Instant::from_unix_seconds((c.now_ms / 1000) as i64),
        ms: c.now_ms,
        offset,
        today,
        day_start: LocalDateTime::midnight(today).to_instant(offset),
    }
}

impl Cal {
    pub fn load(dir: &Path) -> Cal {
        let s = settings::read_map(dir);
        let source = s
            .get(LINK_KEY)
            .and_then(Value::as_str)
            .and_then(Source::parse);
        let mut c = Cal {
            dir: dir.to_path_buf(),
            sync: if source.is_some() {
                Sync::Loading
            } else {
                Sync::NotConnected
            },
            source,
            calendar: None,
            occ: Vec::new(),
            occ_day: None,
            rx: None,
            fired_until: now().at,
            earlier_open: false,
            scroll: 0.0,
            focus: None,
        };
        if c.source.is_some() {
            if let Ok(text) = std::fs::read_to_string(dir.join(CACHE)) {
                c.calendar = Some(cal::parse(&text));
                c.sync = Sync::Ready {
                    read_at: s.get(READ_AT_KEY).and_then(Value::as_i64).unwrap_or(0),
                };
            }
        }
        c
    }

    fn name(&self) -> String {
        self.calendar
            .as_ref()
            .and_then(|c| c.name.clone())
            .filter(|n| !n.trim().is_empty())
            .or_else(|| self.source.as_ref().map(Source::label))
            .unwrap_or_else(|| "Calendar".into())
    }
}

// ---- reading the link ----

fn fetch(src: &Source) -> Result<String, (Fail, String)> {
    match src {
        Source::File(p) => {
            let too_big = std::fs::metadata(p)
                .map(|m| m.len() > MAX_BYTES)
                .unwrap_or(false);
            if too_big {
                return Err((Fail::Broken, "The file is too big for a calendar".into()));
            }
            std::fs::read(p)
                .map(|b| String::from_utf8_lossy(&b).into_owned())
                .map_err(|e| {
                    (
                        Fail::Broken,
                        format!("Couldn't open {} ({})", src.label(), e.kind()),
                    )
                })
        }
        Source::Http(url) => {
            let tls = native_tls::TlsConnector::new().map_err(|_| {
                (
                    Fail::Offline,
                    "Secure connections are not available".to_string(),
                )
            })?;
            let agent = ureq::AgentBuilder::new()
                .tls_connector(Arc::new(tls))
                .timeout_connect(Duration::from_secs(10))
                .timeout(Duration::from_secs(30))
                .user_agent("orb-proto calendar reader")
                .build();
            match agent.get(url).call() {
                // into_string stops at 10 MB.
                Ok(r) => r
                    .into_string()
                    .map_err(|_| (Fail::Offline, "The read stopped halfway".to_string())),
                Err(ureq::Error::Status(code, _)) => {
                    Err((Fail::Broken, format!("The link answered {code}")))
                }
                Err(_) => Err((Fail::Offline, "Couldn't reach the calendar".to_string())),
            }
        }
    }
}

/// Starts a read on a worker thread unless one is in flight. Its answer comes back as WM_APP_CAL.
pub fn read_now(app: &mut App) {
    let Some(src) = app.cal.source.clone() else {
        return;
    };
    if app.cal.rx.is_some() {
        return;
    }
    let (tx, rx) = channel();
    app.cal.rx = Some(rx);
    let hwnd = app.hwnd.0 as isize;
    log_line(app, &format!("calendar: reading {}", src.label()));
    std::thread::spawn(move || {
        let r = fetch(&src).and_then(|text| {
            tm::check_ics(&text).map_err(|e| (Fail::Broken, e))?;
            let parsed = cal::parse(&text);
            Ok((text, parsed))
        });
        if tx.send(r).is_ok() {
            unsafe {
                let _ = PostMessageW(Some(HWND(hwnd as *mut _)), WM_APP_CAL, WPARAM(0), LPARAM(0));
            }
        }
    });
    panels::refresh(app);
}

pub fn on_read_done(app: &mut App) {
    let Some(r) = app.cal.rx.as_ref().and_then(|rx| rx.try_recv().ok()) else {
        return;
    };
    app.cal.rx = None;
    match r {
        Ok((text, parsed)) => {
            let read_at = now().at.unix_seconds();
            let tmp = app.cal.dir.join("calendar-cache.ics.tmp");
            if std::fs::write(&tmp, &text).is_ok() {
                let _ = std::fs::rename(&tmp, app.cal.dir.join(CACHE));
            }
            settings::update_map(&app.cal.dir, |s| {
                s.insert(READ_AT_KEY.into(), Value::from(read_at));
            });
            let n = parsed.events.len();
            app.cal.calendar = Some(parsed);
            app.cal.sync = Sync::Ready { read_at };
            let name = app.cal.name();
            log_line(app, &format!("calendar: read {name}, {n} events"));
        }
        Err((why, detail)) => {
            log_line(app, &format!("calendar: read failed ({why:?}): {detail}"));
            app.cal.sync = app.cal.sync.failed(why, detail);
        }
    }
    rebuild(app);
    schedule_cues(app);
    panels::refresh(app);
}

pub fn on_read_timer(app: &mut App) {
    read_now(app);
}

fn arm_read_timer(app: &App) {
    unsafe {
        if app.cal.source.is_some() {
            SetTimer(Some(app.hwnd), TIMER_CAL_READ, READ_EVERY_MS, None);
        } else {
            let _ = KillTimer(Some(app.hwnd), TIMER_CAL_READ);
        }
    }
}

/// At launch: the cached day, the pill timer, the 10 min reads, and a first read now.
pub fn start(app: &mut App) {
    rebuild(app);
    schedule_cues(app);
    arm_read_timer(app);
    read_now(app);
}

fn set_source(app: &mut App, raw: Option<String>) {
    let source = raw.as_deref().and_then(Source::parse);
    settings::update_map(&app.cal.dir, |s| {
        s.remove(READ_AT_KEY);
        match &raw {
            Some(r) => s.insert(LINK_KEY.into(), Value::from(r.trim())),
            None => s.remove(LINK_KEY),
        };
    });
    let _ = std::fs::remove_file(app.cal.dir.join(CACHE));
    app.cal.sync = if source.is_some() {
        Sync::Loading
    } else {
        Sync::NotConnected
    };
    app.cal.source = source;
    app.cal.calendar = None;
    app.cal.rx = None;
    rebuild(app);
    schedule_cues(app);
    arm_read_timer(app);
    read_now(app);
    panels::refresh(app);
}

/// Tray "Add calendar link" and the panel's Paste: the link on the clipboard becomes the calendar.
pub fn add_link_from_clipboard(app: &mut App) {
    let text = tasks::read_clipboard(app.hwnd).unwrap_or_default();
    match Source::parse(&text) {
        Some(src) => {
            let label = src.label();
            log_line(app, &format!("calendar: link set to {label}"));
            set_source(app, Some(text.trim().to_string()));
            if !(app.panel_open && app.panel_kind == Kind::Today) {
                notices::reply(
                    app,
                    info_pill(
                        Icon::Today,
                        "Calendar link saved",
                        &format!("reading {label}"),
                    ),
                );
            }
        }
        None => {
            log_line(app, "calendar: the clipboard holds no calendar link");
            notices::reply(
                app,
                info_pill(Icon::Link, "Calendar link", "copy an ICS link first"),
            );
        }
    }
}

pub fn remove_link(app: &mut App) {
    if app.cal.source.is_some() {
        log_line(app, "calendar: link removed");
        set_source(app, None);
    }
}

// ---- event pills ----

fn rebuild(app: &mut App) {
    let n = now();
    app.cal.occ = match &app.cal.calendar {
        Some(c) => cal::occurrences(
            c,
            n.day_start.add_seconds(-86_400),
            n.day_start.add_seconds(3 * 86_400),
            n.offset,
        ),
        None => Vec::new(),
    };
    app.cal.occ_day = Some(n.today);
}

/// One exact timer: the next event pill, or midnight (a new day's occurrences).
pub fn schedule_cues(app: &mut App) {
    unsafe {
        let _ = KillTimer(Some(app.hwnd), TIMER_CAL_CUE);
    }
    if app.cal.source.is_none() {
        return;
    }
    let n = now();
    let midnight = n.day_start.add_seconds(86_400);
    let next = tm::next_cue(&app.cal.occ, app.cal.fired_until);
    let target = next.map_or(midnight, |t| t.min(midnight));
    let ms = (target.unix_seconds() * 1000 - n.ms as i64 + 30).clamp(50, u32::MAX as i64) as u32;
    unsafe {
        SetTimer(Some(app.hwnd), TIMER_CAL_CUE, ms, None);
    }
    let what = next.map_or("none".to_string(), |t| tm::hhmm(t, n.offset));
    log_line(
        app,
        &format!("calendar: timer in {ms} ms (next event pill {what}, midnight at the latest)"),
    );
}

pub fn on_cue_timer(app: &mut App) {
    unsafe {
        let _ = KillTimer(Some(app.hwnd), TIMER_CAL_CUE);
    }
    let n = now();
    if app.cal.occ_day != Some(n.today) {
        rebuild(app);
        panels::refresh(app);
    }
    let due = tm::cues_between(&app.cal.occ, app.cal.fired_until, n.at);
    app.cal.fired_until = n.at;
    for cue in [Cue::Start, Cue::Soon] {
        let events: Vec<Occurrence> = due
            .iter()
            .filter(|(_, c)| *c == cue)
            .map(|(i, _)| app.cal.occ[*i].clone())
            .collect();
        if !events.is_empty() {
            show_cue(app, &events, cue);
        }
    }
    panels::refresh(app);
    schedule_cues(app);
}

fn cue_pill(events: &[Occurrence], cue: Cue, at: Instant) -> Pill {
    let first = &events[0];
    let title = match events.len() {
        1 => first.title.clone(),
        n => format!("{} and {} more", first.title, n - 1),
    };
    let link = events.iter().find_map(|o| o.link.clone());
    let (icon, value, ttl_ms) = match cue {
        Cue::Soon => (
            Icon::Today,
            tm::in_words(first.start.unix_seconds() - at.unix_seconds()),
            6000,
        ),
        Cue::Start => (
            if link.is_some() {
                Icon::Video
            } else {
                Icon::Today
            },
            "starting now".to_string(),
            10_000,
        ),
    };
    Pill {
        icon,
        dot: false,
        title,
        value,
        action: link.map(|l| ("Join".into(), PillAction::Join(l.url))),
        closable: true,
        ttl_ms,
    }
}

fn show_cue(app: &mut App, events: &[Occurrence], cue: Cue) {
    let p = cue_pill(events, cue, now().at);
    log_line(app, &format!("calendar: {} {}", p.title, p.value));
    match cue {
        // It waits for a focus break (or on the badge while the orb is hidden), and goes stale
        // once the event has started.
        Cue::Soon => {
            let start = store::local(events[0].start.unix_seconds());
            notices::post(
                app,
                notices::Notice::new(notices::Source::Calendar, Priority::Normal, p)
                    .stale_at(start),
            );
        }
        // Urgent: it shows even while the orb is hidden, then the orb hides again.
        Cue::Start => notices::post(app, notices::urgent(notices::Source::Calendar, p)),
    }
}

/// Tray test items: the pill for the next event today (or the running one, for "starting now").
pub fn test_cue(app: &mut App, cue: Cue) {
    let n = now();
    let next = tm::on_day(&app.cal.occ, n.day_start)
        .into_iter()
        .filter(|o| !o.all_day && o.end > n.at)
        .min_by_key(|o| o.start);
    match next {
        Some(o) => show_cue(app, &[o], cue),
        None => notices::reply(
            app,
            info_pill(Icon::Today, "Calendar", "no event ahead today to test with"),
        ),
    }
}

/// Join on a pill or a row: the call link opens in the browser.
pub fn join(app: &mut App, url: &str) {
    if !(url.starts_with("https://") || url.starts_with("http://")) {
        return;
    }
    log_line(app, "calendar: join");
    unsafe {
        ShellExecuteW(
            None,
            w!("open"),
            &HSTRING::from(url),
            None,
            None,
            SW_SHOWNORMAL,
        );
    }
}

// ---- the panel ----

pub fn open(app: &mut App) {
    app.cal.scroll = 0.0;
    app.cal.focus = None;
    app.cal.earlier_open = false;
    if app.cal.occ_day != Some(now().today) {
        rebuild(app);
    }
    panels::open_panel(app, Kind::Today);
}

fn task_due(t: &Task, n: &Now) -> Instant {
    match t.due {
        Some(d)
            if d.date.year() == n.today.year()
                && d.date.month() == n.today.month()
                && d.date.day() == n.today.day() =>
        {
            d.time.map_or(n.day_start, |x| {
                n.day_start
                    .add_seconds(i64::from(x.hour()) * 3600 + i64::from(x.minute()) * 60)
            })
        }
        // Overdue from an earlier day, or no date: first in the day.
        _ => n.day_start,
    }
}

fn card(sync: &Sync, reading: bool, offset: i32) -> Option<Card> {
    let from = |t: i64| {
        format!(
            "Showing events from {}.",
            tm::hhmm(Instant::from_unix_seconds(t), offset)
        )
    };
    match sync {
        Sync::NotConnected => Some(Card {
            icon: Icon::Today,
            title: "No calendar connected".into(),
            body: vec![
                "Copy an ICS link from Outlook, Google or any app,".into(),
                "then paste it here to see meetings beside tasks.".into(),
            ],
            buttons: vec![
                (Some(Hit::Paste), "Paste ICS link".into()),
                (None, "Google · coming soon".into()),
            ],
        }),
        Sync::Loading => Some(Card {
            icon: Icon::Clock,
            title: "Loading your day…".into(),
            body: vec![
                "Prayer times are worked out on this PC.".into(),
                "Events fill in when the calendar answers.".into(),
            ],
            buttons: Vec::new(),
        }),
        Sync::Ready { .. } => None,
        Sync::Failed {
            why: Fail::Offline,
            last_ok,
            ..
        } => Some(Card {
            icon: Icon::CloudOff,
            title: "Offline".into(),
            body: vec![
                last_ok.map_or("Couldn't reach the calendar.".into(), from),
                "Tasks and prayer times work as usual.".into(),
            ],
            // While it reads again the button stays, quiet, as "Retrying…".
            buttons: if reading {
                vec![(None, "Retrying…".into())]
            } else {
                vec![(Some(Hit::Retry), "Try again".into())]
            },
        }),
        Sync::Failed {
            why: Fail::Broken,
            detail,
            last_ok,
        } => Some(Card {
            icon: Icon::Info,
            title: "Couldn't read the calendar link".into(),
            body: vec![
                format!("{detail}."),
                last_ok.map_or("Paste a new link, or try again later.".into(), from),
            ],
            buttons: if reading {
                vec![(None, "Retrying…".into())]
            } else {
                vec![
                    (Some(Hit::Retry), "Try again".into()),
                    (Some(Hit::Paste), "Paste new link".into()),
                ]
            },
        }),
    }
}

fn build(app: &App) -> (View, Vec<Target>) {
    let n = now();
    let c = tasks::clock();
    let calname = app.cal.name();
    let day_occ = tm::on_day(&app.cal.occ, n.day_start);
    let prayers: Vec<PrayerTime> = tasks::prayers_today(app, &c)
        .into_iter()
        .map(|(name, minute)| PrayerTime {
            name,
            at: n.day_start.add_seconds(i64::from(minute) * 60),
        })
        .collect();
    let (todo, done) = m::view_tasks(&app.tasks.snap, &Selection::Today, &app.tasks.inbox, &c);
    let by_id: HashMap<String, &Task> = todo
        .iter()
        .chain(&done)
        .map(|t| (t.id.0.clone(), t))
        .collect();
    let due: Vec<TaskDue> = todo
        .iter()
        .map(|t| TaskDue {
            id: t.id.0.clone(),
            title: t.title.clone(),
            due: task_due(t, &n),
            done: false,
        })
        .chain(done.iter().map(|t| TaskDue {
            id: t.id.0.clone(),
            title: t.title.clone(),
            due: Instant::from_unix_seconds((t.completed_at.unwrap_or(0) / 1000) as i64),
            done: true,
        }))
        .collect();
    let day = tm::split_day(cal::today_timeline(day_occ, due, prayers), n.at);

    let mut lines = Vec::new();
    let mut targets = Vec::new();
    let push = |item: &Item, lines: &mut Vec<Line>, targets: &mut Vec<Target>| match item {
        Item::Event(o) => {
            let mut sub = vec![calname.clone()];
            if let Some(l) = &o.link {
                sub.push(l.kind.label().to_string());
            } else if let Some(loc) = o.location.as_ref().filter(|l| !l.trim().is_empty()) {
                sub.push(loc.clone());
            }
            let join = tm::join_open(o, n.at);
            lines.push(Line::Event {
                title: o.title.clone(),
                sub: sub.join(" · "),
                time: tm::hhmm(o.start, n.offset),
                join,
                past: o.end <= n.at,
            });
            targets.push(Target::Event(
                o.link.as_ref().filter(|_| join).map(|l| l.url.clone()),
            ));
        }
        Item::Task(td) => {
            let Some(t) = by_id.get(&td.id) else { return };
            let (time, overdue) = if td.done {
                (format!("Done {}", tm::hhmm(td.due, n.offset)), false)
            } else if m::is_overdue(t, &c) {
                (m::due_label(t, &c).0, true)
            } else if t.due.and_then(|d| d.time).is_some() {
                (tm::hhmm(td.due, n.offset), false)
            } else {
                ("Today".to_string(), false)
            };
            lines.push(Line::Task {
                title: t.title.clone(),
                sub: m::list_name(&app.tasks.snap, &t.list_id),
                time,
                done: td.done,
                overdue,
            });
            targets.push(Target::Task(t.id.clone()));
        }
        Item::PrayerMarker(p) => {
            lines.push(Line::Marker {
                name: p.name.clone(),
                time: tm::hhmm(p.at, n.offset),
            });
            targets.push(Target::None);
        }
    };
    if !day.earlier.is_empty() {
        lines.push(Line::Earlier {
            label: tm::earlier_label(&day.earlier),
            open: app.cal.earlier_open,
        });
        targets.push(Target::Earlier);
        if app.cal.earlier_open {
            for item in &day.earlier {
                push(item, &mut lines, &mut targets);
            }
        }
    }
    lines.push(Line::Now(format!("Now {}", tm::hhmm(n.at, n.offset))));
    targets.push(Target::None);
    let loading = app.cal.sync == Sync::Loading;
    if !loading
        && !day
            .ahead
            .iter()
            .any(|i| matches!(i, Item::Event(_) | Item::Task(_)))
    {
        lines.push(Line::Quiet("No events or tasks left today".into()));
        targets.push(Target::None);
    }
    for item in &day.ahead {
        push(item, &mut lines, &mut targets);
    }

    let reading = app.cal.rx.is_some();
    let footer = match (&app.cal.sync, reading) {
        (Sync::NotConnected, _) => String::new(),
        (_, true) => format!("Reading {calname}…"),
        (Sync::Ready { read_at }, _) => format!(
            "{calname} · read {}",
            tm::hhmm(Instant::from_unix_seconds(*read_at), n.offset)
        ),
        _ => String::new(),
    };
    let (np, nat, left) = prayer::next_prayer(app);
    let banner = (
        np.name().to_string(),
        tm::in_words((left * 60.0).round() as i64),
        pr::hhmm(nat),
    );
    let all_day = day
        .all_day
        .iter()
        .map(|o| (o.title.clone(), calname.clone()))
        .collect();
    let v = View {
        date: tm::day_title(n.today),
        all_day,
        banner,
        card: card(&app.cal.sync, reading, n.offset),
        lines,
        footer,
        scroll: app.cal.scroll,
        focus: app.cal.focus,
    };
    (v, targets)
}

fn view(app: &App) -> View {
    let mut v = build(app).0;
    v.scroll = v.scroll.clamp(0.0, tp::max_scroll(&v));
    v
}

pub fn height(app: &App) -> f32 {
    tp::height(&view(app))
}

pub fn draw_panel(app: &App, dc: &ID2D1DeviceContext, hover: Option<Hit>) {
    tp::draw(
        &mut crate::gfx::D2dCanvas::new(&app.g, dc),
        &app.theme,
        &app.fonts,
        &view(app),
        hover,
    );
}

pub fn hit(app: &App, x: f32, y: f32) -> Option<Hit> {
    tp::hit(&app.fonts, &view(app), x, y)
}

fn act(app: &mut App, target: Target, join_only: bool) {
    match target {
        Target::Earlier if !join_only => {
            app.cal.earlier_open = !app.cal.earlier_open;
            app.cal.focus = None;
            panels::refresh(app);
        }
        Target::Task(id) if !join_only => {
            tasks::toggle_done(app, &id);
            // Keyboard focus follows the task; folded into Earlier, it stays at the same place.
            if let Some(old) = app.cal.focus {
                let (v, targets) = build(app);
                let same = targets
                    .iter()
                    .position(|t| matches!(t, Target::Task(x) if *x == id));
                let near = || {
                    (old..v.lines.len())
                        .chain((0..old).rev())
                        .find(|i| v.lines[*i].is_stop())
                };
                app.cal.focus = same.or_else(near);
                panels::refresh(app);
            }
        }
        Target::Event(Some(url)) => join(app, &url),
        _ => {}
    }
}

pub fn click(app: &mut App, h: Hit) {
    match h {
        Hit::Earlier => act(app, Target::Earlier, false),
        Hit::Check(i) | Hit::Join(i) => {
            if let Some(target) = build(app).1.into_iter().nth(i) {
                act(app, target, matches!(h, Hit::Join(_)));
            }
        }
        Hit::Row(_) => {}
        Hit::AddTask => tasks::add_from_today(app),
        Hit::Paste => add_link_from_clipboard(app),
        Hit::Retry => read_now(app),
    }
}

/// Up and Down move between events, tasks and the Earlier row; markers are not stops.
pub fn on_arrow(app: &mut App, up: bool) {
    let v = view(app);
    let stops: Vec<usize> = v
        .lines
        .iter()
        .enumerate()
        .filter(|(_, l)| l.is_stop())
        .map(|(i, _)| i)
        .collect();
    let Some(&first) = stops.first() else { return };
    let next = match app
        .cal
        .focus
        .and_then(|f| stops.iter().position(|s| *s == f))
    {
        None => {
            if up {
                *stops.last().unwrap_or(&first)
            } else {
                first
            }
        }
        Some(k) if up => stops[k.saturating_sub(1)],
        Some(k) => stops[(k + 1).min(stops.len() - 1)],
    };
    app.cal.focus = Some(next);
    app.cal.scroll = tp::reveal(
        &View {
            focus: Some(next),
            ..v
        },
        next,
    );
    panels::refresh(app);
}

/// Enter: ticks the focused task, opens or folds Earlier, or joins the focused call.
pub fn on_enter(app: &mut App) {
    let Some(i) = app.cal.focus else { return };
    if let Some(target) = build(app).1.into_iter().nth(i) {
        act(app, target, false);
    }
}

pub fn on_wheel(app: &mut App, delta: i16) {
    let v = view(app);
    let s = (v.scroll - delta as f32 / 120.0 * 48.0).clamp(0.0, tp::max_scroll(&v));
    if s != app.cal.scroll {
        app.cal.scroll = s;
        panels::refresh(app);
    }
}

/// Ms to just after the next minute: the now line, Join and "in 1 h 6 min" change then.
pub fn ms_to_next_minute() -> u32 {
    let ms = now().ms;
    (60_000 - ms % 60_000 + 50) as u32
}
