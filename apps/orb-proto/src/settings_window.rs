//! Settings on Windows (`settings_sys` here): the window (an ordinary top-level window in the
//! taskbar and Alt+Tab, drawn by DirectComposition, its region rounded), its messages, the global
//! shortcut through RegisterHotKey, the HKCU Run entry, Windows location on a worker thread and
//! the adzan file picker. settings.rs keeps the state and decides; this file only talks to Win32.

use std::path::Path;
use std::sync::mpsc::Sender;

use windows::core::{w, HSTRING, PCWSTR};
use windows::Devices::Geolocation::{GeolocationAccessStatus, Geolocator};
use windows::Foundation::TimeSpan;
use windows::Win32::Foundation::*;
use windows::Win32::Graphics::Gdi::*;
use windows::Win32::System::Registry::*;
use windows::Win32::UI::Input::KeyboardAndMouse::*;
use windows::Win32::UI::WindowsAndMessaging::*;

use crate::gfx::Surface;
use crate::hook::unpack;
use crate::settings::{self, edge_radius};
use crate::settings_model::{Locate, Shortcut, Startup};
use crate::settings_panel::{self as sp, View};
use crate::{
    orb, panels, pill, px, with_app, App, HOTKEY_PROBE, HOTKEY_TOGGLE, MSG_MOUSELEAVE,
    WM_APP_LOCATE,
};

const RUN: PCWSTR = w!("Software\\Microsoft\\Windows\\CurrentVersion\\Run");
const APPROVED: PCWSTR =
    w!("Software\\Microsoft\\Windows\\CurrentVersion\\Explorer\\StartupApproved\\Run");

/// Windows location gives a fix (the WinRT Geolocator).
pub const HAS_LOCATION: bool = true;

/// The window and its swap chain.
pub struct Win {
    pub hwnd: HWND,
    surface: Option<Surface>,
    /// TrackMouseEvent is on (WM_MOUSELEAVE will come).
    tracking: bool,
}

impl Win {
    pub fn new(hwnd: HWND) -> Win {
        Win {
            hwnd,
            surface: None,
            tracking: false,
        }
    }
}

/// The first run always shows on a fresh data folder on Windows.
pub fn no_first_run() -> bool {
    false
}

// ---- the shortcut ----

pub fn register(app: &App, s: Shortcut) -> bool {
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

pub fn unregister(app: &App) {
    unsafe {
        let _ = UnregisterHotKey(Some(app.hwnd), HOTKEY_TOGGLE);
    }
}

/// Whether another app holds `s`: register it under a spare id and let it go again.
pub fn is_free(app: &App, s: Shortcut) -> bool {
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

// ---- start with Windows ----

fn run_name() -> HSTRING {
    // A test drive names its own entry, so the real one is never touched.
    HSTRING::from(std::env::var("ORB_RUN_VALUE").unwrap_or_else(|_| "orb-proto".into()))
}

pub fn read_startup() -> Startup {
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

/// Writes or removes the Run entry (the exe's quoted path). Returns whether the registry took it.
pub fn write_startup(_app: &mut App, on: bool) -> bool {
    let name = run_name();
    unsafe {
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
    }
}

// ---- the place ----

/// Asks Windows location on a worker thread; the answer goes into `tx` and WM_APP_LOCATE wakes
/// `settings::on_located`.
pub fn find_location(app: &mut App, tx: Sender<Locate>) {
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

// ---- the system ----

pub fn open_startup_settings() {
    crate::sys::open_url("ms-settings:startupapps");
}

pub fn open_location_settings() {
    crate::sys::open_url("ms-settings:privacy-location");
}

pub fn open_folder(dir: &Path) {
    crate::sys::open_url(&dir.display().to_string());
}

/// The adzan file picker: its modal loop runs from the orb window's message, with the app free.
pub fn pick_adzan(app: &mut App, subuh: bool) {
    unsafe {
        let _ = PostMessageW(
            Some(app.hwnd),
            crate::WM_APP_PRAYER,
            WPARAM(subuh as usize),
            LPARAM(0),
        );
    }
}

/// Whether Windows draws apps dark.
pub fn system_dark() -> bool {
    crate::ui::windows_dark()
}

/// A new theme: every other window draws again (the drop, the pill, the panel, Disk, Tasks).
pub fn redraw_others(app: &mut App) {
    orb::render_orb(app);
    if app.pill.is_some() {
        pill::render_pill(app);
    }
    panels::redraw(app);
    crate::disk_window::redraw(app);
    crate::tasks_window::redraw(app);
}

// ---- the window ----

/// Centres the window on the orb's monitor at the screen's size, its corners rounded.
pub fn place(app: &mut App) {
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
            app.settings.win.hwnd,
            None,
            x,
            y,
            w,
            h,
            SWP_NOACTIVATE | SWP_NOZORDER,
        );
        let d = px(app, 2.0 * edge_radius(screen)).round() as i32;
        let rgn = CreateRoundRectRgn(0, 0, w + 1, h + 1, d, d);
        let _ = SetWindowRgn(app.settings.win.hwnd, Some(rgn), true);
    }
}

pub fn show(app: &mut App, activate: bool) {
    unsafe {
        let _ = ShowWindow(
            app.settings.win.hwnd,
            if activate { SW_SHOW } else { SW_SHOWNA },
        );
    }
}

pub fn hide(app: &mut App) {
    unsafe {
        let _ = ShowWindow(app.settings.win.hwnd, SW_HIDE);
    }
    // The swap chain is 3 MB at 720 × 580; let it go while the window is hidden.
    app.settings.win.surface = None;
}

pub fn take_foreground(app: &mut App) {
    unsafe {
        let _ = SetForegroundWindow(app.settings.win.hwnd);
        let _ = SetFocus(Some(app.settings.win.hwnd));
    }
}

/// The swap chain at the screen's size (made on the first draw, resized for the other screen).
pub fn prepare(app: &mut App) {
    let Some(screen) = app.settings.screen else {
        return;
    };
    let (lw, lh) = sp::size(screen);
    let (w, h) = (px(app, lw).ceil() as u32, px(app, lh).ceil() as u32);
    match app.settings.win.surface.as_mut() {
        None => {
            app.settings.win.surface =
                Surface::new(&app.g, app.settings.win.hwnd, w, h, app.scale).ok()
        }
        Some(s) if s.w != w || s.h != h => {
            let _ = s.resize(&app.g, w, h);
            place(app);
        }
        _ => {}
    }
}

pub fn paint(app: &App, v: &View) {
    let Some(screen) = app.settings.screen else {
        return;
    };
    let (lw, lh) = sp::size(screen);
    let Some(s) = app.settings.win.surface.as_ref() else {
        return;
    };
    let dc = s.begin(&app.g);
    sp::draw(
        &mut crate::gfx::D2dCanvas::new(&app.g, dc),
        &app.theme,
        &app.fonts,
        v,
    );
    // The T3 lip along the window's edge (depth.rs); radius as in `place`'s region.
    crate::depth::lip(
        &app.g,
        dc,
        &app.theme,
        0.0,
        0.0,
        lw,
        lh,
        edge_radius(screen),
        1.0,
    );
    s.end(&app.g);
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
                settings::drags_at(app, x, y)
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
                if !app.settings.win.tracking {
                    let mut tme = TRACKMOUSEEVENT {
                        cbSize: std::mem::size_of::<TRACKMOUSEEVENT>() as u32,
                        dwFlags: TME_LEAVE,
                        hwndTrack: hwnd,
                        dwHoverTime: 0,
                    };
                    let _ = TrackMouseEvent(&mut tme);
                    app.settings.win.tracking = true;
                }
                let s = app.scale;
                settings::hover_at(app, Some((x / s, y / s)));
            });
            return LRESULT(0);
        }
        MSG_MOUSELEAVE => {
            with_app(|app| {
                app.settings.win.tracking = false;
                settings::hover_at(app, None);
            });
            return LRESULT(0);
        }
        WM_LBUTTONDOWN => return LRESULT(0),
        WM_LBUTTONUP => {
            with_app(|app| {
                let (x, y) = unpack(lparam);
                let s = app.scale;
                settings::click_at(app, x / s, y / s);
            });
            return LRESULT(0);
        }
        WM_MOUSEWHEEL => {
            let delta = ((wparam.0 >> 16) & 0xffff) as u16 as i16;
            with_app(|app| settings::on_wheel(app, delta));
            return LRESULT(0);
        }
        WM_KEYDOWN | WM_SYSKEYDOWN | WM_KEYUP | WM_SYSKEYUP => {
            let down = msg == WM_KEYDOWN || msg == WM_SYSKEYDOWN;
            let sys = msg == WM_SYSKEYDOWN || msg == WM_SYSKEYUP;
            let used =
                with_app(|app| settings::on_key(app, wparam.0 as u16, down, sys)).unwrap_or(false);
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
            with_app(|app| settings::on_char(app, wparam.0 as u16));
            return LRESULT(0);
        }
        WM_ACTIVATE => {
            let active = (wparam.0 & 0xffff) as u32 != WA_INACTIVE;
            with_app(|app| settings::on_activate(app, active));
        }
        WM_CLOSE => {
            with_app(settings::on_close_request);
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
            w!("Orby settings"),
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
