//! The one panel window beside the drop. It shows one panel at a time (Prayer, Focus, Drink or
//! Tasks), grows to that panel's height, and closes on Esc or a click outside. Only Tasks takes
//! the keyboard (its quick add field); the others never activate.

use windows::Win32::Foundation::*;
use windows::Win32::Graphics::Gdi::*;
use windows::Win32::UI::Input::KeyboardAndMouse::*;
use windows::Win32::UI::WindowsAndMessaging::*;

use crate::gfx::Surface;
use crate::hook::unpack;
use crate::orb::{self, arm_tuck_timer, orb_center, set_tucked, work_area, Side, INSET, WIN};
use crate::pill::dismiss_pill;
use crate::{
    calendar, disk, disk_panel, downloads, downloads_panel, drink_panel, focus_panel, habits,
    history, history_panel, hook, log_line, notices, notices_panel, panel, prayer, px, start_anim,
    sync_keys, tasks, tasks_panel, today_panel, with_app, App, MSG_MOUSELEAVE, TIMER_PANEL,
    TIMER_TUCK,
};

#[derive(Clone, Copy, PartialEq, Debug)]
pub enum Kind {
    Prayer,
    Focus,
    Drink,
    Tasks,
    Today,
    Downloads,
    Disk,
    History,
    Notices,
}

#[derive(Clone, Copy, PartialEq, Debug)]
pub enum Hit {
    Prayer(panel::Hit),
    Focus(focus_panel::Hit),
    Drink(drink_panel::Hit),
    Tasks(tasks_panel::Hit),
    Today(today_panel::Hit),
    Downloads(downloads_panel::Hit),
    Disk(disk_panel::Hit),
    History(history_panel::Hit),
    Notices(notices_panel::Hit),
}

/// The panel's logical size; Focus and Drink change height with their state.
fn size(app: &App) -> (f32, f32) {
    match app.panel_kind {
        Kind::Prayer => (panel::W, panel::H),
        Kind::Focus => (
            focus_panel::W,
            focus_panel::height(&habits::focus_view(app)),
        ),
        Kind::Drink => (
            drink_panel::W,
            drink_panel::height(&habits::drink_view(app)),
        ),
        Kind::Tasks => (tasks_panel::W, tasks_panel::H),
        Kind::Today => (today_panel::W, calendar::height(app)),
        Kind::Downloads => (downloads_panel::W, downloads::height(app)),
        Kind::Disk => disk::size(app),
        Kind::History => history::size(app),
        Kind::Notices => notices::size(app),
    }
}

fn hit(app: &App, x: f32, y: f32) -> Option<Hit> {
    match app.panel_kind {
        Kind::Prayer => panel::hit(&app.g, &app.fonts, x, y).map(Hit::Prayer),
        Kind::Focus => {
            focus_panel::hit(&app.g, &app.fonts, &habits::focus_view(app), x, y).map(Hit::Focus)
        }
        Kind::Drink => {
            drink_panel::hit(&app.g, &app.fonts, &habits::drink_view(app), x, y).map(Hit::Drink)
        }
        Kind::Tasks => tasks::hit(app, x, y).map(Hit::Tasks),
        Kind::Today => calendar::hit(app, x, y).map(Hit::Today),
        Kind::Downloads => downloads::hit(app, x, y).map(Hit::Downloads),
        Kind::Disk => disk::hit(app, x, y).map(Hit::Disk),
        Kind::History => history::hit(app, x, y).map(Hit::History),
        Kind::Notices => notices::hit(app, x, y).map(Hit::Notices),
    }
}

/// The Focus setup takes the arrow keys for its knob; Today moves between its rows.
pub fn wants_arrows(app: &App) -> bool {
    app.panel_open
        && ((app.panel_kind == Kind::Focus && !habits::focus_running(app))
            || app.panel_kind == Kind::Today)
}

/// Today takes Enter for the row its keyboard focus is on.
pub fn wants_enter(app: &App) -> bool {
    app.panel_open && app.panel_kind == Kind::Today
}

pub fn on_arrow(app: &mut App, up: bool) {
    match app.panel_kind {
        Kind::Today => calendar::on_arrow(app, up),
        _ => habits::knob_key(app, up),
    }
}

fn place(app: &App) {
    let (w, h) = size(app);
    let (w, h) = (px(app, w), px(app, h));
    let gap = px(app, 12.0);
    // Slides 12 px in from the orb side as it opens.
    let slide = (1.0 - app.panel_reveal.pos.clamp(0.0, 1.0)) * px(app, 12.0);
    let x = match app.side {
        Side::Right => app.x.pos + px(app, INSET) - gap - w + slide,
        Side::Left => app.x.pos + px(app, WIN - INSET) + gap - slide,
    };
    let wa = work_area(app.hwnd);
    let (_, oy) = orb_center(app);
    let y = (oy - h / 2.0).clamp(
        wa.top as f32 + px(app, 8.0),
        wa.bottom as f32 - h - px(app, 8.0),
    );
    let raising = orb::raise_once(app.panel_hwnd).0 == 0;
    unsafe {
        let _ = SetWindowPos(
            app.panel_hwnd,
            Some(HWND_TOPMOST),
            x.round() as i32,
            y.round() as i32,
            w.ceil() as i32,
            h.ceil() as i32,
            SWP_NOACTIVATE | SWP_NOOWNERZORDER | orb::raise_once(app.panel_hwnd),
        );
    }
    // A notice on screen must stay readable: the pill keeps its place above a panel that opens under it.
    if raising && app.pill.is_some() {
        crate::pill::raise(app);
    }
}

fn set_region(app: &App, w: i32, h: i32) {
    unsafe {
        let d = px(app, 40.0).round() as i32;
        let rgn = CreateRoundRectRgn(0, 0, w + 1, h + 1, d, d);
        let _ = SetWindowRgn(app.panel_hwnd, Some(rgn), true);
    }
}

fn render(app: &mut App) {
    if !app.panel_open && app.panel_reveal.pos <= 0.02 {
        return;
    }
    if app.panel_kind == Kind::Disk {
        disk::prepare(app);
    }
    let (lw, lh) = size(app);
    let (w, h) = (px(app, lw).ceil() as u32, px(app, lh).ceil() as u32);
    match app.panel_surface.as_mut() {
        None => app.panel_surface = Surface::new(&app.g, app.panel_hwnd, w, h, app.scale).ok(),
        Some(s) if s.w != w || s.h != h => {
            let _ = s.resize(&app.g, w, h);
            place(app);
            set_region(app, w as i32, h as i32);
        }
        _ => {}
    }
    let Some(s) = app.panel_surface.as_ref() else {
        return;
    };
    let dc = s.begin(&app.g);
    let hover = app.panel_hover;
    match app.panel_kind {
        Kind::Prayer => prayer::draw_panel(
            app,
            dc,
            hover.and_then(|h| {
                if let Hit::Prayer(p) = h {
                    Some(p)
                } else {
                    None
                }
            }),
        ),
        Kind::Focus => habits::draw_focus_panel(
            app,
            dc,
            hover.and_then(|h| if let Hit::Focus(p) = h { Some(p) } else { None }),
        ),
        Kind::Drink => habits::draw_drink_panel(
            app,
            dc,
            hover.and_then(|h| if let Hit::Drink(p) = h { Some(p) } else { None }),
        ),
        Kind::Tasks => tasks::draw_panel(
            app,
            dc,
            hover.and_then(|h| if let Hit::Tasks(p) = h { Some(p) } else { None }),
        ),
        Kind::Today => calendar::draw_panel(
            app,
            dc,
            hover.and_then(|h| if let Hit::Today(p) = h { Some(p) } else { None }),
        ),
        Kind::Downloads => downloads::draw_panel(
            app,
            dc,
            hover.and_then(|h| {
                if let Hit::Downloads(p) = h {
                    Some(p)
                } else {
                    None
                }
            }),
        ),
        Kind::Disk => disk::draw_panel(
            app,
            dc,
            hover.and_then(|h| if let Hit::Disk(p) = h { Some(p) } else { None }),
        ),
        Kind::History => history::draw_panel(
            app,
            dc,
            hover.and_then(|h| {
                if let Hit::History(p) = h {
                    Some(p)
                } else {
                    None
                }
            }),
        ),
        Kind::Notices => notices::draw_panel(
            app,
            dc,
            hover.and_then(|h| {
                if let Hit::Notices(p) = h {
                    Some(p)
                } else {
                    None
                }
            }),
        ),
    }
    s.end(&app.g);
}

/// The countdowns tick once a second while their panel is open, and Today once a minute (its
/// now line and "in 1 h 6 min"); nothing else ticks.
fn sync_timer(app: &App) {
    let live = app.panel_open
        && (app.panel_kind == Kind::Prayer
            || (app.panel_kind == Kind::Focus
                && matches!(
                    app.habits.focus.phase(),
                    app_habits::focus::Phase::Focus { .. } | app_habits::focus::Phase::Break { .. }
                )));
    unsafe {
        if app.panel_open && app.panel_kind == Kind::Today {
            SetTimer(
                Some(app.panel_hwnd),
                TIMER_PANEL,
                calendar::ms_to_next_minute(),
                None,
            );
        } else if app.panel_open && app.panel_kind == Kind::History {
            // Today's square moves on at midnight.
            SetTimer(
                Some(app.panel_hwnd),
                TIMER_PANEL,
                history::ms_to_midnight(),
                None,
            );
        } else if let Some(ms) = notices::panel_timer_ms(app)
            .filter(|_| app.panel_open && app.panel_kind == Kind::Notices)
        {
            // Undo after Clear all goes after 6 s.
            SetTimer(Some(app.panel_hwnd), TIMER_PANEL, ms, None);
        } else if live {
            SetTimer(Some(app.panel_hwnd), TIMER_PANEL, 1000, None);
        } else {
            let _ = KillTimer(Some(app.panel_hwnd), TIMER_PANEL);
        }
    }
}

/// Redraw in place, keeping the hover (download progress arrives several times a second).
pub fn redraw(app: &mut App) {
    if app.panel_open {
        render(app);
    }
}

/// Redraw after the data behind the open panel changed (and fix its keys and timer).
pub fn refresh(app: &mut App) {
    if app.panel_open {
        app.panel_hover = None;
        render(app);
        sync_timer(app);
        sync_keys(app);
    }
}

pub fn open_panel(app: &mut App, kind: Kind) {
    if app.panel_open && app.panel_kind == kind {
        return;
    }
    if app.pill.is_some() {
        dismiss_pill(app);
    }
    set_tucked(app, false);
    let switching = app.panel_open;
    if switching && app.panel_kind == Kind::Tasks {
        tasks::on_panel_hidden(app);
    }
    if switching && app.panel_kind == Kind::Disk {
        disk::on_panel_closed(app);
        release_keyboard(app);
    }
    if switching && app.panel_kind == Kind::History {
        history::on_closed(app);
        release_keyboard(app);
    }
    if switching && app.panel_kind == Kind::Notices {
        notices::on_panel_closed(app);
        release_keyboard(app);
    }
    app.panel_kind = kind;
    app.panel_open = true;
    app.panel_hover = None;
    if !switching {
        app.panel_reveal.k = 190.0; // "settle"
        app.panel_reveal.c = 26.0;
        app.panel_reveal.target = 1.0;
    }
    place(app);
    let (w, h) = size(app);
    set_region(app, px(app, w).ceil() as i32, px(app, h).ceil() as i32);
    unsafe {
        let _ = KillTimer(Some(app.hwnd), TIMER_TUCK);
        let _ = ShowWindow(app.panel_hwnd, SW_SHOWNOACTIVATE);
    }
    hook::set_watch_clicks(true);
    render(app);
    sync_timer(app);
    sync_keys(app);
    start_anim(app);
    if kind == Kind::Tasks {
        tasks::on_panel_shown(app);
    }
    if kind == Kind::Disk {
        let ok = take_keyboard(app);
        log_line(
            app,
            &format!("disk: panel takes the keyboard (foreground={ok})"),
        );
    }
    if kind == Kind::History {
        let ok = take_keyboard(app);
        log_line(
            app,
            &format!("history: panel takes the keyboard (foreground={ok})"),
        );
    }
    log_line(app, &format!("panel {kind:?}"));
}

/// Esc (a hotkey while a panel is open): Tasks clears its field first.
pub fn on_esc(app: &mut App) {
    if app.panel_open && app.panel_kind == Kind::Tasks {
        tasks::on_esc(app);
    } else if app.panel_open && app.panel_kind == Kind::Disk {
        disk::on_esc(app);
    } else {
        close_panel(app);
    }
}

pub fn close_panel(app: &mut App) {
    if !app.panel_open {
        return;
    }
    app.panel_open = false;
    app.knob_drag = false;
    downloads::close_menu(app);
    if app.panel_kind == Kind::Tasks {
        tasks::on_panel_hidden(app);
    }
    if app.panel_kind == Kind::Disk {
        disk::on_panel_closed(app);
        release_keyboard(app);
    }
    if app.panel_kind == Kind::History {
        history::on_closed(app);
        release_keyboard(app);
    }
    if app.panel_kind == Kind::Notices {
        notices::on_panel_closed(app);
        release_keyboard(app);
    }
    app.panel_reveal.k = 480.0;
    app.panel_reveal.c = 40.0;
    app.panel_reveal.target = 0.0;
    sync_timer(app);
    hook::set_watch_clicks(app.dial_open);
    sync_keys(app);
    arm_tuck_timer(app);
    start_anim(app);
}

fn click(app: &mut App, h: Hit) {
    match h {
        Hit::Prayer(p) => prayer::panel_click(app, p),
        Hit::Focus(f) => habits::focus_click(app, f),
        Hit::Drink(d) => habits::drink_click(app, d),
        Hit::Tasks(t) => tasks::click(app, t),
        Hit::Today(t) => calendar::click(app, t),
        Hit::Downloads(d) => downloads::click(app, d),
        Hit::Disk(d) => disk::click(app, d),
        Hit::History(h) => history::click(app, h),
        Hit::Notices(h) => notices::click(app, h),
    }
}

/// One animation frame: the reveal spring and slide, then hide the window once it has closed.
pub fn tick(app: &mut App, dt: f32) {
    let was_visible = app.panel_open || app.panel_reveal.pos > 0.02;
    app.panel_reveal.step(dt);
    if was_visible {
        place(app);
        if !app.panel_open && app.panel_reveal.pos <= 0.02 {
            app.panel_reveal.snap(0.0);
            unsafe {
                let _ = ShowWindow(app.panel_hwnd, SW_HIDE);
            }
        } else {
            render(app);
        }
    }
}

pub fn is_animating(app: &App) -> bool {
    !app.panel_reveal.settled(0.002)
}

fn knob_at(app: &mut App, x: f32, y: f32, persist: bool) {
    let v = focus_panel::knob_value(x / app.scale, y / app.scale);
    habits::set_knob(app, v, persist);
}

fn tasks_open(app: &App) -> bool {
    app.panel_open && app.panel_kind == Kind::Tasks
}

fn disk_open(app: &App) -> bool {
    app.panel_open && app.panel_kind == Kind::Disk
}

fn history_open(app: &App) -> bool {
    app.panel_open && app.panel_kind == Kind::History
}

fn notices_open(app: &App) -> bool {
    app.panel_open && app.panel_kind == Kind::Notices
}

/// A panel that types (Tasks, Disk) takes the keyboard: the window drops WS_EX_NOACTIVATE and
/// comes to the foreground. Returns whether Windows let it.
pub fn take_keyboard(app: &mut App) -> bool {
    unsafe {
        let fg = GetForegroundWindow();
        if fg != app.panel_hwnd {
            app.panel_prev_fg = fg;
        }
        let ex = GetWindowLongPtrW(app.panel_hwnd, GWL_EXSTYLE);
        SetWindowLongPtrW(
            app.panel_hwnd,
            GWL_EXSTYLE,
            ex & !(WS_EX_NOACTIVATE.0 as isize),
        );
        let ok = SetForegroundWindow(app.panel_hwnd).as_bool();
        let _ = SetFocus(Some(app.panel_hwnd));
        ok
    }
}

/// Gives the keyboard back to the window that had it before the panel took it.
pub fn release_keyboard(app: &mut App) {
    unsafe {
        let ex = GetWindowLongPtrW(app.panel_hwnd, GWL_EXSTYLE);
        SetWindowLongPtrW(
            app.panel_hwnd,
            GWL_EXSTYLE,
            ex | WS_EX_NOACTIVATE.0 as isize,
        );
        let prev = app.panel_prev_fg;
        if GetForegroundWindow() == app.panel_hwnd
            && !prev.is_invalid()
            && IsWindow(Some(prev)).as_bool()
        {
            let _ = SetForegroundWindow(prev);
        }
    }
    app.panel_prev_fg = HWND::default();
}

pub unsafe extern "system" fn panel_proc(
    hwnd: HWND,
    msg: u32,
    wparam: WPARAM,
    lparam: LPARAM,
) -> LRESULT {
    match msg {
        WM_MOUSEACTIVATE => {
            let activate = with_app(|app| {
                tasks_open(app) || disk_open(app) || history_open(app) || notices_open(app)
            })
            .unwrap_or(false);
            return LRESULT(if activate { MA_ACTIVATE } else { MA_NOACTIVATE } as isize);
        }
        WM_ACTIVATE => {
            let active = (wparam.0 & 0xffff) as u32 != WA_INACTIVE;
            with_app(|app| {
                if tasks_open(app) {
                    tasks::on_activate(app, active);
                }
            });
        }
        WM_KEYDOWN => {
            let handled = with_app(|app| {
                (tasks_open(app) && tasks::on_keydown(app, wparam.0 as u16))
                    || (disk_open(app) && disk::on_keydown(app, wparam.0 as u16))
                    || (history_open(app) && history::on_keydown(app, wparam.0 as u16))
                    || (notices_open(app) && notices::on_keydown(app, wparam.0 as u16))
            })
            .unwrap_or(false);
            if handled {
                return LRESULT(0);
            }
        }
        WM_CHAR => {
            with_app(|app| {
                if tasks_open(app) {
                    tasks::on_char(app, wparam.0 as u16);
                }
            });
            return LRESULT(0);
        }
        WM_LBUTTONDOWN => {
            with_app(|app| {
                let (x, y) = unpack(lparam);
                match hit(app, x / app.scale, y / app.scale) {
                    Some(Hit::Focus(focus_panel::Hit::Knob)) => {
                        app.knob_drag = true;
                        let _ = SetCapture(hwnd);
                        knob_at(app, x, y, false);
                    }
                    Some(Hit::Tasks(tasks_panel::Hit::Field)) => {
                        tasks::press_field(app, x / app.scale)
                    }
                    _ => {}
                }
            });
            return LRESULT(0);
        }
        WM_MOUSEMOVE => {
            with_app(|app| {
                let (x, y) = unpack(lparam);
                if app.knob_drag {
                    knob_at(app, x, y, false);
                    return;
                }
                let h = hit(app, x / app.scale, y / app.scale);
                if !app.panel_tracking {
                    let mut tme = TRACKMOUSEEVENT {
                        cbSize: std::mem::size_of::<TRACKMOUSEEVENT>() as u32,
                        dwFlags: TME_LEAVE,
                        hwndTrack: hwnd,
                        dwHoverTime: 0,
                    };
                    let _ = TrackMouseEvent(&mut tme);
                    app.panel_tracking = true;
                }
                if h != app.panel_hover {
                    app.panel_hover = h;
                    render(app);
                }
            });
            return LRESULT(0);
        }
        MSG_MOUSELEAVE => {
            with_app(|app| {
                app.panel_tracking = false;
                if app.panel_hover.is_some() {
                    app.panel_hover = None;
                    render(app);
                }
            });
            return LRESULT(0);
        }
        WM_RBUTTONUP => {
            with_app(|app| {
                if notices_open(app) {
                    let (x, y) = unpack(lparam);
                    let h = notices::hit(app, x / app.scale, y / app.scale);
                    notices::right_click(app, h);
                }
            });
            return LRESULT(0);
        }
        WM_LBUTTONUP => {
            with_app(|app| {
                let (x, y) = unpack(lparam);
                if app.knob_drag {
                    app.knob_drag = false;
                    let _ = ReleaseCapture();
                    knob_at(app, x, y, true);
                    return;
                }
                if let Some(h) = hit(app, x / app.scale, y / app.scale) {
                    click(app, h);
                }
            });
            return LRESULT(0);
        }
        WM_MOUSEWHEEL => {
            let delta = ((wparam.0 >> 16) & 0xffff) as u16 as i16;
            with_app(|app| {
                if app.panel_open && app.panel_kind == Kind::Today {
                    calendar::on_wheel(app, delta);
                } else if app.panel_open && app.panel_kind == Kind::Downloads {
                    downloads::on_wheel(app, delta);
                } else if disk_open(app) {
                    let (sx, sy) = unpack(lparam);
                    let mut p = POINT {
                        x: sx as i32,
                        y: sy as i32,
                    };
                    let _ = ScreenToClient(hwnd, &mut p);
                    disk::on_wheel(app, p.x as f32 / app.scale, p.y as f32 / app.scale, delta);
                } else if wants_arrows(app) {
                    habits::knob_key(app, delta > 0);
                } else if tasks_open(app) {
                    // The wheel's point is in screen pixels.
                    let (sx, sy) = unpack(lparam);
                    let mut p = POINT {
                        x: sx as i32,
                        y: sy as i32,
                    };
                    let _ = ScreenToClient(hwnd, &mut p);
                    tasks::on_wheel(app, p.x as f32 / app.scale, delta);
                }
            });
            return LRESULT(0);
        }
        WM_TIMER => {
            if wparam.0 == TIMER_PANEL {
                with_app(|app| {
                    render(app);
                    if matches!(app.panel_kind, Kind::Today | Kind::History | Kind::Notices) {
                        sync_timer(app);
                    }
                });
            }
            return LRESULT(0);
        }
        _ => {}
    }
    DefWindowProcW(hwnd, msg, wparam, lparam)
}
