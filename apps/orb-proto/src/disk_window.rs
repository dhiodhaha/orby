//! The Disk window: an ordinary top-level window like Settings (taskbar, Alt+Tab, its own
//! keyboard), so choosing what to move to the Recycle Bin has room and focus. It is not topmost
//! and a click elsewhere leaves it open; Esc (after the confirm sheet), its close button, Alt+F4
//! or the taskbar close it. disk.rs owns the state, disk_panel.rs draws; this file is the window.

use std::time::Instant;

use windows::core::w;
use windows::Win32::Foundation::*;
use windows::Win32::Graphics::Gdi::*;
use windows::Win32::Media::{timeBeginPeriod, timeEndPeriod};
use windows::Win32::UI::Input::KeyboardAndMouse::*;
use windows::Win32::UI::WindowsAndMessaging::*;

use crate::disk_panel::{self as dp, Hit};
use crate::gfx::Surface;
use crate::hook::unpack;
use crate::{
    dial, disk, downloads, frame, log_line, orb, panels, px, with_app, App, MSG_MOUSELEAVE,
    WM_APP_DISK,
};

pub struct Win {
    pub hwnd: HWND,
    surface: Option<Surface>,
    pub open: bool,
    /// The window's width (logical px): `dp::W` when it opens, then whatever it is resized,
    /// snapped or maximized to.
    pub w: f32,
    /// The window height (logical px) there is room for: from the screen when it opens, then the
    /// window's own height once it is resized, snapped or maximized.
    pub room: f32,
    pub max: bool,
    hover: Option<Hit>,
    /// The cursor in the window (logical px), so a redraw after new data keeps the hover right
    /// (and the treemap's label card follows it).
    pub mouse: Option<(f32, f32)>,
    tracking: bool,
    /// The frame timer runs only while something in the window moves.
    ticking: bool,
    /// Frames drawn since the window opened (for the frame log).
    frames: u64,
}

/// The window's frame timer (on its own hwnd), one per display refresh while anything moves.
const TIMER_FRAME: usize = 1;

/// `ORB_DISK_FRAMES=1` writes one log line per Disk frame with what it cost. Off by default:
/// a frame line every 16 ms would flood the log.
fn frame_log() -> bool {
    static ON: std::sync::OnceLock<bool> = std::sync::OnceLock::new();
    *ON.get_or_init(|| std::env::var_os("ORB_DISK_FRAMES").is_some_and(|v| v != "0"))
}

/// The frame timer's period: one display refresh (13 ms at 74 Hz), and never more than 15 ms.
///
/// A USER timer fires on the system clock's tick, 15.6 ms unless a finer clock was asked for,
/// so the old 16 ms timer missed every other tick: frames came 15.6 or 31.2 ms apart, about 39
/// a second (`disk::tests::frame_cost_of_a_big_list` measures 25.5 ms on average, and 15.3 ms
/// for a 10 ms timer). At 15 ms or less it fires on every tick even on the coarse clock. While
/// the timer runs the window also asks for a 1 ms clock (`timeBeginPeriod`), so a 74 Hz screen
/// gets its 13 ms, and gives it back the moment nothing moves: an idle window costs nothing.
fn frame_ms() -> u32 {
    let mut dm = DEVMODEW {
        dmSize: std::mem::size_of::<DEVMODEW>() as u16,
        ..Default::default()
    };
    let hz = unsafe { EnumDisplaySettingsW(None, ENUM_CURRENT_SETTINGS, &mut dm) }
        .as_bool()
        .then_some(dm.dmDisplayFrequency)
        .filter(|&hz| hz > 1)
        .unwrap_or(60);
    (1000 / hz).clamp(6, 15)
}

/// Starts or stops the frame timer and the fine clock with it.
fn set_ticking(app: &mut App, on: bool) {
    if on == app.disk.win.ticking {
        return;
    }
    app.disk.win.ticking = on;
    let hwnd = app.disk.win.hwnd;
    unsafe {
        if on {
            timeBeginPeriod(1);
            SetTimer(Some(hwnd), TIMER_FRAME, frame_ms(), None);
        } else {
            let _ = KillTimer(Some(hwnd), TIMER_FRAME);
            timeEndPeriod(1);
        }
    }
}

impl Default for Win {
    fn default() -> Self {
        Win {
            hwnd: HWND::default(),
            surface: None,
            open: false,
            w: dp::W,
            room: dp::H,
            max: false,
            hover: None,
            mouse: None,
            tracking: false,
            ticking: false,
            frames: 0,
        }
    }
}

/// `H` tall when the screen has room, less on a short one (the lists then show fewer rows).
fn room(app: &App) -> f32 {
    let wa = orb::work_area(app.hwnd);
    let tall = (wa.bottom - wa.top) as f32 / app.scale - 32.0;
    tall.clamp(MIN_ROOM, dp::H)
}

/// The least room: what a short screen opens at, and the least height a drag can make it.
const MIN_ROOM: f32 = 480.0;

/// Shows the window centred on the orb's screen, or brings it back to the front if it is open.
pub fn show(app: &mut App) {
    dial::close_dial(app);
    panels::close_panel(app);
    let hwnd = app.disk.win.hwnd;
    if app.disk.win.open {
        refresh(app);
        unsafe {
            if IsIconic(hwnd).as_bool() {
                let _ = ShowWindow(hwnd, SW_RESTORE);
            }
        }
        take_foreground(app);
        return;
    }
    app.disk.win.open = true;
    app.disk.win.hover = None;
    app.disk.win.mouse = None;
    // A window closed while maximized opens maximized again; otherwise it opens at its own size,
    // centred on the orb's screen.
    if frame::is_max(hwnd) {
        sync_size(app);
        disk::prepare(app);
    } else {
        app.disk.win.w = dp::W;
        app.disk.win.room = room(app);
        app.disk.win.max = false;
        disk::prepare(app);
        place(app, true);
    }
    render(app);
    unsafe {
        let _ = ShowWindow(hwnd, SW_SHOW);
    }
    take_foreground(app);
    let room = app.disk.win.room;
    log_line(app, &format!("disk: window open, room {room:.0}"));
}

pub fn close(app: &mut App) {
    if !app.disk.win.open {
        return;
    }
    app.disk.win.open = false;
    app.disk.win.hover = None;
    app.disk.win.mouse = None;
    set_ticking(app, false);
    unsafe {
        let _ = ShowWindow(app.disk.win.hwnd, SW_HIDE);
    }
    // The swap chain is about 2 MB at 756 × 704; let it go while the window is hidden.
    app.disk.win.surface = None;
    disk::on_closed(app);
    log_line(app, "disk: window closed");
}

/// Sizes the window to its screen. `center` puts it in the middle of the orb's screen (on
/// open); otherwise it keeps its place and only moves up if it would run off the bottom.
fn place(app: &App, center: bool) {
    let (lw, lh) = disk::size(app);
    let (w, h) = (px(app, lw).ceil() as i32, px(app, lh).ceil() as i32);
    let hwnd = app.disk.win.hwnd;
    let (x, y) = if center {
        let wa = orb::work_area(app.hwnd);
        ((wa.left + wa.right - w) / 2, (wa.top + wa.bottom - h) / 2)
    } else {
        let wa = orb::work_area(hwnd);
        let mut r = RECT::default();
        unsafe {
            let _ = GetWindowRect(hwnd, &mut r);
        }
        (r.left, r.top.min(wa.bottom - h).max(wa.top))
    };
    unsafe {
        let _ = SetWindowPos(hwnd, None, x, y, w, h, SWP_NOACTIVATE | SWP_NOZORDER);
    }
    frame::shape(hwnd, app.scale);
}

/// The panel's size from the window's (resized, snapped, maximized or restored).
fn sync_size(app: &mut App) {
    let hwnd = app.disk.win.hwnd;
    let (cw, ch) = frame::client_size(hwnd);
    let win = &mut app.disk.win;
    win.w = cw as f32 / app.scale;
    win.room = ch as f32 / app.scale;
    win.max = frame::is_max(hwnd);
}

/// The window changed size from outside (a drag on an edge, a snap, maximize or restore): the
/// treemap and the lists take the new size, and the hover follows what is now under the cursor.
fn on_size(app: &mut App) {
    sync_size(app);
    frame::shape(app.disk.win.hwnd, app.scale);
    refresh(app);
}

/// The least height the window can be dragged to: the screen's own minimum with its cards, and
/// never under 480 (what a short screen opens at).
pub fn min_height(app: &App) -> f32 {
    let mut v = disk::view(app);
    v.room = 0.0;
    dp::size(&app.g, &app.fonts, &v).1.max(MIN_ROOM)
}

fn take_foreground(app: &App) {
    unsafe {
        let _ = SetForegroundWindow(app.disk.win.hwnd);
        let _ = SetFocus(Some(app.disk.win.hwnd));
    }
}

fn render(app: &mut App) {
    if !app.disk.win.open {
        return;
    }
    let t0 = Instant::now();
    disk::prepare(app);
    // One view per frame: the size and the drawing both read it.
    let v = disk::view(app);
    let (lw, lh) = dp::size(&app.g, &app.fonts, &v);
    let hwnd = app.disk.win.hwnd;
    let (cw, ch) = frame::client_size(hwnd);
    let (tw, th) = (px(app, lw).ceil() as i32, px(app, lh).ceil() as i32);
    // The window follows what the screen needs (taller for a confirm sheet or more cards), unless
    // it is maximized. Within a pixel it already is that size: a drag set it, and the room came
    // from it (the screen's height rounds down to whole pixels).
    let (w, h) = if app.disk.win.max || ((tw - cw).abs() <= 1 && (th - ch).abs() <= 1) {
        (cw, ch)
    } else {
        place(app, false);
        frame::client_size(hwnd)
    };
    let (w, h) = (w.max(1) as u32, h.max(1) as u32);
    match app.disk.win.surface.as_mut() {
        None => app.disk.win.surface = Surface::new(&app.g, hwnd, w, h, app.scale).ok(),
        Some(s) if s.w != w || s.h != h => {
            let _ = s.resize(&app.g, w, h);
        }
        _ => {}
    }
    let Some(s) = app.disk.win.surface.as_ref() else {
        return;
    };
    let t1 = Instant::now();
    let dc = s.begin(&app.g);
    dp::draw(&app.g, dc, &app.theme, &app.fonts, &v, app.disk.win.hover);
    s.end(&app.g);
    app.disk.win.frames += 1;
    if frame_log() {
        let (view_ms, draw_ms) = (
            (t1 - t0).as_secs_f64() * 1000.0,
            t1.elapsed().as_secs_f64() * 1000.0,
        );
        let n = app.disk.win.frames;
        log_line(
            app,
            &format!(
                "disk: frame {n} {:.2} ms (view {view_ms:.2}, draw {draw_ms:.2})",
                view_ms + draw_ms
            ),
        );
    }
    // Keep drawing frames while something moves; stop the moment it all rests.
    let moving = disk::animating(app);
    set_ticking(app, moving);
}

/// Draw at the next WM_PAINT. Windows makes WM_PAINT only once the input queue is empty, so a
/// burst of wheel or mouse messages costs one frame instead of one each.
fn invalidate(app: &App) {
    unsafe {
        let _ = InvalidateRect(Some(app.disk.win.hwnd), None, false);
    }
}

/// The data moved under the pointer (a scroll): the hover follows what is now under the
/// cursor, and the frame comes with the next WM_PAINT.
pub fn refresh_soon(app: &mut App) {
    if !app.disk.win.open {
        return;
    }
    disk::prepare(app);
    let h = app.disk.win.mouse.and_then(|(x, y)| disk::hit(app, x, y));
    if h != app.disk.win.hover {
        app.disk.win.hover = h;
        disk::on_hover(app, h);
    }
    invalidate(app);
}

/// Redraw after the data behind the window changed; the hover follows what is now under the
/// cursor (a row may have moved).
pub fn refresh(app: &mut App) {
    if !app.disk.win.open {
        return;
    }
    disk::prepare(app);
    let h = app.disk.win.mouse.and_then(|(x, y)| disk::hit(app, x, y));
    if h != app.disk.win.hover {
        app.disk.win.hover = h;
        disk::on_hover(app, h);
    }
    render(app);
}

/// Redraw as it is (the theme changed).
pub fn redraw(app: &mut App) {
    render(app);
}

/// The wheel's point comes in screen pixels.
fn client_point(hwnd: HWND, lparam: LPARAM) -> POINT {
    let (sx, sy) = unpack(lparam);
    let mut p = POINT {
        x: sx as i32,
        y: sy as i32,
    };
    unsafe {
        let _ = ScreenToClient(hwnd, &mut p);
    }
    p
}

pub unsafe extern "system" fn disk_proc(
    hwnd: HWND,
    msg: u32,
    wparam: WPARAM,
    lparam: LPARAM,
) -> LRESULT {
    if msg == WM_GETMINMAXINFO {
        // The App is busy while it sizes the window itself; the plain minimum does then.
        let min_h = with_app(|app| min_height(app)).unwrap_or(MIN_ROOM);
        return frame::nc_message(hwnd, msg, wparam, lparam, dp::W, min_h).unwrap_or_default();
    }
    if let Some(r) = frame::nc_message(hwnd, msg, wparam, lparam, dp::W, MIN_ROOM) {
        return r;
    }
    match msg {
        WM_NCHITTEST => {
            // The edges resize; the title band's empty part drags the window (a double click on
            // it maximizes or restores).
            let p = client_point(hwnd, lparam);
            let ht = with_app(|app| {
                if let Some(e) = frame::edge_hit(hwnd, p, app.scale) {
                    return e;
                }
                let (x, y) = (p.x as f32 / app.scale, p.y as f32 / app.scale);
                if y < dp::TOP && disk::hit(app, x, y).is_none() {
                    HTCAPTION
                } else {
                    HTCLIENT
                }
            })
            .unwrap_or(HTCLIENT);
            return LRESULT(ht as isize);
        }
        WM_SIZE if wparam.0 as u32 != SIZE_MINIMIZED => {
            with_app(|app| {
                if app.disk.win.open {
                    on_size(app);
                }
            });
            return LRESULT(0);
        }
        WM_MOUSEMOVE => {
            with_app(|app| {
                let (x, y) = unpack(lparam);
                let (x, y) = (x / app.scale, y / app.scale);
                if !app.disk.win.tracking {
                    let mut tme = TRACKMOUSEEVENT {
                        cbSize: std::mem::size_of::<TRACKMOUSEEVENT>() as u32,
                        dwFlags: TME_LEAVE,
                        hwndTrack: hwnd,
                        dwHoverTime: 0,
                    };
                    let _ = TrackMouseEvent(&mut tme);
                    app.disk.win.tracking = true;
                }
                app.disk.win.mouse = Some((x, y));
                let h = disk::hit(app, x, y);
                if h != app.disk.win.hover {
                    app.disk.win.hover = h;
                    disk::on_hover(app, h);
                    invalidate(app);
                } else if matches!(h, Some(Hit::Tile(_))) {
                    // The label card follows the pointer across a tile.
                    invalidate(app);
                }
            });
            return LRESULT(0);
        }
        MSG_MOUSELEAVE => {
            with_app(|app| {
                app.disk.win.tracking = false;
                app.disk.win.mouse = None;
                if app.disk.win.hover.is_some() {
                    app.disk.win.hover = None;
                    disk::on_hover(app, None);
                    render(app);
                }
            });
            return LRESULT(0);
        }
        WM_TIMER if wparam.0 == TIMER_FRAME => {
            with_app(render);
            return LRESULT(0);
        }
        WM_PAINT => {
            let _ = ValidateRect(Some(hwnd), None);
            with_app(render);
            return LRESULT(0);
        }
        WM_LBUTTONDOWN => return LRESULT(0),
        WM_LBUTTONUP => {
            with_app(|app| {
                let (x, y) = unpack(lparam);
                if let Some(h) = disk::hit(app, x / app.scale, y / app.scale) {
                    disk::click(app, h);
                }
            });
            return LRESULT(0);
        }
        WM_MOUSEWHEEL => {
            let delta = ((wparam.0 >> 16) & 0xffff) as u16 as i16;
            let p = client_point(hwnd, lparam);
            with_app(|app| {
                disk::on_wheel(app, p.x as f32 / app.scale, p.y as f32 / app.scale, delta)
            });
            return LRESULT(0);
        }
        WM_KEYDOWN => {
            let vk = wparam.0 as u16;
            let used = with_app(|app| {
                if VIRTUAL_KEY(vk) == VK_ESCAPE {
                    disk::on_esc(app);
                    true
                } else {
                    disk::on_keydown(app, vk)
                }
            })
            .unwrap_or(false);
            if used {
                return LRESULT(0);
            }
        }
        WM_CHAR => return LRESULT(0),
        WM_APP_DISK if wparam.0 == disk::PICK_FOLDER => {
            // The picker runs its own message loop, so the app is not borrowed while it is up.
            let folder = downloads::pick_folder(hwnd, None, "Scan a folder");
            with_app(|app| disk::on_folder_picked(app, folder));
            return LRESULT(0);
        }
        WM_CLOSE => {
            with_app(close);
            return LRESULT(0);
        }
        _ => {}
    }
    DefWindowProcW(hwnd, msg, wparam, lparam)
}

/// An ordinary top-level window (taskbar, Alt+Tab), drawn by DirectComposition.
pub fn make_window(hinst: HINSTANCE) -> windows::core::Result<HWND> {
    unsafe {
        CreateWindowExW(
            WS_EX_NOREDIRECTIONBITMAP | WS_EX_APPWINDOW,
            w!("OrbDisk"),
            w!("Orby disk"),
            frame::STYLE,
            0,
            0,
            dp::W as i32,
            dp::H as i32,
            None,
            None,
            Some(hinst),
            None,
        )
    }
}
