//! The one panel window beside the drop. It shows one panel at a time (Prayer, Focus, Drink or
//! Tasks), grows to that panel's height, and closes on Esc or a click outside. Only Tasks takes
//! the keyboard (its quick add field); the others never activate.

use windows::Win32::Foundation::*;
use windows::Win32::Graphics::Gdi::*;
use windows::Win32::UI::Input::KeyboardAndMouse::*;
use windows::Win32::UI::WindowsAndMessaging::*;

use windows::Win32::Graphics::Direct2D::Common::{D2D1_COLOR_F, D2D_RECT_F};
use windows::Win32::Graphics::Direct2D::{
    ID2D1DeviceContext, D2D1_ANTIALIAS_MODE_PER_PRIMITIVE, D2D1_LAYER_PARAMETERS1,
};

use crate::gfx::{brush, rrect, Surface};
use crate::hook::unpack;
use crate::morph::{self, Rect};
use crate::orb::{self, arm_tuck_timer, orb_center, set_tucked, work_area, Side, INSET, WIN};
use crate::pill::dismiss_pill;
use crate::{
    calendar, downloads, downloads_panel, drink_panel, focus_panel, habits, history, history_panel,
    hook, log_line, notices, notices_panel, panel, prayer, px, start_anim, sync_keys, tasks,
    tasks_panel, today_panel, with_app, App, MSG_MOUSELEAVE, TIMER_PANEL, TIMER_TUCK,
};

#[derive(Clone, Copy, PartialEq, Debug)]
pub enum Kind {
    Prayer,
    Focus,
    Drink,
    Tasks,
    Today,
    Downloads,
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
    History(history_panel::Hit),
    Notices(notices_panel::Hit),
}

/// A panel's logical size; Focus and Drink change height with their state.
fn size_of(app: &App, kind: Kind) -> (f32, f32) {
    match kind {
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
        Kind::History => history::size(app),
        Kind::Notices => notices::size(app),
    }
}

fn hit(app: &App, x: f32, y: f32) -> Option<Hit> {
    let (ox, oy) = content_origin(app);
    let (x, y) = (x - ox, y - oy);
    match app.panel_kind {
        Kind::Prayer => panel::hit(&app.fonts, x, y).map(Hit::Prayer),
        Kind::Focus => {
            focus_panel::hit(&app.g, &app.fonts, &habits::focus_view(app), x, y).map(Hit::Focus)
        }
        Kind::Drink => {
            drink_panel::hit(&app.g, &app.fonts, &habits::drink_view(app), x, y).map(Hit::Drink)
        }
        Kind::Tasks => tasks::hit(app, x, y).map(Hit::Tasks),
        Kind::Today => calendar::hit(app, x, y).map(Hit::Today),
        Kind::Downloads => downloads::hit(app, x, y).map(Hit::Downloads),
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

/// How far the panel fades while the dial is open over it.
const DIMMED: f32 = 0.35;

/// Where a panel of `kind` rests beside the drop, in screen pixels (before the opening slide).
fn rest_rect(app: &App, kind: Kind) -> Rect {
    let (w, h) = size_of(app, kind);
    let (w, h) = (px(app, w).ceil(), px(app, h).ceil());
    let gap = px(app, 12.0);
    let x = match app.side {
        Side::Right => app.x.pos + px(app, INSET) - gap - w,
        Side::Left => app.x.pos + px(app, WIN - INSET) + gap,
    };
    let wa = work_area(app.hwnd);
    let (_, oy) = orb_center(app);
    let y = (oy - h / 2.0).clamp(
        wa.top as f32 + px(app, 8.0),
        wa.bottom as f32 - h - px(app, 8.0),
    );
    let (l, t) = (x.round(), y.round());
    Rect {
        l,
        t,
        r: l + w,
        b: t + h,
    }
}

/// This frame's geometry. At rest the window, the shape and the panel are one rect. Mid-switch the
/// window covers both panels' rects, the visible shape moves from the old rect to the new one, and
/// each panel's content hugs the shape's orb-side top corner.
struct Frame {
    /// The window, in screen pixels, before the opening slide.
    win: Rect,
    /// The visible panel shape, in screen pixels.
    shape: Rect,
    /// Where the open panel's content starts, in pixels from the window's corner.
    at: (f32, f32),
    /// The panel being left, where its content starts, and both contents' opacity.
    old: Option<(Kind, (f32, f32))>,
    fade: (f32, f32),
}

fn frame(app: &App) -> Frame {
    let to = rest_rect(app, app.panel_kind);
    let Some((old, from)) = app.panel_morph.from else {
        return Frame {
            win: to,
            shape: to,
            at: (0.0, 0.0),
            old: None,
            fade: (0.0, 1.0),
        };
    };
    let e = app.panel_morph.e();
    let u = Rect::union(from, to);
    let win = Rect {
        l: u.l.floor(),
        t: u.t.floor(),
        r: u.r.ceil(),
        b: u.b.ceil(),
    };
    let shape = Rect::lerp(from, to, e);
    let fade = morph::fades(e);
    let right = matches!(app.side, Side::Right);
    // Whole pixels, so text stays crisp while it moves.
    let at = |w: f32, rise: f32| {
        let (x, y) = morph::anchor(shape, w, right);
        ((x - win.l).round(), (y + rise - win.t).round())
    };
    let old_w = px(app, size_of(app, old).0).ceil();
    Frame {
        win,
        shape,
        at: at(to.w(), px(app, morph::RISE) * (1.0 - fade.1)),
        old: Some((old, at(old_w, 0.0))),
        fade,
    }
}

/// Where the open panel's content starts in the window, in logical pixels (zero at rest).
fn content_origin(app: &App) -> (f32, f32) {
    if !app.panel_morph.active() {
        return (0.0, 0.0);
    }
    let (x, y) = frame(app).at;
    (x / app.scale, y / app.scale)
}

fn place(app: &mut App) {
    let f = frame(app);
    // Slides 12 px in from the orb side as it opens.
    let slide = (1.0 - app.panel_reveal.pos.clamp(0.0, 1.0)) * px(app, 12.0);
    let dx = match app.side {
        Side::Right => slide,
        Side::Left => -slide,
    };
    let raising = orb::raise_once(app.panel_hwnd).0 == 0;
    unsafe {
        let _ = SetWindowPos(
            app.panel_hwnd,
            Some(HWND_TOPMOST),
            (f.win.l + dx).round() as i32,
            f.win.t.round() as i32,
            f.win.w().round() as i32,
            f.win.h().round() as i32,
            SWP_NOACTIVATE | SWP_NOOWNERZORDER | orb::raise_once(app.panel_hwnd),
        );
    }
    // The region is the visible shape, so clicks and the click-outside test follow it mid-switch.
    let rgn = (
        (f.shape.l - f.win.l).round() as i32,
        (f.shape.t - f.win.t).round() as i32,
        (f.shape.r - f.win.l).round() as i32,
        (f.shape.b - f.win.t).round() as i32,
    );
    if app.panel_morph.rgn != Some(rgn) {
        set_region(app, rgn);
        app.panel_morph.rgn = Some(rgn);
    }
    // The shadow window sits right under the visible shape (depth.rs), fading with the panel.
    let shown = app.panel_reveal.pos.clamp(0.0, 1.0) * app.panel_dim.pos.clamp(0.0, 1.0);
    if app.panel_open || app.panel_reveal.pos > 0.02 {
        crate::depth::follow(
            &app.g,
            app.panel_hwnd,
            (f.shape.l + dx, f.shape.t, f.shape.r + dx, f.shape.b),
            20.0,
            app.scale,
            app.theme.dark,
            shown,
        );
    } else {
        crate::depth::hide(app.panel_hwnd);
    }
    // A notice on screen must stay readable: the pill keeps its place above a panel that opens under it.
    if raising && app.pill.is_some() {
        crate::pill::raise(app);
    }
    // The click-outside test reads a copy of our window rects. A panel that grows without an
    // animation frame (a notice cleared, a task added) must update that copy at once, or the
    // next click on its new part counts as outside and closes it.
    crate::hook::update_hook(app);
}

fn set_region(app: &App, (l, t, r, b): (i32, i32, i32, i32)) {
    unsafe {
        let d = px(app, 40.0).round() as i32;
        let rgn = CreateRoundRectRgn(l, t, r + 1, b + 1, d, d);
        let _ = SetWindowRgn(app.panel_hwnd, Some(rgn), true);
    }
}

fn render(app: &mut App) {
    if !app.panel_open && app.panel_reveal.pos <= 0.02 {
        return;
    }
    let f = frame(app);
    let (w, h) = (f.win.w().round() as u32, f.win.h().round() as u32);
    let resized = match app.panel_surface.as_mut() {
        None => {
            app.panel_surface = Surface::new(&app.g, app.panel_hwnd, w, h, app.scale).ok();
            false
        }
        Some(s) if s.w != w || s.h != h => {
            let _ = s.resize(&app.g, w, h);
            true
        }
        _ => false,
    };
    if resized {
        place(app);
    }
    let Some(s) = app.panel_surface.as_ref() else {
        return;
    };
    let dc = s.begin(&app.g);
    let hover = app.panel_hover;
    match f.old {
        None => {
            draw_kind(app, dc, app.panel_kind, hover);
            // The T3 lip along the panel's edge (depth.rs).
            let (w, h) = (f.win.w() / app.scale, f.win.h() / app.scale);
            crate::depth::lip(&app.g, dc, &app.theme, 0.0, 0.0, w, h, 20.0, 1.0);
        }
        Some((old, old_at)) => {
            let sc = app.scale;
            let shape = D2D_RECT_F {
                left: (f.shape.l - f.win.l) / sc,
                top: (f.shape.t - f.win.t) / sc,
                right: (f.shape.r - f.win.l) / sc,
                bottom: (f.shape.b - f.win.t) / sc,
            };
            let t = &app.theme;
            unsafe {
                dc.FillRoundedRectangle(
                    &rrect(shape.left, shape.top, shape.right, shape.bottom, 20.0),
                    &brush(dc, t.panel),
                );
            }
            in_shape(dc, &shape, old_at, sc, f.fade.0, || {
                draw_kind(app, dc, old, None)
            });
            in_shape(dc, &shape, f.at, sc, f.fade.1, || {
                draw_kind(app, dc, app.panel_kind, hover)
            });
            stroke_shape(dc, &shape, t.hair);
            crate::depth::lip(
                &app.g,
                dc,
                t,
                shape.left,
                shape.top,
                shape.right,
                shape.bottom,
                20.0,
                1.0,
            );
        }
    }
    s.end(&app.g);
    // A rolling number or a row ripple started in this draw: keep frames coming until it settles.
    if crate::motion::active() && app.anim_start.is_none() {
        start_anim(app);
    }
}

/// Draws one panel's content into the moving shape: clipped to it, faded, offset to `at` (pixels).
fn in_shape(
    dc: &ID2D1DeviceContext,
    shape: &D2D_RECT_F,
    at: (f32, f32),
    scale: f32,
    opacity: f32,
    draw: impl FnOnce(),
) {
    if opacity <= 0.003 {
        return;
    }
    let layer = D2D1_LAYER_PARAMETERS1 {
        contentBounds: D2D_RECT_F {
            left: -1e6,
            top: -1e6,
            right: 1e6,
            bottom: 1e6,
        },
        maskTransform: windows_numerics::Matrix3x2::identity(),
        opacity,
        ..Default::default()
    };
    unsafe {
        dc.PushAxisAlignedClip(shape, D2D1_ANTIALIAS_MODE_PER_PRIMITIVE);
        dc.PushLayer(&layer, None);
        dc.SetTransform(&translate(at.0 / scale, at.1 / scale));
    }
    draw();
    unsafe {
        dc.SetTransform(&translate(0.0, 0.0));
        dc.PopLayer();
        dc.PopAxisAlignedClip();
    }
}

fn stroke_shape(dc: &ID2D1DeviceContext, s: &D2D_RECT_F, hair: D2D1_COLOR_F) {
    unsafe {
        dc.DrawRoundedRectangle(
            &rrect(
                s.left + 0.5,
                s.top + 0.5,
                s.right - 0.5,
                s.bottom - 0.5,
                19.5,
            ),
            &brush(dc, hair),
            1.0,
            None,
        );
    }
}

fn translate(x: f32, y: f32) -> windows_numerics::Matrix3x2 {
    windows_numerics::Matrix3x2 {
        M11: 1.0,
        M12: 0.0,
        M21: 0.0,
        M22: 1.0,
        M31: x,
        M32: y,
    }
}

fn draw_kind(app: &App, dc: &ID2D1DeviceContext, kind: Kind, hover: Option<Hit>) {
    match kind {
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
        } else if live && app.panel_kind == Kind::Prayer {
            // Wake just after each wall-clock second turns, which is when the countdown
            // changes. A free-running 1000 ms timer drifts against it: two ticks land in one
            // second and the next one skips a second, so the digits stutter.
            let ms = 1000
                - u32::from(
                    windows::Win32::System::SystemInformation::GetLocalTime().wMilliseconds,
                )
                .min(999)
                + 15;
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
    // The shape on screen now: a switch grows from here (mid-switch too, so it never jumps).
    let leaving = (app.panel_kind, frame(app).shape);
    if switching && app.panel_kind == Kind::Tasks {
        tasks::on_panel_hidden(app);
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
    // Its rows ripple in on the first draw.
    crate::motion::ripple_reset_all();
    if !switching {
        app.panel_reveal.k = 190.0; // "settle"
        app.panel_reveal.c = 26.0;
        app.panel_reveal.target = 1.0;
    }
    // A switch while open morphs the window from one panel to the other (Animation effects off:
    // it changes at once).
    app.panel_morph.back = None;
    if switching && !downloads::reduced_motion() {
        app.panel_morph.start(leaving.0, leaving.1);
    } else {
        app.panel_morph.finish();
    }
    place(app);
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
    } else {
        back_or_close(app);
    }
}

/// Opens `kind` from inside the open panel (Today's "Add task"); Esc there comes back.
pub fn drill_in(app: &mut App, kind: Kind) {
    let back = app.panel_open.then_some(app.panel_kind);
    open_panel(app, kind);
    app.panel_morph.back = back.filter(|b| *b != kind);
}

/// Esc with nothing left to clear: back to the panel this one was opened from, else close.
pub fn back_or_close(app: &mut App) {
    match app.panel_morph.back.take() {
        Some(k) if app.panel_open => open_panel(app, k),
        _ => close_panel(app),
    }
}

pub fn close_panel(app: &mut App) {
    if !app.panel_open {
        return;
    }
    app.panel_open = false;
    app.panel_morph.back = None;
    app.knob_drag = false;
    downloads::close_menu(app);
    if app.panel_kind == Kind::Tasks {
        tasks::on_panel_hidden(app);
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
        Hit::History(h) => history::click(app, h),
        Hit::Notices(h) => notices::click(app, h),
    }
}

/// One animation frame: the reveal spring and slide, then hide the window once it has closed.
pub fn tick(app: &mut App, dt: f32) {
    let was_visible = app.panel_open || app.panel_reveal.pos > 0.02;
    app.panel_reveal.step(dt);
    if app.panel_morph.active() {
        app.panel_morph.p.step(dt);
        if app.panel_morph.p.settled(0.002) {
            app.panel_morph.finish();
        }
    }
    // With the dial open over it, the panel steps back so the eye goes to the dial.
    app.panel_dim.target = if app.dial_open && app.panel_open {
        DIMMED
    } else {
        1.0
    };
    app.panel_dim.step(dt);
    let dim = app.panel_dim.pos;
    if let Some(s) = app.panel_surface.as_mut() {
        s.set_opacity(&app.g, dim);
    }
    if was_visible {
        if !app.panel_open && app.panel_reveal.pos <= 0.02 {
            place(app);
            app.panel_reveal.snap(0.0);
            crate::depth::hide(app.panel_hwnd);
            unsafe {
                let _ = ShowWindow(app.panel_hwnd, SW_HIDE);
            }
        } else {
            // Draw first: when a switch ends, the surface shrinks and the window moves together.
            render(app);
            place(app);
        }
    }
}

pub fn is_animating(app: &App) -> bool {
    let dim_target = if app.dial_open && app.panel_open {
        DIMMED
    } else {
        1.0
    };
    app.panel_morph.active()
        || (app.panel_open && crate::motion::active())
        || !app.panel_reveal.settled(0.002)
        || !app.panel_dim.settled(0.002)
        || (app.panel_dim.pos - dim_target).abs() > 0.002
}

fn knob_at(app: &mut App, x: f32, y: f32, persist: bool) {
    let (ox, oy) = content_origin(app);
    let v = focus_panel::knob_value(x / app.scale - ox, y / app.scale - oy);
    habits::set_knob(app, v, persist);
}

fn tasks_open(app: &App) -> bool {
    app.panel_open && app.panel_kind == Kind::Tasks
}

fn history_open(app: &App) -> bool {
    app.panel_open && app.panel_kind == Kind::History
}

fn notices_open(app: &App) -> bool {
    app.panel_open && app.panel_kind == Kind::Notices
}

/// A panel that types takes the keyboard: the window drops WS_EX_NOACTIVATE and
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
            let activate =
                with_app(|app| tasks_open(app) || history_open(app) || notices_open(app))
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
                        tasks::press_field(app, x / app.scale - content_origin(app).0)
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
                    tasks::on_wheel(app, p.x as f32 / app.scale - content_origin(app).0, delta);
                }
            });
            return LRESULT(0);
        }
        WM_TIMER => {
            if wparam.0 == TIMER_PANEL {
                with_app(|app| {
                    render(app);
                    if matches!(
                        app.panel_kind,
                        Kind::Today | Kind::History | Kind::Notices | Kind::Prayer
                    ) {
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
