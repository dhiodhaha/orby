//! The notice queue on the orb: every pill goes through `post` (a notice from a source) or
//! `reply` (an answer to something you just did). notices_model.rs decides; this file finds the
//! quiet time (full screen on the orb's monitor, focus, a pause, the orb hidden), hides the orb
//! for a full-screen app, keeps one exact timer, saves the mutes, and runs the Notices panel
//! ("Earlier today", notices_panel.rs draws it).

use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use serde::{Deserialize, Serialize};
use windows::Win32::Foundation::*;
use windows::Win32::Graphics::Direct2D::ID2D1DeviceContext;
use windows::Win32::Graphics::Gdi::{
    GetMonitorInfoW, MonitorFromWindow, MONITORINFO, MONITOR_DEFAULTTONEAREST,
};
use windows::Win32::System::SystemInformation::GetTickCount64;
use windows::Win32::UI::Accessibility::{SetWinEventHook, HWINEVENTHOOK};
use windows::Win32::UI::Input::KeyboardAndMouse::*;
use windows::Win32::UI::Shell::{SHQueryUserNotificationState, QUNS_PRESENTATION_MODE};
use windows::Win32::UI::WindowsAndMessaging::*;

use app_habits::Moment;

use crate::notices_model::{self as m, Keep, Merge, Mute, MuteFor, Next, Now, Offered, Quiet};
pub use crate::notices_model::{Notice, Priority, Source};
use crate::notices_panel::{self as np, Hit, MenuItem, Row, View};
use crate::panels::{self, Kind};
use crate::ui::{DialChild, Icon, Pill, PillAction};
use crate::{habits, log_line, orb, pill, settings, App, TIMER_NOTICES, WM_APP_FOREGROUND};

pub struct Notices {
    pub q: m::Queue,
    /// Full screen or presenting on the orb's monitor, as of the last foreground change.
    pub full_screen: bool,
    /// Tray test item: act as if an app were full screen.
    pub test_full_screen: bool,
    /// The orb hid itself for a full-screen app and comes back when it ends.
    fs_hidden: bool,
    /// An urgent notice showed the hidden orb; it hides again once the pill has gone.
    rehide: bool,
    /// Pause notices until then.
    pub paused_until: Option<Moment>,
    /// The badge count last drawn, so the drop redraws only when it changes.
    badge: usize,
    /// The panel: the keyboard's row, the open row menu (entry id) and its keyboard item.
    focus_row: Option<usize>,
    menu: Option<u64>,
    menu_item: usize,
    /// "Downloads muted for good" with Undo, until the panel closes: (source, the mute it replaced).
    note: Option<(Source, Option<Mute>)>,
    /// The focus hover pill ("Focus 18 min left · 2 held") is up.
    hover_named: bool,
    path: PathBuf,
}

#[derive(Serialize, Deserialize, Default)]
struct Saved {
    #[serde(default)]
    mutes: Vec<(Source, Mute)>,
    #[serde(default)]
    cooldowns: Vec<(String, i64)>,
}

impl Notices {
    pub fn load(dir: &std::path::Path) -> Notices {
        let path = dir.join("notices.json");
        let mut q = m::Queue::new();
        if let Some(s) = std::fs::read_to_string(&path)
            .ok()
            .and_then(|t| serde_json::from_str::<Saved>(&t).ok())
        {
            q.mutes = s.mutes.into_iter().collect();
            q.cooldowns = s.cooldowns.into_iter().collect();
        }
        Notices {
            q,
            full_screen: false,
            test_full_screen: false,
            fs_hidden: false,
            rehide: false,
            paused_until: None,
            badge: 0,
            focus_row: None,
            menu: None,
            menu_item: 0,
            note: None,
            hover_named: false,
            path,
        }
    }

    fn save(&self) -> std::io::Result<()> {
        let s = Saved {
            mutes: self.q.mutes.iter().map(|(s, m)| (*s, *m)).collect(),
            cooldowns: self
                .q
                .cooldowns
                .iter()
                .map(|(k, v)| (k.clone(), *v))
                .collect(),
        };
        let tmp = self.path.with_extension("json.tmp");
        std::fs::write(&tmp, serde_json::to_vec_pretty(&s)?)?;
        std::fs::rename(&tmp, &self.path)
    }
}

fn save(app: &mut App) {
    if let Err(e) = app.notices.save() {
        log_line(app, &format!("notices: save failed: {e}"));
    }
    // A mute set from the Notices panel shows on an open Settings Notices section too.
    settings::on_sources_changed(app);
}

fn unix_now() -> i64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_or(0, |d| d.as_secs() as i64)
}

fn clock() -> Now {
    Now {
        at: habits::now(),
        ms: unsafe { GetTickCount64() },
    }
}

/// The quiet time right now.
pub fn quiet(app: &App) -> Quiet {
    Quiet {
        hidden: (!app.visible && !app.notices.fs_hidden) || app.notices.rehide,
        full_screen: app.notices.full_screen,
        focus: app.habits.focus.is_holding(),
        paused: app.notices.paused_until.is_some_and(|t| habits::now() < t),
    }
}

/// The orb is hidden because you hid it (not for a full-screen app): drink and prayer reminders
/// are skipped then, as before.
pub fn hidden_by_you(app: &App) -> bool {
    !app.visible && !app.notices.fs_hidden
}

// ---- posting ----

/// A notice from a source. It shows now, waits its turn, waits for a quiet time to end, merges,
/// or is muted.
pub fn post(app: &mut App, n: Notice) {
    let q = quiet(app);
    let busy = app.pill.is_some();
    let urgent = n.priority == Priority::Urgent;
    let (src, title, cools) = (n.source, n.pill.title.clone(), n.cooldown.is_some());
    let out = app.notices.q.offer(n, clock(), unix_now(), q, busy);
    match &out {
        Offered::Show(p) => display(app, p.clone(), urgent),
        Offered::Update(p) if busy && !app.pill_closing => pill::update_pill(app, p.clone()),
        Offered::Update(p) => display(app, p.clone(), false),
        _ => {}
    }
    if cools && matches!(out, Offered::Show(_)) {
        save(app);
    }
    let what = match out {
        Offered::Show(_) => "shows",
        Offered::Update(_) => "merged into the pill on screen",
        Offered::Queued => "waits its turn",
        Offered::Held => "held (quiet time)",
        Offered::Listed => "listed (quiet)",
        Offered::Muted => "muted",
        Offered::Dropped => "dropped (quiet time)",
    };
    log_line(app, &format!("notice {src:?} \"{title}\" {what}"));
    after(app);
}

/// A plain normal notice from a source (most callers).
pub fn notify(app: &mut App, source: Source, pill: Pill) {
    post(app, Notice::new(source, Priority::Normal, pill));
}

/// An answer to something you just did: shows at once over whatever is in front, never waits,
/// never listed.
pub fn reply(app: &mut App, p: Pill) {
    app.notices.q.reply(clock());
    app.notices.hover_named = false;
    pill::show_pill(app, p);
    after(app);
}

/// New words for a live pill already on screen (a hover pill counting), or a fresh reply.
pub fn live(app: &mut App, p: Pill) {
    if app.pill.is_some() && !app.pill_closing {
        pill::update_pill(app, p);
    } else {
        reply(app, p);
    }
}

fn display(app: &mut App, p: Pill, urgent: bool) {
    if urgent && !app.visible {
        // Urgent shows even while the orb is hidden, then the orb hides again.
        orb::set_visible(app, true);
        app.notices.rehide = true;
    }
    pill::show_pill(app, p);
    if urgent && app.notices.full_screen {
        log_line(
            app,
            "notices: urgent, silent (full screen): no sound, no shake",
        );
    } else if urgent {
        pill::shake(app);
    }
}

/// The pill's action was taken: it is done with, not listed.
pub fn acted(app: &mut App) {
    app.notices.q.close_front(true);
}

/// The pill starts to leave (timed out, closed, or covered by a panel or the dial).
pub fn on_dismiss(app: &mut App) {
    app.notices.q.close_front(false);
}

/// The pill has gone: the next one, or hide the orb again after an urgent notice.
pub fn on_pill_gone(app: &mut App) {
    pump(app);
}

/// The quiet time as it is now (a summary if one that summarizes just ended), then the next pill
/// if the slot is free.
pub fn pump(app: &mut App) {
    if let Some(r) = sync_quiet(app) {
        // Focus folds its count into the break pill (habits.rs), so only the others summarize.
        if r.waited > 0 && !r.focus {
            let (title, value) = m::summary_words(r.waited);
            let p = Pill {
                icon: Icon::Bell,
                dot: false,
                title,
                value,
                action: Some(("Show".into(), PillAction::ShowWaited)),
                closable: true,
                ttl_ms: 8000,
            };
            post(app, Notice::new(Source::App, Priority::Normal, p));
        }
    }
    // The open Notices panel lists what waits; its pills come after it closes.
    if app.pill.is_none() && !panel_shown(app) {
        let q = quiet(app);
        match app.notices.q.next(clock(), unix_now(), q) {
            Next::Show(p, prio) => {
                log_line(app, &format!("notice \"{}\" shows (its turn)", p.title));
                display(app, p, prio == Priority::Urgent);
                // A cooldown stamp may have moved.
                save(app);
            }
            Next::Wait(_) | Next::Idle if app.notices.rehide => {
                app.notices.rehide = false;
                orb::set_visible(app, false);
                app.notices.fs_hidden = app.notices.full_screen;
            }
            _ => {}
        }
    }
    after(app);
}

/// After any change: the badge, the panel, and one exact timer to the next change.
fn after(app: &mut App) {
    pill::restack(app);
    let n = badge_count(app);
    if n != app.notices.badge {
        app.notices.badge = n;
        orb::render_orb(app);
    }
    if panel_shown(app) {
        panels::refresh(app);
    }
    arm(app);
}

/// The orb's count: notices waiting behind the front pill. None while focus runs.
pub fn badge_count(app: &App) -> usize {
    if habits::focus_running(app) {
        0
    } else {
        app.notices.q.waiting()
    }
}

/// One timer: a paced or cooling pill, a waiting notice going stale, midnight, or a pause ending.
fn arm(app: &mut App) {
    let now = clock();
    let mut ms: Option<u64> = None;
    let mut take = |v: u64| ms = Some(ms.map_or(v, |m: u64| m.min(v)));
    if app.pill.is_none() && !panel_shown(app) {
        if let Some(w) = app.notices.q.wait_ms(now, unix_now()) {
            take(w);
        }
    }
    if let Some(at) = app.notices.q.next_change(now.at) {
        take(at.since(now.at).as_millis() as u64 + 50);
    }
    if let Some(t) = app.notices.paused_until.filter(|t| *t > now.at) {
        take(t.since(now.at).as_millis() as u64 + 50);
    }
    unsafe {
        match ms {
            Some(ms) => {
                SetTimer(
                    Some(app.hwnd),
                    TIMER_NOTICES,
                    ms.clamp(50, u32::MAX as u64) as u32,
                    None,
                );
            }
            None => {
                let _ = KillTimer(Some(app.hwnd), TIMER_NOTICES);
            }
        }
    }
}

pub fn on_timer(app: &mut App) {
    unsafe {
        let _ = KillTimer(Some(app.hwnd), TIMER_NOTICES);
    }
    if app.notices.paused_until.is_some_and(|t| habits::now() >= t) {
        app.notices.paused_until = None;
        log_line(app, "notices: pause ended");
        pump(app);
        return;
    }
    pump(app);
}

// ---- quiet times ----

/// Records the quiet time. Returns what a summarizing quiet time left waiting when it ended.
pub fn sync_quiet(app: &mut App) -> Option<m::Release> {
    let q = quiet(app);
    let r = app.notices.q.set_quiet(q, habits::now());
    if let Some(r) = r {
        log_line(
            app,
            &format!("notices: quiet time ended, {} waited", r.waited),
        );
    }
    r
}

static FG_PENDING: AtomicBool = AtomicBool::new(false);
static mut FG_HWND: HWND = HWND(std::ptr::null_mut());

unsafe extern "system" fn win_event(
    _: HWINEVENTHOOK,
    event: u32,
    hwnd: HWND,
    id_object: i32,
    id_child: i32,
    _: u32,
    _: u32,
) {
    let relevant = event == EVENT_SYSTEM_FOREGROUND
        || (id_object == OBJID_WINDOW.0 && id_child == 0 && hwnd == GetForegroundWindow());
    // Many location changes in a row become one message.
    if relevant && !FG_PENDING.swap(true, Ordering::AcqRel) {
        let _ = PostMessageW(Some(FG_HWND), WM_APP_FOREGROUND, WPARAM(0), LPARAM(0));
    }
}

/// Watches foreground changes, and the foreground window's size (a video going full screen
/// without a change of window). Both arrive as one coalesced WM_APP_FOREGROUND.
pub fn start(app: &mut App) {
    unsafe {
        FG_HWND = app.hwnd;
        let flags = WINEVENT_OUTOFCONTEXT | WINEVENT_SKIPOWNPROCESS;
        let a = SetWinEventHook(
            EVENT_SYSTEM_FOREGROUND,
            EVENT_SYSTEM_FOREGROUND,
            None,
            Some(win_event),
            0,
            0,
            flags,
        );
        let b = SetWinEventHook(
            EVENT_OBJECT_LOCATIONCHANGE,
            EVENT_OBJECT_LOCATIONCHANGE,
            None,
            Some(win_event),
            0,
            0,
            flags,
        );
        let ok = !a.is_invalid() && !b.is_invalid();
        log_line(
            app,
            &format!(
                "notices: watching the foreground ({ok}); {} mute(s) saved",
                app.notices.q.mutes.len()
            ),
        );
    }
    on_foreground(app);
}

fn class_of(h: HWND) -> String {
    let mut buf = [0u16; 64];
    let n = unsafe { GetClassNameW(h, &mut buf) };
    String::from_utf16_lossy(&buf[..n.max(0) as usize])
}

/// Presentation mode, or a full-screen foreground window on the orb's own monitor. A maximized
/// window with a title bar is not full screen.
fn full_screen_now(app: &App) -> bool {
    if app.notices.test_full_screen {
        return true;
    }
    unsafe {
        if matches!(SHQueryUserNotificationState(), Ok(s) if s == QUNS_PRESENTATION_MODE) {
            return true;
        }
        let fg = GetForegroundWindow();
        if fg.is_invalid() || [app.hwnd, app.pill_hwnd, app.dial_hwnd, app.panel_hwnd].contains(&fg)
        {
            return false;
        }
        if matches!(
            class_of(fg).as_str(),
            "Progman" | "WorkerW" | "Shell_TrayWnd" | "Shell_SecondaryTrayWnd"
        ) {
            return false;
        }
        let mon = MonitorFromWindow(fg, MONITOR_DEFAULTTONEAREST);
        if mon != MonitorFromWindow(app.hwnd, MONITOR_DEFAULTTONEAREST) {
            return false;
        }
        let mut mi = MONITORINFO {
            cbSize: std::mem::size_of::<MONITORINFO>() as u32,
            ..Default::default()
        };
        let _ = GetMonitorInfoW(mon, &mut mi);
        let mut r = RECT::default();
        let _ = GetWindowRect(fg, &mut r);
        let m = mi.rcMonitor;
        let covers =
            r.left <= m.left && r.top <= m.top && r.right >= m.right && r.bottom >= m.bottom;
        let style = GetWindowLongW(fg, GWL_STYLE) as u32;
        let titled = IsZoomed(fg).as_bool() && style & WS_CAPTION.0 == WS_CAPTION.0;
        covers && !titled
    }
}

/// WM_APP_FOREGROUND: the orb hides itself for a full-screen app and comes back after it.
pub fn on_foreground(app: &mut App) {
    FG_PENDING.store(false, Ordering::Release);
    let fs = full_screen_now(app);
    if fs == app.notices.full_screen {
        return;
    }
    app.notices.full_screen = fs;
    log_line(
        app,
        if fs {
            "notices: full screen, the orb hides and notices wait"
        } else {
            "notices: full screen ended"
        },
    );
    if fs {
        if app.visible && !app.notices.rehide {
            crate::dial::close_dial(app);
            panels::close_panel(app);
            if app.pill.is_some() {
                pill::dismiss_pill(app);
            }
            orb::set_visible(app, false);
            app.notices.fs_hidden = true;
        }
    } else {
        if app.notices.fs_hidden && !app.notices.rehide {
            orb::set_visible(app, true);
        }
        app.notices.fs_hidden = false;
    }
    pump(app);
}

/// You showed or hid the orb (the shortcut or the tray).
pub fn on_user_visibility(app: &mut App) {
    app.notices.fs_hidden = false;
    app.notices.rehide = false;
    pump(app);
}

/// Tray: pause notices for a while, or (None) resume them.
pub fn pause(app: &mut App, d: Option<Duration>, until_tomorrow: bool) {
    let n = habits::now();
    app.notices.paused_until = if until_tomorrow {
        Some(n.date().next().at_minute(0))
    } else {
        d.map(|d| n + d)
    };
    match app.notices.paused_until {
        Some(t) => log_line(app, &format!("notices: paused until {}", m::hhmm(t))),
        None => log_line(app, "notices: resumed"),
    }
    pump(app);
}

pub fn test_full_screen(app: &mut App) {
    app.notices.test_full_screen = !app.notices.test_full_screen;
    log_line(
        app,
        &format!("notices: test full screen {}", app.notices.test_full_screen),
    );
    on_foreground(app);
}

// ---- the focus hover pill ----

/// Close to the drop while focus runs: "Focus 18 min left · 2 held".
pub fn on_orb_near(app: &mut App, close: bool) {
    if !close {
        app.notices.hover_named = false;
        return;
    }
    if app.notices.hover_named || app.pill.is_some() || app.dial_open || !habits::focus_running(app)
    {
        return;
    }
    let held = app.notices.q.held_now();
    let mut value = habits::dial_focus_value(app);
    if held > 0 {
        value = format!("{value} · {held} held");
    }
    reply(
        app,
        Pill {
            icon: Icon::Focus,
            dot: false,
            title: "Focus".into(),
            value,
            action: None,
            closable: false,
            ttl_ms: 6000,
        },
    );
    app.notices.hover_named = true;
}

pub fn on_orb_gone(app: &mut App) {
    if app.notices.hover_named
        && !app.pill_closing
        && app
            .pill
            .as_ref()
            .is_some_and(|p| p.title == "Focus" && p.action.is_none())
    {
        pill::dismiss_pill(app);
    }
    app.notices.hover_named = false;
}

// ---- the stack ----

/// The notices waiting behind the front pill, in the order they would come: (id, pill).
pub fn stack(app: &App) -> Vec<(u64, Pill)> {
    app.notices
        .q
        .waiting_entries()
        .into_iter()
        .map(|e| (e.id, e.pill()))
        .collect()
}

/// A waiting pill in the spread stack, clicked: it comes to the front.
pub fn bring_front(app: &mut App, id: u64) {
    if let Some(p) = app.notices.q.open(id, clock()) {
        pill::show_pill(app, p);
    }
    after(app);
}

/// An action on a pill in the spread stack: it runs, and that notice is done with.
pub fn stack_action(app: &mut App, id: u64) {
    app.notices.q.acted(id);
    after(app);
}

// ---- sources: small builders the callers share ----

pub fn prayer_before(prayer: &str, at: Moment, p: Pill) -> Notice {
    Notice::new(Source::Prayer, Priority::Normal, p).before_prayer(prayer, at)
}

pub fn urgent(source: Source, p: Pill) -> Notice {
    Notice::new(source, Priority::Urgent, p)
}

/// Focus phase pills (break, done, paused for a prayer): they wait like any normal notice but
/// never stay in the list.
pub fn focus(p: Pill) -> Notice {
    Notice::new(Source::Focus, Priority::Normal, p).keep(Keep::Moment)
}

pub fn merging(
    n: Notice,
    key: &'static str,
    many: &str,
    value: &str,
    action: Option<(String, PillAction)>,
) -> Notice {
    n.merging(Merge {
        key,
        many: many.into(),
        value: value.into(),
        action,
    })
}

/// The clipboard offer: dropped in a quiet time, never listed, merges with nothing.
pub fn offer(p: Pill) -> Notice {
    Notice::new(Source::Downloads, Priority::Normal, p).keep(Keep::Drop)
}

/// The clipboard offer for a link that just joined the queue is out of date wherever it waits.
pub fn drop_offers(app: &mut App, f: impl Fn(&Pill) -> bool) {
    app.notices.q.drop_where(f);
}

/// "Memasuki waktu": reminders ahead of this prayer are stale wherever they wait.
pub fn drop_prayer(app: &mut App, prayer: &str) {
    app.notices.q.drop_prayer(prayer);
    after(app);
}

/// Listed notices held since the current quiet time began (the focus panel's line).
pub fn held_now(app: &App) -> usize {
    app.notices.q.held_now()
}

/// Tray test: forget the last low-space pill's cooldown.
pub fn forget_cooldown(app: &mut App, kind: &str) {
    app.notices.q.cooldowns.remove(kind);
    save(app);
}

// ---- the Notices panel ----

fn panel_shown(app: &App) -> bool {
    app.panel_open && app.panel_kind == Kind::Notices
}

pub fn open_panel(app: &mut App) {
    app.notices.focus_row = None;
    app.notices.menu = None;
    app.notices.note = None;
    panels::open_panel(app, Kind::Notices);
    let ok = panels::take_keyboard(app);
    log_line(
        app,
        &format!(
            "notices: panel, {} waiting (foreground={ok})",
            app.notices.q.waiting()
        ),
    );
}

pub fn on_panel_closed(app: &mut App) {
    app.notices.menu = None;
    app.notices.note = None;
    app.notices.focus_row = None;
    // What waited its turn comes once the panel has gone (or another panel took its place).
    unsafe {
        SetTimer(Some(app.hwnd), TIMER_NOTICES, 50, None);
    }
}

fn row(e: &m::Entry, unseen: bool) -> Row {
    let p = e.pill();
    Row {
        id: e.id,
        icon: p.icon,
        title: p.title,
        value: p.value,
        time: m::hhmm(e.at),
        unseen,
        source: e.notice.source,
    }
}

pub fn view(app: &App) -> View {
    let now = habits::now();
    let (waiting, earlier) = app.notices.q.list();
    let unix = unix_now();
    let tomorrow = unix + now.date().next().at_minute(0).since(now).as_secs() as i64;
    let muted = app
        .notices
        .q
        .active_mutes(unix)
        .into_iter()
        .map(|(s, mu)| {
            (
                s,
                m::mute_words(
                    mu,
                    unix,
                    |t| m::hhmm(now + Duration::from_secs((t - unix).max(0) as u64)),
                    tomorrow,
                ),
            )
        })
        .collect();
    let rows: Vec<Row> = waiting
        .iter()
        .map(|e| row(e, true))
        .chain(earlier.iter().map(|e| row(e, false)))
        .collect();
    let n_waiting = waiting.len();
    let menu = app
        .notices
        .menu
        .and_then(|id| rows.iter().position(|r| r.id == id));
    View {
        rows,
        n_waiting,
        muted,
        undo_clear: app.notices.q.can_undo_clear(now),
        focus: app.notices.focus_row,
        menu,
        menu_item: app.notices.menu_item,
        note: app.notices.note.map(|(s, _)| {
            let words = app
                .notices
                .q
                .mutes
                .get(&s)
                .map(|mu| {
                    m::mute_words(
                        *mu,
                        unix,
                        |t| m::hhmm(now + Duration::from_secs((t - unix).max(0) as u64)),
                        tomorrow,
                    )
                })
                .unwrap_or_default();
            (format!("{} muted", s.name()), words)
        }),
    }
}

pub fn size(app: &App) -> (f32, f32) {
    (np::W, np::height(&view(app)))
}

pub fn hit(app: &App, x: f32, y: f32) -> Option<Hit> {
    np::hit(&app.g, &app.fonts, &view(app), x, y)
}

pub fn draw_panel(app: &App, dc: &ID2D1DeviceContext, hover: Option<Hit>) {
    np::draw(&app.g, dc, &app.theme, &app.fonts, &view(app), hover);
}

/// The panel's timer: Undo after Clear all goes after 6 s.
pub fn panel_timer_ms(app: &App) -> Option<u32> {
    app.notices.q.can_undo_clear(habits::now()).then_some(1000)
}

/// A row opens as its pill from the drop, with its action (the panel closes for it).
fn open_row(app: &mut App, id: u64) {
    panels::close_panel(app);
    if let Some(p) = app.notices.q.open(id, clock()) {
        log_line(
            app,
            &format!("notices: opened \"{}\" from the list", p.title),
        );
        pill::show_pill(app, p);
    }
    after(app);
}

fn mute(app: &mut App, s: Source, f: MuteFor) {
    let unix = unix_now();
    let now = habits::now();
    let tomorrow = unix + now.date().next().at_minute(0).since(now).as_secs() as i64;
    let before = app.notices.q.mute(s, m::mute_until(f, unix, tomorrow));
    app.notices.note = Some((s, before));
    log_line(app, &format!("notices: {} muted ({f:?})", s.name()));
    save(app);
}

fn menu_pick(app: &mut App, item: MenuItem) {
    let Some(id) = app.notices.menu.take() else {
        return;
    };
    let Some(s) = app.notices.q.source_of(id) else {
        return;
    };
    match item {
        MenuItem::Seen => app.notices.q.mark_seen(id),
        MenuItem::Hour => mute(app, s, MuteFor::Hour),
        MenuItem::Tomorrow => mute(app, s, MuteFor::Tomorrow),
        MenuItem::Always => mute(app, s, MuteFor::Always),
    }
    // The ring stays on the same notice, wherever it moved in the list.
    app.notices.focus_row = view(app).rows.iter().position(|r| r.id == id);
    after(app);
}

fn open_menu(app: &mut App, i: usize) {
    let v = view(app);
    if let Some(r) = v.rows.get(i) {
        app.notices.menu = Some(r.id);
        app.notices.menu_item = 0;
        app.notices.focus_row = Some(i);
        panels::refresh(app);
    }
}

pub fn click(app: &mut App, h: Hit) {
    let v = view(app);
    match h {
        Hit::ClearAll => {
            app.notices.q.clear_all(habits::now());
            app.notices.focus_row = None;
            log_line(app, "notices: cleared");
            after(app);
            panels::refresh(app);
        }
        Hit::UndoClear => {
            if app.notices.q.undo_clear(habits::now()) {
                log_line(app, "notices: clear undone");
            }
            after(app);
        }
        Hit::Row(i) => {
            if app.notices.menu.is_some() {
                app.notices.menu = None;
                panels::refresh(app);
            } else if let Some(r) = v.rows.get(i) {
                open_row(app, r.id);
            }
        }
        Hit::More(i) => open_menu(app, i),
        Hit::Menu(item) => menu_pick(app, item),
        Hit::Unmute(s) => {
            app.notices.q.set_mute(s, None);
            if app.notices.note.is_some_and(|(n, _)| n == s) {
                app.notices.note = None;
            }
            log_line(app, &format!("notices: {} unmuted", s.name()));
            save(app);
            after(app);
        }
        Hit::UndoMute => {
            if let Some((s, before)) = app.notices.note.take() {
                app.notices.q.set_mute(s, before);
                log_line(app, &format!("notices: mute of {} undone", s.name()));
                save(app);
            }
            after(app);
        }
    }
}

/// Right-click on a row opens its menu.
pub fn right_click(app: &mut App, h: Option<Hit>) {
    if let Some(Hit::Row(i) | Hit::More(i)) = h {
        open_menu(app, i);
    }
}

/// Up and Down move the ring (or the menu's item), Enter opens the row (or picks the item),
/// Shift+F10 or the menu key opens the row menu, Esc closes the menu, then the panel.
pub fn on_keydown(app: &mut App, vk: u16) -> bool {
    let v = view(app);
    let k = VIRTUAL_KEY(vk);
    let shift = unsafe { GetKeyState(VK_SHIFT.0 as i32) } < 0;
    if app.notices.menu.is_some() {
        let n = np::MENU.len();
        match k {
            VK_UP => app.notices.menu_item = (app.notices.menu_item + n - 1) % n,
            VK_DOWN => app.notices.menu_item = (app.notices.menu_item + 1) % n,
            VK_RETURN | VK_SPACE => {
                let item = np::MENU[app.notices.menu_item.min(n - 1)];
                menu_pick(app, item);
                return true;
            }
            VK_ESCAPE => app.notices.menu = None,
            _ => return false,
        }
        panels::refresh(app);
        return true;
    }
    let n = v.rows.len();
    match k {
        VK_UP | VK_DOWN if n > 0 => {
            let cur = app.notices.focus_row;
            app.notices.focus_row = Some(match (cur, k == VK_UP) {
                (None, _) => 0,
                (Some(i), true) => i.saturating_sub(1),
                (Some(i), false) => (i + 1).min(n - 1),
            });
        }
        VK_RETURN => {
            if let Some(r) = app.notices.focus_row.and_then(|i| v.rows.get(i)) {
                open_row(app, r.id);
            }
            return true;
        }
        VK_APPS => {
            if let Some(i) = app.notices.focus_row {
                open_menu(app, i);
            }
            return true;
        }
        VK_F10 if shift => {
            if let Some(i) = app.notices.focus_row {
                open_menu(app, i);
            }
            return true;
        }
        VK_ESCAPE => {
            panels::close_panel(app);
            return true;
        }
        _ => return false,
    }
    panels::refresh(app);
    true
}

/// The More branch's Notices child: "3 waiting", or nothing new.
pub fn dial_child(app: &App) -> DialChild {
    let n = app.notices.q.waiting();
    let value = match n {
        0 => "Today".to_string(),
        n => format!("{n} waiting"),
    };
    DialChild {
        text: String::new(),
        icon: Some(Icon::Bell),
        name: "Notices".into(),
        value,
    }
}

/// Tray test: sample notices from three sources at once (queue, stack, badge, merge).
pub fn test_samples(app: &mut App) {
    let task = Pill {
        icon: Icon::Check,
        dot: false,
        title: "Kirim invoice (sample)".into(),
        value: "due 14:00".into(),
        action: Some(("Open".into(), PillAction::OpenTasks)),
        closable: true,
        ttl_ms: 8000,
    };
    notify(app, Source::Tasks, task);
    for name in ["Lagu (sample)", "Video (sample)", "Klip (sample)"] {
        let p = Pill {
            icon: Icon::Tick,
            dot: false,
            title: "Saved".into(),
            value: name.into(),
            action: Some(("Show".into(), PillAction::OpenDownloads)),
            closable: true,
            ttl_ms: 6000,
        };
        let n = merging(
            Notice::new(Source::Downloads, Priority::Normal, p),
            "done",
            "downloads done",
            "Videos",
            Some(("Show".into(), PillAction::OpenDownloads)),
        );
        post(app, n);
    }
    let ml = app.habits.drink.config().default_glass_ml;
    let drink = habits::drink_pill(
        app,
        "Minum dulu",
        Some((format!("+{ml} ml"), PillAction::AddDrink(ml))),
        6000,
    );
    notify(app, Source::Drink, drink);
}

/// Tray test: a quiet notice (family activity is not in yet, so a sample stands in).
pub fn test_quiet(app: &mut App) {
    let p = Pill {
        icon: Icon::Tick,
        dot: false,
        title: "Ayah finished Gula 1 kg (sample)".into(),
        value: "Belanja".into(),
        action: None,
        closable: true,
        ttl_ms: 6000,
    };
    post(app, Notice::new(Source::Tasks, Priority::Quiet, p));
}

/// The Settings window's Notices section: whether a source speaks, and the mute's words if not
/// ("for good", "until 15:20").
pub fn source_state(app: &App, s: Source) -> Option<String> {
    let unix = unix_now();
    if !app.notices.q.muted(s, unix) {
        return None;
    }
    let now = habits::now();
    let tomorrow = unix + now.date().next().at_minute(0).since(now).as_secs() as i64;
    app.notices.q.mutes.get(&s).map(|mu| {
        m::mute_words(
            *mu,
            unix,
            |t| m::hhmm(now + Duration::from_secs((t - unix).max(0) as u64)),
            tomorrow,
        )
    })
}

/// The Settings window's switch: off mutes the source for good, on lifts any mute.
pub fn set_source_on(app: &mut App, s: Source, on: bool) {
    if !s.mutable() {
        return;
    }
    if on {
        app.notices.q.set_mute(s, None);
    } else {
        app.notices.q.mute(s, Mute::Always);
    }
    if app.notices.note.is_some_and(|(n, _)| n == s) {
        app.notices.note = None;
    }
    log_line(
        app,
        &format!(
            "notices: {} {} from Settings",
            s.name(),
            if on { "on" } else { "muted" }
        ),
    );
    save(app);
    after(app);
}

/// Tray: every source speaks again.
pub fn unmute_all(app: &mut App) {
    app.notices.q.mutes.clear();
    log_line(app, "notices: every source unmuted");
    save(app);
    after(app);
}

/// The downloads offer checks this instead of its own full-screen test.
pub fn offer_blocked(app: &App) -> Option<&'static str> {
    let q = quiet(app);
    if q.hidden {
        Some("the orb is hidden")
    } else if q.focus || habits::focus_running(app) {
        Some("focus is on")
    } else if q.full_screen {
        Some("an app is full screen")
    } else if q.paused {
        Some("notices are paused")
    } else {
        None
    }
}
