//! The Settings window and the first run, on both systems. settings_model.rs holds the values,
//! settings_panel.rs draws; this file keeps `settings.json` (shared with the calendar link, each
//! writer keeps the other's keys), runs the shortcut's capture, the place and its city search,
//! the sections and the first run's steps, and applies every change at once: the theme, the
//! shortcut, the place and the prayer reminders, the notice mutes.
//!
//! Everything it asks of the system goes through `settings_sys`: the window (Windows:
//! settings_window.rs over Win32 and DirectComposition; macOS: mac/settings_win.rs, an NSWindow),
//! the global shortcut (RegisterHotKey / a Carbon hot key), start at login (the HKCU Run entry /
//! SMAppService) and a location fix (the WinRT Geolocator; none on the Mac).

use std::path::{Path, PathBuf};
use std::sync::mpsc::{channel, Receiver};

use serde_json::{Map, Value};

use crate::notices::{self, Notice, Priority, Source};
use crate::orb;
use crate::settings_model::{
    self as m, Capture, CaptureKey, Locate, Place, Prefs, Section, Shortcut, Startup, Step,
};
use crate::settings_panel::{self as sp, CityPick, Hit, Screen, SourceRow, View};
use crate::settings_sys;
use crate::sys::{self, vk};
use crate::textfield::{Key, TextField};
use crate::ui::{self, DialChild, Icon, Pill, PillAction};
use crate::{dial, log_line, panels, pill, prayer, App};

const SETTINGS: &str = "settings.json";
/// The notice sources the Notices section can switch off, in the panel's order.
const SOURCES: [(Source, &str); 6] = [
    (
        Source::Calendar,
        "Event pills 10 min before and at the start",
    ),
    (Source::Tasks, "A pill when a task is due"),
    (Source::Drink, "\"Minum dulu\" through the day"),
    (Source::Focus, "Breaks and the end of a session"),
    (
        Source::Downloads,
        "Copied links, saved and failed downloads",
    ),
    (Source::Disk, "Low space and finished scans"),
];

pub struct Settings {
    pub prefs: Prefs,
    dir: PathBuf,
    /// The window and what the system keeps for it (settings_sys).
    pub win: settings_sys::Win,
    /// What the window shows; None while it is hidden.
    pub screen: Option<Screen>,
    pub(crate) hover: Option<Hit>,
    pub(crate) focus: Option<Hit>,
    pub(crate) scroll: f32,
    pub(crate) capture: Option<Capture>,
    taken: Option<Shortcut>,
    /// The shortcut in use is registered (another app may hold it).
    pub hotkey_ok: bool,
    suggestion: Option<Shortcut>,
    /// The first run's "try it": the shortcut was pressed on the shortcut step.
    tried: bool,
    pub(crate) want_windows: bool,
    pub(crate) locate: Locate,
    loc_rx: Option<Receiver<Locate>>,
    /// The radio's city: the first run's draft, or the city in use.
    pub(crate) city: Option<String>,
    pub(crate) picking: bool,
    pub(crate) field: TextField,
    rows: Vec<usize>,
    sel: usize,
    list_scroll: usize,
    /// The first run's Start with Windows (Open at login), written at Finish.
    fr_start: bool,
    pub(crate) startup: Startup,
    undo_shortcut: Option<Shortcut>,
}

// ---- settings.json ----

pub fn read_map(dir: &Path) -> Map<String, Value> {
    std::fs::read_to_string(dir.join(SETTINGS))
        .ok()
        .and_then(|s| serde_json::from_str::<Value>(&s).ok())
        .and_then(|v| v.as_object().cloned())
        .unwrap_or_default()
}

/// Read, change, write (then rename): other keys in the file are kept as they are.
pub fn update_map(dir: &Path, f: impl FnOnce(&mut Map<String, Value>)) {
    let mut s = read_map(dir);
    f(&mut s);
    if let Ok(bytes) = serde_json::to_vec_pretty(&Value::Object(s)) {
        let path = dir.join(SETTINGS);
        let tmp = path.with_extension("json.tmp");
        if std::fs::write(&tmp, bytes).is_ok() {
            let _ = std::fs::rename(&tmp, &path);
        }
    }
}

impl Settings {
    pub fn load(dir: &Path, win: settings_sys::Win) -> Settings {
        let prefs = Prefs::from_map(&read_map(dir));
        let city = match &prefs.place {
            Place::City { name } => Some(name.clone()),
            Place::Windows { .. } => None,
        };
        Settings {
            want_windows: prefs.place.is_windows() && settings_sys::HAS_LOCATION,
            prefs,
            dir: dir.to_path_buf(),
            win,
            screen: None,
            hover: None,
            focus: None,
            scroll: 0.0,
            capture: None,
            taken: None,
            hotkey_ok: false,
            suggestion: None,
            tried: false,
            locate: Locate::Idle,
            loc_rx: None,
            city,
            picking: false,
            field: TextField::default(),
            rows: Vec::new(),
            sel: 0,
            list_scroll: 0,
            fr_start: m::os::W.start_default,
            startup: Startup::Off,
            undo_shortcut: None,
        }
    }
}

fn save(app: &mut App) {
    let p = app.settings.prefs.clone();
    update_map(&app.settings.dir, |m| p.write_into(m));
}

/// The theme the app draws with, from the setting and the system's own.
pub fn theme_now(prefs: &Prefs) -> ui::Theme {
    ui::theme(prefs.theme.is_dark(settings_sys::system_dark()))
}

// ---- launch ----

/// After the windows exist: the shortcut, the first run or the orb's start state, and a fresh
/// Windows location when the place follows it.
pub fn start(app: &mut App) {
    let s = app.settings.prefs.shortcut;
    app.settings.hotkey_ok = settings_sys::register(app, s);
    log_line(
        app,
        &format!(
            "settings: shortcut {} registered={}",
            s.label(),
            app.settings.hotkey_ok
        ),
    );
    if !app.settings.prefs.first_run_done && !settings_sys::no_first_run() {
        // At sign-in the welcome must not pull the keyboard out of whatever you already opened.
        first_run(app, false);
        return;
    }
    if !app.settings.hotkey_ok {
        let p = Pill {
            icon: Icon::Keyboard,
            dot: false,
            title: format!("Couldn't use {}", s.shown()),
            value: "another app uses it".into(),
            action: Some(("Change".into(), PillAction::OpenSettings)),
            closable: true,
            ttl_ms: 10_000,
        };
        notices::post(app, Notice::new(Source::App, Priority::Normal, p));
    }
    if !app.settings.prefs.show_orb_at_start {
        orb::set_visible(app, false);
        notices::on_user_visibility(app);
    }
    if app.settings.prefs.place.is_windows() && settings_sys::HAS_LOCATION {
        locate(app);
    }
}

// ---- the shortcut ----

/// Moves the shortcut to `s`. On failure the old one stays (registered again) and the card says
/// which one is taken.
fn set_shortcut(app: &mut App, s: Shortcut) -> bool {
    let old = app.settings.prefs.shortcut;
    settings_sys::unregister(app);
    if settings_sys::register(app, s) {
        app.settings.prefs.shortcut = s;
        app.settings.hotkey_ok = true;
        app.settings.taken = None;
        save(app);
        log_line(app, &format!("settings: shortcut now {}", s.label()));
        return true;
    }
    app.settings.hotkey_ok = settings_sys::register(app, old);
    app.settings.taken = Some(s);
    log_line(
        app,
        &format!("settings: {} is taken, kept {}", s.label(), old.label()),
    );
    false
}

fn begin_capture(app: &mut App) {
    app.settings.capture = Some(Capture::default());
    app.settings.taken = None;
    // The shortcut in use must reach the window as keys, so it can be chosen again.
    settings_sys::unregister(app);
    settings_sys::take_foreground(app);
}

pub(crate) fn cancel_capture(app: &mut App) {
    if app.settings.capture.take().is_some() {
        let s = app.settings.prefs.shortcut;
        app.settings.hotkey_ok = settings_sys::register(app, s);
    }
}

fn save_capture(app: &mut App) {
    let Some(s) = app.settings.capture.and_then(|c| c.got) else {
        return;
    };
    app.settings.capture = None;
    let old = app.settings.prefs.shortcut;
    if s == old {
        app.settings.hotkey_ok = settings_sys::register(app, s);
        return;
    }
    // The old shortcut was let go for the capture; set_shortcut registers one of the two.
    app.settings.hotkey_ok = settings_sys::register(app, old);
    if set_shortcut(app, s) {
        app.settings.tried = false;
        app.settings.undo_shortcut = Some(old);
        if app.visible && matches!(app.settings.screen, Some(Screen::Settings(_))) {
            let p = Pill {
                icon: Icon::Keyboard,
                dot: false,
                title: "Shortcut".into(),
                value: format!("now {}", s.shown()),
                action: Some(("Undo".into(), PillAction::UndoShortcut)),
                closable: true,
                ttl_ms: 6000,
            };
            notices::reply(app, p);
        }
    }
}

/// The pill's Undo after a new shortcut.
pub fn undo_shortcut(app: &mut App) {
    if let Some(old) = app.settings.undo_shortcut.take() {
        set_shortcut(app, old);
        pill::dismiss_pill(app);
        render(app);
    }
}

/// The shortcut in use, as this system names its keys.
pub fn shortcut_label(app: &App) -> String {
    app.settings.prefs.shortcut.shown()
}

/// During the first run the shortcut belongs to its "try it", and the orb stays put.
pub fn takes_shortcut(app: &App) -> bool {
    matches!(app.settings.screen, Some(Screen::FirstRun(_)))
}

pub fn on_shortcut_pressed(app: &mut App) {
    if app.settings.screen == Some(Screen::FirstRun(Step::Shortcut)) {
        app.settings.tried = true;
        log_line(app, "settings: first run, the shortcut works");
        render(app);
    }
}

// ---- start with Windows (open at login) ----

fn write_startup(app: &mut App, on: bool) {
    let ok = settings_sys::write_startup(app, on);
    app.settings.startup = settings_sys::read_startup();
    log_line(
        app,
        &format!(
            "settings: start with Windows {} (ok={ok}, now {:?})",
            if on { "on" } else { "off" },
            app.settings.startup
        ),
    );
}

/// The startup entry may have changed outside (Task Manager, Login Items, the menu bar): the
/// General section reads it again.
pub fn refresh_startup(app: &mut App) {
    if matches!(
        app.settings.screen,
        Some(Screen::Settings(Section::General))
    ) {
        let now = settings_sys::read_startup();
        if now != app.settings.startup {
            app.settings.startup = now;
            render(app);
        }
    }
}

// ---- the place ----

fn locate(app: &mut App) {
    if !settings_sys::HAS_LOCATION || app.settings.locate == Locate::Finding {
        return;
    }
    app.settings.locate = Locate::Finding;
    let (tx, rx) = channel();
    app.settings.loc_rx = Some(rx);
    settings_sys::find_location(app, tx);
    log_line(app, "settings: asking Windows location");
}

pub fn on_located(app: &mut App) {
    let Some(l) = app
        .settings
        .loc_rx
        .as_ref()
        .and_then(|rx| rx.try_recv().ok())
    else {
        return;
    };
    app.settings.loc_rx = None;
    app.settings.locate = l;
    log_line(
        app,
        &format!(
            "settings: Windows location {}",
            match l {
                Locate::Found { lat, lng } => format!("found near {}", m::near_words(lat, lng)),
                other => format!("{other:?}"),
            }
        ),
    );
    // Outside the first run a fix is used at once (Settings, or following you at launch).
    if let Locate::Found { lat, lng } = l {
        let first_run = matches!(app.settings.screen, Some(Screen::FirstRun(_)));
        if app.settings.want_windows && !first_run {
            let moved = match app.settings.prefs.place {
                Place::Windows { lat: a, lng: b } => m::distance_km((a, b), (lat, lng)) > 1.0,
                Place::City { .. } => true,
            };
            if moved {
                set_place(app, Place::Windows { lat, lng });
            }
        }
    }
    render(app);
}

fn set_place(app: &mut App, p: Place) {
    log_line(app, &format!("settings: place {}", p.words()));
    app.settings.prefs.place = p;
    save(app);
    prayer::apply_place(app);
}

/// The place the times row shows: what the radio would use.
fn shown_coords(app: &App) -> (f64, f64) {
    let s = &app.settings;
    match (s.want_windows, s.locate, &s.city) {
        (true, Locate::Found { lat, lng }, _) => (lat, lng),
        (false, _, Some(c)) => Place::City { name: c.clone() }.coords(),
        _ => s.prefs.place.coords(),
    }
}

fn choose_windows(app: &mut App) {
    if !settings_sys::HAS_LOCATION {
        return;
    }
    app.settings.want_windows = true;
    match app.settings.locate {
        Locate::Found { lat, lng } if !matches!(app.settings.screen, Some(Screen::FirstRun(_))) => {
            set_place(app, Place::Windows { lat, lng })
        }
        Locate::Found { .. } | Locate::Finding => {}
        _ => locate(app),
    }
}

fn open_picker(app: &mut App) {
    app.settings.picking = true;
    app.settings.field.clear();
    app.settings.rows = m::filter_cities("");
    // Start on the city in use.
    let cur = app
        .settings
        .city
        .as_deref()
        .and_then(|c| m::CITIES.iter().position(|x| x.name == c));
    app.settings.sel = cur
        .and_then(|i| app.settings.rows.iter().position(|r| *r == i))
        .unwrap_or(0);
    app.settings.list_scroll = 0;
    keep_sel_in_view(app);
    app.settings.focus = None;
    settings_sys::take_foreground(app);
}

fn pick_city(app: &mut App, i: usize) {
    let name = m::CITIES[i].name.to_string();
    app.settings.picking = false;
    app.settings.want_windows = false;
    app.settings.city = Some(name.clone());
    if !matches!(app.settings.screen, Some(Screen::FirstRun(_))) {
        set_place(app, Place::City { name });
    }
}

fn keep_sel_in_view(app: &mut App) {
    let fit = sp::city_rows_fit(matches!(app.settings.screen, Some(Screen::FirstRun(_))));
    let s = &mut app.settings;
    if s.sel < s.list_scroll {
        s.list_scroll = s.sel;
    } else if s.sel >= s.list_scroll + fit {
        s.list_scroll = s.sel + 1 - fit;
    }
}

fn on_field_changed(app: &mut App) {
    app.settings.rows = m::filter_cities(app.settings.field.text());
    app.settings.sel = 0;
    app.settings.list_scroll = 0;
}

/// The city search's text changed from outside the keys (a paste or cut on the Mac).
#[cfg_attr(windows, allow(dead_code))]
pub fn on_field_edited(app: &mut App) {
    if app.settings.picking {
        on_field_changed(app);
        render(app);
    }
}

// ---- the theme ----

pub fn apply_theme(app: &mut App) {
    app.theme = theme_now(&app.settings.prefs);
    settings_sys::redraw_others(app);
    render(app);
    let dark = app.theme.dark;
    log_line(
        app,
        &format!(
            "settings: theme {:?}, dark={dark}",
            app.settings.prefs.theme
        ),
    );
}

/// The system switched between light and dark (Windows: WM_SETTINGCHANGE "ImmersiveColorSet";
/// macOS: AppleInterfaceThemeChangedNotification).
pub fn on_system_theme(app: &mut App) {
    if app.settings.prefs.theme == m::ThemeMode::Windows
        && settings_sys::system_dark() != app.theme.dark
    {
        apply_theme(app);
    } else {
        render(app);
    }
}

// ---- the window ----

/// Opens Settings on a section (the dial's More branch, the tray, a pill's Change).
pub fn open(app: &mut App, section: Section) {
    if takes_shortcut(app) {
        // The first run comes first.
        settings_sys::take_foreground(app);
        return;
    }
    dial::close_dial(app);
    panels::close_panel(app);
    if let Some(Screen::Settings(_)) = app.settings.screen {
        if app.settings.screen != Some(Screen::Settings(section)) {
            go_section(app, section);
        }
        settings_sys::take_foreground(app);
        return;
    }
    app.settings.startup = settings_sys::read_startup();
    app.settings.want_windows = app.settings.prefs.place.is_windows() && settings_sys::HAS_LOCATION;
    app.settings.city = match &app.settings.prefs.place {
        Place::City { name } => Some(name.clone()),
        Place::Windows { .. } => app.settings.city.clone(),
    };
    app.settings.picking = false;
    app.settings.taken = None;
    show(app, Screen::Settings(section), true);
    log_line(app, &format!("settings: open on {}", section.name()));
}

/// The first run from the tray's test item. The orb waits hidden until Finish.
pub fn open_first_run(app: &mut App) {
    first_run(app, true);
}

fn first_run(app: &mut App, activate: bool) {
    dial::close_dial(app);
    panels::close_panel(app);
    if app.pill.is_some() {
        pill::dismiss_pill(app);
    }
    if app.visible {
        orb::set_visible(app, false);
    }
    // Hidden as if by you: notices wait for Finish, and a full-screen app ending does not bring
    // the orb back over the welcome.
    notices::on_user_visibility(app);
    let s = &mut app.settings;
    s.want_windows = settings_sys::HAS_LOCATION;
    s.city = match &s.prefs.place {
        Place::City { name } => Some(name.clone()),
        Place::Windows { .. } => Some("Yogyakarta".into()),
    };
    s.fr_start = m::os::W.start_default;
    s.tried = false;
    s.picking = false;
    s.taken = None;
    s.capture = None;
    s.suggestion = None;
    if !app.settings.hotkey_ok {
        app.settings.suggestion = m::SUGGESTIONS
            .iter()
            .copied()
            .find(|k| settings_sys::is_free(app, *k));
    }
    show(app, Screen::FirstRun(Step::Welcome), activate);
    log_line(app, "settings: first run");
}

fn show(app: &mut App, screen: Screen, activate: bool) {
    app.settings.screen = Some(screen);
    app.settings.hover = None;
    app.settings.focus = None;
    app.settings.scroll = 0.0;
    settings_sys::place(app);
    render(app);
    settings_sys::show(app, activate);
    if activate {
        settings_sys::take_foreground(app);
    }
}

fn go_section(app: &mut App, section: Section) {
    cancel_capture(app);
    app.settings.picking = false;
    app.settings.taken = None;
    app.settings.scroll = 0.0;
    app.settings.hover = None;
    app.settings.screen = Some(Screen::Settings(section));
    if section == Section::General {
        app.settings.startup = settings_sys::read_startup();
    }
    if section == Section::Prayer
        && app.settings.want_windows
        && !matches!(app.settings.locate, Locate::Found { .. } | Locate::Finding)
    {
        locate(app);
    }
    render(app);
}

fn go_step(app: &mut App, step: Step) {
    cancel_capture(app);
    app.settings.picking = false;
    app.settings.focus = None;
    app.settings.hover = None;
    app.settings.screen = Some(Screen::FirstRun(step));
    if step == Step::Place
        && app.settings.want_windows
        && !matches!(app.settings.locate, Locate::Found { .. } | Locate::Finding)
    {
        locate(app);
    }
    log_line(
        app,
        &format!("settings: first run step {} of 5", step.index() + 1),
    );
    render(app);
}

/// Shows the section (a test command, the menu bar), or opens Settings on it.
#[cfg_attr(windows, allow(dead_code))]
pub fn go_to(app: &mut App, section: Section) {
    if matches!(app.settings.screen, Some(Screen::Settings(_))) {
        go_section(app, section);
    } else {
        open(app, section);
    }
}

pub fn close(app: &mut App) {
    if app.settings.screen.is_none() {
        return;
    }
    cancel_capture(app);
    app.settings.screen = None;
    app.settings.picking = false;
    // The window hides and lets its drawing surface go.
    settings_sys::hide(app);
    log_line(app, "settings: closed");
}

fn finish_first_run(app: &mut App) {
    let on = app.settings.fr_start;
    if on || settings_sys::read_startup() != Startup::Off {
        write_startup(app, on);
    }
    app.settings.prefs.first_run_done = true;
    save(app);
    close(app);
    orb::set_visible(app, true);
    notices::on_user_visibility(app);
    let label = shortcut_label(app);
    let p = Pill {
        icon: Icon::Tick,
        dot: false,
        title: "Orby is ready".into(),
        value: label,
        action: None,
        closable: true,
        ttl_ms: 6000,
    };
    notices::post(app, Notice::new(Source::App, Priority::Normal, p));
    log_line(app, "settings: first run done");
}

/// Alt+F4 (Cmd+W) on the first run: the defaults stay (the place in use, no startup entry) and
/// the orb comes out, so closing it never leaves the app with nothing on screen.
pub fn skip_first_run(app: &mut App) {
    app.settings.prefs.first_run_done = true;
    save(app);
    close(app);
    orb::set_visible(app, true);
    notices::on_user_visibility(app);
    log_line(app, "settings: first run closed early, defaults kept");
}

/// The window's close request (Alt+F4, Cmd+W): the first run keeps its defaults, Settings closes.
pub fn on_close_request(app: &mut App) {
    if takes_shortcut(app) {
        skip_first_run(app);
    } else {
        close(app);
    }
}

/// Commits the place step's choice (on Continue).
fn commit_place(app: &mut App) {
    let p = match (
        app.settings.want_windows,
        app.settings.locate,
        &app.settings.city,
    ) {
        (true, Locate::Found { lat, lng }, _) => Place::Windows { lat, lng },
        (_, _, Some(c)) => Place::City { name: c.clone() },
        _ => Place::default(),
    };
    if p != app.settings.prefs.place {
        set_place(app, p);
    }
}

/// The window's edge radius for its screen (the window's own rounding and the lip along it).
pub fn edge_radius(screen: Screen) -> f32 {
    if matches!(screen, Screen::FirstRun(_)) {
        20.0
    } else {
        16.0
    }
}

pub(crate) fn view(app: &App) -> View<'_> {
    let s = &app.settings;
    let screen = s.screen.unwrap_or(Screen::Settings(Section::General));
    let (lat, lng) = shown_coords(app);
    let (times, _) = prayer::compute_times(&s.prefs, (lat, lng), prayer::today());
    let names = app_core::prayer::Prayer::ALL;
    let times = names.map(|p| (p.name(), app_core::prayer::hhmm(times.of(p))));
    let b = app_core::qibla::bearing(lat, lng);
    let qibla = format!(
        "{:.0}° {} · {} km",
        b,
        app_core::qibla::compass(b),
        m::thousands(app_core::qibla::distance_km(lat, lng).round() as u64)
    );
    let (y, mo, d) = prayer::today();
    let day = m::day_words(weekday(y, mo, d), d, mo);
    let sources = SOURCES
        .iter()
        .map(|(src, what)| {
            let muted = notices::source_state(app, *src);
            SourceRow {
                source: *src,
                on: muted.is_none(),
                words: muted
                    .map(|w| format!("Muted {w}"))
                    .unwrap_or_else(|| (*what).to_string()),
            }
        })
        .collect();
    View {
        screen,
        prefs: &s.prefs,
        windows_dark: settings_sys::system_dark(),
        startup: s.startup,
        fr_start: s.fr_start,
        capture: s.capture,
        taken: s.taken,
        hotkey_ok: s.hotkey_ok,
        suggestion: s.suggestion,
        tried: s.tried,
        want_windows: s.want_windows,
        locate: s.locate,
        city: s.city.clone(),
        picking: s.picking.then(|| CityPick {
            field: &s.field,
            rows: &s.rows,
            sel: s.sel,
            scroll: s.list_scroll,
            current: if s.want_windows {
                None
            } else {
                s.city.as_deref()
            },
        }),
        day,
        times,
        qibla,
        sources,
        data_dir: s.dir.display().to_string(),
        scroll: s.scroll,
        hover: s.hover,
        focus: s.focus,
    }
}

/// Day of the week, 0 = Sunday (Sakamoto's method).
fn weekday(y: i32, m: u32, d: u32) -> u32 {
    const T: [i32; 12] = [0, 3, 2, 5, 0, 3, 5, 1, 4, 6, 2, 4];
    let y = if m < 3 { y - 1 } else { y };
    ((y + y / 4 - y / 100 + y / 400 + T[(m - 1) as usize] + d as i32).rem_euclid(7)) as u32
}

/// Every control's rectangle in Tab order (logical px), and the content's bottom.
pub(crate) fn layout(app: &App) -> (Vec<(Hit, sp::Rect)>, f32) {
    sp::layout(&app.theme, &app.fonts, &view(app))
}

/// The control at a point in the window (logical px, y down).
pub(crate) fn hit(app: &App, x: f32, y: f32) -> Option<Hit> {
    let (hits, _) = layout(app);
    sp::hit_at(&hits, x, y)
}

/// Empty space that drags the window: the first run's card, the Settings title band.
pub(crate) fn drags_at(app: &App, x: f32, y: f32) -> bool {
    if hit(app, x, y).is_some() {
        return false;
    }
    match app.settings.screen {
        Some(Screen::FirstRun(_)) => !app.settings.picking || y < 80.0,
        Some(Screen::Settings(_)) => y < 60.0,
        None => false,
    }
}

/// Draws the window now (the system's half sizes its surface first).
pub fn render(app: &mut App) {
    if app.settings.screen.is_none() {
        return;
    }
    settings_sys::prepare(app);
    let v = view(app);
    settings_sys::paint(app, &v);
}

// ---- input ----

pub(crate) fn click(app: &mut App, h: Hit) {
    log_line(app, &format!("settings: click {h:?}"));
    match h {
        Hit::Section(s) => go_section(app, s),
        Hit::Close => close(app),
        Hit::StartWithWindows => {
            if matches!(app.settings.screen, Some(Screen::FirstRun(_))) {
                app.settings.fr_start = !app.settings.fr_start;
            } else {
                let on = app.settings.startup == Startup::Off;
                write_startup(app, on);
            }
        }
        Hit::ShowAtStart => {
            app.settings.prefs.show_orb_at_start = !app.settings.prefs.show_orb_at_start;
            save(app);
        }
        Hit::Theme(mode) => {
            if app.settings.prefs.theme != mode {
                app.settings.prefs.theme = mode;
                save(app);
                apply_theme(app);
            }
        }
        Hit::ChangeShortcut | Hit::PickAnother => begin_capture(app),
        Hit::CaptureCancel => cancel_capture(app),
        Hit::CaptureSave => save_capture(app),
        Hit::CaptureReset => {
            if let Some(c) = app.settings.capture.as_mut() {
                c.got = Some(Shortcut::DEFAULT);
                c.needs_modifier = false;
            }
        }
        Hit::UseSuggestion => {
            if let Some(s) = app.settings.suggestion {
                if set_shortcut(app, s) {
                    app.settings.suggestion = None;
                    app.settings.tried = false;
                }
            }
        }
        Hit::StartupApps => settings_sys::open_startup_settings(),
        Hit::LocationSettings => settings_sys::open_location_settings(),
        Hit::PlaceWindows => choose_windows(app),
        Hit::PlaceCity => open_picker(app),
        Hit::CityBack => app.settings.picking = false,
        Hit::CityRow(i) => pick_city(app, i),
        Hit::Reminder(i) => {
            toggle_reminder(app, i);
            return;
        }
        Hit::Method(mm) => {
            if app.settings.prefs.prayer_method != mm {
                app.settings.prefs.prayer_method = mm;
                save(app);
                log_line(app, &format!("settings: prayer method {}", mm.name()));
                prayer::apply_place(app);
            }
        }
        Hit::Adjust(i, step) => {
            let a = &mut app.settings.prefs.prayer_adjust[i.min(4)];
            let v = (*a + step).clamp(-m::ADJUST_MAX, m::ADJUST_MAX);
            if v != *a {
                *a = v;
                save(app);
                prayer::apply_place(app);
            }
        }
        Hit::AtTime(s) => {
            if app.settings.prefs.at_time_sound != s {
                app.settings.prefs.at_time_sound = s;
                save(app);
                // A choice away from Adzan silences one playing now.
                if s != m::AtTimeSound::Adzan {
                    prayer::stop_adzan(app);
                }
            }
        }
        Hit::PickAdzan(subuh) => settings_sys::pick_adzan(app, subuh),
        Hit::ClearAdzanSubuh => {
            app.settings.prefs.adzan_subuh_file = None;
            save(app);
        }
        Hit::Source(s) => {
            let on = notices::source_state(app, s).is_some();
            notices::set_source_on(app, s, on);
        }
        Hit::OpenData => {
            let dir = app.settings.dir.clone();
            settings_sys::open_folder(&dir);
        }
        Hit::Next => {
            if let Some(Screen::FirstRun(step)) = app.settings.screen {
                if step == Step::Place {
                    if !m::place_ready(app.settings.want_windows, app.settings.locate) {
                        return;
                    }
                    commit_place(app);
                }
                go_step(app, step.next());
                return;
            }
        }
        Hit::Back => {
            if let Some(Screen::FirstRun(step)) = app.settings.screen {
                go_step(app, step.back());
                return;
            }
        }
        Hit::Finish => {
            finish_first_run(app);
            return;
        }
    }
    render(app);
}

/// A click (button up) at a point in the window: the control under it, else nothing; the Tab
/// ring goes away either way.
pub(crate) fn click_at(app: &mut App, x: f32, y: f32) {
    let h = hit(app, x, y);
    app.settings.focus = None;
    match h {
        Some(h) => click(app, h),
        None => render(app),
    }
}

/// The pointer moved over the window (None: it left).
pub(crate) fn hover_at(app: &mut App, at: Option<(f32, f32)>) {
    let h = at.and_then(|(x, y)| hit(app, x, y));
    if h != app.settings.hover {
        app.settings.hover = h;
        render(app);
    }
}

/// One reminder on or off, from the Settings switch or the Prayer panel chip: both show the new
/// value at once, whichever of the two windows is open.
pub fn toggle_reminder(app: &mut App, i: usize) {
    let r = &mut app.settings.prefs.prayer_reminders;
    r[i] = !r[i];
    save(app);
    prayer::schedule_reminders(app);
    panels::refresh(app);
    render(app);
}

/// The adzan file picker closed: keep the path (the file stays where you have it; nothing is
/// copied or uploaded).
#[cfg_attr(not(windows), allow(dead_code))] // the Mac offers no adzan file yet
pub fn on_adzan_picked(app: &mut App, subuh: bool, path: Option<String>) {
    let Some(path) = path else {
        return;
    };
    if !m::is_audio(&path) {
        log_line(app, "settings: adzan file is not mp3 or wav, ignored");
        return;
    }
    log_line(
        app,
        &format!(
            "settings: adzan{} file {}",
            if subuh { " (Subuh)" } else { "" },
            m::file_name(&path)
        ),
    );
    if subuh {
        app.settings.prefs.adzan_subuh_file = Some(path);
    } else {
        app.settings.prefs.adzan_file = Some(path);
    }
    save(app);
    render(app);
}

/// The notice mutes changed somewhere: an open Notices section redraws its switches.
pub fn on_sources_changed(app: &mut App) {
    if app.settings.screen == Some(Screen::Settings(Section::Notices)) {
        render(app);
    }
}

/// Tab and Shift+Tab move the ring through the controls in reading order.
fn move_focus(app: &mut App, back: bool) {
    let (hits, _) = layout(app);
    if hits.is_empty() {
        return;
    }
    let n = hits.len();
    let cur = app
        .settings
        .focus
        .and_then(|f| hits.iter().position(|(h, _)| *h == f));
    let i = match (cur, back) {
        (None, false) => 0,
        (None, true) => n - 1,
        (Some(i), false) => (i + 1) % n,
        (Some(i), true) => (i + n - 1) % n,
    };
    app.settings.focus = Some(hits[i].0);
    // Keep the focused control in view.
    if let (Some(Screen::Settings(_)), false) = (app.settings.screen, app.settings.picking) {
        let r = hits[i].1;
        if r.1 < sp::CONTENT_TOP {
            app.settings.scroll = (app.settings.scroll - (sp::CONTENT_TOP - r.1) - 12.0).max(0.0);
        } else if r.3 > sp::CONTENT_BOTTOM {
            app.settings.scroll += r.3 - sp::CONTENT_BOTTOM + 12.0;
            clamp_scroll(app);
        }
    }
}

fn clamp_scroll(app: &mut App) {
    let (_, bottom) = layout(app);
    let max = (bottom - sp::CONTENT_BOTTOM).max(0.0);
    app.settings.scroll = app.settings.scroll.clamp(0.0, max);
}

/// Keys in the window (Windows virtual keys on both systems). Returns whether they were used.
/// Alt combinations (`alt_sys`, WM_SYSKEYDOWN) only count while a shortcut is being chosen;
/// otherwise Alt+Tab and Alt+F4 stay with the system.
pub(crate) fn on_key(app: &mut App, k: u16, down: bool, alt_sys: bool) -> bool {
    if let Some(mut c) = app.settings.capture {
        let r = c.key(k as u32, down);
        app.settings.capture = Some(c);
        match r {
            CaptureKey::Cancel => cancel_capture(app),
            CaptureKey::Ignored if down && k == vk::VK_RETURN && c.got.is_some() => {
                save_capture(app)
            }
            _ => {}
        }
        render(app);
        return true;
    }
    if !down || alt_sys {
        return false;
    }
    let ctrl = sys::ctrl_down();
    let shift = sys::shift_down();
    if app.settings.picking {
        let n = app.settings.rows.len();
        match k {
            vk::VK_ESCAPE => app.settings.picking = false,
            vk::VK_UP if n > 0 => {
                app.settings.sel = app.settings.sel.saturating_sub(1);
                keep_sel_in_view(app);
            }
            vk::VK_DOWN if n > 0 => {
                app.settings.sel = (app.settings.sel + 1).min(n - 1);
                keep_sel_in_view(app);
            }
            vk::VK_RETURN => {
                if let Some(i) = app.settings.rows.get(app.settings.sel).copied() {
                    pick_city(app, i);
                }
            }
            vk::VK_LEFT | vk::VK_RIGHT | vk::VK_HOME | vk::VK_END | vk::VK_BACK | vk::VK_DELETE => {
                let key = match k {
                    vk::VK_LEFT => Key::Left,
                    vk::VK_RIGHT => Key::Right,
                    vk::VK_HOME => Key::Home,
                    vk::VK_END => Key::End,
                    vk::VK_BACK => Key::Backspace,
                    _ => Key::Delete,
                };
                app.settings.field.key(key, ctrl, shift);
                on_field_changed(app);
            }
            _ => return false,
        }
        render(app);
        return true;
    }
    match k {
        vk::VK_TAB => move_focus(app, shift),
        vk::VK_RETURN | vk::VK_SPACE => {
            if let Some(h) = app.settings.focus {
                click(app, h);
                return true;
            }
            if k == vk::VK_RETURN {
                // Enter without a ring takes the step's main button.
                if let Some(Screen::FirstRun(step)) = app.settings.screen {
                    click(
                        app,
                        if step == Step::Done {
                            Hit::Finish
                        } else {
                            Hit::Next
                        },
                    );
                    return true;
                }
            }
            return false;
        }
        vk::VK_ESCAPE => match app.settings.screen {
            Some(Screen::FirstRun(step)) if step != Step::Welcome => {
                go_step(app, step.back());
                return true;
            }
            Some(Screen::FirstRun(_)) => return true,
            _ => {
                close(app);
                return true;
            }
        },
        vk::VK_UP | vk::VK_DOWN if matches!(app.settings.focus, Some(Hit::Section(_))) => {
            let Some(Hit::Section(cur)) = app.settings.focus else {
                return false;
            };
            let i = Section::ALL.iter().position(|s| *s == cur).unwrap_or(0);
            let n = Section::ALL.len();
            let next = Section::ALL[if k == vk::VK_UP {
                (i + n - 1) % n
            } else {
                (i + 1) % n
            }];
            app.settings.focus = Some(Hit::Section(next));
            go_section(app, next);
            return true;
        }
        vk::VK_UP | vk::VK_DOWN | vk::VK_NEXT | vk::VK_PRIOR => {
            let d = match k {
                vk::VK_UP => -40.0,
                vk::VK_DOWN => 40.0,
                vk::VK_PRIOR => -300.0,
                _ => 300.0,
            };
            app.settings.scroll += d;
            clamp_scroll(app);
        }
        _ => return false,
    }
    render(app);
    true
}

/// One UTF-16 unit from WM_CHAR.
#[cfg_attr(not(windows), allow(dead_code))] // the Mac types whole strings (on_text)
pub(crate) fn on_char(app: &mut App, unit: u16) {
    if app.settings.picking && app.settings.field.type_utf16(unit) {
        on_field_changed(app);
        render(app);
    }
}

/// Committed text (the Mac's input system: a key, an accent, a paste from the character viewer).
#[cfg_attr(windows, allow(dead_code))] // Windows types through WM_CHAR (on_char)
pub(crate) fn on_text(app: &mut App, s: &str) {
    if app.settings.picking && !s.is_empty() {
        app.settings.field.insert(s);
        on_field_changed(app);
        render(app);
    }
}

/// The wheel: `delta` as Windows gives it (+120 a notch away from you, which scrolls up).
pub(crate) fn on_wheel(app: &mut App, delta: i16) {
    if app.settings.picking {
        let fit = sp::city_rows_fit(matches!(app.settings.screen, Some(Screen::FirstRun(_))));
        let n = app.settings.rows.len();
        let s = &mut app.settings;
        s.list_scroll = if delta > 0 {
            s.list_scroll.saturating_sub(3)
        } else {
            (s.list_scroll + 3).min(n.saturating_sub(fit))
        };
    } else if matches!(app.settings.screen, Some(Screen::Settings(_))) {
        app.settings.scroll -= delta as f32 / 120.0 * 48.0;
        clamp_scroll(app);
    } else {
        return;
    }
    app.settings.hover = None;
    render(app);
}

/// The window became active or went to the background. Clicking away while choosing a shortcut
/// gives the old one back at once. Coming back (from Startup apps, say) reads the startup entry
/// again.
pub(crate) fn on_activate(app: &mut App, active: bool) {
    if !active && app.settings.capture.is_some() {
        cancel_capture(app);
        render(app);
    } else if active {
        refresh_startup(app);
    }
}

/// One line for tests: the screen, theme, shortcut, place, method and startup.
#[cfg_attr(windows, allow(dead_code))] // the Mac's test channel
pub(crate) fn state_line(app: &App) -> String {
    let s = &app.settings;
    let screen = match s.screen {
        None => "closed".to_string(),
        Some(Screen::Settings(sec)) => sec.name().to_string(),
        Some(Screen::FirstRun(step)) => format!("first run {} of 5", step.index() + 1),
    };
    format!(
        "{screen}, theme {:?} (dark {}), shortcut {} (registered {}), capture {}, place {}, picking {} \"{}\", method {}, reminders {:?}, sound {:?}, startup {:?}, scroll {:.0}",
        s.prefs.theme,
        app.theme.dark,
        s.prefs.shortcut.label(),
        s.hotkey_ok,
        s.capture
            .map_or("none".into(), |c| c.got.map_or("waiting".into(), |g| g.label())),
        s.prefs.place.words(),
        s.picking,
        s.field.text(),
        s.prefs.prayer_method.name(),
        s.prefs.prayer_reminders,
        s.prefs.at_time_sound,
        s.startup,
        s.scroll,
    )
}

// ---- the dial and pills ----

pub fn dial_child(app: &App) -> DialChild {
    let _ = app;
    DialChild {
        text: String::new(),
        icon: Some(Icon::Gear),
        name: "Settings".into(),
        value: "General".into(),
    }
}

#[cfg(test)]
mod tests {
    use super::weekday;

    #[test]
    fn weekdays_are_right() {
        assert_eq!(weekday(2026, 11, 7), 6); // Saturday
        assert_eq!(weekday(2026, 11, 6), 5);
        assert_eq!(weekday(2027, 1, 1), 5);
        assert_eq!(weekday(2028, 2, 29), 2);
    }
}
