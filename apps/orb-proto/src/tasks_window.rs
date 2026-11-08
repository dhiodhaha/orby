//! The Tasks window: every task in one ordinary top-level window (taskbar, Alt+Tab, its own
//! keyboard), next to the small Tasks panel. Both read and write the same `app.db` store through
//! `tasks::commit`, so a change in one shows in the other. Like the Disk window it is not
//! topmost, a click elsewhere leaves it open, and Esc, its close button, Alt+F4 or the taskbar
//! close it. tasks_model.rs decides (groups, reorder, edits); tasks_window_panel.rs draws.

use std::time::Instant;

use windows::core::w;
use windows::Win32::Foundation::*;
use windows::Win32::Graphics::Gdi::*;
use windows::Win32::UI::Input::KeyboardAndMouse::*;
use windows::Win32::UI::WindowsAndMessaging::*;

use app_store::{EntityRef, Task, TaskEdit, TaskId};

use crate::gfx::Surface;
use crate::hook::unpack;
use crate::tasks::{self, clock};
use crate::tasks_model::{self as m, Clock, Group, Selection, WinItem, DUE_PICKS};
use crate::tasks_window_panel::{self as twp, Hit, ItemV, RowV};
use crate::textfield::{Key, TextField};
use crate::ui::{self, Icon};
use crate::{dial, log_line, orb, panels, px, with_app, App, MSG_MOUSELEAVE};

/// About 60 frames a second, only while a ripple or a rolling number moves.
const TIMER_FRAME: usize = 1;
/// The Undo bar after a delete goes after this long.
const TIMER_UNDO: usize = 2;
const UNDO_MS: u32 = 8000;

/// Where the keyboard is: the add field, a task, or the Done heading.
#[derive(Clone, Debug, Default, PartialEq)]
enum Focus {
    #[default]
    Field,
    Task(TaskId),
    DoneHeader,
}

#[derive(Default)]
pub struct Win {
    pub hwnd: HWND,
    surface: Option<Surface>,
    pub open: bool,
    /// The window's height (logical px), set from the screen when it opens.
    h: f32,
    hover: Option<Hit>,
    mouse: Option<(f32, f32)>,
    tracking: bool,
    ticking: bool,
    /// The window is the active one (the caret shows only then).
    active: bool,
    field: TextField,
    focus: Focus,
    /// The ring shows after a key, not after a click.
    kb: bool,
    editing: Option<(TaskId, TextField)>,
    /// The due menu: the task it is for and the keyboard's row.
    menu: Option<(TaskId, usize)>,
    done_open: bool,
    scroll: f32,
    /// The last deleted task, while Undo is offered.
    undo: Option<Task>,
    /// A one-off line under the field (an error, a hint), until the next action.
    notice: Option<String>,
    /// The last click on a task's title, to tell a double click.
    last_click: Option<(TaskId, Instant)>,
    /// A key that did its job on WM_KEYDOWN also arrives as WM_CHAR; drop that one.
    eat_char: bool,
}

// ---- what it shows ----

fn items(app: &App) -> Vec<WinItem> {
    m::window_items(
        m::groups(&app.tasks.snap, &clock()),
        app.tasks.win.done_open,
    )
}

fn header_key(g: Group) -> &'static str {
    match g {
        Group::Today => "tasks.win.today",
        Group::Upcoming => "tasks.win.upcoming",
        Group::NoDate => "tasks.win.nodate",
        Group::Done => "tasks.win.done",
    }
}

fn row(app: &App, t: &Task, c: &Clock) -> RowV {
    let (due, overdue) = match t.completed_at.filter(|_| t.done) {
        Some(ms) => {
            let (day, minute) = c.day_of_ms(ms);
            let when = if day == c.today {
                m::hhmm(minute)
            } else {
                m::day_phrase(day, c.today)
            };
            (format!("Done {when}"), false)
        }
        None => m::due_label(t, c),
    };
    RowV {
        title: t.title.clone(),
        list: if t.list_id == app.tasks.inbox {
            String::new()
        } else {
            m::list_name(&app.tasks.snap, &t.list_id)
        },
        due,
        overdue,
        repeat: t.repeat != app_store::Repeat::None && !t.done && !overdue,
        done: t.done,
    }
}

fn index_of(its: &[WinItem], focus: &Focus) -> Option<usize> {
    its.iter().position(|it| match (it, focus) {
        (WinItem::Task(_, t), Focus::Task(id)) => &t.id == id,
        (WinItem::Header(Group::Done, _), Focus::DoneHeader) => true,
        _ => false,
    })
}

fn task_index(its: &[WinItem], id: &TaskId) -> Option<usize> {
    index_of(its, &Focus::Task(id.clone()))
}

fn field_view(f: &TextField, focused: bool) -> twp::FieldV {
    twp::FieldV {
        text: f.text().to_owned(),
        caret: f.caret(),
        selection: f.selection(),
        focused,
    }
}

fn view(app: &App) -> twp::View {
    let c = clock();
    let w = &app.tasks.win;
    let its = items(app);
    let open_n = app.tasks.snap.tasks.values().filter(|t| !t.done).count();
    let line = match &w.notice {
        Some(n) => n.clone(),
        None if w.field.text().trim().is_empty() => {
            "Enter adds it. Dates work in Indonesian or English, and # picks a list.".into()
        }
        None => {
            let q = app.tasks.parse_text(w.field.text(), &c);
            if q.title.is_empty() {
                "Add a title too.".into()
            } else {
                m::understood(
                    &app.tasks.snap,
                    &q,
                    &Selection::Inbox,
                    &app.tasks.inbox,
                    &c,
                    &app.tasks.me,
                )
            }
        }
    };
    let editing = w
        .editing
        .as_ref()
        .and_then(|(id, f)| task_index(&its, id).map(|i| (i, field_view(f, w.active))));
    let menu = w.menu.as_ref().and_then(|(id, sel)| {
        let item = task_index(&its, id)?;
        Some(twp::MenuV {
            item,
            rows: DUE_PICKS
                .iter()
                .map(|p| {
                    let date = m::picked_due(*p, None, c.today)
                        .map(|d| m::day_phrase(d.date, c.today))
                        .filter(|s| s != p.label())
                        .unwrap_or_default();
                    (p.label().to_owned(), date)
                })
                .collect(),
            sel: *sel,
        })
    });
    let ring = if w.kb && w.focus != Focus::Field {
        index_of(&its, &w.focus)
    } else {
        None
    };
    let empty = its.is_empty().then(|| {
        (
            "No tasks yet".to_string(),
            "Add one above. Dates work too, like besok 9am.".to_string(),
        )
    });
    let items = its
        .iter()
        .map(|it| match it {
            WinItem::Header(g, n) => ItemV::Header {
                name: g.name(),
                count: *n,
                key: header_key(*g),
                fold: (*g == Group::Done).then_some(w.done_open),
            },
            WinItem::Task(_, t) => ItemV::Task(row(app, t, &c)),
        })
        .collect();
    let mut v = twp::View {
        h: w.h,
        open_n,
        field: field_view(
            &w.field,
            w.active && w.focus == Focus::Field && w.editing.is_none(),
        ),
        line,
        items,
        scroll: 0.0,
        ring,
        editing,
        menu,
        undo: w.undo.as_ref().map(|t| t.title.clone()),
        empty,
    };
    v.scroll = w.scroll.clamp(0.0, twp::max_scroll(&v));
    v
}

pub fn dial_child(app: &App) -> ui::DialChild {
    let n = app.tasks.snap.tasks.values().filter(|t| !t.done).count();
    ui::DialChild {
        text: String::new(),
        icon: Some(Icon::List),
        name: "All tasks".into(),
        value: if n == 0 {
            String::new()
        } else {
            format!("{n} open")
        },
    }
}

// ---- the window ----

/// `H` tall when the screen has room, less on a short one.
fn room(app: &App) -> f32 {
    let wa = orb::work_area(app.hwnd);
    let tall = (wa.bottom - wa.top) as f32 / app.scale - 32.0;
    tall.clamp(twp::MIN_H, twp::H)
}

/// Opens the window centred on the orb's screen, or brings it to the front if it is open.
pub fn show(app: &mut App) {
    dial::close_dial(app);
    if app.panel_open {
        // The panel would hand the keyboard back to the app it took it from; this window takes it.
        app.panel_prev_fg = HWND::default();
        panels::close_panel(app);
    }
    let hwnd = app.tasks.win.hwnd;
    if app.tasks.win.open {
        refresh(app);
        unsafe {
            if IsIconic(hwnd).as_bool() {
                let _ = ShowWindow(hwnd, SW_RESTORE);
            }
        }
        take_foreground(app);
        return;
    }
    let w = &mut app.tasks.win;
    w.open = true;
    w.hover = None;
    w.mouse = None;
    w.focus = Focus::Field;
    w.kb = false;
    w.scroll = 0.0;
    w.done_open = false;
    w.menu = None;
    w.editing = None;
    w.notice = None;
    app.tasks.win.h = room(app);
    crate::motion::ripple_reset("tasks.win");
    place(app, true);
    render(app);
    unsafe {
        let _ = ShowWindow(hwnd, SW_SHOW);
    }
    take_foreground(app);
    let n = app.tasks.snap.tasks.len();
    log_line(app, &format!("tasks: window open, {n} tasks"));
}

pub fn close(app: &mut App) {
    if !app.tasks.win.open {
        return;
    }
    save_edit(app);
    let w = &mut app.tasks.win;
    w.open = false;
    w.hover = None;
    w.mouse = None;
    w.ticking = false;
    w.menu = None;
    w.undo = None;
    unsafe {
        let _ = KillTimer(Some(w.hwnd), TIMER_FRAME);
        let _ = KillTimer(Some(w.hwnd), TIMER_UNDO);
        let _ = ShowWindow(w.hwnd, SW_HIDE);
    }
    // The swap chain is let go while the window is hidden.
    w.surface = None;
    log_line(app, "tasks: window closed");
}

fn place(app: &App, center: bool) {
    let (w, h) = (
        px(app, twp::W).ceil() as i32,
        px(app, app.tasks.win.h).ceil() as i32,
    );
    let hwnd = app.tasks.win.hwnd;
    let (x, y) = if center {
        let wa = orb::work_area(app.hwnd);
        ((wa.left + wa.right - w) / 2, (wa.top + wa.bottom - h) / 2)
    } else {
        let mut r = RECT::default();
        unsafe {
            let _ = GetWindowRect(hwnd, &mut r);
        }
        (r.left, r.top)
    };
    unsafe {
        let _ = SetWindowPos(hwnd, None, x, y, w, h, SWP_NOACTIVATE | SWP_NOZORDER);
        let d = px(app, 32.0).round() as i32;
        let rgn = CreateRoundRectRgn(0, 0, w + 1, h + 1, d, d);
        let _ = SetWindowRgn(hwnd, Some(rgn), true);
    }
}

fn take_foreground(app: &mut App) {
    let hwnd = app.tasks.win.hwnd;
    unsafe {
        let _ = SetForegroundWindow(hwnd);
        let _ = SetFocus(Some(hwnd));
        // WM_ACTIVATE arrives inside those calls, while the App is borrowed, so read the result.
        app.tasks.win.active = GetForegroundWindow() == hwnd;
    }
    render(app);
}

fn render(app: &mut App) {
    if !app.tasks.win.open {
        return;
    }
    let (w, h) = (
        px(app, twp::W).ceil() as u32,
        px(app, app.tasks.win.h).ceil() as u32,
    );
    let hwnd = app.tasks.win.hwnd;
    match app.tasks.win.surface.as_mut() {
        None => app.tasks.win.surface = Surface::new(&app.g, hwnd, w, h, app.scale).ok(),
        Some(s) if s.w != w || s.h != h => {
            let _ = s.resize(&app.g, w, h);
            place(app, false);
        }
        _ => {}
    }
    let v = view(app);
    let Some(s) = app.tasks.win.surface.as_ref() else {
        return;
    };
    let dc = s.begin(&app.g);
    twp::draw(&app.g, dc, &app.theme, &app.fonts, &v, app.tasks.win.hover);
    s.end(&app.g);
    // Frames only while a ripple or a rolling number moves.
    let moving = crate::motion::active();
    if moving != app.tasks.win.ticking {
        app.tasks.win.ticking = moving;
        unsafe {
            if moving {
                SetTimer(Some(hwnd), TIMER_FRAME, 16, None);
            } else {
                let _ = KillTimer(Some(hwnd), TIMER_FRAME);
            }
        }
    }
}

/// Redraw after the tasks changed (here, in the panel, Today or a due pill). A focused task that
/// went away hands the keyboard back to the field; the hover follows what is now under the cursor.
pub fn refresh(app: &mut App) {
    if !app.tasks.win.open {
        return;
    }
    let its = items(app);
    let w = &mut app.tasks.win;
    if w.focus != Focus::Field && index_of(&its, &w.focus).is_none() {
        w.focus = Focus::Field;
    }
    if w.editing
        .as_ref()
        .is_some_and(|(id, _)| task_index(&its, id).is_none())
    {
        w.editing = None;
    }
    if w.menu
        .as_ref()
        .is_some_and(|(id, _)| task_index(&its, id).is_none())
    {
        w.menu = None;
    }
    let v = view(app);
    app.tasks.win.hover = app
        .tasks
        .win
        .mouse
        .and_then(|(x, y)| twp::hit(&app.g, &app.fonts, &v, x, y));
    render(app);
}

/// Redraw as it is (the theme changed).
pub fn redraw(app: &mut App) {
    render(app);
}

// ---- what it does ----

fn report(app: &mut App, what: &str, r: app_store::Result<()>) {
    if let Err(e) = r {
        log_line(app, &format!("tasks: window: {what} failed: {e}"));
        app.tasks.win.notice = Some(format!("Couldn't {what}. {e}"));
    }
}

fn task_at(app: &App, i: usize) -> Option<Task> {
    match items(app).into_iter().nth(i)? {
        WinItem::Task(_, t) => Some(*t),
        WinItem::Header(..) => None,
    }
}

fn find(app: &App, id: &TaskId) -> Option<Task> {
    app.tasks.snap.tasks.get(id).cloned()
}

fn add(app: &mut App) {
    let c = clock();
    let text = app.tasks.win.field.text().trim().to_owned();
    if text.is_empty() {
        return;
    }
    let q = app.tasks.parse_text(&text, &c);
    if q.title.is_empty() {
        app.tasks.win.notice = Some("Add a title as well as the date.".into());
        return render(app);
    }
    let t = &app.tasks;
    let list = m::target_list(&t.snap, &q, &Selection::Inbox, &t.inbox);
    let new = m::new_task(&t.snap, &q, &Selection::Inbox, &list, &c);
    let r = app.tasks.store.create_task(&list, new, c.now_ms);
    let name = m::list_name(&app.tasks.snap, &list);
    log_line(
        app,
        &format!("tasks: window added \"{}\" to {name}", q.title),
    );
    match r {
        Ok(id) => {
            app.tasks.win.field.clear();
            app.tasks.store.snapshot().map(|s| app.tasks.snap = s).ok();
            reveal_task(app, &id);
        }
        Err(e) => report(app, "add the task", Err(e)),
    }
    tasks::commit(app);
}

/// Scrolls just enough to show a task.
fn reveal_task(app: &mut App, id: &TaskId) {
    let its = items(app);
    if let Some(i) = task_index(&its, id) {
        let v = view(app);
        app.tasks.win.scroll = twp::reveal(&v, i);
    }
}

fn toggle(app: &mut App, t: &Task) {
    let c = clock();
    let (what, r) = if t.done {
        (
            "reopen the task",
            app.tasks.store.set_done(&t.id, false, c.now_ms),
        )
    } else {
        ("finish the task", m::complete(&mut app.tasks.store, t, &c))
    };
    log_line(app, &format!("tasks: window: {what}: {}", t.title));
    report(app, what, r);
    tasks::commit(app);
}

fn delete(app: &mut App, t: Task) {
    let c = clock();
    // The keyboard moves on to the next stop, or the one before at the end.
    let stops = stops(&items(app));
    let at = stops.iter().position(|f| *f == Focus::Task(t.id.clone()));
    let r = app
        .tasks
        .store
        .delete(&EntityRef::Task(t.id.clone()), c.now_ms);
    log_line(app, &format!("tasks: window deleted {}", t.title));
    if r.is_ok() {
        if let Some(at) = at {
            app.tasks.win.focus = stops
                .get(at + 1)
                .or(at.checked_sub(1).and_then(|k| stops.get(k)))
                .cloned()
                .unwrap_or_default();
        }
        app.tasks.win.undo = Some(t);
        unsafe {
            SetTimer(Some(app.tasks.win.hwnd), TIMER_UNDO, UNDO_MS, None);
        }
    }
    report(app, "delete the task", r);
    tasks::commit(app);
}

fn undo(app: &mut App) {
    let Some(t) = app.tasks.win.undo.take() else {
        return;
    };
    unsafe {
        let _ = KillTimer(Some(app.tasks.win.hwnd), TIMER_UNDO);
    }
    let c = clock();
    let r = app
        .tasks
        .store
        .create_task(&t.list_id, m::restored(&t), c.now_ms);
    match r {
        Ok(id) => {
            if t.done {
                let r = app.tasks.store.set_done(&id, true, c.now_ms);
                report(app, "put the task back", r);
            }
            log_line(app, &format!("tasks: window put back {}", t.title));
            app.tasks.store.snapshot().map(|s| app.tasks.snap = s).ok();
            app.tasks.win.focus = Focus::Task(id.clone());
            reveal_task(app, &id);
        }
        Err(e) => report(app, "put the task back", Err(e)),
    }
    tasks::commit(app);
}

fn start_edit(app: &mut App, t: &Task) {
    let mut f = TextField::default();
    f.insert(&t.title);
    app.tasks.win.editing = Some((t.id.clone(), f));
    app.tasks.win.focus = Focus::Task(t.id.clone());
    app.tasks.win.menu = None;
}

/// Saves the inline edit, if it changed anything.
fn save_edit(app: &mut App) {
    let Some((id, f)) = app.tasks.win.editing.take() else {
        return;
    };
    let Some(t) = find(app, &id) else { return };
    if let Some(title) = m::edited_title(&t.title, f.text()) {
        let r = app
            .tasks
            .store
            .edit_task(&id, TaskEdit::Title(title.clone()), clock().now_ms);
        log_line(
            app,
            &format!("tasks: window renamed {} to {title}", t.title),
        );
        report(app, "rename the task", r);
        tasks::commit(app);
    } else {
        render(app);
    }
}

fn set_due(app: &mut App, id: &TaskId, k: usize) {
    app.tasks.win.menu = None;
    let (Some(t), Some(pick)) = (find(app, id), DUE_PICKS.get(k).copied()) else {
        return render(app);
    };
    let c = clock();
    let due = m::picked_due(pick, t.due, c.today);
    let r = app.tasks.store.edit_task(id, TaskEdit::Due(due), c.now_ms);
    log_line(app, &format!("tasks: window due {} {:?}", t.title, pick));
    report(app, "change the date", r);
    app.tasks.store.snapshot().map(|s| app.tasks.snap = s).ok();
    reveal_task(app, id);
    tasks::commit(app);
}

/// Alt+Up or Alt+Down on the focused task.
fn move_task(app: &mut App, up: bool) {
    let Focus::Task(id) = app.tasks.win.focus.clone() else {
        return;
    };
    let c = clock();
    let Some(group) = m::groups(&app.tasks.snap, &c)
        .into_iter()
        .map(|(_, v)| v)
        .find(|v| v.iter().any(|t| t.id == id))
    else {
        return;
    };
    let i = group.iter().position(|t| t.id == id).unwrap_or(0);
    match m::reorder(&group, i, up) {
        Some(writes) => {
            for (tid, o) in writes {
                let r = app
                    .tasks
                    .store
                    .edit_task(&tid, TaskEdit::Order(o), c.now_ms);
                report(app, "move the task", r);
            }
            log_line(
                app,
                &format!(
                    "tasks: window moved {} {}",
                    i,
                    if up { "up" } else { "down" }
                ),
            );
            app.tasks.store.snapshot().map(|s| app.tasks.snap = s).ok();
            reveal_task(app, &id);
            tasks::commit(app);
        }
        None => {
            let edge = if up { i == 0 } else { i + 1 >= group.len() };
            if !edge && !group[i].done {
                app.tasks.win.notice =
                    Some("Tasks move past others due at the same time. Change the date to move it further.".into());
            }
            render(app);
        }
    }
}

fn toggle_done_group(app: &mut App) {
    let w = &mut app.tasks.win;
    w.done_open = !w.done_open;
    w.focus = Focus::DoneHeader;
    if w.done_open {
        crate::motion::ripple_reset("tasks.win");
    }
    render(app);
}

// ---- the keyboard ----

/// The keyboard's stops in the list, top to bottom: tasks and the Done heading.
fn stops(its: &[WinItem]) -> Vec<Focus> {
    its.iter()
        .filter_map(|it| match it {
            WinItem::Task(_, t) => Some(Focus::Task(t.id.clone())),
            WinItem::Header(Group::Done, _) => Some(Focus::DoneHeader),
            WinItem::Header(..) => None,
        })
        .collect()
}

fn step_focus(app: &mut App, down: bool) {
    let its = items(app);
    let stops = stops(&its);
    let w = &mut app.tasks.win;
    let at = stops.iter().position(|f| *f == w.focus);
    w.focus = match (at, down) {
        (None, true) => stops.first().cloned().unwrap_or_default(),
        (None, false) => Focus::Field,
        (Some(0), false) => Focus::Field,
        (Some(k), false) => stops[k - 1].clone(),
        (Some(k), true) => stops
            .get(k + 1)
            .cloned()
            .unwrap_or_else(|| stops[k].clone()),
    };
    w.kb = true;
    if let Some(i) = index_of(&its, &w.focus) {
        let v = view(app);
        app.tasks.win.scroll = twp::reveal(&v, i);
    }
}

fn ctrl() -> bool {
    unsafe { GetKeyState(VK_CONTROL.0 as i32) < 0 }
}

fn shift() -> bool {
    unsafe { GetKeyState(VK_SHIFT.0 as i32) < 0 }
}

fn edit_key(f: &mut TextField, vk: VIRTUAL_KEY, hwnd: HWND) -> bool {
    let k = match vk {
        VK_LEFT => Key::Left,
        VK_RIGHT => Key::Right,
        VK_HOME => Key::Home,
        VK_END => Key::End,
        VK_BACK => Key::Backspace,
        VK_DELETE => Key::Delete,
        VIRTUAL_KEY(0x41) if ctrl() => {
            f.select_all();
            return true;
        }
        VIRTUAL_KEY(0x56) if ctrl() => {
            if let Some(s) = tasks::read_clipboard(hwnd) {
                f.insert(&s);
            }
            return true;
        }
        _ => return false,
    };
    f.key(k, ctrl(), shift());
    true
}

fn on_keydown(app: &mut App, vk: u16) -> bool {
    let vk = VIRTUAL_KEY(vk);
    let hwnd = app.tasks.win.hwnd;
    app.tasks.win.notice = None;
    // The due menu first: it holds the keys while it is open.
    if let Some((id, sel)) = app.tasks.win.menu.clone() {
        match vk {
            VK_UP | VK_DOWN => {
                let n = DUE_PICKS.len();
                let sel = if vk == VK_UP {
                    (sel + n - 1) % n
                } else {
                    (sel + 1) % n
                };
                app.tasks.win.menu = Some((id, sel));
            }
            VK_RETURN | VK_SPACE => {
                app.tasks.win.eat_char = vk == VK_SPACE;
                set_due(app, &id, sel);
                return true;
            }
            VK_ESCAPE => app.tasks.win.menu = None,
            _ => return true,
        }
        render(app);
        return true;
    }
    // Then an inline edit: Enter saves, Esc puts the title back.
    if app.tasks.win.editing.is_some() {
        match vk {
            VK_RETURN | VK_TAB => save_edit(app),
            VK_ESCAPE => {
                app.tasks.win.editing = None;
                render(app);
            }
            _ => {
                if let Some((_, f)) = app.tasks.win.editing.as_mut() {
                    if !edit_key(f, vk, hwnd) {
                        return false;
                    }
                }
                render(app);
            }
        }
        return true;
    }
    if vk == VIRTUAL_KEY(0x5A) && ctrl() && app.tasks.win.undo.is_some() {
        undo(app);
        return true;
    }
    match (app.tasks.win.focus.clone(), vk) {
        (_, VK_UP) => step_focus(app, false),
        (_, VK_DOWN) => step_focus(app, true),
        (Focus::Field, VK_TAB) => step_focus(app, true),
        (_, VK_TAB) => app.tasks.win.focus = Focus::Field,
        (Focus::Field, VK_ESCAPE) => {
            if app.tasks.win.field.is_empty() {
                close(app);
                return true;
            }
            app.tasks.win.field.clear();
        }
        (_, VK_ESCAPE) => {
            close(app);
            return true;
        }
        (Focus::Field, VK_RETURN) => {
            add(app);
            return true;
        }
        (Focus::Field, _) => {
            if !edit_key(&mut app.tasks.win.field, vk, hwnd) {
                return false;
            }
        }
        (Focus::DoneHeader, VK_RETURN | VK_SPACE) => {
            app.tasks.win.eat_char = vk == VK_SPACE;
            toggle_done_group(app);
            return true;
        }
        (Focus::Task(id), _) => {
            let Some(t) = find(app, &id) else {
                return false;
            };
            app.tasks.win.kb = true;
            match vk {
                VK_RETURN | VK_F2 => start_edit(app, &t),
                VK_SPACE => {
                    app.tasks.win.eat_char = true;
                    toggle(app, &t);
                    return true;
                }
                VK_DELETE => {
                    delete(app, t);
                    return true;
                }
                // D opens the due menu.
                VIRTUAL_KEY(0x44) if !t.done && !ctrl() => {
                    app.tasks.win.eat_char = true;
                    app.tasks.win.menu = Some((id, 0));
                }
                _ => return false,
            }
        }
        _ => return false,
    }
    render(app);
    true
}

/// Typing: into the inline edit, else the add field (a letter typed on a task moves there).
fn on_char(app: &mut App, unit: u16) {
    let w = &mut app.tasks.win;
    if std::mem::take(&mut w.eat_char) || w.menu.is_some() {
        return;
    }
    let changed = match w.editing.as_mut() {
        Some((_, f)) => f.type_utf16(unit),
        None => {
            let typed = w.field.type_utf16(unit);
            if typed {
                w.focus = Focus::Field;
            }
            typed
        }
    };
    if changed {
        w.notice = None;
        render(app);
    }
}

// ---- the mouse ----

fn press(app: &mut App, x: f32, y: f32) {
    let v = view(app);
    match twp::hit(&app.g, &app.fonts, &v, x, y) {
        Some(Hit::Field) => {
            save_edit(app);
            let v = view(app);
            let b = twp::field_byte_at(&app.g, &app.fonts, &v, x);
            let w = &mut app.tasks.win;
            w.menu = None;
            w.focus = Focus::Field;
            w.field.place(b, shift());
            render(app);
        }
        Some(Hit::Edit(_)) => {
            let b = twp::edit_byte_at(&app.g, &app.fonts, &v, x);
            if let Some((_, f)) = app.tasks.win.editing.as_mut() {
                f.place(b, shift());
            }
            render(app);
        }
        _ => {}
    }
}

fn click(app: &mut App, h: Option<Hit>) {
    app.tasks.win.notice = None;
    app.tasks.win.kb = false;
    if let Some((id, _)) = app.tasks.win.menu.clone() {
        if let Some(Hit::Menu(k)) = h {
            return set_due(app, &id, k);
        }
        app.tasks.win.menu = None;
        if h.is_none() || matches!(h, Some(Hit::Due(_))) {
            return render(app);
        }
    }
    if app.tasks.win.editing.is_some() && !matches!(h, Some(Hit::Edit(_))) {
        save_edit(app);
    }
    let Some(h) = h else { return render(app) };
    match h {
        Hit::Close => return close(app),
        Hit::Field | Hit::Edit(_) | Hit::Menu(_) => {}
        Hit::Add => return add(app),
        Hit::Undo => return undo(app),
        Hit::Header(_) => return toggle_done_group(app),
        Hit::Check(i) => {
            if let Some(t) = task_at(app, i) {
                app.tasks.win.focus = Focus::Task(t.id.clone());
                return toggle(app, &t);
            }
        }
        Hit::Delete(i) => {
            if let Some(t) = task_at(app, i) {
                return delete(app, t);
            }
        }
        Hit::Due(i) => {
            if let Some(t) = task_at(app, i) {
                app.tasks.win.focus = Focus::Task(t.id.clone());
                app.tasks.win.menu = Some((t.id, 0));
            }
        }
        Hit::Title(i) => {
            if let Some(t) = task_at(app, i) {
                let twice = unsafe { GetDoubleClickTime() } as u128;
                let double = app
                    .tasks
                    .win
                    .last_click
                    .as_ref()
                    .is_some_and(|(id, at)| *id == t.id && at.elapsed().as_millis() <= twice);
                if double {
                    app.tasks.win.last_click = None;
                    start_edit(app, &t);
                } else {
                    app.tasks.win.last_click = Some((t.id.clone(), Instant::now()));
                    app.tasks.win.focus = Focus::Task(t.id);
                }
            }
        }
    }
    render(app);
}

fn on_wheel(app: &mut App, delta: i16) {
    let v = view(app);
    let s = (v.scroll - delta as f32 / 120.0 * 48.0).clamp(0.0, twp::max_scroll(&v));
    if s != v.scroll {
        app.tasks.win.scroll = s;
        app.tasks.win.menu = None;
        render(app);
    }
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

pub unsafe extern "system" fn tasks_proc(
    hwnd: HWND,
    msg: u32,
    wparam: WPARAM,
    lparam: LPARAM,
) -> LRESULT {
    match msg {
        WM_NCHITTEST => {
            // The title band's empty part drags the window.
            let p = client_point(hwnd, lparam);
            let caption = with_app(|app| {
                let (x, y) = (p.x as f32 / app.scale, p.y as f32 / app.scale);
                y < twp::TOP && twp::hit(&app.g, &app.fonts, &view(app), x, y).is_none()
            })
            .unwrap_or(false);
            return LRESULT(if caption { HTCAPTION } else { HTCLIENT } as isize);
        }
        WM_ACTIVATE => {
            let on = (wparam.0 & 0xffff) != 0;
            with_app(|app| {
                if app.tasks.win.active != on {
                    app.tasks.win.active = on;
                    render(app);
                }
            });
        }
        WM_MOUSEMOVE => {
            with_app(|app| {
                let (x, y) = unpack(lparam);
                let (x, y) = (x / app.scale, y / app.scale);
                if !app.tasks.win.tracking {
                    let mut tme = TRACKMOUSEEVENT {
                        cbSize: std::mem::size_of::<TRACKMOUSEEVENT>() as u32,
                        dwFlags: TME_LEAVE,
                        hwndTrack: hwnd,
                        dwHoverTime: 0,
                    };
                    let _ = TrackMouseEvent(&mut tme);
                    app.tasks.win.tracking = true;
                }
                app.tasks.win.mouse = Some((x, y));
                let h = twp::hit(&app.g, &app.fonts, &view(app), x, y);
                if h != app.tasks.win.hover {
                    app.tasks.win.hover = h;
                    render(app);
                }
            });
            return LRESULT(0);
        }
        MSG_MOUSELEAVE => {
            with_app(|app| {
                app.tasks.win.tracking = false;
                app.tasks.win.mouse = None;
                if app.tasks.win.hover.take().is_some() {
                    render(app);
                }
            });
            return LRESULT(0);
        }
        WM_TIMER if wparam.0 == TIMER_FRAME => {
            with_app(render);
            return LRESULT(0);
        }
        WM_TIMER if wparam.0 == TIMER_UNDO => {
            with_app(|app| {
                let _ = KillTimer(Some(hwnd), TIMER_UNDO);
                if app.tasks.win.undo.take().is_some() {
                    refresh(app);
                }
            });
            return LRESULT(0);
        }
        WM_LBUTTONDOWN => {
            with_app(|app| {
                let (x, y) = unpack(lparam);
                press(app, x / app.scale, y / app.scale);
            });
            return LRESULT(0);
        }
        WM_LBUTTONUP => {
            with_app(|app| {
                let (x, y) = unpack(lparam);
                let h = twp::hit(&app.g, &app.fonts, &view(app), x / app.scale, y / app.scale);
                click(app, h);
            });
            return LRESULT(0);
        }
        WM_MOUSEWHEEL => {
            let delta = ((wparam.0 >> 16) & 0xffff) as u16 as i16;
            with_app(|app| on_wheel(app, delta));
            return LRESULT(0);
        }
        WM_KEYDOWN => {
            if with_app(|app| on_keydown(app, wparam.0 as u16)).unwrap_or(false) {
                return LRESULT(0);
            }
        }
        // Alt+Up and Alt+Down move the focused task; other Alt keys (Alt+F4) go on as usual.
        WM_SYSKEYDOWN
            if VIRTUAL_KEY(wparam.0 as u16) == VK_UP || VIRTUAL_KEY(wparam.0 as u16) == VK_DOWN =>
        {
            let up = VIRTUAL_KEY(wparam.0 as u16) == VK_UP;
            with_app(|app| {
                app.tasks.win.kb = true;
                move_task(app, up);
            });
            return LRESULT(0);
        }
        WM_CHAR => {
            with_app(|app| on_char(app, wparam.0 as u16));
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
            w!("OrbTasks"),
            w!("Orby tasks"),
            WS_POPUP,
            0,
            0,
            twp::W as i32,
            twp::H as i32,
            None,
            None,
            Some(hinst),
            None,
        )
    }
}
