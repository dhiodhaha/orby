//! The notification pill window: the front pill grows out of the drop (one action, close,
//! auto-dismiss, hover pauses), up to two waiting notices peek out below it, and hovering spreads
//! the stack into a column with "N more" (docs/design/notices.md). What shows and when is
//! notices.rs; this file only draws and takes clicks.

use std::time::Instant;

use windows::Win32::Foundation::*;
use windows::Win32::Graphics::Direct2D::Common::D2D_RECT_F;
use windows::Win32::Graphics::Direct2D::*;
use windows::Win32::Graphics::Gdi::*;
use windows::Win32::UI::Input::KeyboardAndMouse::*;
use windows::Win32::UI::WindowsAndMessaging::*;

use crate::gfx::{self, Surface};
use crate::hook::unpack;
use crate::orb::{set_tucked, work_area, Side, Spring, INSET, WIN};
use crate::ui::{self, Pill, PillAction, PillLayout, PillPart};
use crate::{
    calendar, disk, downloads, habits, notices, orb, px, start_anim, tasks, tasks_model, with_app,
    App, MSG_MOUSELEAVE, TIMER_PILL, TIMER_SHAKE,
};

const GAP: f32 = 8.0;
/// Room below the front pill for the two peeking layers (6 px and 12 px).
const PEEK: f32 = 12.0;
const MORE_H: f32 = 28.0;
const SHAKE_S: f32 = 0.42;

/// The stack around the front pill.
pub struct Stack {
    /// Waiting pills shown while the stack is spread: (entry id, pill, layout).
    items: Vec<(u64, Pill, PillLayout)>,
    /// How many wait behind the front pill (two of them peek).
    behind: usize,
    spread: Spring,
    shake_at: Option<Instant>,
    shakes: u32,
    /// Hovered or clicked: an urgent pill stops asking for attention.
    read: bool,
}

impl Stack {
    pub fn new() -> Stack {
        Stack {
            items: Vec::new(),
            behind: 0,
            spread: Spring::new(0.0, 190.0, 26.0),
            shake_at: None,
            shakes: 0,
            read: false,
        }
    }
}

/// What the pointer is over in the pill window.
#[derive(Clone, Copy, PartialEq)]
pub enum StackHit {
    Front(PillPart),
    Item(usize, PillPart),
    More,
    None,
}

fn spread_on(app: &App) -> bool {
    app.stack.spread.target > 0.5 || app.stack.spread.pos > 0.01
}

fn more_count(app: &App) -> usize {
    app.stack.behind.saturating_sub(app.stack.items.len())
}

fn more_label(n: usize) -> (String, &'static str) {
    (format!("{n} more"), "open the list")
}

fn more_w(app: &App) -> f32 {
    let (a, b) = more_label(more_count(app));
    app.g.text_width(&a, &app.fonts.title) + 6.0 + app.g.text_width(b, &app.fonts.body) + 24.0
}

/// The logical size of what the window holds (without its margin).
fn content(app: &App) -> (f32, f32) {
    let mut w = app.pill_layout.w;
    let mut h = ui::PILL_H + PEEK;
    if spread_on(app) {
        for (_, _, l) in &app.stack.items {
            w = w.max(l.w);
        }
        h = ui::PILL_H + app.stack.items.len() as f32 * (ui::PILL_H + GAP);
        if more_count(app) > 0 {
            w = w.max(more_w(app));
            h += GAP + MORE_H;
        }
        h = h.max(ui::PILL_H + PEEK);
    }
    (w, h)
}

fn pill_window_size(app: &App) -> (i32, i32) {
    let (w, h) = content(app);
    (
        px(app, w + ui::PILL_PAD * 2.0).ceil() as i32,
        px(app, h + ui::PILL_PAD * 2.0).ceil() as i32,
    )
}

/// A pill of width `w` sits against the orb's side of the column.
fn x_of(app: &App, w: f32) -> f32 {
    match app.side {
        Side::Right => ui::PILL_PAD + content(app).0 - w,
        Side::Left => ui::PILL_PAD,
    }
}

fn item_top(k: usize) -> f32 {
    ui::PILL_PAD + (k as f32 + 1.0) * (ui::PILL_H + GAP)
}

fn more_rect(app: &App) -> (f32, f32, f32, f32) {
    let w = more_w(app);
    let x = x_of(app, w);
    let y = ui::PILL_PAD + ui::PILL_H + app.stack.items.len() as f32 * (ui::PILL_H + GAP) + GAP;
    (x, y, x + w, y + MORE_H)
}

fn shake_dx(app: &App) -> f32 {
    match app.stack.shake_at {
        Some(t) => {
            let e = t.elapsed().as_secs_f32();
            if e >= SHAKE_S {
                0.0
            } else {
                3.0 * (e * std::f32::consts::TAU * 7.0).sin() * (1.0 - e / SHAKE_S)
            }
        }
        None => 0.0,
    }
}

fn place_pill(app: &App) {
    if app.pill.is_none() {
        return;
    }
    let (w, h) = pill_window_size(app);
    let gap = 8.0;
    let x = match app.side {
        Side::Right => app.x.pos + px(app, INSET - gap) - w as f32 + px(app, ui::PILL_PAD),
        Side::Left => app.x.pos + px(app, WIN - INSET + gap) - px(app, ui::PILL_PAD),
    } + px(app, shake_dx(app));
    // The front pill's middle on the drop's middle; a spread column stays on screen.
    let mut y = app.y + px(app, WIN / 2.0) - px(app, ui::PILL_H / 2.0 + ui::PILL_PAD);
    let wa = work_area(app.hwnd);
    if y + h as f32 > wa.bottom as f32 {
        y = (wa.bottom - h) as f32;
    }
    unsafe {
        let _ = SetWindowPos(
            app.pill_hwnd,
            Some(HWND_TOPMOST),
            x.round() as i32,
            y.round() as i32,
            w,
            h,
            SWP_NOACTIVATE | SWP_NOOWNERZORDER | orb::raise_once(app.pill_hwnd),
        );
    }
    // The tucked shadow under the front pill's visible shape as it grows out of the drop.
    let fw = app.pill_layout.w;
    let r = app.pill_reveal.pos.clamp(0.0, 1.2);
    let width = 48.0 + (fw - 48.0) * r.min(1.0);
    let fx = x_of(app, fw);
    let (l, rr) = if app.side == Side::Right {
        (fx + fw - width, fx + fw)
    } else {
        (fx, fx + width)
    };
    let (x0, y0) = (x.round(), y.round());
    gfx_shadow(
        app,
        (
            x0 + px(app, l),
            y0 + px(app, ui::PILL_PAD),
            x0 + px(app, rr),
            y0 + px(app, ui::PILL_PAD + ui::PILL_H),
        ),
        r.min(1.0),
    );
}

fn gfx_shadow(app: &App, shape: (f32, f32, f32, f32), opacity: f32) {
    crate::depth::follow(
        &app.g,
        app.pill_hwnd,
        shape,
        ui::PILL_H / 2.0,
        app.scale,
        app.theme.dark,
        opacity,
    );
}

/// Clicks outside the shapes go through: the front pill (and the peeks under it), or the spread
/// column and its "more" button.
fn set_region(app: &App) {
    let s = app.scale;
    let round = |r: (f32, f32, f32, f32), rad: f32| unsafe {
        CreateRoundRectRgn(
            (r.0 * s).round() as i32,
            (r.1 * s).round() as i32,
            (r.2 * s).round() as i32 + 1,
            (r.3 * s).round() as i32 + 1,
            (rad * 2.0 * s).round() as i32,
            (rad * 2.0 * s).round() as i32,
        )
    };
    let fw = app.pill_layout.w;
    let fx = x_of(app, fw);
    let peek = if app.stack.behind > 0 { PEEK } else { 0.0 };
    let rgn = round(
        (fx, ui::PILL_PAD, fx + fw, ui::PILL_PAD + ui::PILL_H + peek),
        ui::PILL_H / 2.0,
    );
    if app.stack.spread.target > 0.5 {
        for (k, (_, _, l)) in app.stack.items.iter().enumerate() {
            let x = x_of(app, l.w);
            let r = round(
                (x, item_top(k), x + l.w, item_top(k) + ui::PILL_H),
                ui::PILL_H / 2.0,
            );
            unsafe {
                let _ = CombineRgn(Some(rgn), Some(rgn), Some(r), RGN_OR);
                let _ = DeleteObject(r.into());
            }
        }
        if more_count(app) > 0 {
            let r = round(more_rect(app), MORE_H / 2.0);
            unsafe {
                let _ = CombineRgn(Some(rgn), Some(rgn), Some(r), RGN_OR);
                let _ = DeleteObject(r.into());
            }
        }
    }
    unsafe {
        let _ = SetWindowRgn(app.pill_hwnd, Some(rgn), true);
    }
}

fn layer(dc: &ID2D1DeviceContext, opacity: f32) {
    let p = D2D1_LAYER_PARAMETERS1 {
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
        dc.PushLayer(&p, None);
    }
}

pub(crate) fn render_pill(app: &mut App) {
    let Some(p) = app.pill.clone() else { return };
    let (w, h) = pill_window_size(app);
    let scale = app.scale;
    if app.pill_surface.is_none() {
        app.pill_surface = Surface::new(&app.g, app.pill_hwnd, w as u32, h as u32, scale).ok();
    }
    let Some(s) = app.pill_surface.as_mut() else {
        return;
    };
    let _ = s.resize(&app.g, w as u32, h as u32);
    let s = app.pill_surface.as_ref().unwrap();
    let dc = s.begin(&app.g);
    let t = &app.theme;
    let grow_left = app.side == Side::Right;
    let fw = app.pill_layout.w;
    let fx = x_of(app, fw);
    let top = ui::PILL_PAD;
    let sp = app.stack.spread.pos.clamp(0.0, 1.0);
    let ca = ((app.pill_reveal.pos - 0.55) / 0.4).clamp(0.0, 1.0);
    // Peeks: the front pill's width, no text, 6 and 12 px lower at .96 and .92, fading as it spreads.
    let peeks = app.stack.behind.min(2);
    for k in (1..=peeks).rev() {
        let a = [0.7, 0.45][k - 1] * (1.0 - sp) * ca;
        if a <= 0.01 {
            continue;
        }
        let sc = 1.0 - 0.04 * k as f32;
        let (pw, ph) = (fw * sc, ui::PILL_H * sc);
        let cx = fx + fw / 2.0;
        let bottom = top + ui::PILL_H + 6.0 * k as f32;
        unsafe {
            let r = gfx::rrect(cx - pw / 2.0, bottom - ph, cx + pw / 2.0, bottom, ph / 2.0);
            dc.FillRoundedRectangle(&r, &gfx::brush(dc, ui::with_alpha(t.surface, a)));
            dc.DrawRoundedRectangle(&r, &gfx::brush(dc, ui::with_alpha(t.hair, a)), 1.0, None);
        }
    }
    if sp > 0.01 {
        for (k, (_, ip, il)) in app.stack.items.iter().enumerate() {
            let from = top + 6.0 * (k as f32 + 1.0).min(2.0);
            let y = from + (item_top(k) - from) * sp;
            layer(dc, sp);
            let hover = match app.pill_hover {
                StackHit::Item(j, part) if j == k => part,
                _ => PillPart::None,
            };
            ui::draw_pill(
                &mut gfx::D2dCanvas::new(&app.g, dc),
                t,
                &app.fonts,
                ip,
                il,
                1.0,
                grow_left,
                hover,
                x_of(app, il.w),
                y,
            );
            unsafe {
                dc.PopLayer();
            }
        }
        let n = more_count(app);
        if n > 0 {
            let r = more_rect(app);
            layer(dc, sp);
            let hov = app.pill_hover == StackHit::More;
            unsafe {
                dc.FillRoundedRectangle(
                    &gfx::rrect(r.0, r.1, r.2, r.3, MORE_H / 2.0),
                    &gfx::brush(dc, if hov { t.raised } else { t.surface }),
                );
                dc.DrawRoundedRectangle(
                    &gfx::rrect(
                        r.0 + 0.5,
                        r.1 + 0.5,
                        r.2 - 0.5,
                        r.3 - 0.5,
                        MORE_H / 2.0 - 0.5,
                    ),
                    &gfx::brush(dc, t.hair),
                    1.0,
                    None,
                );
            }
            let (a, b) = more_label(n);
            let aw = app.g.text_width(&a, &app.fonts.title);
            gfx::text(
                dc,
                &a,
                &app.fonts.title,
                r.0 + 12.0,
                r.1,
                aw + 2.0,
                MORE_H,
                &gfx::brush(dc, t.text),
            );
            gfx::text(
                dc,
                b,
                &app.fonts.body,
                r.0 + 12.0 + aw + 6.0,
                r.1,
                r.2 - r.0,
                MORE_H,
                &gfx::brush(dc, t.muted),
            );
            unsafe {
                dc.PopLayer();
            }
        }
    }
    let hover = match app.pill_hover {
        StackHit::Front(part) => part,
        _ => PillPart::None,
    };
    ui::draw_pill(
        &mut gfx::D2dCanvas::new(&app.g, dc),
        t,
        &app.fonts,
        &p,
        &app.pill_layout,
        app.pill_reveal.pos,
        grow_left,
        hover,
        fx,
        top,
    );
    s.end(&app.g);
}

/// Where the pointer is, in logical window coordinates.
fn hit(app: &App, x: f32, y: f32) -> StackHit {
    let menu = has_menu(app);
    let local = |px0: f32, py0: f32| (x - px0 + ui::PILL_PAD, y - py0 + ui::PILL_PAD);
    let fx = x_of(app, app.pill_layout.w);
    let (lx, ly) = local(fx, ui::PILL_PAD);
    match ui::hit_pill(&app.pill_layout, menu, lx, ly) {
        PillPart::None => {}
        part => return StackHit::Front(part),
    }
    if app.stack.spread.target > 0.5 {
        for (k, (_, ip, il)) in app.stack.items.iter().enumerate() {
            let (lx, ly) = local(x_of(app, il.w), item_top(k));
            let m = ip.action.as_ref().is_some_and(|(_, a)| a.has_menu());
            match ui::hit_pill(il, m, lx, ly) {
                PillPart::None => {}
                part => return StackHit::Item(k, part),
            }
        }
        let r = more_rect(app);
        if more_count(app) > 0 && x >= r.0 && x <= r.2 && y >= r.1 && y <= r.3 {
            return StackHit::More;
        }
    }
    if app.stack.behind > 0
        && x >= fx
        && x <= fx + app.pill_layout.w
        && y <= ui::PILL_PAD + ui::PILL_H + PEEK
    {
        return StackHit::Front(PillPart::Body);
    }
    StackHit::None
}

pub fn show_pill(app: &mut App, p: Pill) {
    set_tucked(app, false);
    app.pill_closing = false;
    app.pill_hover = StackHit::None;
    app.stack.shake_at = None;
    app.stack.read = false;
    if !app.pill_reveal.settled(0.01) || app.pill_reveal.pos < 0.5 {
        app.pill_reveal.snap(0.0);
    }
    app.pill_reveal.target = 1.0;
    app.pill_reveal.k = 320.0; // Gooey "bouncy"
    app.pill_reveal.c = 17.0;
    app.stack.behind = notices::badge_count(app);
    put_pill(app, p);
    // The drop swells as it pushes the pill out.
    orb::kick_notice(app);
    start_anim(app);
}

/// New words for the pill already on screen (live numbers, a merge): the reveal carries on where
/// it is, so an update in the middle of the grow does not start it over.
pub fn update_pill(app: &mut App, p: Pill) {
    if app.pill.is_none() || app.pill_closing {
        show_pill(app, p);
    } else {
        put_pill(app, p);
    }
}

fn put_pill(app: &mut App, p: Pill) {
    app.pill_layout = ui::layout_pill(&app.fonts, &p);
    let ttl = p.ttl_ms;
    app.pill = Some(p);
    place_pill(app);
    set_region(app);
    unsafe {
        let _ = ShowWindow(app.pill_hwnd, SW_SHOWNOACTIVATE);
        if !app.pill_tracking {
            SetTimer(Some(app.pill_hwnd), TIMER_PILL, ttl, None);
        }
    }
    // New words always come out on top, even over a panel that opened while an older pill showed.
    raise(app);
    render_pill(app);
}

/// Lift the pill above every other topmost window of the app (panels included).
pub fn raise(app: &App) {
    unsafe {
        let _ = SetWindowPos(
            app.pill_hwnd,
            Some(HWND_TOPMOST),
            0,
            0,
            0,
            0,
            SWP_NOMOVE | SWP_NOSIZE | SWP_NOACTIVATE | SWP_NOOWNERZORDER,
        );
    }
}

/// The count behind the front pill changed: the peeks follow.
pub fn restack(app: &mut App) {
    let n = notices::badge_count(app);
    if n == app.stack.behind {
        return;
    }
    app.stack.behind = n;
    if app.pill.is_some() && !app.pill_closing {
        if spread_on(app) {
            fill_items(app);
        }
        place_pill(app);
        set_region(app);
        render_pill(app);
    }
}

fn fill_items(app: &mut App) {
    // In the column only the front pill has a close; the others carry their action.
    let items: Vec<(u64, Pill)> = notices::stack(app)
        .into_iter()
        .take(2)
        .map(|(id, p)| {
            (
                id,
                Pill {
                    closable: false,
                    ..p
                },
            )
        })
        .collect();
    app.stack.items = items
        .into_iter()
        .map(|(id, p)| (id, ui::layout_pill(&app.fonts, &p), p))
        .map(|(id, l, p)| (id, p, l))
        .collect();
}

fn spread(app: &mut App, open: bool) {
    if open {
        if app.stack.behind == 0 || app.pill_closing {
            return;
        }
        fill_items(app);
        if app.stack.items.is_empty() {
            return;
        }
    }
    let target = if open { 1.0 } else { 0.0 };
    if app.stack.spread.target == target {
        return;
    }
    // DESIGN.md: the stack splits off the front pill on "smooth" (190 / 26).
    app.stack.spread.target = target;
    place_pill(app);
    set_region(app);
    start_anim(app);
}

pub fn dismiss_pill(app: &mut App) {
    if app.pill.is_none() || app.pill_closing {
        return;
    }
    app.pill_closing = true;
    notices::on_dismiss(app);
    downloads::on_pill_dismissed(app);
    app.pill_reveal.target = 0.0;
    app.pill_reveal.k = 480.0; // exits are faster
    app.pill_reveal.c = 40.0;
    app.stack.spread.target = 0.0;
    app.stack.shake_at = None;
    unsafe {
        let _ = KillTimer(Some(app.pill_hwnd), TIMER_PILL);
        let _ = KillTimer(Some(app.pill_hwnd), TIMER_SHAKE);
    }
    start_anim(app);
}

/// The attention shake for an urgent pill: ±3 px on arrival, again at 5.5 s and 11 s while it is
/// still unread, then still. Never under reduced motion.
pub fn shake(app: &mut App) {
    if downloads::reduced_motion() {
        return;
    }
    crate::log_line(app, "notices: urgent, shake 1 of 3");
    app.stack.shake_at = Some(Instant::now());
    app.stack.shakes = 1;
    unsafe {
        SetTimer(Some(app.pill_hwnd), TIMER_SHAKE, 5500, None);
    }
    start_anim(app);
}

fn on_shake_timer(app: &mut App) {
    unsafe {
        let _ = KillTimer(Some(app.pill_hwnd), TIMER_SHAKE);
    }
    if app.stack.read
        || app.pill.is_none()
        || app.pill_closing
        || app.stack.shakes >= 3
        || app.notices.full_screen
    {
        return;
    }
    app.stack.shakes += 1;
    let n = app.stack.shakes;
    crate::log_line(
        app,
        &format!("notices: urgent still unread, shake {n} of 3"),
    );
    app.stack.shake_at = Some(Instant::now());
    if app.stack.shakes < 3 {
        unsafe {
            SetTimer(Some(app.pill_hwnd), TIMER_SHAKE, 5500, None);
        }
    }
    start_anim(app);
}

/// A pill's action, from the front pill or a waiting one in the spread stack.
fn pill_action(app: &mut App, a: PillAction) {
    match a {
        PillAction::AddDrink(ml) => {
            let p = habits::log_drink(app, ml);
            notices::reply(app, p);
        }
        PillAction::UndoDrink => {
            habits::undo_drink(app);
            dismiss_pill(app);
        }
        PillAction::UndoFocus => habits::undo_focus(app),
        PillAction::ResumeFocus => {
            habits::resume(app);
            dismiss_pill(app);
        }
        PillAction::StartFocus => {
            let m = app.habits.prefs.length_min;
            habits::start_focus(app, m);
            dismiss_pill(app);
        }
        PillAction::ShowWaited => {
            dismiss_pill(app);
            notices::open_panel(app);
        }
        PillAction::TaskDone(id) => tasks::done_from_pill(app, &id),
        PillAction::Join(url) => {
            dismiss_pill(app);
            calendar::join(app, &url);
        }
        PillAction::OpenTasks => {
            dismiss_pill(app);
            tasks::open(app, Some(tasks_model::Selection::Today));
        }
        PillAction::SaveLink(link, format) => {
            dismiss_pill(app);
            downloads::save(app, link, format);
        }
        PillAction::OpenFile(path) => {
            dismiss_pill(app);
            downloads::open_file(app, &path);
        }
        PillAction::OpenDownloads => {
            dismiss_pill(app);
            downloads::open(app);
        }
        PillAction::ChangeFolder(id) => {
            dismiss_pill(app);
            downloads::change_folder(app, id);
        }
        PillAction::RetryDownload(id) => {
            dismiss_pill(app);
            downloads::retry(app, id);
        }
        PillAction::FindBigFolders(drive) => {
            dismiss_pill(app);
            disk::find_big_folders(app, drive);
        }
        PillAction::OpenDisk => {
            dismiss_pill(app);
            disk::open(app);
        }
        PillAction::UndoRecycle => {
            dismiss_pill(app);
            disk::undo(app);
        }
        PillAction::RetryScan => {
            dismiss_pill(app);
            disk::retry_scan(app);
        }
        PillAction::OpenSettings => {
            dismiss_pill(app);
            crate::settings::open(app, crate::settings_model::Section::General);
        }
        PillAction::UndoShortcut => crate::settings::undo_shortcut(app),
        PillAction::StopAdzan => {
            crate::prayer::stop_adzan(app);
            dismiss_pill(app);
        }
    }
}

pub use crate::ui::info_pill;

/// One animation frame: the reveal and spread springs and the shake, then hide the window once
/// the pill has closed.
pub fn tick(app: &mut App, dt: f32) {
    if app.pill.is_none() {
        return;
    }
    app.pill_reveal.step(dt);
    let was_spread = spread_on(app);
    app.stack.spread.step(dt);
    if app.stack.spread.target == 0.0 && app.stack.spread.pos < 0.01 && was_spread {
        app.stack.spread.snap(0.0);
        app.stack.items.clear();
        set_region(app);
    }
    if app
        .stack
        .shake_at
        .is_some_and(|t| t.elapsed().as_secs_f32() >= SHAKE_S)
    {
        app.stack.shake_at = None;
    }
    place_pill(app);
    if app.pill_closing && app.pill_reveal.pos <= 0.02 {
        app.pill = None;
        app.pill_closing = false;
        app.pill_reveal.snap(0.0);
        app.stack.spread.snap(0.0);
        app.stack.items.clear();
        crate::depth::hide(app.pill_hwnd);
        unsafe {
            let _ = ShowWindow(app.pill_hwnd, SW_HIDE);
        }
        notices::on_pill_gone(app);
    } else {
        render_pill(app);
    }
}

/// The pill on screen has a split button (the offer's Save with its format menu).
fn has_menu(app: &App) -> bool {
    app.pill
        .as_ref()
        .and_then(|p| p.action.as_ref())
        .is_some_and(|(_, a)| a.has_menu())
}

pub fn is_animating(app: &App) -> bool {
    app.pill.is_some()
        && (!app.pill_reveal.settled(0.002)
            || !app.stack.spread.settled(0.002)
            || app.stack.shake_at.is_some())
}

fn on_click(app: &mut App, x: f32, y: f32) {
    // A closing pill still shows for a moment; its action was already taken or dropped.
    if app.pill_closing {
        return;
    }
    app.stack.read = true;
    match hit(app, x, y) {
        StackHit::Front(PillPart::Menu) if downloads::menu_open(app) => downloads::close_menu(app),
        StackHit::Front(PillPart::Menu) => downloads::open_offer_menu(app),
        StackHit::Front(PillPart::Action) => {
            if let Some((_, a)) = app.pill.as_ref().and_then(|p| p.action.clone()) {
                notices::acted(app);
                pill_action(app, a);
            }
        }
        StackHit::Front(PillPart::Close) => {
            // Your X: a pill that stays until closed is done with, not put back in line.
            notices::closed_by_you(app);
            dismiss_pill(app);
        }
        StackHit::Item(k, PillPart::Action | PillPart::Menu) => {
            if let Some((id, p, _)) = app.stack.items.get(k).cloned() {
                notices::stack_action(app, id);
                if let Some((_, a)) = p.action {
                    pill_action(app, a);
                }
            }
        }
        StackHit::Item(k, _) => {
            if let Some((id, ..)) = app.stack.items.get(k).cloned() {
                notices::bring_front(app, id);
            }
        }
        StackHit::More => {
            dismiss_pill(app);
            notices::open_panel(app);
        }
        _ => {}
    }
}

pub unsafe extern "system" fn pill_proc(
    hwnd: HWND,
    msg: u32,
    wparam: WPARAM,
    lparam: LPARAM,
) -> LRESULT {
    match msg {
        WM_MOUSEACTIVATE => return LRESULT(MA_NOACTIVATE as isize),
        WM_MOUSEMOVE => {
            with_app(|app| {
                let (x, y) = unpack(lparam);
                if !app.pill_tracking {
                    let mut tme = TRACKMOUSEEVENT {
                        cbSize: std::mem::size_of::<TRACKMOUSEEVENT>() as u32,
                        dwFlags: TME_LEAVE,
                        hwndTrack: hwnd,
                        dwHoverTime: 0,
                    };
                    let _ = TrackMouseEvent(&mut tme);
                    app.pill_tracking = true;
                    app.stack.read = true;
                    // Hovering pauses the auto-dismiss and spreads the stack.
                    let _ = KillTimer(Some(hwnd), TIMER_PILL);
                    spread(app, true);
                }
                let part = hit(app, x / app.scale, y / app.scale);
                if part != app.pill_hover {
                    app.pill_hover = part;
                    render_pill(app);
                }
            });
            return LRESULT(0);
        }
        MSG_MOUSELEAVE => {
            with_app(|app| {
                app.pill_tracking = false;
                app.pill_hover = StackHit::None;
                spread(app, false);
                render_pill(app);
                if !app.pill_closing && !downloads::menu_open(app) {
                    if let Some(p) = &app.pill {
                        SetTimer(Some(hwnd), TIMER_PILL, p.ttl_ms, None);
                    }
                }
            });
            return LRESULT(0);
        }
        WM_LBUTTONUP => {
            let (x, y) = unpack(lparam);
            with_app(|app| on_click(app, x / app.scale, y / app.scale));
            return LRESULT(0);
        }
        WM_RBUTTONUP => {
            with_app(|app| {
                if has_menu(app) && !app.pill_closing {
                    downloads::open_offer_menu(app);
                }
            });
            return LRESULT(0);
        }
        WM_TIMER => {
            match wparam.0 {
                TIMER_PILL => {
                    let _ = KillTimer(Some(hwnd), TIMER_PILL);
                    with_app(dismiss_pill);
                }
                TIMER_SHAKE => {
                    with_app(on_shake_timer);
                }
                _ => {}
            }
            return LRESULT(0);
        }
        _ => {}
    }
    DefWindowProcW(hwnd, msg, wparam, lparam)
}
