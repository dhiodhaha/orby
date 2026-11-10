//! The panel host on the Mac (`crate::panels` on macOS, the same calls as Windows' panels.rs):
//! the one panel window beside the drop shows one panel at a time, sized to it, and routes the
//! pointer and keys to that panel's unit.
//!
//! It draws only when something changed. `refresh`/`redraw` and a hover change mark the panel
//! dirty; `App::tick` draws it once (the run loop wakes one frame later for that). While a
//! number rolls or rows ripple (`motion::active`) it draws every frame, and once more at rest.
//! The Prayer panel's countdown has its own small layer: a second turning redraws only it (the
//! panel timer wakes just after each second), the whole panel once a minute. Focus running
//! ticks once a second, History at midnight, Notices while its Undo lasts, Today once a minute;
//! nothing else ticks. Downloads is not on the Mac yet: opening it says so in a pill.
//!
//! The Tasks panel types: it takes the keyboard (Orby activates, the panel window becomes key
//! and its `InputView` first responder), and `on_input` feeds what the input view delivers to
//! tasks.rs as Windows' WM_KEYDOWN and WM_CHAR would.

use crate::canvas::Canvas;
use crate::mac::app::Ev;
use crate::mac::cg::Bitmap;
use crate::mac::clock;
use crate::mac::input::Input;
use crate::sys::{self, vk, Timer};
use crate::ui::{info_pill, Icon};
use crate::{
    calendar, downloads_panel, drink_panel, focus_panel, habits, history, history_panel, notices,
    notices_panel, panel, prayer, tasks, tasks_panel, today_panel, App,
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

/// Whether the panel's unit runs on the Mac yet.
fn on_mac(kind: Kind) -> bool {
    kind != Kind::Downloads
}

/// A panel's logical size; Focus, Drink, History and Notices change height with their state.
pub fn size_of(app: &App, kind: Kind) -> (f32, f32) {
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
        Kind::History => history::size(app),
        Kind::Notices => notices::size(app),
        Kind::Tasks => (tasks_panel::W, tasks_panel::H),
        Kind::Today => (today_panel::W, calendar::height(app)),
        // Not on the Mac yet (never open).
        Kind::Downloads => (panel::W, panel::H),
    }
}

fn hit(app: &App, x: f32, y: f32) -> Option<Hit> {
    match app.panel_kind {
        Kind::Prayer => panel::hit(&app.fonts, x, y).map(Hit::Prayer),
        Kind::Focus => focus_panel::hit(&app.fonts, &habits::focus_view(app), x, y).map(Hit::Focus),
        Kind::Drink => drink_panel::hit(&app.fonts, &habits::drink_view(app), x, y).map(Hit::Drink),
        Kind::History => history::hit(app, x, y).map(Hit::History),
        Kind::Notices => notices::hit(app, x, y).map(Hit::Notices),
        Kind::Tasks => tasks::hit(app, x, y).map(Hit::Tasks),
        Kind::Today => calendar::hit(app, x, y).map(Hit::Today),
        Kind::Downloads => None,
    }
}

/// One panel's content. `apart`: the Prayer countdown is left to its own layer.
fn draw_kind(app: &App, cv: &mut dyn Canvas, kind: Kind, hover: Option<Hit>, apart: bool) {
    match kind {
        Kind::Prayer => prayer::draw_panel(
            app,
            cv,
            hover.and_then(|h| {
                if let Hit::Prayer(p) = h {
                    Some(p)
                } else {
                    None
                }
            }),
            apart,
        ),
        Kind::Focus => habits::draw_focus_panel(
            app,
            cv,
            hover.and_then(|h| if let Hit::Focus(p) = h { Some(p) } else { None }),
        ),
        Kind::Drink => habits::draw_drink_panel(
            app,
            cv,
            hover.and_then(|h| if let Hit::Drink(p) = h { Some(p) } else { None }),
        ),
        Kind::History => history::draw_panel(
            app,
            cv,
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
            cv,
            hover.and_then(|h| {
                if let Hit::Notices(p) = h {
                    Some(p)
                } else {
                    None
                }
            }),
        ),
        Kind::Tasks => tasks::draw_panel(
            app,
            cv,
            hover.and_then(|h| if let Hit::Tasks(p) = h { Some(p) } else { None }),
        ),
        Kind::Today => calendar::draw_panel(
            app,
            cv,
            hover.and_then(|h| if let Hit::Today(p) = h { Some(p) } else { None }),
        ),
        Kind::Downloads => {}
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

/// What a panel does as it leaves the window (closed, or another takes its place).
fn leave(app: &mut App, kind: Kind) {
    match kind {
        Kind::Tasks => tasks::on_panel_hidden(app),
        Kind::History => {
            history::on_closed(app);
            release_keyboard(app);
        }
        Kind::Notices => {
            notices::on_panel_closed(app);
            release_keyboard(app);
        }
        _ => {}
    }
}

pub fn open_panel(app: &mut App, kind: Kind) {
    if !on_mac(kind) {
        crate::log_line(app, &format!("panel {kind:?} is coming to Mac"));
        notices::reply(
            app,
            info_pill(Icon::Info, "Coming to Mac", &format!("{kind:?}")),
        );
        return;
    }
    if app.panel_open && app.panel_kind == kind {
        return;
    }
    if app.pill.is_some() {
        crate::pill::dismiss_pill(app);
    }
    app.close_dial();
    app.set_tucked(false);
    app.tuck_at = None;
    let switching = app.panel_open;
    if switching {
        let old = app.panel_kind;
        leave(app, old);
    }
    app.theme = crate::ui::theme(crate::mac::app::dark_mode());
    if kind == Kind::Prayer {
        app.refresh_day();
    }
    app.panel_kind = kind;
    app.panel_open = true;
    app.panel_hover = None;
    app.panel_back = None;
    app.knob_drag = false;
    // Its rows ripple in on the first draw.
    crate::motion::ripple_reset_all();
    app.countdown_view.setHidden(kind != Kind::Prayer);
    render(app);
    app.panel_win.show();
    app.panel_win.make_key();
    // The panel's own view takes the keys (Tasks hands them to its input view below).
    app.panel_win.set_responder(None);
    sync_timer(app);
    if kind == Kind::Tasks {
        tasks::on_panel_shown(app);
    }
    if kind == Kind::History {
        let ok = take_keyboard(app);
        crate::log_line(
            app,
            &format!("history: panel takes the keyboard (foreground={ok})"),
        );
    }
    let (w, h) = size_of(app, kind);
    crate::log_line(app, &format!("panel {kind:?} {w:.0}x{h:.0}"));
}

pub fn close_panel(app: &mut App) {
    if !app.panel_open {
        return;
    }
    app.panel_open = false;
    app.panel_back = None;
    app.knob_drag = false;
    app.panel_hover = None;
    let kind = app.panel_kind;
    leave(app, kind);
    app.panel_win.hide();
    sync_timer(app);
    app.arm_tuck();
    crate::log_line(app, &format!("panel {kind:?} closed"));
}

/// Redraw after the data behind the open panel changed (and fix its timer).
pub fn refresh(app: &mut App) {
    if app.panel_open {
        app.panel_hover = None;
        app.panel_dirty = true;
        sync_timer(app);
    }
}

/// Redraw in place, keeping the hover.
pub fn redraw(app: &mut App) {
    if app.panel_open {
        app.panel_dirty = true;
    }
}

/// A panel takes the keyboard. The panel window becomes key; for Tasks, which types, Orby also
/// activates (as Windows' SetForegroundWindow; the app that was in front is remembered) and the
/// panel's input view becomes first responder, so keys and input methods reach the field.
/// Returns whether the window is key.
pub fn take_keyboard(app: &mut App) -> bool {
    if app.panel_kind == Kind::Tasks {
        app.take_panel_keys()
    } else {
        app.panel_win.make_key()
    }
}

/// Gives the keyboard back: the input view resigns (a composition is dropped) and the app that
/// was in front before Orby took the keyboard comes back. The rest goes by itself when the
/// panel window hides.
pub fn release_keyboard(app: &mut App) {
    app.release_panel_keys();
}

/// Opens `kind` from inside the open panel; Esc there comes back.
pub fn drill_in(app: &mut App, kind: Kind) {
    let back = app.panel_open.then_some(app.panel_kind);
    open_panel(app, kind);
    app.panel_back = back.filter(|b| *b != kind);
}

/// Esc with nothing left to clear: back to the panel this one was opened from, else close.
pub fn back_or_close(app: &mut App) {
    match app.panel_back.take() {
        Some(k) if app.panel_open => open_panel(app, k),
        _ => close_panel(app),
    }
}

/// Esc: Tasks clears its field first.
pub fn on_esc(app: &mut App) {
    if app.panel_open && app.panel_kind == Kind::Tasks {
        tasks::on_esc(app);
    } else {
        back_or_close(app);
    }
}

// ---- the Mac host ----

/// The panel's tick, as on Windows: the countdown each second, Focus running each second,
/// History at midnight, Notices while Undo lasts.
fn sync_timer(app: &mut App) {
    let focus_live = app.panel_kind == Kind::Focus
        && matches!(
            app.habits.focus.phase(),
            app_habits::focus::Phase::Focus { .. } | app_habits::focus::Phase::Break { .. }
        );
    let ms = if !app.panel_open {
        None
    } else {
        match app.panel_kind {
            // Just after the wall-clock second turns, when the countdown changes.
            Kind::Prayer => Some(clock::ms_to_next_second() as u32 + 10),
            Kind::History => Some(history::ms_to_midnight()),
            Kind::Notices => notices::panel_timer_ms(app),
            // The now line, Join and "in 1 h 6 min" change on the minute.
            Kind::Today => Some(calendar::ms_to_next_minute()),
            Kind::Focus if focus_live => Some(1000),
            _ => None,
        }
    };
    match ms {
        Some(ms) => sys::arm(app, Timer::Panel, ms),
        None => sys::disarm(app, Timer::Panel),
    }
}

/// `Timer::Panel` fired.
pub(crate) fn on_timer(app: &mut App) {
    if !app.panel_open {
        sys::disarm(app, Timer::Panel);
        return;
    }
    if app.panel_kind == Kind::Prayer {
        if clock::now_minutes().floor() as i64 != app.panel_min {
            render(app);
        } else {
            render_countdown(app);
        }
    } else {
        app.panel_dirty = true;
    }
    sync_timer(app);
}

/// The whole panel: sized to its kind (the window keeps its place by the drop), drawn once.
pub(crate) fn render(app: &mut App) {
    let kind = app.panel_kind;
    let (w, h) = size_of(app, kind);
    if (w - app.panel_win.w).abs() > 0.01 || (h - app.panel_win.h).abs() > 0.01 {
        app.panel_win.resize(w, h);
    }
    app.place_panel();
    {
        let a: &App = app;
        let hover = a.panel_hover;
        a.panel_win
            .paint(a.scale, |cv| draw_kind(a, cv, kind, hover, true));
    }
    app.panel_dirty = false;
    app.panel_min = clock::now_minutes().floor() as i64;
    if kind == Kind::Prayer {
        app.countdown_shown.clear();
        render_countdown(app);
    }
}

/// Only the Prayer countdown's small layer: when its digits change and while they roll.
fn render_countdown(app: &mut App) {
    let shown = prayer::countdown_text(app);
    let rolling = crate::motion::active();
    if shown == app.countdown_shown && !rolling && !app.countdown_rolling {
        return;
    }
    app.countdown_rolling = rolling;
    let (l, t, r, b) = panel::countdown_rect(&app.fonts);
    {
        let a: &App = app;
        a.panel_win
            .paint_sub(&a.countdown_view, r - l, b - t, a.scale, |cv| {
                prayer::draw_countdown(a, cv)
            });
    }
    app.countdown_shown = shown;
}

/// One run-loop wake: draw what changed, or the frames of a roll or ripple.
pub(crate) fn tick(app: &mut App) {
    if !app.panel_open {
        return;
    }
    let moving = crate::motion::active();
    if app.panel_dirty {
        render(app);
    } else if app.panel_kind == Kind::Prayer {
        render_countdown(app);
    } else if moving || app.panel_moving {
        // Every frame while it moves, and one more once it has settled.
        render(app);
    }
    app.panel_moving = moving && app.panel_kind != Kind::Prayer;
}

/// Whether the run loop must wake next frame for the panel.
pub(crate) fn animating(app: &App) -> bool {
    app.panel_open
        && (app.panel_dirty || crate::motion::active() || app.panel_moving || app.countdown_rolling)
}

fn knob_at(app: &mut App, x: f32, y: f32, persist: bool) {
    let v = focus_panel::knob_value(x, y);
    habits::set_knob(app, v, persist);
}

fn click(app: &mut App, h: Hit) {
    match h {
        Hit::Prayer(p) => prayer::panel_click(app, p),
        Hit::Focus(f) => habits::focus_click(app, f),
        Hit::Drink(d) => habits::drink_click(app, d),
        Hit::History(h) => history::click(app, h),
        Hit::Notices(h) => notices::click(app, h),
        Hit::Tasks(t) => tasks::click(app, t),
        Hit::Today(t) => calendar::click(app, t),
        Hit::Downloads(_) => {}
    }
}

/// The pointer in the panel window (`x`, `y` in points from its top-left).
pub(crate) fn on_mouse(app: &mut App, ev: Ev, x: f32, y: f32) {
    if !app.panel_open {
        return;
    }
    if ev == Ev::Down && app.panel_kind == Kind::Tasks {
        // A click makes the panel's view first responder; the field's input view keeps the keys.
        app.panel_win.set_responder(Some(&app.panel_input));
    }
    match ev {
        Ev::Down => match hit(app, x, y) {
            Some(Hit::Focus(focus_panel::Hit::Knob)) => {
                app.knob_drag = true;
                knob_at(app, x, y, false);
            }
            Some(Hit::Tasks(tasks_panel::Hit::Field)) => tasks::press_field(app, x),
            _ => {}
        },
        Ev::Drag if app.knob_drag => knob_at(app, x, y, false),
        Ev::Move if app.knob_drag => knob_at(app, x, y, false),
        Ev::Move | Ev::Exit => {
            let h = if ev == Ev::Exit { None } else { hit(app, x, y) };
            if h != app.panel_hover {
                app.panel_hover = h;
                app.panel_dirty = true;
            }
        }
        Ev::Up => {
            if app.knob_drag {
                app.knob_drag = false;
                knob_at(app, x, y, true);
                return;
            }
            if let Some(h) = hit(app, x, y) {
                click(app, h);
            }
        }
        Ev::RightUp => {
            if app.panel_kind == Kind::Notices {
                let h = notices::hit(app, x, y);
                notices::right_click(app, h);
            }
        }
        Ev::Drag => {}
    }
}

/// A scroll over the window in Windows' wheel units (120 a notch, which the panels turn into
/// 48 px): a trackpad's pixels go through as they are (`dy` px is `dy * 2.5` units), a mouse
/// wheel's lines as notches.
pub(crate) fn wheel_delta(dy: f64, precise: bool) -> i16 {
    let units = if precise { dy * 2.5 } else { dy * 120.0 };
    units.round().clamp(-3600.0, 3600.0) as i16
}

/// The scroll wheel over the panel: the Focus knob, Today's and Tasks' lists.
pub(crate) fn on_wheel(app: &mut App, x: f32, dy: f64, precise: bool) {
    if !app.panel_open || dy == 0.0 {
        return;
    }
    match app.panel_kind {
        Kind::Today => calendar::on_wheel(app, wheel_delta(dy, precise)),
        Kind::Tasks => tasks::on_wheel(app, x, wheel_delta(dy, precise)),
        _ if wants_arrows(app) => habits::knob_key(app, dy > 0.0),
        _ => {}
    }
}

/// A key while the panel window is key (Windows virtual-key number). Returns whether it was used.
pub(crate) fn on_key(app: &mut App, k: u16) -> bool {
    if !app.panel_open {
        return false;
    }
    let used = match app.panel_kind {
        Kind::History => history::on_keydown(app, k),
        Kind::Notices => notices::on_keydown(app, k),
        Kind::Tasks => tasks::on_keydown(app, k),
        _ => false,
    };
    if used {
        return true;
    }
    match k {
        vk::VK_ESCAPE => on_esc(app),
        vk::VK_UP | vk::VK_DOWN if wants_arrows(app) => on_arrow(app, k == vk::VK_UP),
        vk::VK_RETURN if wants_enter(app) => calendar::on_enter(app),
        _ => return false,
    }
    true
}

/// The Windows virtual key a typed character's key has ('A'..'Z', '0'..'9', Space): a unit's
/// keydown sees it before the character, as WM_KEYDOWN comes before WM_CHAR.
pub(crate) fn vk_of_char(c: char) -> Option<u16> {
    match c {
        ' ' => Some(vk::VK_SPACE),
        c if c.is_ascii_alphanumeric() => Some(c.to_ascii_uppercase() as u16),
        _ => None,
    }
}

/// Typed text as Windows delivers it: one character's key as WM_KEYDOWN first, then each
/// UTF-16 unit as WM_CHAR.
pub(crate) fn type_text(
    app: &mut App,
    s: &str,
    keydown: fn(&mut App, u16) -> bool,
    char_unit: fn(&mut App, u16),
) {
    let mut chars = s.chars();
    if let (Some(c), None) = (chars.next(), chars.next()) {
        if let Some(k) = vk_of_char(c) {
            keydown(app, k);
        }
    }
    for u in s.encode_utf16() {
        char_unit(app, u);
    }
}

/// What the panel's input view delivers (only the Tasks panel types).
pub(crate) fn on_input(app: &mut App, i: Input) {
    if !(app.panel_open && app.panel_kind == Kind::Tasks) {
        return;
    }
    match i {
        Input::Text(s) => type_text(app, &s, tasks::on_keydown, tasks::on_char),
        Input::Marked(s) => {
            app.tasks.field.compose(&s);
            app.tasks.notice = None;
            refresh(app);
        }
        Input::Key { vk: k, mods } if mods.cmd => match k {
            // Copy and cut: the Mac's own (Windows' quick add has neither).
            0x43 | 0x58 => {
                let sel = app.tasks.field.selected_text().to_owned();
                if !sel.is_empty() {
                    sys::write_clipboard(&sel);
                    if k == 0x58 {
                        app.tasks.field.insert("");
                        refresh(app);
                    }
                }
            }
            0x57 => close_panel(app),
            0x51 => {
                app.log("quit (Cmd+Q in the Tasks panel)");
                std::process::exit(0);
            }
            _ => {
                on_key(app, k);
            }
        },
        Input::Key { vk: k, .. } => {
            on_key(app, k);
        }
    }
}

/// Test channel `panel-shot:<png>`: the open panel, whole (the countdown drawn in), at 2x.
pub(crate) fn shot(app: &mut App, path: &str) {
    if !app.panel_open {
        app.log("panel-shot: no panel open");
        return;
    }
    let kind = app.panel_kind;
    let (w, h) = size_of(app, kind);
    let b = Bitmap::new(w, h, 2.0);
    draw_kind(app, &mut b.canvas(), kind, app.panel_hover, false);
    let p = std::path::Path::new(path);
    if let (Some(dir), Some(name)) = (p.parent(), p.file_stem()) {
        crate::snapshot::write(
            dir,
            &name.to_string_lossy(),
            b.pw as u32,
            b.ph as u32,
            b.rgba(),
        );
    }
    app.log(&format!("panel-shot: {kind:?} {w:.0}x{h:.0} -> {path}"));
}
