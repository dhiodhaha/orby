//! Focus and drink on the orb, on the app_habits engines: their exact timers, pills, the two
//! panels' data and persistence. Notices held during focus wait in the notice queue
//! (notices.rs). Prayer only pauses focus here; it is never counted, logged or tracked.

use std::path::PathBuf;
use std::time::Duration;

use windows::Win32::Foundation::HWND;
use windows::Win32::Graphics::Direct2D::ID2D1DeviceContext;
use windows::Win32::System::SystemInformation::{GetLocalTime, GetTickCount};
use windows::Win32::UI::Input::KeyboardAndMouse::{GetLastInputInfo, LASTINPUTINFO};
use windows::Win32::UI::Input::{
    RegisterRawInputDevices, RAWINPUTDEVICE, RIDEV_INPUTSINK, RIDEV_REMOVE,
};
use windows::Win32::UI::WindowsAndMessaging::{KillTimer, SetTimer};

use app_habits::drink::{
    Drink, DrinkConfig, PrayerTimes, Presence, Reminder, ReminderContext, ReminderKind, Source,
};
use app_habits::focus::{
    BreakKind, FocusConfig, FocusEvent, FocusTimer, Outcome, PauseReason, Phase, PhaseKind,
};
use app_habits::{mins, Date, Moment};

use crate::notices;
use crate::pill::{dismiss_pill, info_pill};
use crate::store::{self, FocusPrefs, Saved, SavedSession};
use crate::ui::{DialChild, Icon, Pill, PillAction};
use crate::{drink_panel, focus_panel, log_line, orb, panels, App, TIMER_DRINK, TIMER_FOCUS};

pub struct Habits {
    pub focus: FocusTimer,
    pub prefs: FocusPrefs,
    pub drink: Drink,
    /// What `Drink::next_reminder` last said; `at_break` ones ride on the break pill.
    pub drink_due: Option<Reminder>,
    pub locked: bool,
    /// Idle past the threshold: raw input is watched until the first key or mouse move.
    pub away: bool,
    pub sessions: Vec<SavedSession>,
    path: PathBuf,
}

fn config_from(p: &FocusPrefs) -> FocusConfig {
    FocusConfig {
        focus: mins(p.length_min as u64),
        short_break: mins(p.short_break_min as u64),
        long_break: mins(p.long_break_min as u64),
        sessions_per_long_break: 4,
        auto_start_next: p.auto_start,
    }
}

/// Break presets the setup's Change button steps through (short, long).
const BREAKS: [(u32, u32); 3] = [(5, 15), (10, 20), (15, 30)];

impl Habits {
    pub fn load(path: PathBuf) -> Habits {
        let saved = store::load(&path);
        let mut drink = Drink::new(DrinkConfig::default());
        let mut entries = saved.drinks.clone();
        entries.sort_by_key(|d| d.at);
        for d in entries {
            let _ = drink.log(d.ml, store::local(d.at), d.source());
        }
        drink.start_watching(now());
        Habits {
            focus: FocusTimer::new(config_from(&saved.focus)),
            prefs: saved.focus,
            drink,
            drink_due: None,
            locked: false,
            away: false,
            sessions: saved.sessions,
            path,
        }
    }

    fn saved(&self) -> Saved {
        Saved {
            drinks: self
                .drink
                .entries()
                .iter()
                .map(|e| store::SavedDrink {
                    ml: e.ml,
                    at: store::unix(e.at),
                    device: match &e.source {
                        Source::Local => None,
                        Source::Device(d) => Some(d.clone()),
                    },
                })
                .collect(),
            sessions: self.sessions.clone(),
            focus: self.prefs.clone(),
            ..Saved::default()
        }
    }
}

fn save(app: &mut App) {
    if let Err(e) = store::save(&app.habits.path, &app.habits.saved()) {
        log_line(app, &format!("save failed: {e}"));
    }
}

/// Local wall-clock now, to the second.
pub fn now() -> Moment {
    let st = unsafe { GetLocalTime() };
    Date::new(st.wYear as i32, st.wMonth as u32, st.wDay as u32)
        .map(|d| d.at(st.wHour as u32, st.wMinute as u32) + Duration::from_secs(st.wSecond as u64))
        .unwrap_or(Moment::from_unix(0, 0))
}

fn ms_into_second() -> u64 {
    unsafe { GetLocalTime() }.wMilliseconds as u64
}

/// Milliseconds from now until `at` (whole seconds), landing just after that second begins.
pub fn ms_until(now: Moment, at: Moment) -> u32 {
    let ms = (at.since(now).as_secs() * 1000).saturating_sub(ms_into_second()) + 30;
    ms.clamp(50, u32::MAX as u64) as u32
}

pub fn hhmm(m: Moment) -> String {
    let t = m.minute_of_day();
    format!("{:02}:{:02}", t / 60, t % 60)
}

fn mmss(d: Duration) -> String {
    let s = d.as_secs();
    format!("{}:{:02}", s / 60, s % 60)
}

fn weekday(d: Date) -> &'static str {
    let epoch = Moment::from_unix(0, 0);
    let days = d.at_minute(0).since(epoch).as_secs() / 86_400;
    ["Thu", "Fri", "Sat", "Sun", "Mon", "Tue", "Wed"][(days % 7) as usize]
}

fn days_before(d: Date, n: u32) -> Date {
    (0..n).fold(d, |d, _| d.prev())
}

fn refresh(app: &mut App) {
    orb::render_orb(app);
    panels::refresh(app);
}

// ---- notices held during focus (the queue in notices.rs holds them) ----

/// The summary after focus: Show opens the Notices panel, where they wait.
fn waited_pill(w: usize) -> Pill {
    Pill {
        icon: Icon::Bell,
        dot: false,
        title: "Notices".into(),
        value: waited_label(w),
        action: Some(("Show".into(), PillAction::ShowWaited)),
        closable: true,
        ttl_ms: 8000,
    }
}

fn waited_label(n: usize) -> String {
    if n == 1 {
        "1 waited".into()
    } else {
        format!("{n} waited")
    }
}

// ---- focus ----

pub fn focus_running(app: &App) -> bool {
    app.habits.focus.phase() != &Phase::Idle
}

/// Progress for the orb's ring, while a session or break runs.
pub fn ring(app: &App) -> Option<f32> {
    focus_running(app).then(|| app.habits.focus.progress(now()))
}

pub fn dial_focus_value(app: &App) -> String {
    let n = now();
    let left = app.habits.focus.remaining(n).unwrap_or_default();
    let m = left.as_secs().div_ceil(60);
    match app.habits.focus.phase() {
        Phase::Idle => String::new(),
        Phase::Focus { .. } => format!("{m} min left"),
        Phase::Paused { .. } => "paused".into(),
        Phase::Break { .. } => format!("break {m} min"),
    }
}

/// Wakes once per degree of the ring's sweep (so the ring creeps without a per-second loop) and
/// exactly when the phase ends.
pub fn arm_focus(app: &mut App) {
    unsafe {
        let _ = KillTimer(Some(app.hwnd), TIMER_FOCUS);
    }
    let n = now();
    let f = &app.habits.focus;
    let (end, length) = match f.phase() {
        Phase::Focus {
            started_at, length, ..
        }
        | Phase::Break {
            started_at, length, ..
        } => (*started_at + *length, *length),
        _ => return,
    };
    let step = (length.as_millis() / 360).clamp(1000, 10_000) as u32;
    let ms = ms_until(n, end).min(step);
    unsafe {
        SetTimer(Some(app.hwnd), TIMER_FOCUS, ms, None);
    }
}

pub fn on_focus_timer(app: &mut App, _hwnd: HWND) {
    let out = app.habits.focus.tick(now());
    apply(app, out);
}

/// Folds a focus outcome into the app: logs sessions, releases held notices when focus ends,
/// shows the phase pills, then re-arms the timers and redraws.
fn apply(app: &mut App, out: Outcome) {
    let n = now();
    let mut done_minutes = None;
    let mut break_done = false;
    let mut logged = false;
    for ev in &out.events {
        match ev {
            FocusEvent::FocusDone { minutes, .. } => done_minutes = Some(*minutes),
            FocusEvent::BreakDone { .. } => break_done = true,
            FocusEvent::SessionLogged(rec) => {
                app.habits.sessions.push(SavedSession {
                    ended: store::unix(rec.ended_at),
                    minutes: rec.minutes,
                    task: rec.task.as_ref().map(|t| t.0.clone()),
                });
                logged = true;
            }
        }
    }
    for t in &out.transitions {
        log_line(app, &format!("focus {:?} -> {:?}", t.from, t.to));
    }
    let left_focus = out
        .transitions
        .iter()
        .find(|t| {
            matches!(t.from, PhaseKind::Focus | PhaseKind::Paused)
                && !matches!(t.to, PhaseKind::Focus | PhaseKind::Paused)
        })
        .map(|t| t.to);
    // The queue learns the focus state; leaving focus says how many waited through it.
    let waited = notices::sync_quiet(app)
        .filter(|r| r.focus)
        .map_or(0, |r| r.waited);
    if logged {
        save(app);
    }

    match (left_focus, app.habits.focus.phase().clone()) {
        (Some(PhaseKind::Break(_)), Phase::Break { kind, length, .. }) => {
            break_pill(app, kind, length, done_minutes, n, waited)
        }
        (Some(PhaseKind::Idle), Phase::Idle) if waited > 0 => {
            notices::post(app, notices::focus(waited_pill(waited)))
        }
        // A prayer pause holds nothing, so what waited so far comes after "Focus paused".
        (
            None,
            Phase::Paused {
                reason: PauseReason::Prayer(_),
                ..
            },
        ) if waited > 0 => notices::post(app, notices::focus(waited_pill(waited))),
        _ => {}
    }
    if break_done {
        match app.habits.focus.phase().clone() {
            Phase::Focus {
                session, length, ..
            } => {
                let p = Pill {
                    icon: Icon::Focus,
                    dot: false,
                    title: "Focus".into(),
                    value: format!(
                        "session {} of {} · {} min",
                        session.n,
                        session.of,
                        length.as_secs() / 60
                    ),
                    action: Some(("Undo".into(), PillAction::UndoFocus)),
                    closable: true,
                    ttl_ms: 6000,
                };
                notices::post(app, notices::focus(p));
            }
            Phase::Idle => {
                let m = app.habits.prefs.length_min;
                let p = Pill {
                    icon: Icon::Focus,
                    dot: false,
                    title: "Break done".into(),
                    value: String::new(),
                    action: Some((format!("Start {m} min"), PillAction::StartFocus)),
                    closable: true,
                    ttl_ms: 8000,
                };
                notices::post(app, notices::focus(p));
            }
            _ => {}
        }
    }
    // Reminders that waited unlisted come now as pills.
    notices::pump(app);

    arm_focus(app);
    if !out.transitions.is_empty() {
        schedule_drink(app);
    }
    refresh(app);
}

/// The one pill at the break: a drink reminder that waited, else the waited notices, else done.
fn break_pill(
    app: &mut App,
    kind: BreakKind,
    length: Duration,
    done_minutes: Option<u32>,
    n: Moment,
    waited: usize,
) {
    let title = format!(
        "{} {} min",
        if kind == BreakKind::Long {
            "Long break"
        } else {
            "Break"
        },
        length.as_secs() / 60
    );
    let summary = Pill {
        icon: Icon::Bell,
        dot: false,
        title: title.clone(),
        value: waited_label(waited),
        action: Some(("Show".into(), PillAction::ShowWaited)),
        closable: true,
        ttl_ms: 8000,
    };
    if app.habits.drink_due.is_some_and(|r| r.at_break) {
        app.habits.drink.reminder_shown(n);
        app.habits.drink_due = None;
        let ml = app.habits.drink.config().default_glass_ml;
        let p = Pill {
            icon: Icon::Drink,
            dot: false,
            title,
            value: "minum dulu?".into(),
            action: Some((format!("+{ml} ml"), PillAction::AddDrink(ml))),
            closable: true,
            ttl_ms: 8000,
        };
        notices::post(app, notices::focus(p));
        if waited > 0 {
            notices::post(app, notices::focus(summary));
        }
    } else if waited > 0 {
        notices::post(app, notices::focus(summary));
    } else {
        let value = match done_minutes {
            Some(m) => format!("{m} min, then {title}"),
            None => title,
        };
        let p = Pill {
            icon: Icon::Focus,
            dot: false,
            title: "Focus done".into(),
            value,
            action: None,
            closable: true,
            ttl_ms: 6000,
        };
        notices::post(app, notices::focus(p));
    }
}

pub fn start_focus(app: &mut App, minutes: u32) {
    match app
        .habits
        .focus
        .start(now(), Some(mins(minutes as u64)), None)
    {
        Ok(out) => {
            log_line(app, &format!("focus start {minutes} min"));
            apply(app, out);
        }
        Err(e) => log_line(app, &format!("focus start refused: {e:?}")),
    }
}

pub fn pause(app: &mut App) {
    if let Ok(out) = app.habits.focus.pause(now()) {
        apply(app, out);
    }
}

pub fn resume(app: &mut App) {
    if let Ok(out) = app.habits.focus.resume(now()) {
        apply(app, out);
    }
}

pub fn skip_to_break(app: &mut App) {
    if let Ok(out) = app.habits.focus.skip_to_break(now()) {
        apply(app, out);
    }
}

/// Stop never logs the current session (the engine's rule); Undo on a fresh start uses it too.
pub fn stop(app: &mut App) {
    if let Ok(out) = app.habits.focus.stop(now()) {
        apply(app, out);
    }
}

/// "Memasuki waktu": reminders ahead of this prayer are stale now, wherever they wait. A running
/// session pauses for the prayer (which holds nothing), and a pill offers Resume after the
/// prayer pill has had its turn.
pub fn on_prayer_time(app: &mut App, prayer: &str) {
    let n = now();
    notices::drop_prayer(app, prayer);
    panels::refresh(app);
    let out = app.habits.focus.pause_for_prayer(n, prayer);
    if out.transitions.is_empty() {
        return;
    }
    let left = app.habits.focus.remaining(n).unwrap_or_default();
    let p = Pill {
        icon: Icon::Focus,
        dot: false,
        title: format!("Focus paused for {prayer}"),
        value: format!("{} left", mmss(left)),
        action: Some(("Resume".into(), PillAction::ResumeFocus)),
        closable: true,
        ttl_ms: 8000,
    };
    // It names the prayer, so it never stays in the list; it waits for the prayer pill.
    notices::post(app, notices::focus(p));
    apply(app, out);
}

/// Dial branch children for Focus: 15/25/50 when idle or on a break, else Pause, Skip, Stop.
pub fn focus_children(app: &App) -> (Vec<DialChild>, usize) {
    let left = app.habits.focus.remaining(now()).unwrap_or_default();
    let child = |icon: Icon, name: &str, value: String| DialChild {
        text: String::new(),
        icon: Some(icon),
        name: name.into(),
        value,
    };
    match app.habits.focus.phase() {
        Phase::Focus { .. } => (
            vec![
                child(Icon::Pause, "Pause", format!("{} left", mmss(left))),
                child(Icon::Skip, "Skip to break", String::new()),
                child(Icon::Stop, "Stop", String::new()),
            ],
            0,
        ),
        Phase::Paused { .. } => (
            vec![
                child(Icon::Play, "Resume", format!("{} left", mmss(left))),
                child(Icon::Skip, "Skip to break", String::new()),
                child(Icon::Stop, "Stop", String::new()),
            ],
            0,
        ),
        _ => (
            focus_panel::PRESETS
                .iter()
                .map(|m| DialChild {
                    text: m.to_string(),
                    icon: None,
                    name: "Focus".into(),
                    value: format!("{m} min"),
                })
                .collect(),
            1,
        ),
    }
}

pub fn pick_focus_child(app: &mut App, j: usize) {
    match app.habits.focus.phase() {
        Phase::Focus { .. } | Phase::Paused { .. } => match j {
            0 if matches!(app.habits.focus.phase(), Phase::Focus { .. }) => pause(app),
            0 => resume(app),
            1 => skip_to_break(app),
            _ => stop(app),
        },
        _ => {
            let m = focus_panel::PRESETS[j.min(2)];
            start_focus(app, m);
            if focus_running(app) {
                let p = Pill {
                    icon: Icon::Focus,
                    dot: false,
                    title: "Focus".into(),
                    value: format!("{m} min"),
                    action: Some(("Undo".into(), PillAction::UndoFocus)),
                    closable: true,
                    ttl_ms: 6000,
                };
                notices::reply(app, p);
            }
        }
    }
}

pub fn undo_focus(app: &mut App) {
    stop(app);
    dismiss_pill(app);
}

pub fn focus_view(app: &App) -> focus_panel::View {
    use focus_panel::{Run, View};
    let n = now();
    let f = &app.habits.focus;
    let p = &app.habits.prefs;
    let left = f.remaining(n).unwrap_or_default().as_secs();
    let progress = f.progress(n);
    let done = f.sessions_done();
    let of = f.config().sessions_per_long_break;
    let held = notices::held_now(app);
    let task_name = |t: &Option<app_habits::focus::TaskId>| t.as_ref().map(|t| t.0.clone());
    match f.phase() {
        Phase::Idle => View::Setup {
            minutes: p.length_min,
            short_break: p.short_break_min,
            long_break: p.long_break_min,
            every: of,
            auto_start: p.auto_start,
            task: None,
        },
        Phase::Focus {
            session,
            length,
            task,
            ..
        } => View::Running {
            run: Run::Focus,
            left_secs: left,
            length_secs: length.as_secs(),
            progress,
            session: session.n,
            of,
            done,
            held,
            task: task_name(task),
        },
        Phase::Paused {
            session,
            length,
            task,
            reason,
            ..
        } => {
            let prayer = match reason {
                PauseReason::Prayer(name) => Some(name.clone()),
                PauseReason::User => None,
            };
            View::Running {
                run: Run::Paused { prayer },
                left_secs: left,
                length_secs: length.as_secs(),
                progress,
                session: session.n,
                of,
                done,
                held,
                task: task_name(task),
            }
        }
        Phase::Break { kind, length, .. } => View::Running {
            run: Run::Break {
                long: *kind == BreakKind::Long,
            },
            left_secs: left,
            length_secs: length.as_secs(),
            progress,
            session: done,
            of,
            done,
            held,
            task: None,
        },
    }
}

fn set_prefs(app: &mut App, prefs: FocusPrefs) {
    app.habits.prefs = prefs;
    // The config lives in the timer, so a new one takes it. Only offered while idle.
    if !focus_running(app) {
        app.habits.focus = FocusTimer::new(config_from(&app.habits.prefs));
    }
    save(app);
    panels::refresh(app);
}

pub fn focus_click(app: &mut App, hit: focus_panel::Hit) {
    use focus_panel::Hit;
    match hit {
        Hit::Knob => {}
        Hit::Preset(m) => {
            let p = FocusPrefs {
                length_min: m,
                ..app.habits.prefs.clone()
            };
            set_prefs(app, p);
        }
        Hit::Task => {
            panels::close_panel(app);
            let p = info_pill(Icon::Focus, "Focus on a task", "comes with the Tasks panel");
            notices::reply(app, p);
        }
        Hit::Breaks => {
            let cur = (
                app.habits.prefs.short_break_min,
                app.habits.prefs.long_break_min,
            );
            let i = BREAKS
                .iter()
                .position(|b| *b == cur)
                .map(|i| (i + 1) % BREAKS.len())
                .unwrap_or(0);
            let p = FocusPrefs {
                short_break_min: BREAKS[i].0,
                long_break_min: BREAKS[i].1,
                ..app.habits.prefs.clone()
            };
            set_prefs(app, p);
        }
        Hit::AutoStart => {
            let p = FocusPrefs {
                auto_start: !app.habits.prefs.auto_start,
                ..app.habits.prefs.clone()
            };
            set_prefs(app, p);
        }
        Hit::Start => {
            let m = app.habits.prefs.length_min;
            start_focus(app, m);
        }
        Hit::Primary => match app.habits.focus.phase() {
            Phase::Focus { .. } => pause(app),
            Phase::Paused { .. } => resume(app),
            _ => {
                let m = app.habits.prefs.length_min;
                start_focus(app, m);
            }
        },
        Hit::Skip => skip_to_break(app),
        Hit::Stop => stop(app),
    }
}

/// The knob, by drag, wheel or arrows. Saved when the drag ends (or at once for keys).
pub fn set_knob(app: &mut App, v: u32, persist: bool) {
    if focus_running(app) {
        return;
    }
    if v != app.habits.prefs.length_min {
        app.habits.prefs.length_min = v;
        app.habits.focus = FocusTimer::new(config_from(&app.habits.prefs));
        panels::refresh(app);
    }
    if persist {
        save(app);
    }
}

pub fn knob_key(app: &mut App, up: bool) {
    let v = focus_panel::knob_step(app.habits.prefs.length_min, up);
    set_knob(app, v, true);
}

// ---- drink ----

fn goal(app: &App) -> u32 {
    app.habits.drink.config().goal_ml
}

pub fn drink_total(app: &App) -> u32 {
    app.habits.drink.total_for(now().date())
}

pub fn dial_drink_value(app: &App) -> String {
    format!("{} / {}", drink_total(app), goal(app))
}

pub fn drink_pill(app: &App, title: &str, action: Option<(String, PillAction)>, ttl: u32) -> Pill {
    Pill {
        icon: Icon::Drink,
        dot: false,
        title: title.into(),
        value: format!("{} / {} ml", drink_total(app), goal(app)),
        action,
        closable: true,
        ttl_ms: ttl,
    }
}

fn default_glass(app: &App) -> u32 {
    app.habits.drink.config().default_glass_ml
}

/// The drink notice with its quick button (tray test item): normal priority, so held in focus.
pub fn drink_notice(app: &mut App) {
    let ml = default_glass(app);
    let p = drink_pill(
        app,
        "Minum dulu",
        Some((format!("+{ml} ml"), PillAction::AddDrink(ml))),
        6000,
    );
    notices::post(
        app,
        notices::Notice::new(notices::Source::Drink, notices::Priority::Normal, p)
            .replacing("remind"),
    );
}

/// Logs a drink now and returns the confirmation pill with Undo.
pub fn log_drink(app: &mut App, ml: u32) -> Pill {
    let _ = app.habits.drink.log(ml, now(), Source::Local);
    log_line(app, &format!("drink +{ml} ml"));
    save(app);
    schedule_drink(app);
    panels::refresh(app);
    let p = drink_pill(
        app,
        &format!("+{ml} ml"),
        Some(("Undo".into(), PillAction::UndoDrink)),
        6000,
    );
    Pill {
        value: format!("{} / {} ml", drink_total(app), goal(app)),
        ..p
    }
}

pub fn undo_drink(app: &mut App) {
    if let Some(e) = app.habits.drink.undo_last() {
        log_line(app, &format!("drink undo {} ml", e.ml));
        save(app);
        schedule_drink(app);
        panels::refresh(app);
    }
}

pub fn drink_children(app: &App) -> (Vec<DialChild>, usize) {
    let total = drink_total(app);
    let g = goal(app);
    let glasses = &app.habits.drink.config().glasses_ml;
    let def = glasses
        .iter()
        .position(|m| *m == default_glass(app))
        .unwrap_or(0);
    (
        glasses
            .iter()
            .map(|m| DialChild {
                text: m.to_string(),
                icon: None,
                name: format!("+{m} ml"),
                value: format!("to {} / {g}", total + m),
            })
            .collect(),
        def,
    )
}

pub fn pick_drink_child(app: &mut App, j: usize) {
    let glasses = app.habits.drink.config().glasses_ml.clone();
    if let Some(ml) = glasses.get(j) {
        let p = log_drink(app, *ml);
        notices::reply(app, p);
    }
}

fn prayer_times(app: &App) -> PrayerTimes {
    let t = &app.times;
    PrayerTimes {
        imsak: t.imsak,
        subuh: t.subuh,
        dzuhur: t.dzuhur,
        ashar: t.ashar,
        maghrib: t.maghrib,
        isya: t.isya,
    }
}

fn idle_ms() -> u32 {
    let mut lii = LASTINPUTINFO {
        cbSize: std::mem::size_of::<LASTINPUTINFO>() as u32,
        dwTime: 0,
    };
    unsafe {
        if !GetLastInputInfo(&mut lii).as_bool() {
            return 0;
        }
        GetTickCount().wrapping_sub(lii.dwTime)
    }
}

fn context(app: &App, n: Moment) -> ReminderContext {
    let presence = if app.habits.locked {
        Presence::Locked
    } else {
        match idle_ms() {
            ms if ms < 2000 => Presence::Active,
            ms => Presence::Idle {
                since: n - Duration::from_millis(ms as u64),
            },
        }
    };
    ReminderContext {
        presence,
        focus_until: app.habits.focus.break_at(n),
        meeting_recording: false,
        prayers: prayer_times(app),
        fasting: false,
    }
}

/// One exact timer to the next drink reminder. While away it watches raw input instead, and
/// after active hours it sleeps until tomorrow's first minute of them.
pub fn schedule_drink(app: &mut App) {
    unsafe {
        let _ = KillTimer(Some(app.hwnd), TIMER_DRINK);
    }
    let n = now();
    let ctx = context(app, n);
    let r = app.habits.drink.next_reminder(n, &ctx);
    app.habits.drink_due = r;
    let away = matches!(ctx.presence, Presence::Idle { since } if n.since(since) > app.habits.drink.config().idle_after);
    let wake = match r {
        Some(r) if r.at_break => {
            log_line(
                app,
                &format!("next drink reminder waits for the break at {}", hhmm(r.at)),
            );
            None
        }
        Some(r) => {
            log_line(
                app,
                &format!("next drink reminder {} ({:?})", hhmm(r.at), r.kind),
            );
            Some(r.at)
        }
        None if app.habits.locked => None,
        None if away => {
            watch_input(app, true);
            log_line(app, "drink reminders paused: away");
            None
        }
        None => {
            let start = app.habits.drink.config().active_hours.start_min;
            let day = if n.minute_of_day() < start {
                n.date()
            } else {
                n.date().next()
            };
            let at = day.at_minute(start) + mins(1);
            log_line(
                app,
                &format!("no drink reminder before {} {}", weekday(day), hhmm(at)),
            );
            Some(at)
        }
    };
    if let Some(at) = wake {
        unsafe {
            SetTimer(Some(app.hwnd), TIMER_DRINK, ms_until(n, at), None);
        }
    }
}

pub fn on_drink_timer(app: &mut App, _hwnd: HWND) {
    unsafe {
        let _ = KillTimer(Some(app.hwnd), TIMER_DRINK);
    }
    let n = now();
    let ctx = context(app, n);
    if let Some(r) = app.habits.drink.next_reminder(n, &ctx) {
        if r.at <= n && !r.at_break {
            fire_drink(app, r, n);
        }
    }
    schedule_drink(app);
}

fn fire_drink(app: &mut App, r: Reminder, n: Moment) {
    app.habits.drink.reminder_shown(n);
    // Drink never shows while you have hidden the orb; the clock restarts as if it had.
    if notices::hidden_by_you(app) {
        log_line(app, "drink reminder skipped: the orb is hidden");
        return;
    }
    let title = match r.kind {
        ReminderKind::AfterPrayer(_) => "Habis sholat, minum dulu",
        ReminderKind::Sahur { .. } => "Sahur, minum dulu",
        ReminderKind::Buka => "Buka, minum dulu",
        ReminderKind::Interval | ReminderKind::CatchUp => "Minum dulu",
    };
    let ml = default_glass(app);
    let p = drink_pill(
        app,
        title,
        Some((format!("+{ml} ml"), PillAction::AddDrink(ml))),
        6000,
    );
    log_line(app, &format!("drink reminder {:?}", r.kind));
    notices::post(
        app,
        notices::Notice::new(notices::Source::Drink, notices::Priority::Normal, p)
            .replacing("remind"),
    );
}

/// Tray test item: a drink reminder falls due now. During focus it waits for the break, as a
/// real one would (`next_reminder` moves it to `break_at`).
pub fn test_drink_due(app: &mut App) {
    let n = now();
    match app.habits.focus.break_at(n) {
        Some(at) => {
            unsafe {
                let _ = KillTimer(Some(app.hwnd), TIMER_DRINK);
            }
            app.habits.drink_due = Some(Reminder {
                at,
                kind: ReminderKind::Interval,
                quiet: false,
                at_break: true,
            });
            log_line(
                app,
                &format!("test drink reminder waits for the break at {}", hhmm(at)),
            );
            panels::refresh(app);
        }
        None => {
            fire_drink(
                app,
                Reminder {
                    at: n,
                    kind: ReminderKind::Interval,
                    quiet: false,
                    at_break: false,
                },
                n,
            );
            schedule_drink(app);
        }
    }
}

/// WM_WTSSESSION_CHANGE: a lock pauses and resets the reminder clock, an unlock restarts it.
pub fn on_session_change(app: &mut App, code: usize) {
    const WTS_SESSION_LOCK: usize = 7;
    const WTS_SESSION_UNLOCK: usize = 8;
    match code {
        WTS_SESSION_LOCK => app.habits.locked = true,
        WTS_SESSION_UNLOCK => app.habits.locked = false,
        _ => return,
    }
    log_line(
        app,
        if app.habits.locked {
            "session locked"
        } else {
            "session unlocked"
        },
    );
    schedule_drink(app);
    panels::refresh(app);
}

fn watch_input(app: &mut App, on: bool) {
    if app.habits.away == on {
        return;
    }
    app.habits.away = on;
    let devices = [(1u16, 2u16), (1, 6)].map(|(page, usage)| RAWINPUTDEVICE {
        usUsagePage: page,
        usUsage: usage,
        dwFlags: if on { RIDEV_INPUTSINK } else { RIDEV_REMOVE },
        hwndTarget: if on { app.hwnd } else { HWND::default() },
    });
    unsafe {
        let _ = RegisterRawInputDevices(&devices, std::mem::size_of::<RAWINPUTDEVICE>() as u32);
    }
}

/// WM_INPUT while away: the first key or mouse move means the user is back.
pub fn on_input(app: &mut App) {
    if app.habits.away {
        watch_input(app, false);
        log_line(app, "back from away");
        schedule_drink(app);
        panels::refresh(app);
    }
}

pub fn drink_view(app: &App) -> drink_panel::View {
    use drink_panel::{Bar, LogRow, View};
    let n = now();
    let today = n.date();
    let d = &app.habits.drink;
    let cfg = d.config();
    let total = d.total_for(today);
    let g = cfg.goal_ml;
    let to_go = if total >= g {
        "Goal met for today.".to_string()
    } else {
        format!("{} ml to go.", g - total)
    };
    let (start, end) = (
        today.at_minute(cfg.active_hours.start_min),
        today.at_minute(cfg.active_hours.end_min),
    );
    let target = d.pace_target(n);
    let pace = if total >= g {
        String::new()
    } else if n < start {
        format!("Your day starts at {}.", hhmm(start))
    } else if total >= target {
        let at = start
            + Duration::from_secs(
                (end.since(start).as_secs() as f64 * total as f64 / g as f64) as u64,
            );
        format!("Ahead of pace for {}.", hhmm(at))
    } else {
        format!("On pace would be {target} ml by now.")
    };
    let mut rows: Vec<_> = d
        .entries()
        .iter()
        .filter(|e| e.at.date() == today)
        .collect();
    rows.sort_by_key(|e| std::cmp::Reverse(e.at));
    let rows = rows
        .into_iter()
        .take(drink_panel::MAX_ROWS)
        .map(|e| LogRow {
            time: hhmm(e.at),
            ml: e.ml,
            source: match &e.source {
                Source::Local => "glass".into(),
                Source::Device(name) => format!("from your {name}"),
            },
        })
        .collect();
    let week = (0..7u32)
        .rev()
        .map(|k| {
            let day = days_before(today, k);
            Bar {
                label: if k == 0 {
                    "Today".into()
                } else {
                    weekday(day).into()
                },
                ml: d.total_for(day),
                today: k == 0,
            }
        })
        .collect();
    let every = cfg
        .interval_for(d.consumed_in(n, app_habits::drink::DayMode::Normal), target)
        .as_secs()
        / 60;
    let next = match app.habits.drink_due {
        Some(r) if r.at_break => "Next reminder at your focus break".to_string(),
        Some(r) => format!("Next reminder {} · every {every} min", hhmm(r.at)),
        None if app.habits.locked || app.habits.away => {
            "Reminders wait while you are away".to_string()
        }
        None => "No more reminders today".to_string(),
    };
    View {
        total,
        goal: g,
        to_go,
        pace,
        glasses: cfg.glasses_ml.clone(),
        default_glass: cfg.default_glass_ml,
        rows,
        week,
        next,
    }
}

pub fn drink_click(app: &mut App, hit: drink_panel::Hit) {
    match hit {
        drink_panel::Hit::Glass(ml) => {
            let _ = log_drink(app, ml);
        }
        drink_panel::Hit::Undo => undo_drink(app),
        drink_panel::Hit::Week => crate::history::open(app, Some(crate::history_model::Tab::Drink)),
    }
}

pub fn draw_focus_panel(app: &App, dc: &ID2D1DeviceContext, hover: Option<focus_panel::Hit>) {
    focus_panel::draw(&app.g, dc, &app.theme, &app.fonts, &focus_view(app), hover);
}

pub fn draw_drink_panel(app: &App, dc: &ID2D1DeviceContext, hover: Option<drink_panel::Hit>) {
    drink_panel::draw(&app.g, dc, &app.theme, &app.fonts, &drink_view(app), hover);
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn weekdays_follow_the_calendar() {
        assert_eq!(weekday(Date::new(2026, 11, 6).unwrap()), "Fri");
        assert_eq!(weekday(Date::new(1970, 1, 1).unwrap()), "Thu");
        assert_eq!(
            weekday(days_before(Date::new(2026, 11, 6).unwrap(), 6)),
            "Sat"
        );
    }

    #[test]
    fn prefs_become_the_timer_config() {
        let c = config_from(&FocusPrefs {
            length_min: 50,
            short_break_min: 10,
            long_break_min: 20,
            auto_start: true,
        });
        assert_eq!(c.focus, mins(50));
        assert_eq!(c.short_break, mins(10));
        assert_eq!(c.long_break, mins(20));
        assert!(c.auto_start_next);
    }

    #[test]
    fn saved_drinks_rebuild_the_log_in_time_order() {
        let dir =
            std::env::temp_dir().join(format!("orb-proto-habits-test-{}", std::process::id()));
        let _ = std::fs::create_dir_all(&dir);
        let path = dir.join("habits.json");
        let day = Date::new(2026, 11, 6).unwrap();
        let s = Saved {
            drinks: vec![
                store::SavedDrink {
                    ml: 500,
                    at: store::unix(day.at(12, 0)),
                    device: None,
                },
                store::SavedDrink {
                    ml: 250,
                    at: store::unix(day.at(9, 15)),
                    device: Some("phone".into()),
                },
            ],
            ..Saved::default()
        };
        store::save(&path, &s).unwrap();
        let mut h = Habits::load(path);
        assert_eq!(h.drink.total_for(day), 750);
        // The latest by time is the one Undo removes.
        assert_eq!(h.drink.undo_last().map(|e| e.ml), Some(500));
        assert_eq!(h.saved().drinks.len(), 1);
    }
}
