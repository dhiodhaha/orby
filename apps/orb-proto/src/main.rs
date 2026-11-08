//! Orb prototype: the mercury drop, its pill, dial and Prayer panel.
//!
//! This file owns the state (`App`), the shared ids, the orb window's message dispatch and
//! the loop. Each window's behaviour lives in its own module:
//! - orb.rs: the drop (shader, springs, docking, tuck, drag)
//! - pill.rs: the notification pill
//! - dial.rs: the half dial and its picks
//! - prayer.rs: prayer times and reminders (panel.rs draws the Prayer panel)
//! - habits.rs: focus and drink on the app_habits engines (focus_panel.rs, drink_panel.rs draw)
//! - panels.rs: the one panel window that shows Prayer, Focus or Drink
//! - store.rs: the drink log and focus sessions on disk
//! - tasks.rs: the Tasks panel, due pills and the overdue badge (tasks_model.rs decides,
//!   tasks_panel.rs draws, textfield.rs edits the quick add line)
//! - calendar.rs: the calendar link, its 10 min reads, event pills and the Today panel
//!   (today_model.rs decides, today_panel.rs draws)
//! - downloads.rs: the clipboard offer, the download queue on yt-dlp, its ring, pills, panel and
//!   format menu (downloads_model.rs decides, downloads_panel.rs draws)
//! - disk.rs: the low-space pill, scans, results, Safe to clean, Move to Recycle Bin and Undo
//!   in its own window (disk_window.rs; disk_model.rs decides, disk_panel.rs draws,
//!   disk_restore.rs puts items back)
//! - history.rs: the History panel, drink and focus by day (history_model.rs decides,
//!   history_panel.rs draws)
//! - notices.rs: the notice queue every pill goes through, quiet times, the badge count and the
//!   Notices panel (notices_model.rs decides, notices_panel.rs draws)
//! - settings.rs: the Settings window and the first run, settings.json, the shortcut, start with
//!   Windows, Windows location (settings_model.rs decides, settings_panel.rs draws)
//! - hook.rs: the low-level mouse hook
//! - tray.rs: the tray icon and menu commands
//!
//! Renders only while something moves. At rest the process sleeps in GetMessage.

#![windows_subsystem = "windows"]

mod calendar;
mod depth;
mod dial;
mod disk;
mod disk_model;
mod disk_panel;
mod disk_restore;
mod disk_window;
mod downloads;
mod downloads_model;
mod downloads_panel;
mod drink_panel;
mod focus_panel;
mod gfx;
mod habits;
mod history;
mod history_model;
mod history_panel;
mod hook;
mod khgt;
mod morph;
mod motion;
mod notices;
mod notices_model;
mod notices_panel;
mod orb;
mod panel;
mod panels;
mod pill;
mod prayer;
mod settings;
mod settings_model;
mod settings_panel;
mod store;
mod tasks;
mod tasks_model;
mod tasks_panel;
mod textfield;
mod today_model;
mod today_panel;
mod tray;
mod ui;

use app_core::prayer::{Prayer, Times};

use std::cell::RefCell;
use std::fs::OpenOptions;
use std::io::Write as _;
use std::time::Instant;

use windows::core::{w, PCWSTR};
use windows::Win32::Foundation::*;
use windows::Win32::System::LibraryLoader::GetModuleHandleW;
use windows::Win32::System::ProcessStatus::{GetProcessMemoryInfo, PROCESS_MEMORY_COUNTERS};
use windows::Win32::System::RemoteDesktop::{
    WTSRegisterSessionNotification, NOTIFY_FOR_THIS_SESSION,
};
use windows::Win32::System::Threading::GetCurrentProcess;
use windows::Win32::UI::HiDpi::*;
use windows::Win32::UI::Input::KeyboardAndMouse::*;
use windows::Win32::UI::WindowsAndMessaging::*;

use gfx::{Shared, Surface};
use orb::{OrbGpu, Side, Spring, SS, WIN};
use ui::{Pill, PillLayout};

// ---- ids: window messages, timers and hotkeys share the orb window, so they live in one
// place where a collision is visible. Tray menu ids are in tray.rs. ----

const WM_APP_NEAR: u32 = WM_APP + 1;
const MSG_MOUSELEAVE: u32 = 0x02A3; // WM_MOUSELEAVE
const WM_APP_TRAY: u32 = WM_APP + 2;
const WM_APP_DIALMOVE: u32 = WM_APP + 3;
const WM_APP_OUTSIDE: u32 = WM_APP + 4;
const WM_APP_CAL: u32 = WM_APP + 5; // a calendar read finished on its worker thread
const WM_APP_DL: u32 = WM_APP + 6; // a download or the yt-dlp fetch reported from its thread
const WM_APP_DISK: u32 = WM_APP + 7; // a scan, the clean-up list or a move reported from its thread
const WM_APP_FOREGROUND: u32 = WM_APP + 8; // the foreground window changed or moved (full screen?)
const WM_APP_LOCATE: u32 = WM_APP + 9; // Windows location answered on its worker thread
const WM_APP_PRAYER: u32 = WM_APP + 20; // Settings asks for the adzan file picker (wparam 1: Subuh's)
const MSG_CLIPBOARD: u32 = 0x031D; // WM_CLIPBOARDUPDATE
const TIMER_TUCK: usize = 1;
const TIMER_STATS: usize = 2;
const TIMER_PILL: usize = 3; // on the pill window
const TIMER_FOCUS: usize = 4;
const TIMER_REMIND: usize = 5;
const TIMER_PANEL: usize = 6; // on the panel window
const TIMER_DRINK: usize = 7;
const TIMER_BRANCH: usize = 8;
const TIMER_TASKS: usize = 9;
const TIMER_CAL_CUE: usize = 10;
const TIMER_CAL_READ: usize = 11;
const TIMER_DISK: usize = 12; // the low-space check, every 10 min
const TIMER_DISK_RELEASE: usize = 13; // drops a scan's tree a while after the Disk window closes
const TIMER_NOTICES: usize = 14; // the notice queue's next change (pacing, cooldown, stale, midnight, pause end)
const TIMER_SHAKE: usize = 15; // on the pill window: the urgent pill's next attention shake
const MSG_INPUT: u32 = 0x00FF; // WM_INPUT, only while away
const MSG_SESSION_CHANGE: u32 = 0x02B1; // WM_WTSSESSION_CHANGE
const HOTKEY_TOGGLE: i32 = 1; // the shortcut from Settings, Ctrl+Alt+Space by default
const HOTKEY_PROBE: i32 = 2; // briefly, to ask whether another app holds a shortcut
const HK_UP: i32 = 21;
const HK_DOWN: i32 = 22;
const HK_LEFT: i32 = 23;
const HK_RIGHT: i32 = 24;
const HK_ENTER: i32 = 25;
const HK_ESC: i32 = 26;
const HK_NUM: i32 = 31; // 31..36 for keys 1..6

struct App {
    g: Shared,
    orb_gpu: OrbGpu,
    theme: ui::Theme,
    fonts: ui::Fonts,
    hwnd: HWND,
    scale: f32,
    side: Side,
    tucked: bool,
    near: bool,
    visible: bool,
    x: Spring,
    y: f32,
    opacity: Spring,
    lean: Spring,
    /// Press, hover, click pulse, dock wobble and the metal's flow while you use the drop.
    life: orb::Life,
    sun_angle: f32,
    brightness: f32,
    shader_time: f32,
    flow: Option<Instant>,
    distortion: f32,
    press: Option<(POINT, i32, i32)>,
    dragging: bool,
    // pill
    pill_hwnd: HWND,
    pill_surface: Option<Surface>,
    pill: Option<Pill>,
    pill_layout: PillLayout,
    pill_reveal: Spring,
    pill_closing: bool,
    pill_hover: pill::StackHit,
    stack: pill::Stack,
    pill_tracking: bool,
    // dial
    dial_hwnd: HWND,
    dial_surface: Option<Surface>,
    dial_open: bool,
    dial_reveal: Spring,
    dial_aim: Option<usize>,
    dial_keyboard: bool,
    /// The cursor (screen px) at the last dial key: the mouse takes over only once it leaves here.
    dial_key_anchor: Option<(f32, f32)>,
    dial_branch: Option<usize>,
    dial_child: Option<usize>,
    branch_reveal: Spring,
    /// The main ring's value tags: hidden while a branch parent is aimed or open, so sliding
    /// between Focus, Drink and More never flashes them back.
    dial_tags: Spring,
    keys_on: u32,
    // focus and drink
    habits: habits::Habits,
    tasks: tasks::Tasks,
    cal: calendar::Cal,
    dl: downloads::Downloads,
    disk: disk::Disk,
    history: history::History,
    notices: notices::Notices,
    settings: settings::Settings,
    // prayer
    times: Times,
    times_day: (i32, u32, u32),
    tomorrow_subuh: u32,
    next_reminder: Option<(Prayer, i32)>,
    /// The adzan file is playing (winmm MCI, alias orb_adzan).
    adzan_playing: bool,
    // the panel window (Prayer, Focus or Drink)
    panel_hwnd: HWND,
    panel_surface: Option<Surface>,
    panel_kind: panels::Kind,
    panel_open: bool,
    panel_reveal: Spring,
    /// The open panel steps back (fades) while the dial is open over it.
    panel_dim: Spring,
    /// A switch between two panels while the window is open (see morph.rs).
    panel_morph: morph::Morph<panels::Kind>,
    panel_hover: Option<panels::Hit>,
    panel_tracking: bool,
    knob_drag: bool,
    /// The window that had the keyboard before a panel that types took it.
    panel_prev_fg: HWND,
    // stats
    anim_start: Option<Instant>,
    anim_frames: u32,
    last_frame: Instant,
    log: std::fs::File,
}

thread_local! {
    static APP: RefCell<Option<App>> = const { RefCell::new(None) };
}

fn with_app<R>(f: impl FnOnce(&mut App) -> R) -> Option<R> {
    APP.with(|a| match a.try_borrow_mut() {
        Ok(mut b) => b.as_mut().map(f),
        Err(_) => None,
    })
}

fn log_line(app: &mut App, msg: &str) {
    let _ = writeln!(app.log, "{msg}");
    let _ = app.log.flush();
}

fn px(app: &App, v: f32) -> f32 {
    v * app.scale
}

fn exe_dir() -> std::path::PathBuf {
    std::env::current_exe()
        .ok()
        .and_then(|p| p.parent().map(|d| d.to_path_buf()))
        .unwrap_or_default()
}

fn memory_mb() -> (f64, f64) {
    unsafe {
        let mut pmc = PROCESS_MEMORY_COUNTERS {
            cb: std::mem::size_of::<PROCESS_MEMORY_COUNTERS>() as u32,
            ..Default::default()
        };
        let _ = GetProcessMemoryInfo(GetCurrentProcess(), &mut pmc, pmc.cb);
        (
            pmc.WorkingSetSize as f64 / 1048576.0,
            pmc.PagefileUsage as f64 / 1048576.0,
        )
    }
}

// ---- animation ----

fn is_animating(app: &App) -> bool {
    orb::is_animating(app)
        || pill::is_animating(app)
        || dial::is_animating(app)
        || panels::is_animating(app)
}

fn tick(app: &mut App) -> bool {
    let now = Instant::now();
    let dt = (now - app.last_frame).as_secs_f32().min(1.0 / 30.0);
    app.last_frame = now;
    // The drop's active loop alone redraws only the drop.
    let full = orb::full_frame(app);
    orb::tick(app, now, dt);
    if full {
        pill::tick(app, dt);
        dial::tick(app, dt);
        panels::tick(app, dt);
    }
    app.anim_frames += 1;
    is_animating(app)
}

fn start_anim(app: &mut App) {
    app.life.repaint = true;
    if app.anim_start.is_none() {
        app.anim_start = Some(Instant::now());
        app.anim_frames = 0;
        app.last_frame = Instant::now();
    }
}

fn end_anim(app: &mut App) {
    if let Some(s) = app.anim_start.take() {
        let secs = s.elapsed().as_secs_f32();
        if secs > 0.2 {
            let fps = app.anim_frames as f32 / secs;
            let (ws, private) = memory_mb();
            log_line(
                app,
                &format!(
                    "anim {:.2}s frames={} fps={:.1} ws={:.1}MB private={:.1}MB",
                    secs, app.anim_frames, fps, ws, private
                ),
            );
        }
    }
}

/// The hotkeys that should be live right now: the dial's keys while it is open, Esc while the
/// dial or a panel is open, and the arrows for the Focus knob. Registered only while needed, so
/// other apps keep their keys the rest of the time.
fn sync_keys(app: &mut App) {
    let dial = app.dial_open;
    let mut keys: Vec<(i32, u32)> = Vec::new();
    if dial || app.panel_open {
        keys.push((HK_ESC, VK_ESCAPE.0 as u32));
    }
    if dial || panels::wants_arrows(app) {
        keys.push((HK_UP, VK_UP.0 as u32));
        keys.push((HK_DOWN, VK_DOWN.0 as u32));
    }
    if dial || panels::wants_enter(app) {
        keys.push((HK_ENTER, VK_RETURN.0 as u32));
    }
    if dial {
        keys.push((HK_LEFT, VK_LEFT.0 as u32));
        keys.push((HK_RIGHT, VK_RIGHT.0 as u32));
        for n in 0..6 {
            keys.push((HK_NUM + n, (b'1' + n as u8) as u32));
        }
    }
    let want: u32 = keys.iter().fold(0, |m, (id, _)| m | (1 << (id - 20)));
    unsafe {
        for (id, vk) in &keys {
            if app.keys_on & (1 << (id - 20)) == 0 {
                let _ = RegisterHotKey(Some(app.hwnd), *id, MOD_NOREPEAT, *vk);
            }
        }
        for id in 21..=HK_NUM + 5 {
            let bit = 1 << (id - 20);
            if app.keys_on & bit != 0 && want & bit == 0 {
                let _ = UnregisterHotKey(Some(app.hwnd), id);
            }
        }
    }
    app.keys_on = want;
}

/// Every 10 s: a new day's times, the sun's highlight angle, and a RAM line in the log.
fn on_stats_timer(app: &mut App) {
    prayer::refresh_day(app);
    let (a, b) = orb::sun_from_clock(&app.times);
    if (a - app.sun_angle).abs() > 0.5 || b != app.brightness {
        app.sun_angle = a;
        app.brightness = b;
        orb::render_orb(app);
    }
    let (ws, private) = memory_mb();
    log_line(
        app,
        &format!("idle ws={:.1}MB private={:.1}MB", ws, private),
    );
}

// ---- the orb window: dispatch ----

unsafe extern "system" fn orb_proc(
    hwnd: HWND,
    msg: u32,
    wparam: WPARAM,
    lparam: LPARAM,
) -> LRESULT {
    match msg {
        WM_MOUSEACTIVATE => return LRESULT(MA_NOACTIVATE as isize),
        WM_LBUTTONDOWN => {
            with_app(|app| orb::on_press(app, hwnd));
            return LRESULT(0);
        }
        WM_MOUSEMOVE => {
            with_app(orb::on_move);
            return LRESULT(0);
        }
        WM_LBUTTONUP => {
            let _ = ReleaseCapture();
            with_app(|app| orb::on_release(app, hwnd));
            return LRESULT(0);
        }
        WM_APP_NEAR => {
            let (x, y) = hook::unpack(lparam);
            with_app(|app| orb::on_near(app, hwnd, wparam.0 != 0, x, y));
            return LRESULT(0);
        }
        WM_APP_DIALMOVE => {
            let (x, y) = hook::unpack(lparam);
            with_app(|app| dial::on_pointer(app, x, y));
            return LRESULT(0);
        }
        WM_APP_OUTSIDE => {
            with_app(|app| {
                downloads::close_menu(app);
                dial::close_dial(app);
                panels::close_panel(app);
            });
            return LRESULT(0);
        }
        WM_APP_CAL => {
            with_app(calendar::on_read_done);
            return LRESULT(0);
        }
        WM_APP_DL if wparam.0 == downloads::PICK_FOLDER => {
            let start = with_app(downloads::save_folder);
            let folder = downloads::pick_folder(hwnd, start.as_deref(), "Save downloads to");
            with_app(|app| downloads::on_folder_picked(app, folder));
            return LRESULT(0);
        }
        WM_APP_DL => {
            with_app(downloads::on_msg);
            return LRESULT(0);
        }
        WM_APP_DISK => {
            with_app(disk::on_msg);
            return LRESULT(0);
        }
        WM_APP_FOREGROUND => {
            with_app(notices::on_foreground);
            return LRESULT(0);
        }
        WM_APP_PRAYER => {
            if let Some(owner) = with_app(|app| app.settings.hwnd) {
                let subuh = wparam.0 == 1;
                let path = prayer::pick_adzan_file(owner, subuh);
                with_app(|app| settings::on_adzan_picked(app, subuh, path));
            }
            return LRESULT(0);
        }
        prayer::MM_MCINOTIFY => {
            with_app(|app| prayer::on_adzan_notify(app, wparam.0));
            return LRESULT(0);
        }
        WM_APP_LOCATE => {
            with_app(settings::on_located);
            return LRESULT(0);
        }
        WM_SETTINGCHANGE => {
            let what = if lparam.0 == 0 {
                String::new()
            } else {
                PCWSTR(lparam.0 as *const u16)
                    .to_string()
                    .unwrap_or_default()
            };
            if what == "ImmersiveColorSet" {
                with_app(settings::on_system_theme);
            }
        }
        MSG_CLIPBOARD => {
            with_app(downloads::on_clipboard);
            return LRESULT(0);
        }
        MSG_INPUT => {
            with_app(habits::on_input);
        }
        MSG_SESSION_CHANGE => {
            with_app(|app| habits::on_session_change(app, wparam.0));
            return LRESULT(0);
        }
        WM_TIMER => {
            with_app(|app| match wparam.0 {
                TIMER_TUCK => orb::on_tuck_timer(app, hwnd),
                TIMER_FOCUS => habits::on_focus_timer(app, hwnd),
                TIMER_DRINK => habits::on_drink_timer(app, hwnd),
                TIMER_BRANCH => dial::on_branch_timer(app, hwnd),
                TIMER_REMIND => prayer::on_remind_timer(app, hwnd),
                TIMER_TASKS => tasks::on_timer(app),
                TIMER_CAL_CUE => calendar::on_cue_timer(app),
                TIMER_CAL_READ => calendar::on_read_timer(app),
                TIMER_DISK => disk::on_timer(app),
                TIMER_DISK_RELEASE => disk::on_release_timer(app),
                TIMER_NOTICES => notices::on_timer(app),
                TIMER_STATS => on_stats_timer(app),
                _ => {}
            });
            return LRESULT(0);
        }
        WM_HOTKEY => {
            let id = wparam.0 as i32;
            with_app(|app| match id {
                HOTKEY_TOGGLE if settings::takes_shortcut(app) => {
                    settings::on_shortcut_pressed(app)
                }
                HOTKEY_TOGGLE => dial::on_shortcut(app),
                HK_ESC if downloads::menu_open(app) => downloads::close_menu(app),
                HK_ESC if app.dial_open => dial::on_esc(app),
                HK_ESC => panels::on_esc(app),
                HK_UP | HK_DOWN if !app.dial_open && panels::wants_arrows(app) => {
                    panels::on_arrow(app, id == HK_UP)
                }
                HK_ENTER if !app.dial_open && panels::wants_enter(app) => calendar::on_enter(app),
                _ => {
                    dial::on_key(app, id);
                }
            });
            return LRESULT(0);
        }
        WM_APP_TRAY => {
            let ev = (lparam.0 & 0xffff) as u32;
            if ev == WM_RBUTTONUP || ev == WM_CONTEXTMENU || ev == WM_LBUTTONUP {
                tray::tray_menu(hwnd);
            }
            return LRESULT(0);
        }
        WM_COMMAND => {
            tray::on_command(hwnd, wparam.0 & 0xffff);
            return LRESULT(0);
        }
        WM_DISPLAYCHANGE => {
            with_app(orb::on_display_change);
            return LRESULT(0);
        }
        WM_DESTROY => {
            with_app(downloads::shutdown);
            with_app(disk::shutdown);
            tray::remove_tray(hwnd);
            PostQuitMessage(0);
            return LRESULT(0);
        }
        _ => {}
    }
    DefWindowProcW(hwnd, msg, wparam, lparam)
}

fn register_class(hinst: HINSTANCE, name: PCWSTR, proc_: WNDPROC) {
    unsafe {
        let wc = WNDCLASSW {
            lpfnWndProc: proc_,
            hInstance: hinst,
            lpszClassName: name,
            hCursor: LoadCursorW(None, IDC_ARROW).unwrap_or_default(),
            ..Default::default()
        };
        RegisterClassW(&wc);
    }
}

fn make_window(hinst: HINSTANCE, class: PCWSTR, title: PCWSTR) -> windows::core::Result<HWND> {
    unsafe {
        CreateWindowExW(
            WS_EX_TOPMOST | WS_EX_TOOLWINDOW | WS_EX_NOACTIVATE | WS_EX_NOREDIRECTIONBITMAP,
            class,
            title,
            WS_POPUP,
            0,
            0,
            68,
            68,
            None,
            None,
            Some(hinst),
            None,
        )
    }
}

fn step(name: &str) {
    let _ = std::fs::write(exe_dir().join("orb-proto-step.txt"), name);
}

fn main() {
    if let Err(e) = run() {
        let last =
            std::fs::read_to_string(exe_dir().join("orb-proto-step.txt")).unwrap_or_default();
        let _ = std::fs::write(
            exe_dir().join("orb-proto-error.txt"),
            format!(
                "failed at step: {last}
error: {e:?}
"
            ),
        );
        std::process::exit(1);
    }
}

fn run() -> windows::core::Result<()> {
    unsafe {
        let _ = SetProcessDpiAwarenessContext(DPI_AWARENESS_CONTEXT_PER_MONITOR_AWARE_V2);
        step("module");
        let hinst: HINSTANCE = GetModuleHandleW(None)?.into();
        register_class(hinst, w!("OrbProto"), Some(orb_proc));
        register_class(hinst, w!("OrbPill"), Some(pill::pill_proc));
        register_class(hinst, w!("OrbDial"), Some(dial::dial_proc));
        register_class(hinst, w!("OrbPanel"), Some(panels::panel_proc));
        register_class(hinst, w!("OrbMenu"), Some(downloads::menu_proc));
        register_class(hinst, w!("OrbSettings"), Some(settings::settings_proc));
        register_class(hinst, w!("OrbDisk"), Some(disk_window::disk_proc));
        step("orb window");
        let hwnd = make_window(hinst, w!("OrbProto"), w!("Orby orb"))?;
        step("pill window");
        let pill_hwnd = make_window(hinst, w!("OrbPill"), w!("Orby notice"))?;
        step("dial window");
        let dial_hwnd = make_window(hinst, w!("OrbDial"), w!("Orby menu"))?;
        let panel_hwnd = make_window(hinst, w!("OrbPanel"), w!("Orby panel"))?;
        let menu_hwnd = make_window(hinst, w!("OrbMenu"), w!("Orby menu"))?;
        let settings_hwnd = settings::make_window(hinst)?;
        let disk_hwnd = disk_window::make_window(hinst)?;
        let settings = settings::Settings::load(&store::data_dir(), settings_hwnd);
        let day = prayer::today();
        let (times, tomorrow_subuh) =
            prayer::compute_times(&settings.prefs, settings.prefs.place.coords(), day);

        let dpi = GetDpiForWindow(hwnd).max(96);
        let scale = dpi as f32 / 96.0;
        let size = (WIN * scale).round() as i32;
        let _ = SetWindowPos(
            hwnd,
            None,
            0,
            0,
            size,
            size,
            SWP_NOMOVE | SWP_NOACTIVATE | SWP_NOZORDER,
        );

        step("shared");
        let g = Shared::new()?;
        step("orb gpu");
        let orb_gpu = orb::create_orb_gpu(&g, hwnd, (WIN * scale * SS).round() as u32)?;
        step("fonts");
        let fonts = ui::Fonts::new(&g);
        let font_note = g.font_note.clone();
        step("app");
        let log = OpenOptions::new()
            .create(true)
            .append(true)
            .open(exe_dir().join("orb-proto.log"))
            .expect("log");
        let (sun_angle, brightness) = orb::sun_from_clock(&times);

        let mut app = App {
            g,
            orb_gpu,
            theme: settings::theme_now(&settings.prefs),
            fonts,
            hwnd,
            scale,
            side: Side::Right,
            tucked: false,
            near: false,
            visible: true,
            x: Spring::new(0.0, 480.0, 34.0),
            y: 0.0,
            opacity: Spring::new(1.0, 300.0, 30.0),
            lean: Spring::new(0.0, 120.0, 18.0),
            life: orb::Life::new(),
            sun_angle,
            brightness,
            shader_time: 1.7,
            flow: None,
            distortion: 0.1,
            press: None,
            dragging: false,
            pill_hwnd,
            pill_surface: None,
            pill: None,
            pill_layout: PillLayout::default(),
            pill_reveal: Spring::new(0.0, 320.0, 17.0),
            pill_closing: false,
            pill_hover: pill::StackHit::None,
            stack: pill::Stack::new(),
            pill_tracking: false,
            dial_hwnd,
            dial_surface: None,
            dial_open: false,
            dial_reveal: Spring::new(0.0, 190.0, 26.0),
            dial_aim: None,
            dial_keyboard: false,
            dial_key_anchor: None,
            dial_branch: None,
            dial_child: None,
            branch_reveal: Spring::new(0.0, 190.0, 26.0),
            dial_tags: Spring::new(1.0, 600.0, 50.0),
            keys_on: 0,
            habits: habits::Habits::load(store::data_dir().join("habits.json")),
            tasks: tasks::Tasks::load(&store::data_dir()),
            cal: calendar::Cal::load(&store::data_dir()),
            dl: downloads::Downloads::load(menu_hwnd),
            disk: disk::Disk::load(disk_hwnd),
            history: history::History::new(),
            notices: notices::Notices::load(&store::data_dir()),
            settings,
            times,
            times_day: day,
            tomorrow_subuh,
            next_reminder: None,
            adzan_playing: false,
            panel_hwnd,
            panel_surface: None,
            panel_kind: panels::Kind::Prayer,
            panel_open: false,
            panel_reveal: Spring::new(0.0, 190.0, 26.0),
            panel_dim: Spring::new(1.0, 260.0, 32.0),
            panel_morph: morph::Morph::default(),
            panel_hover: None,
            panel_tracking: false,
            knob_drag: false,
            panel_prev_fg: HWND::default(),
            anim_start: None,
            anim_frames: 0,
            last_frame: Instant::now(),
            log,
        };
        let wa = orb::work_area(hwnd);
        app.x.snap(orb::dock_x(&app, Side::Right, false));
        app.y = orb::clamp_y(
            &app,
            wa.top as f32 + (wa.bottom - wa.top) as f32 * 0.7 - px(&app, WIN / 2.0),
        );
        orb::set_orb_region(&app);
        orb::place_orb(&app);
        orb::render_orb(&mut app);
        let _ = ShowWindow(hwnd, SW_SHOWNOACTIVATE);

        hook::update_hook(&app);
        step("hook");
        let hook = hook::install(hwnd, hinst)?;
        tray::add_tray(hwnd);
        SetTimer(Some(hwnd), TIMER_STATS, 10_000, None);
        orb::arm_tuck_timer(&app);
        // Notices, then the first run or a hidden start, before any source can post: a pill must
        // not show for an orb that is about to hide.
        notices::start(&mut app);
        settings::start(&mut app);
        prayer::schedule_reminders(&mut app);
        prayer::log_times(&mut app);
        // Lock and unlock reset the drink reminder clock.
        let _ = WTSRegisterSessionNotification(hwnd, NOTIFY_FOR_THIS_SESSION);
        habits::schedule_drink(&mut app);
        habits::arm_focus(&mut app);
        tasks::schedule(&mut app);
        calendar::start(&mut app);
        downloads::start(&mut app);
        disk::start(&mut app);
        khgt::start();

        let (ws, private) = memory_mb();
        let dark = app.theme.dark;
        let hotkey_ok = app.settings.hotkey_ok;
        log_line(&mut app, &font_note);
        log_line(&mut app, &format!("start dpi={dpi} scale={scale} ss={SS} dark={dark} hotkey={hotkey_ok} ws={ws:.1}MB private={private:.1}MB"));
        APP.with(|a| *a.borrow_mut() = Some(app));

        step("loop");
        let mut msg = MSG::default();
        loop {
            let animating = with_app(|app| app.anim_start.is_some()).unwrap_or(false);
            if animating {
                while PeekMessageW(&mut msg, None, 0, 0, PM_REMOVE).as_bool() {
                    if msg.message == WM_QUIT {
                        let _ = UnhookWindowsHookEx(hook);
                        return Ok(());
                    }
                    let _ = TranslateMessage(&msg);
                    DispatchMessageW(&msg);
                }
                with_app(|app| {
                    let moving = tick(app);
                    hook::update_hook(app);
                    if !moving {
                        end_anim(app);
                    }
                });
            } else {
                let r = GetMessageW(&mut msg, None, 0, 0);
                if r.0 <= 0 {
                    break;
                }
                let _ = TranslateMessage(&msg);
                DispatchMessageW(&msg);
            }
        }
        let _ = UnhookWindowsHookEx(hook);
    }
    Ok(())
}
