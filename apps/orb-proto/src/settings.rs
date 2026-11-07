//! The Settings window and the first run. settings_model.rs holds the values, settings_panel.rs
//! draws; this file keeps `settings.json` (shared with the calendar link, each writer keeps the
//! other's keys), registers the global shortcut, writes the HKCU Run entry, asks Windows location
//! on a worker thread, and applies every change at once: the theme, the shortcut, the place and
//! the prayer reminders, the notice mutes.

use std::path::{Path, PathBuf};
use std::sync::mpsc::{channel, Receiver};

use serde_json::{Map, Value};
use windows::core::{w, HSTRING, PCWSTR};
use windows::Devices::Geolocation::{GeolocationAccessStatus, Geolocator};
use windows::Foundation::TimeSpan;
use windows::Win32::Foundation::*;
use windows::Win32::Graphics::Gdi::*;
use windows::Win32::System::Registry::*;
use windows::Win32::UI::Input::KeyboardAndMouse::*;
use windows::Win32::UI::Shell::ShellExecuteW;
use windows::Win32::UI::WindowsAndMessaging::*;

use crate::gfx::Surface;
use crate::hook::unpack;
use crate::notices::{self, Notice, Priority, Source};
use crate::orb;
use crate::settings_model::{
    self as m, Capture, CaptureKey, Locate, Place, Prefs, Section, Shortcut, Startup, Step,
};
use crate::settings_panel::{self as sp, CityPick, Hit, Screen, SourceRow, View};
use crate::textfield::{Key, TextField};
use crate::ui::{self, DialChild, Icon, Pill, PillAction};
use crate::{
    dial, log_line, panels, pill, prayer, px, with_app, App, HOTKEY_PROBE, HOTKEY_TOGGLE,
    MSG_MOUSELEAVE, WM_APP_LOCATE,
};

const SETTINGS: &str = "settings.json";
const RUN: PCWSTR = w!("Software\\Microsoft\\Windows\\CurrentVersion\\Run");
const APPROVED: PCWSTR =
    w!("Software\\Microsoft\\Windows\\CurrentVersion\\Explorer\\StartupApproved\\Run");
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
    pub hwnd: HWND,
    surface: Option<Surface>,
    /// What the window shows; None while it is hidden.
    pub screen: Option<Screen>,
    hover: Option<Hit>,
    focus: Option<Hit>,
    tracking: bool,
    scroll: f32,
    capture: Option<Capture>,
    taken: Option<Shortcut>,
    /// The shortcut in use is registered (another app may hold it).
    pub hotkey_ok: bool,
    suggestion: Option<Shortcut>,
    /// The first run's "try it": the shortcut was pressed on the shortcut step.
    tried: bool,
    want_windows: bool,
    locate: Locate,
    loc_rx: Option<Receiver<Locate>>,
    /// The radio's city: the first run's draft, or the city in use.
    city: Option<String>,
    picking: bool,
    field: TextField,
    rows: Vec<usize>,
    sel: usize,
    list_scroll: usize,
    /// The first run's Start with Windows, written at Finish.
    fr_start: bool,
    startup: Startup,
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
    pub fn load(dir: &Path, hwnd: HWND) -> Settings {
        let prefs = Prefs::from_map(&read_map(dir));
        let city = match &prefs.place {
            Place::City { name } => Some(name.clone()),
            Place::Windows { .. } => None,
        };
        Settings {
            want_windows: prefs.place.is_windows(),
            prefs,
            dir: dir.to_path_buf(),
            hwnd,
            surface: None,
            screen: None,
            hover: None,
            focus: None,
            tracking: false,
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
            fr_start: true,
            startup: Startup::Off,
            undo_shortcut: None,
        }
    }
}

fn save(app: &mut App) {
    let p = app.settings.prefs.clone();
    update_map(&app.settings.dir, |m| p.write_into(m));
}

/// The theme the app draws with, from the setting and Windows.
pub fn theme_now(prefs: &Prefs) -> ui::Theme {
    ui::theme(prefs.theme.is_dark(ui::windows_dark()))
}

// ---- launch ----

/// After the windows exist: the shortcut, the first run or the orb's start state, and a fresh
/// Windows location when the place follows it.
pub fn start(app: &mut App) {
    let s = app.settings.prefs.shortcut;
    app.settings.hotkey_ok = register(app, s);
    log_line(
        app,
        &format!(
            "settings: shortcut {} registered={}",
            s.label(),
            app.settings.hotkey_ok
        ),
    );
    if !app.settings.prefs.first_run_done {
        // At sign-in the welcome must not pull the keyboard out of whatever you already opened.
        first_run(app, false);
        return;
    }
    if !app.settings.hotkey_ok {
        let p = Pill {
            icon: Icon::Keyboard,
            dot: false,
            title: format!("{} is taken", s.label()),
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
    if app.settings.prefs.place.is_windows() {
        locate(app);
    }
}

// ---- the shortcut ----

fn register(app: &App, s: Shortcut) -> bool {
    unsafe {
        RegisterHotKey(
            Some(app.hwnd),
            HOTKEY_TOGGLE,
            HOT_KEY_MODIFIERS(s.mods()) | MOD_NOREPEAT,
            s.vk,
        )
        .is_ok()
    }
}

fn unregister(app: &App) {
    unsafe {
        let _ = UnregisterHotKey(Some(app.hwnd), HOTKEY_TOGGLE);
    }
}

/// Whether another app holds `s`: register it under a spare id and let it go again.
fn is_free(app: &App, s: Shortcut) -> bool {
    unsafe {
        let ok = RegisterHotKey(
            Some(app.hwnd),
            HOTKEY_PROBE,
            HOT_KEY_MODIFIERS(s.mods()) | MOD_NOREPEAT,
            s.vk,
        )
        .is_ok();
        if ok {
            let _ = UnregisterHotKey(Some(app.hwnd), HOTKEY_PROBE);
        }
        ok
    }
}

/// Moves the shortcut to `s`. On failure the old one stays (registered again) and the card says
/// which one is taken.
fn set_shortcut(app: &mut App, s: Shortcut) -> bool {
    let old = app.settings.prefs.shortcut;
    unregister(app);
    if register(app, s) {
        app.settings.prefs.shortcut = s;
        app.settings.hotkey_ok = true;
        app.settings.taken = None;
        save(app);
        log_line(app, &format!("settings: shortcut now {}", s.label()));
        return true;
    }
    app.settings.hotkey_ok = register(app, old);
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
    unregister(app);
    take_foreground(app);
}

fn cancel_capture(app: &mut App) {
    if app.settings.capture.take().is_some() {
        let s = app.settings.prefs.shortcut;
        app.settings.hotkey_ok = register(app, s);
    }
}

fn save_capture(app: &mut App) {
    let Some(s) = app.settings.capture.and_then(|c| c.got) else {
        return;
    };
    app.settings.capture = None;
    let old = app.settings.prefs.shortcut;
    if s == old {
        app.settings.hotkey_ok = register(app, s);
        return;
    }
    // The old shortcut was let go for the capture; set_shortcut registers one of the two.
    app.settings.hotkey_ok = register(app, old);
    if set_shortcut(app, s) {
        app.settings.tried = false;
        app.settings.undo_shortcut = Some(old);
        if app.visible && matches!(app.settings.screen, Some(Screen::Settings(_))) {
            let p = Pill {
                icon: Icon::Keyboard,
                dot: false,
                title: "Shortcut".into(),
                value: format!("now {}", s.label()),
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

pub fn shortcut_label(app: &App) -> String {
    app.settings.prefs.shortcut.label()
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

// ---- start with Windows ----

fn run_name() -> HSTRING {
    // A test drive names its own entry, so the real one is never touched.
    HSTRING::from(std::env::var("ORB_RUN_VALUE").unwrap_or_else(|_| "orb-proto".into()))
}

fn read_startup() -> Startup {
    let name = run_name();
    unsafe {
        let mut size = 0u32;
        let present = RegGetValueW(
            HKEY_CURRENT_USER,
            RUN,
            &name,
            RRF_RT_REG_SZ,
            None,
            None,
            Some(&mut size),
        )
        .is_ok();
        if !present {
            return Startup::Off;
        }
        let mut buf = [0u8; 12];
        let mut n = buf.len() as u32;
        let approved = RegGetValueW(
            HKEY_CURRENT_USER,
            APPROVED,
            &name,
            RRF_RT_REG_BINARY,
            None,
            Some(buf.as_mut_ptr() as *mut _),
            Some(&mut n),
        )
        .is_ok();
        // Task Manager marks a switched-off entry with an odd first byte (03, 07).
        if approved && n > 0 && buf[0] & 1 == 1 {
            Startup::Blocked
        } else {
            Startup::On
        }
    }
}

fn write_startup(app: &mut App, on: bool) {
    let name = run_name();
    let ok = unsafe {
        if on {
            let exe = std::env::current_exe().unwrap_or_default();
            let cmd: Vec<u16> = format!("\"{}\"", exe.display())
                .encode_utf16()
                .chain(std::iter::once(0))
                .collect();
            RegSetKeyValueW(
                HKEY_CURRENT_USER,
                RUN,
                &name,
                REG_SZ.0,
                Some(cmd.as_ptr() as *const _),
                (cmd.len() * 2) as u32,
            )
            .is_ok()
        } else {
            let r = RegDeleteKeyValueW(HKEY_CURRENT_USER, RUN, &name);
            r.is_ok() || r == ERROR_FILE_NOT_FOUND
        }
    };
    app.settings.startup = read_startup();
    log_line(
        app,
        &format!(
            "settings: start with Windows {} (ok={ok}, now {:?})",
            if on { "on" } else { "off" },
            app.settings.startup
        ),
    );
}

// ---- the place ----

fn locate(app: &mut App) {
    if app.settings.locate == Locate::Finding {
        return;
    }
    app.settings.locate = Locate::Finding;
    let (tx, rx) = channel();
    app.settings.loc_rx = Some(rx);
    let hwnd = app.hwnd.0 as isize;
    std::thread::spawn(move || {
        let out = ask_windows();
        let _ = tx.send(out);
        unsafe {
            let _ = PostMessageW(
                Some(HWND(hwnd as *mut _)),
                WM_APP_LOCATE,
                WPARAM(0),
                LPARAM(0),
            );
        }
    });
    log_line(app, "settings: asking Windows location");
}

/// Windows location through the WinRT Geolocator: a fix up to 10 min old, or a new one within 10 s.
fn ask_windows() -> Locate {
    let r = (|| -> windows::core::Result<Locate> {
        if Geolocator::RequestAccessAsync()?.join()? != GeolocationAccessStatus::Allowed {
            return Ok(Locate::Off);
        }
        let g = Geolocator::new()?;
        let ten_min = TimeSpan {
            Duration: 10 * 60 * 10_000_000,
        };
        let ten_s = TimeSpan {
            Duration: 10 * 10_000_000,
        };
        let pos = g
            .GetGeopositionAsyncWithAgeAndTimeout(ten_min, ten_s)?
            .join()?;
        let p = pos.Coordinate()?.Point()?.Position()?;
        Ok(Locate::Found {
            lat: p.Latitude,
            lng: p.Longitude,
        })
    })();
    match r {
        Ok(l) => l,
        Err(e) if e.code() == E_ACCESSDENIED => Locate::Off,
        Err(_) => Locate::Failed,
    }
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
    take_foreground(app);
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

// ---- the theme ----

pub fn apply_theme(app: &mut App) {
    app.theme = theme_now(&app.settings.prefs);
    orb::render_orb(app);
    if app.pill.is_some() {
        pill::render_pill(app);
    }
    panels::redraw(app);
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

/// WM_SETTINGCHANGE "ImmersiveColorSet": Windows switched between light and dark.
pub fn on_system_theme(app: &mut App) {
    if app.settings.prefs.theme == m::ThemeMode::Windows && ui::windows_dark() != app.theme.dark {
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
        take_foreground(app);
        return;
    }
    dial::close_dial(app);
    panels::close_panel(app);
    if let Some(Screen::Settings(_)) = app.settings.screen {
        if app.settings.screen != Some(Screen::Settings(section)) {
            go_section(app, section);
        }
        take_foreground(app);
        return;
    }
    app.settings.startup = read_startup();
    app.settings.want_windows = app.settings.prefs.place.is_windows();
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
    s.want_windows = true;
    s.city = match &s.prefs.place {
        Place::City { name } => Some(name.clone()),
        Place::Windows { .. } => Some("Yogyakarta".into()),
    };
    s.fr_start = true;
    s.tried = false;
    s.picking = false;
    s.taken = None;
    s.capture = None;
    s.suggestion = None;
    if !app.settings.hotkey_ok {
        app.settings.suggestion = m::SUGGESTIONS.iter().copied().find(|k| is_free(app, *k));
    }
    show(app, Screen::FirstRun(Step::Welcome), activate);
    log_line(app, "settings: first run");
}

fn show(app: &mut App, screen: Screen, activate: bool) {
    app.settings.screen = Some(screen);
    app.settings.hover = None;
    app.settings.focus = None;
    app.settings.scroll = 0.0;
    place(app);
    render(app);
    unsafe {
        let _ = ShowWindow(
            app.settings.hwnd,
            if activate { SW_SHOW } else { SW_SHOWNA },
        );
    }
    if activate {
        take_foreground(app);
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
        app.settings.startup = read_startup();
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

pub fn close(app: &mut App) {
    if app.settings.screen.is_none() {
        return;
    }
    cancel_capture(app);
    app.settings.screen = None;
    app.settings.picking = false;
    unsafe {
        let _ = ShowWindow(app.settings.hwnd, SW_HIDE);
    }
    // The swap chain is 3 MB at 720 × 580; let it go while the window is hidden.
    app.settings.surface = None;
    log_line(app, "settings: closed");
}

fn finish_first_run(app: &mut App) {
    let on = app.settings.fr_start;
    if on || read_startup() != Startup::Off {
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
        title: "[App] is ready".into(),
        value: label,
        action: None,
        closable: true,
        ttl_ms: 6000,
    };
    notices::post(app, Notice::new(Source::App, Priority::Normal, p));
    log_line(app, "settings: first run done");
}

/// Alt+F4 on the first run: the defaults stay (the place in use, no startup entry) and the orb
/// comes out, so closing it never leaves the app with nothing on screen.
fn skip_first_run(app: &mut App) {
    app.settings.prefs.first_run_done = true;
    save(app);
    close(app);
    orb::set_visible(app, true);
    notices::on_user_visibility(app);
    log_line(app, "settings: first run closed early, defaults kept");
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

fn place(app: &App) {
    let Some(screen) = app.settings.screen else {
        return;
    };
    let (w, h) = sp::size(screen);
    let (w, h) = (px(app, w).ceil() as i32, px(app, h).ceil() as i32);
    let wa = orb::work_area(app.hwnd);
    let x = (wa.left + wa.right - w) / 2;
    let y = (wa.top + wa.bottom - h) / 2;
    unsafe {
        let _ = SetWindowPos(
            app.settings.hwnd,
            None,
            x,
            y,
            w,
            h,
            SWP_NOACTIVATE | SWP_NOZORDER,
        );
        let d = px(
            app,
            if matches!(screen, Screen::FirstRun(_)) {
                40.0
            } else {
                32.0
            },
        )
        .round() as i32;
        let rgn = CreateRoundRectRgn(0, 0, w + 1, h + 1, d, d);
        let _ = SetWindowRgn(app.settings.hwnd, Some(rgn), true);
    }
}

fn take_foreground(app: &App) {
    unsafe {
        let _ = SetForegroundWindow(app.settings.hwnd);
        let _ = SetFocus(Some(app.settings.hwnd));
    }
}

fn view(app: &App) -> View<'_> {
    let s = &app.settings;
    let screen = s.screen.unwrap_or(Screen::Settings(Section::General));
    let (lat, lng) = shown_coords(app);
    let (times, _) = prayer::compute_times((lat, lng), prayer::today());
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
        windows_dark: ui::windows_dark(),
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

fn layout(app: &App) -> (Vec<(Hit, sp::Rect)>, f32) {
    sp::layout(&app.g, &app.theme, &app.fonts, &view(app))
}

pub fn render(app: &mut App) {
    let Some(screen) = app.settings.screen else {
        return;
    };
    let (lw, lh) = sp::size(screen);
    let (w, h) = (px(app, lw).ceil() as u32, px(app, lh).ceil() as u32);
    match app.settings.surface.as_mut() {
        None => {
            app.settings.surface = Surface::new(&app.g, app.settings.hwnd, w, h, app.scale).ok()
        }
        Some(s) if s.w != w || s.h != h => {
            let _ = s.resize(&app.g, w, h);
            place(app);
        }
        _ => {}
    }
    let v = view(app);
    let Some(s) = app.settings.surface.as_ref() else {
        return;
    };
    let dc = s.begin(&app.g);
    sp::draw(&app.g, dc, &app.theme, &app.fonts, &v);
    s.end(&app.g);
}

// ---- input ----

fn click(app: &mut App, h: Hit) {
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
        Hit::StartupApps => open_uri("ms-settings:startupapps"),
        Hit::LocationSettings => open_uri("ms-settings:privacy-location"),
        Hit::PlaceWindows => choose_windows(app),
        Hit::PlaceCity => open_picker(app),
        Hit::CityBack => app.settings.picking = false,
        Hit::CityRow(i) => pick_city(app, i),
        Hit::Reminder(i) => {
            toggle_reminder(app, i);
            return;
        }
        Hit::Source(s) => {
            let on = notices::source_state(app, s).is_some();
            notices::set_source_on(app, s, on);
        }
        Hit::OpenData => {
            let dir = app.settings.dir.display().to_string();
            open_uri(&dir);
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

/// The notice mutes changed somewhere: an open Notices section redraws its switches.
pub fn on_sources_changed(app: &mut App) {
    if app.settings.screen == Some(Screen::Settings(Section::Notices)) {
        render(app);
    }
}

fn open_uri(s: &str) {
    unsafe {
        ShellExecuteW(
            None,
            w!("open"),
            &HSTRING::from(s),
            None,
            None,
            SW_SHOWNORMAL,
        );
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

/// Keys in the window. Returns whether they were used. Alt combinations (`sys`) only count while
/// a shortcut is being chosen; otherwise Alt+Tab and Alt+F4 stay with Windows.
fn on_key(app: &mut App, vk: u16, down: bool, sys: bool) -> bool {
    let k = VIRTUAL_KEY(vk);
    if let Some(mut c) = app.settings.capture {
        let r = c.key(vk as u32, down);
        app.settings.capture = Some(c);
        match r {
            CaptureKey::Cancel => cancel_capture(app),
            CaptureKey::Ignored if down && k == VK_RETURN && c.got.is_some() => save_capture(app),
            _ => {}
        }
        render(app);
        return true;
    }
    if !down || sys {
        return false;
    }
    let ctrl = unsafe { GetKeyState(VK_CONTROL.0 as i32) } < 0;
    let shift = unsafe { GetKeyState(VK_SHIFT.0 as i32) } < 0;
    if app.settings.picking {
        let n = app.settings.rows.len();
        match k {
            VK_ESCAPE => app.settings.picking = false,
            VK_UP if n > 0 => {
                app.settings.sel = app.settings.sel.saturating_sub(1);
                keep_sel_in_view(app);
            }
            VK_DOWN if n > 0 => {
                app.settings.sel = (app.settings.sel + 1).min(n - 1);
                keep_sel_in_view(app);
            }
            VK_RETURN => {
                if let Some(i) = app.settings.rows.get(app.settings.sel).copied() {
                    pick_city(app, i);
                }
            }
            VK_LEFT | VK_RIGHT | VK_HOME | VK_END | VK_BACK | VK_DELETE => {
                let key = match k {
                    VK_LEFT => Key::Left,
                    VK_RIGHT => Key::Right,
                    VK_HOME => Key::Home,
                    VK_END => Key::End,
                    VK_BACK => Key::Backspace,
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
        VK_TAB => move_focus(app, shift),
        VK_RETURN | VK_SPACE => {
            if let Some(h) = app.settings.focus {
                click(app, h);
                return true;
            }
            if k == VK_RETURN {
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
        VK_ESCAPE => match app.settings.screen {
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
        VK_UP | VK_DOWN if matches!(app.settings.focus, Some(Hit::Section(_))) => {
            let Some(Hit::Section(cur)) = app.settings.focus else {
                return false;
            };
            let i = Section::ALL.iter().position(|s| *s == cur).unwrap_or(0);
            let n = Section::ALL.len();
            let next = Section::ALL[if k == VK_UP {
                (i + n - 1) % n
            } else {
                (i + 1) % n
            }];
            app.settings.focus = Some(Hit::Section(next));
            go_section(app, next);
            return true;
        }
        VK_UP | VK_DOWN | VK_NEXT | VK_PRIOR => {
            let d = match k {
                VK_UP => -40.0,
                VK_DOWN => 40.0,
                VK_PRIOR => -300.0,
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

fn on_char(app: &mut App, unit: u16) {
    if app.settings.picking && app.settings.field.type_utf16(unit) {
        on_field_changed(app);
        render(app);
    }
}

fn on_wheel(app: &mut App, delta: i16) {
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

// ---- the dial and pills ----

pub fn dial_child(app: &App) -> DialChild {
    let _ = app;
    DialChild {
        text: String::new(),
        icon: Some(Icon::Sliders),
        name: "Settings".into(),
        value: "General".into(),
    }
}

pub unsafe extern "system" fn settings_proc(
    hwnd: HWND,
    msg: u32,
    wparam: WPARAM,
    lparam: LPARAM,
) -> LRESULT {
    match msg {
        WM_NCHITTEST => {
            // Empty space drags the window: the first run's card, the Settings title band.
            let (sx, sy) = unpack(lparam);
            let mut p = POINT {
                x: sx as i32,
                y: sy as i32,
            };
            let _ = ScreenToClient(hwnd, &mut p);
            let caption = with_app(|app| {
                let (x, y) = (p.x as f32 / app.scale, p.y as f32 / app.scale);
                let (hits, _) = layout(app);
                if sp::hit_at(&hits, x, y).is_some() {
                    return false;
                }
                match app.settings.screen {
                    Some(Screen::FirstRun(_)) => !app.settings.picking || y < 80.0,
                    Some(Screen::Settings(_)) => y < 60.0,
                    None => false,
                }
            })
            .unwrap_or(false);
            if caption {
                return LRESULT(HTCAPTION as isize);
            }
            return LRESULT(HTCLIENT as isize);
        }
        WM_MOUSEMOVE => {
            with_app(|app| {
                let (x, y) = unpack(lparam);
                let (hits, _) = layout(app);
                let h = sp::hit_at(&hits, x / app.scale, y / app.scale);
                if !app.settings.tracking {
                    let mut tme = TRACKMOUSEEVENT {
                        cbSize: std::mem::size_of::<TRACKMOUSEEVENT>() as u32,
                        dwFlags: TME_LEAVE,
                        hwndTrack: hwnd,
                        dwHoverTime: 0,
                    };
                    let _ = TrackMouseEvent(&mut tme);
                    app.settings.tracking = true;
                }
                if h != app.settings.hover {
                    app.settings.hover = h;
                    render(app);
                }
            });
            return LRESULT(0);
        }
        MSG_MOUSELEAVE => {
            with_app(|app| {
                app.settings.tracking = false;
                if app.settings.hover.is_some() {
                    app.settings.hover = None;
                    render(app);
                }
            });
            return LRESULT(0);
        }
        WM_LBUTTONDOWN => return LRESULT(0),
        WM_LBUTTONUP => {
            with_app(|app| {
                let (x, y) = unpack(lparam);
                let (hits, _) = layout(app);
                app.settings.focus = None;
                if let Some(h) = sp::hit_at(&hits, x / app.scale, y / app.scale) {
                    click(app, h);
                } else {
                    render(app);
                }
            });
            return LRESULT(0);
        }
        WM_MOUSEWHEEL => {
            let delta = ((wparam.0 >> 16) & 0xffff) as u16 as i16;
            with_app(|app| on_wheel(app, delta));
            return LRESULT(0);
        }
        WM_KEYDOWN | WM_SYSKEYDOWN | WM_KEYUP | WM_SYSKEYUP => {
            let down = msg == WM_KEYDOWN || msg == WM_SYSKEYDOWN;
            let sys = msg == WM_SYSKEYDOWN || msg == WM_SYSKEYUP;
            let used = with_app(|app| on_key(app, wparam.0 as u16, down, sys)).unwrap_or(false);
            if used {
                return LRESULT(0);
            }
        }
        WM_SYSCHAR => {
            // Alt+letter while choosing a shortcut must not beep.
            if with_app(|app| app.settings.capture.is_some()).unwrap_or(false) {
                return LRESULT(0);
            }
        }
        WM_CHAR => {
            with_app(|app| on_char(app, wparam.0 as u16));
            return LRESULT(0);
        }
        WM_ACTIVATE => {
            // Clicking away while choosing a shortcut gives the old one back at once. Coming
            // back (from Startup apps, say) reads the Run entry again.
            let active = (wparam.0 & 0xffff) as u32 != WA_INACTIVE;
            with_app(|app| {
                if !active && app.settings.capture.is_some() {
                    cancel_capture(app);
                    render(app);
                } else if active
                    && matches!(
                        app.settings.screen,
                        Some(Screen::Settings(Section::General))
                    )
                {
                    let now = read_startup();
                    if now != app.settings.startup {
                        app.settings.startup = now;
                        render(app);
                    }
                }
            });
        }
        WM_CLOSE => {
            with_app(|app| {
                if takes_shortcut(app) {
                    skip_first_run(app);
                } else {
                    close(app);
                }
            });
            return LRESULT(0);
        }
        _ => {}
    }
    DefWindowProcW(hwnd, msg, wparam, lparam)
}

/// The window: an ordinary top-level window (taskbar, Alt+Tab), drawn by DirectComposition.
pub fn make_window(hinst: HINSTANCE) -> windows::core::Result<HWND> {
    unsafe {
        CreateWindowExW(
            WS_EX_NOREDIRECTIONBITMAP | WS_EX_APPWINDOW,
            w!("OrbSettings"),
            w!("[App] settings"),
            WS_POPUP,
            0,
            0,
            720,
            580,
            None,
            None,
            Some(hinst),
            None,
        )
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
