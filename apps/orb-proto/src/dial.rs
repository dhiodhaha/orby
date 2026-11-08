//! The half dial: opens from the drop, aim by direction or keys, the picks, and the Focus and
//! Drink branches (15/25/50 min, 150/250/500 ml) and More (Downloads, Disk, History, Notices, Settings) that fan out after 120 ms
//! of steady aim.

use windows::Win32::Foundation::*;
use windows::Win32::UI::WindowsAndMessaging::*;

use app_core::prayer as pr;

use crate::gfx::Surface;
use crate::orb::{arm_tuck_timer, orb_center, set_tucked, set_visible, Side};
use crate::panels::{self, Kind};
use crate::pill::dismiss_pill;
use crate::settings_model::Section;
use crate::ui::{self, DialBranch, DialChild, DialItem, Icon};
use crate::{
    calendar, disk, downloads, habits, history, hook, log_line, notices, prayer, px, settings,
    start_anim, sync_keys, tasks, with_app, App, HK_DOWN, HK_ENTER, HK_LEFT, HK_NUM, HK_RIGHT,
    HK_UP, TIMER_BRANCH, TIMER_TUCK,
};

const FOCUS: usize = 3;
const DRINK: usize = 4;
const MORE: usize = 5;
/// Steady aim on a parent this long opens its branch, so a sweep across the ring never flashes one.
const BRANCH_DELAY_MS: u32 = 120;
/// After a dial key, the pointer must move this far (logical px) before it aims again, so a
/// nudge of the mouse never redirects a keyboard pick.
const MOUSE_TAKEOVER: f32 = 32.0;

/// Keys are in charge from here on, until the pointer moves away on purpose.
fn latch_keyboard(app: &mut App) {
    app.dial_keyboard = true;
    // A branch the pointer's aim had queued must not open under the keys.
    unsafe {
        let _ = KillTimer(Some(app.hwnd), TIMER_BRANCH);
    }
    let mut pt = POINT::default();
    app.dial_key_anchor = unsafe { GetCursorPos(&mut pt) }
        .ok()
        .map(|_| (pt.x as f32, pt.y as f32));
}

fn has_branch(i: usize) -> bool {
    i == FOCUS || i == DRINK || i == MORE
}

fn dial_items(app: &App) -> Vec<DialItem> {
    let (np, nat, _) = prayer::next_prayer(app);
    vec![
        DialItem {
            icon: Icon::Tasks,
            name: "Tasks",
            value: tasks::dial_value(app),
        },
        DialItem {
            icon: Icon::Today,
            name: "Today",
            value: tasks::dial_today_value(app),
        },
        DialItem {
            icon: Icon::Prayer,
            name: "Prayer",
            value: format!("{} {}", np.name(), pr::hhmm(nat)),
        },
        DialItem {
            icon: Icon::Focus,
            name: "Focus",
            value: habits::dial_focus_value(app),
        },
        DialItem {
            icon: Icon::Drink,
            name: "Drink",
            value: habits::dial_drink_value(app),
        },
        DialItem {
            icon: Icon::More,
            name: "More",
            value: disk::dial_value(app).unwrap_or_else(|| downloads::dial_value(app)),
        },
    ]
}

/// The open branch's children and its default child.
fn children(app: &App, parent: usize) -> (Vec<DialChild>, usize) {
    match parent {
        FOCUS => habits::focus_children(app),
        DRINK => habits::drink_children(app),
        MORE => {
            let (mut kids, default) = downloads::dial_children(app);
            kids.push(disk::dial_child(app));
            kids.push(history::dial_child(app));
            kids.push(notices::dial_child(app));
            kids.push(settings::dial_child(app));
            kids.push(crate::tasks_window::dial_child(app));
            (kids, default)
        }
        _ => (Vec::new(), 0),
    }
}

pub fn mirrored(app: &App) -> bool {
    app.side == Side::Left
}

fn place_dial(app: &App) {
    let (cx, cy) = ui::dial_center(mirrored(app));
    let (ox, oy) = orb_center(app);
    let x = ox - px(app, cx);
    let y = oy - px(app, cy);
    unsafe {
        let _ = SetWindowPos(
            app.dial_hwnd,
            Some(HWND_TOPMOST),
            x.round() as i32,
            y.round() as i32,
            px(app, ui::DIAL_W).ceil() as i32,
            px(app, ui::DIAL_H).ceil() as i32,
            SWP_NOACTIVATE | SWP_NOOWNERZORDER,
        );
        // Keep the drop above the dial.
        let _ = SetWindowPos(
            app.hwnd,
            Some(HWND_TOPMOST),
            0,
            0,
            0,
            0,
            SWP_NOMOVE | SWP_NOSIZE | SWP_NOACTIVATE,
        );
    }
}

/// Value tags leave fast when a branch parent is aimed and come back slowly, so a slide from
/// Drink to Focus keeps them away instead of flashing Subuh and 1250/2000 in between.
fn aim_tags(app: &mut App) {
    let hide = app.dial_branch.is_some() || app.dial_aim.is_some_and(has_branch);
    let want = if hide { 0.0 } else { 1.0 };
    if app.dial_tags.target != want {
        (app.dial_tags.k, app.dial_tags.c) = if hide { (600.0, 50.0) } else { (110.0, 21.0) };
        app.dial_tags.target = want;
        start_anim(app);
    }
}

fn render_dial(app: &mut App) {
    aim_tags(app);
    let (w, h) = (
        px(app, ui::DIAL_W).ceil() as u32,
        px(app, ui::DIAL_H).ceil() as u32,
    );
    if app.dial_surface.is_none() {
        app.dial_surface = Surface::new(&app.g, app.dial_hwnd, w, h, app.scale).ok();
    }
    let items = dial_items(app);
    let kids = app
        .dial_branch
        .map(|p| children(app, p).0)
        .unwrap_or_default();
    let branch = app.dial_branch.map(|parent| DialBranch {
        parent,
        children: &kids,
        aimed: app.dial_child,
        reveal: app.branch_reveal.pos,
    });
    let ring = habits::ring(app);
    let Some(s) = app.dial_surface.as_ref() else {
        return;
    };
    let dc = s.begin(&app.g);
    ui::draw_dial(
        &app.g,
        dc,
        &app.theme,
        &app.fonts,
        &items,
        app.dial_aim,
        app.dial_reveal.pos,
        mirrored(app),
        app.dial_keyboard,
        branch.as_ref(),
        ring,
        app.dial_tags.pos,
    );
    s.end(&app.g);
}

pub fn open_dial(app: &mut App, keyboard: bool) {
    if app.dial_open {
        return;
    }
    if app.pill.is_some() {
        dismiss_pill(app);
    }
    set_tucked(app, false);
    app.dial_open = true;
    app.dial_keyboard = false;
    app.dial_key_anchor = None;
    if keyboard {
        latch_keyboard(app);
    }
    app.dial_aim = if keyboard { Some(0) } else { None };
    app.dial_branch = None;
    app.dial_child = None;
    app.branch_reveal.snap(0.0);
    app.dial_tags.snap(1.0);
    app.dial_reveal.k = 190.0; // "settle"
    app.dial_reveal.c = 26.0;
    app.dial_reveal.target = 1.0;
    place_dial(app);
    unsafe {
        let _ = KillTimer(Some(app.hwnd), TIMER_TUCK);
        let _ = ShowWindow(app.dial_hwnd, SW_SHOWNOACTIVATE);
    }
    hook::set_watch_clicks(true);
    sync_keys(app);
    render_dial(app);
    start_anim(app);
}

pub fn close_dial(app: &mut App) {
    if !app.dial_open {
        return;
    }
    app.dial_open = false;
    app.dial_branch = None;
    app.dial_child = None;
    app.dial_reveal.k = 480.0;
    app.dial_reveal.c = 40.0;
    app.dial_reveal.target = 0.0;
    unsafe {
        let _ = KillTimer(Some(app.hwnd), TIMER_BRANCH);
    }
    hook::set_watch_clicks(app.panel_open);
    sync_keys(app);
    arm_tuck_timer(app);
    start_anim(app);
}

fn open_branch(app: &mut App, parent: usize, aim_default: bool) {
    app.dial_aim = Some(parent);
    if app.dial_branch != Some(parent) {
        app.dial_branch = Some(parent);
        app.branch_reveal.snap(0.0);
        app.branch_reveal.k = 190.0;
        app.branch_reveal.c = 26.0;
        app.branch_reveal.target = 1.0;
        start_anim(app);
    }
    app.dial_child = if aim_default {
        Some(children(app, parent).1)
    } else {
        None
    };
    render_dial(app);
}

fn close_branch(app: &mut App) {
    app.dial_branch = None;
    app.dial_child = None;
    app.branch_reveal.snap(0.0);
}

/// A main item: Tasks, Today, Prayer, Focus and Drink open their panels. More has no panel of
/// its own, so picking it (a click or Enter) only fans out its children; it never jumps to one.
fn pick(app: &mut App, i: usize) {
    if i == MORE {
        open_branch(app, MORE, app.dial_keyboard);
        return;
    }
    close_dial(app);
    crate::orb::kick(app, 1.0);
    let name = ["tasks", "today", "prayer", "focus", "drink", "more"]
        .get(i)
        .copied()
        .unwrap_or("?");
    log_line(app, &format!("dial pick {name}"));
    match i {
        0 => tasks::open(app, None),
        1 => calendar::open(app),
        2 => prayer::open_panel(app),
        FOCUS => panels::open_panel(app, Kind::Focus),
        DRINK => panels::open_panel(app, Kind::Drink),
        _ => downloads::open(app),
    }
}

/// A branch child does its action at once: start a session, or log a glass.
fn pick_child(app: &mut App, parent: usize, j: usize) {
    close_dial(app);
    crate::orb::kick(app, 1.0);
    let name = match parent {
        FOCUS => "focus",
        DRINK => "drink",
        _ => "more",
    };
    log_line(app, &format!("dial pick {name} child {j}"));
    match parent {
        FOCUS => habits::pick_focus_child(app, j),
        DRINK => habits::pick_drink_child(app, j),
        _ if j == 1 => disk::open(app),
        _ if j == 2 => history::open(app, None),
        _ if j == 3 => notices::open_panel(app),
        _ if j == 4 => settings::open(app, Section::General),
        _ if j == 5 => crate::tasks_window::show(app),
        _ => downloads::open(app),
    }
}

/// Ctrl+Alt+Space: opens the dial (showing the orb first if hidden); a second press while
/// the dial is open hides the orb.
pub fn on_shortcut(app: &mut App) {
    if !app.visible {
        set_visible(app, true);
        notices::on_user_visibility(app);
        open_dial(app, true);
    } else if app.dial_open {
        close_dial(app);
        set_visible(app, false);
        notices::on_user_visibility(app);
    } else {
        open_dial(app, true);
    }
}

/// Esc steps back: branch to ring, ring to closed.
pub fn on_esc(app: &mut App) {
    if app.dial_branch.is_some() {
        close_branch(app);
        latch_keyboard(app);
        render_dial(app);
    } else {
        close_dial(app);
    }
}

/// Dial keys while it is open. Arrows go round the ring (or the open branch), the arrow toward
/// the screen centre enters a branch and the other steps back out, Enter picks or opens a
/// branch, 1 to 6 jump to an item (1 to 3 pick a child inside a branch).
pub fn on_key(app: &mut App, id: i32) -> bool {
    if !app.dial_open {
        return false;
    }
    latch_keyboard(app);
    let (inward, outward) = if mirrored(app) {
        (HK_RIGHT, HK_LEFT)
    } else {
        (HK_LEFT, HK_RIGHT)
    };
    let aim = app.dial_aim.unwrap_or(0);
    match (id, app.dial_branch, app.dial_child) {
        (HK_UP | HK_DOWN, Some(parent), Some(j)) => {
            let n = children(app, parent).0.len().max(1);
            app.dial_child = Some(if id == HK_UP {
                (j + n - 1) % n
            } else {
                (j + 1) % n
            });
            render_dial(app);
        }
        (HK_UP | HK_DOWN, _, _) => {
            close_branch(app);
            app.dial_aim = Some(if id == HK_UP {
                (aim + 5) % 6
            } else {
                (aim + 1) % 6
            });
            render_dial(app);
        }
        (k, _, None) if k == inward && has_branch(aim) => open_branch(app, aim, true),
        (k, Some(_), Some(_)) if k == outward => {
            app.dial_child = None;
            render_dial(app);
        }
        (HK_ENTER, Some(parent), Some(j)) => pick_child(app, parent, j),
        // With the branch open and the parent aimed, Enter opens the parent's panel.
        (HK_ENTER, Some(parent), None) => pick(app, parent),
        (HK_ENTER, None, _) if has_branch(aim) => open_branch(app, aim, true),
        (HK_ENTER, None, _) => pick(app, aim),
        (n, Some(parent), _) if (HK_NUM..HK_NUM + 6).contains(&n) => {
            let j = (n - HK_NUM) as usize;
            if j < children(app, parent).0.len() {
                pick_child(app, parent, j);
            }
        }
        (n, None, _) if (HK_NUM..HK_NUM + 6).contains(&n) => {
            let i = (n - HK_NUM) as usize;
            if has_branch(i) {
                open_branch(app, i, true);
            } else {
                pick(app, i);
            }
        }
        _ => return false,
    }
    true
}

/// The pointer moved anywhere while the dial is open: aim by direction from the drop, and past
/// the main band aim the open branch's children.
pub fn on_pointer(app: &mut App, x: f32, y: f32) {
    if !app.dial_open {
        return;
    }
    if app.dial_keyboard && !moved_away(app.dial_key_anchor, (x, y), px(app, MOUSE_TAKEOVER)) {
        return;
    }
    let (cx, cy) = orb_center(app);
    let (dx, dy) = ((x - cx) / app.scale, (y - cy) / app.scale);
    let far = (dx * dx + dy * dy).sqrt() >= ui::BRANCH_AIM;
    let (aim, child) = match app.dial_branch {
        Some(parent) if far => {
            let n = children(app, parent).0.len();
            (
                Some(parent),
                ui::aim_child(dx, dy, mirrored(app), parent, n),
            )
        }
        _ => (ui::aim(dx, dy, mirrored(app), 6), None),
    };
    if aim == app.dial_aim && child == app.dial_child && !app.dial_keyboard {
        return;
    }
    if aim != app.dial_aim {
        if app.dial_branch.is_some_and(|b| Some(b) != aim) {
            close_branch(app);
        }
        unsafe {
            let _ = KillTimer(Some(app.hwnd), TIMER_BRANCH);
            if aim.is_some_and(has_branch) && app.dial_branch.is_none() {
                SetTimer(Some(app.hwnd), TIMER_BRANCH, BRANCH_DELAY_MS, None);
            }
        }
    }
    app.dial_aim = aim;
    app.dial_child = child;
    app.dial_keyboard = false;
    app.dial_key_anchor = None;
    render_dial(app);
}

/// Whether the pointer at `p` has left the keyboard's anchor by `radius` (no anchor: it has).
fn moved_away(anchor: Option<(f32, f32)>, p: (f32, f32), radius: f32) -> bool {
    anchor.is_none_or(|(ax, ay)| (p.0 - ax).hypot(p.1 - ay) >= radius)
}

/// 120 ms of steady aim on Focus or Drink: fan out its children.
pub fn on_branch_timer(app: &mut App, hwnd: HWND) {
    unsafe {
        let _ = KillTimer(Some(hwnd), TIMER_BRANCH);
    }
    if let Some(i) = app
        .dial_aim
        .filter(|i| app.dial_open && has_branch(*i) && app.dial_branch.is_none())
    {
        open_branch(app, i, false);
    }
}

/// One animation frame: the reveal springs, then hide the window once it has closed.
pub fn tick(app: &mut App, dt: f32) {
    let dial_was_visible = app.dial_open || app.dial_reveal.pos > 0.02;
    app.dial_reveal.step(dt);
    app.branch_reveal.step(dt);
    app.dial_tags.step(dt);
    if dial_was_visible {
        place_dial(app);
        if !app.dial_open && app.dial_reveal.pos <= 0.02 {
            app.dial_reveal.snap(0.0);
            unsafe {
                let _ = ShowWindow(app.dial_hwnd, SW_HIDE);
            }
        } else {
            render_dial(app);
        }
    }
}

pub fn is_animating(app: &App) -> bool {
    !app.dial_reveal.settled(0.002)
        || (app.dial_branch.is_some() && !app.branch_reveal.settled(0.002))
        || (app.dial_open && !app.dial_tags.settled(0.002))
}

pub unsafe extern "system" fn dial_proc(
    hwnd: HWND,
    msg: u32,
    wparam: WPARAM,
    lparam: LPARAM,
) -> LRESULT {
    match msg {
        WM_MOUSEACTIVATE => return LRESULT(MA_NOACTIVATE as isize),
        WM_LBUTTONUP => {
            with_app(
                |app| match (app.dial_branch, app.dial_child, app.dial_aim) {
                    (Some(parent), Some(j), _) => pick_child(app, parent, j),
                    (_, _, Some(i)) => pick(app, i),
                    _ => close_dial(app),
                },
            );
            return LRESULT(0);
        }
        _ => {}
    }
    DefWindowProcW(hwnd, msg, wparam, lparam)
}

#[cfg(test)]
mod tests {
    use super::moved_away;

    #[test]
    fn a_mouse_nudge_never_takes_the_dial_from_the_keys() {
        let anchor = Some((500.0, 400.0));
        assert!(!moved_away(anchor, (510.0, 405.0), 32.0));
        assert!(!moved_away(anchor, (500.0, 431.0), 32.0));
        assert!(moved_away(anchor, (540.0, 400.0), 32.0));
        assert!(moved_away(None, (500.0, 400.0), 32.0));
    }
}
