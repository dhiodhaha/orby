//! Tasks on the orb, on `app_store` (`%APPDATA%\orb-proto\app.db`) and `app_quickadd`: the
//! Tasks panel's state and input, the one exact timer to the next due task (its pill has Done),
//! and the overdue count behind the orb's badge. tasks_model.rs decides; tasks_panel.rs draws.

use std::collections::HashSet;
use std::path::Path;

use serde::{Deserialize, Serialize};
use windows::Win32::Foundation::*;
use windows::Win32::Graphics::Direct2D::ID2D1DeviceContext;
use windows::Win32::System::DataExchange::{CloseClipboard, GetClipboardData, OpenClipboard};
use windows::Win32::System::Memory::{GlobalLock, GlobalUnlock};
use windows::Win32::UI::Input::KeyboardAndMouse::*;
use windows::Win32::UI::WindowsAndMessaging::*;

use app_habits::held::Priority;
use app_quickadd as qa;
use app_store::{DeviceId, EntityRef, ListId, Snapshot, Store, Task, TaskId, UserId};

use crate::panels::{self, Kind};
use crate::pill::dismiss_pill;
use crate::tasks_model::{self as m, Clock, Row, Selection, SideKey, Stamp, Tab};
use crate::tasks_panel::{self as tp, FieldView, RowView, View};
use crate::textfield::{Key, TextField};
use crate::ui::{Icon, Pill, PillAction};
use crate::{habits, log_line, notices, orb, prayer, App, TIMER_TASKS};

const GRAMMAR: &str = "# list · / folder · @ person";

#[derive(Clone, Copy, Debug, PartialEq)]
pub enum FieldMode {
    Task,
    NewList,
}

/// `tasks.json`: this install's device id and the inbox list. The tasks are in `app.db`.
#[derive(Serialize, Deserialize, Default)]
struct Config {
    device: String,
    #[serde(default)]
    inbox: Option<String>,
}

pub struct Tasks {
    /// Shared with the Tasks window (tasks_window.rs), so both show the same tasks.
    pub(crate) store: Store,
    /// Set when app.db could not be opened: the store is in memory and nothing is saved.
    store_error: Option<String>,
    pub inbox: ListId,
    pub me: UserId,
    pub snap: Snapshot,
    pub sel: Selection,
    pub tab: Tab,
    pub field: TextField,
    pub mode: FieldMode,
    /// Space and folder ids folded shut in the sidebar.
    pub collapsed: HashSet<String>,
    pub scroll: f32,
    pub side_scroll: f32,
    /// The panel has keyboard focus.
    pub focused: bool,
    /// A one-off line under the field (e.g. "Add a title"), until the next key.
    pub notice: Option<String>,
    /// Timed tasks due up to here have had their pill (or were overdue at launch).
    fired_until: Stamp,
    pub overdue: usize,
    /// The Tasks window, on the same store as the panel.
    pub win: crate::tasks_window::Win,
}

fn read_config(path: &Path) -> Config {
    std::fs::read_to_string(path)
        .ok()
        .and_then(|s| serde_json::from_str(&s).ok())
        .unwrap_or_default()
}

fn write_config(path: &Path, c: &Config) {
    if let Ok(s) = serde_json::to_vec_pretty(c) {
        let tmp = path.with_extension("json.tmp");
        if std::fs::write(&tmp, s).is_ok() {
            let _ = std::fs::rename(&tmp, path);
        }
    }
}

fn unix_ms() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_millis() as u64)
        .unwrap_or(0)
}

/// Local now, read once.
pub fn clock() -> Clock {
    let n = habits::now();
    let d = n.date();
    let now_ms = unix_ms();
    let local_secs = n.since(app_habits::Moment::from_unix(0, 0)).as_secs() as i64;
    let offset = ((local_secs - (now_ms / 1000) as i64) as f64 / 60.0).round() as i64 * 60;
    Clock {
        today: app_store::Date::new(d.year(), d.month() as u8, d.day() as u8).expect("today"),
        minute: n.minute_of_day(),
        now_ms,
        offset_secs: offset,
    }
}

fn moment_of((date, minute): Stamp) -> app_habits::Moment {
    app_habits::Date::new(date.year(), date.month() as u32, date.day() as u32)
        .expect("valid date")
        .at_minute(minute)
}

impl Tasks {
    pub fn load(dir: &Path) -> Tasks {
        let config_path = dir.join("tasks.json");
        let mut cfg = read_config(&config_path);
        if cfg.device.is_empty() {
            cfg.device = format!("pc-{:x}{:x}", unix_ms(), std::process::id());
        }
        let me = UserId(
            std::env::var("USERNAME")
                .ok()
                .filter(|s| !s.is_empty())
                .unwrap_or_else(|| "me".into()),
        );
        let (mut store, store_error) =
            match Store::open(dir.join("app.db"), DeviceId(cfg.device.clone()), me.clone()) {
                Ok(s) => (s, None),
                Err(e) => (
                    Store::open_in_memory(DeviceId(cfg.device.clone()), me.clone())
                        .expect("in-memory store"),
                    Some(e.to_string()),
                ),
            };
        let c = clock();
        let snap = store.snapshot().unwrap_or_default();
        let inbox = m::ensure_inbox(
            &mut store,
            &snap,
            cfg.inbox.as_ref().map(|s| ListId(s.clone())).as_ref(),
            c.now_ms,
        )
        .unwrap_or_else(|_| ListId("inbox".into()));
        cfg.inbox = Some(inbox.0.clone());
        write_config(&config_path, &cfg);
        let snap = store.snapshot().unwrap_or_default();
        let overdue = m::overdue_count(&snap, &c);
        Tasks {
            store,
            store_error,
            inbox,
            me,
            snap,
            sel: Selection::Inbox,
            tab: Tab::Todo,
            field: TextField::default(),
            mode: FieldMode::Task,
            collapsed: HashSet::new(),
            scroll: 0.0,
            side_scroll: 0.0,
            focused: false,
            notice: None,
            fired_until: c.stamp(),
            overdue,
            win: Default::default(),
        }
    }

    fn people(&self) -> Vec<String> {
        vec![self.me.0.clone()]
    }

    fn parse(&self, c: &Clock) -> qa::QuickAdd {
        self.parse_text(self.field.text(), c)
    }

    /// Quick add's reading of `text` (the panel's field or the window's).
    pub(crate) fn parse_text(&self, text: &str, c: &Clock) -> qa::QuickAdd {
        let now = qa::Now {
            date: m::to_qa(c.today),
            time: qa::Time::new((c.minute / 60) as u8, (c.minute % 60) as u8).expect("time"),
        };
        qa::parse(text, now, &m::known(&self.snap, &self.people()))
    }

    fn display_name(&self, u: &UserId) -> String {
        if *u == self.me {
            "You".into()
        } else {
            u.0.clone()
        }
    }
}

// ---- after every change ----

/// Re-reads the store, then fixes up everything that depends on it: the badge, the due timer,
/// the open panel and the Tasks window.
pub(crate) fn commit(app: &mut App) {
    match app.tasks.store.snapshot() {
        Ok(s) => app.tasks.snap = s,
        Err(e) => log_line(app, &format!("tasks: snapshot failed: {e}")),
    }
    let sel_gone = match &app.tasks.sel {
        Selection::List(l) => !app.tasks.snap.lists.contains_key(l),
        Selection::Folder(f) => !app.tasks.snap.folders.contains_key(f),
        _ => false,
    };
    if sel_gone {
        app.tasks.sel = Selection::Inbox;
    }
    update_badge(app, &clock());
    schedule(app);
    panels::refresh(app);
    crate::tasks_window::refresh(app);
}

fn update_badge(app: &mut App, c: &Clock) {
    let n = m::overdue_count(&app.tasks.snap, c);
    if n != app.tasks.overdue {
        app.tasks.overdue = n;
        log_line(app, &format!("tasks: {n} overdue"));
        orb::render_orb(app);
    }
}

fn report(app: &mut App, what: &str, r: app_store::Result<()>) {
    if let Err(e) = r {
        log_line(app, &format!("tasks: {what} failed: {e}"));
        app.tasks.notice = Some(format!("Couldn't {what}: {e}"));
    }
}

// ---- the due timer ----

/// One exact timer to whichever comes first: the next timed task's minute, the minute a prayer
/// marker leaves Today, or midnight (all-day tasks turn overdue and Today turns over then).
pub fn schedule(app: &mut App) {
    unsafe {
        let _ = KillTimer(Some(app.hwnd), TIMER_TASKS);
    }
    let c = clock();
    let midnight = (m::add_days(c.today, 1), 0);
    let next = m::next_due_after(&app.tasks.snap, app.tasks.fired_until);
    let marker = m::next_marker_change(&prayers_today(app, &c), c.minute).map(|mi| (c.today, mi));
    let target = [next, marker]
        .into_iter()
        .flatten()
        .fold(midnight, std::cmp::min);
    let ms = if target <= c.stamp() {
        50
    } else {
        habits::ms_until(habits::now(), moment_of(target))
    };
    unsafe {
        SetTimer(Some(app.hwnd), TIMER_TASKS, ms, None);
    }
    let next = next.map_or("none".to_string(), |(d, mi)| {
        format!("{} {}", m::weekday_name(d), m::hhmm(mi))
    });
    let marker = marker.map_or("none".to_string(), |(_, mi)| m::hhmm(mi));
    log_line(app, &format!("tasks: timer in {ms} ms (next due {next}, prayer marker change {marker}, midnight at the latest)"));
}

pub fn on_timer(app: &mut App) {
    unsafe {
        let _ = KillTimer(Some(app.hwnd), TIMER_TASKS);
    }
    // At midnight this fires before the 10 s stats timer, so load the new day's prayer times
    // first or Today would draw the new date without its markers.
    prayer::refresh_day(app);
    let c = clock();
    let due = m::due_between(&app.tasks.snap, app.tasks.fired_until, c.stamp());
    app.tasks.fired_until = c.stamp();
    update_badge(app, &c);
    match due.len() {
        0 => {}
        _ if !app.visible => log_line(
            app,
            &format!(
                "tasks: {} due while the orb is hidden (badge only)",
                due.len()
            ),
        ),
        1 => {
            let p = due_pill(&due[0]);
            log_line(app, &format!("tasks: due now {}", due[0].title));
            notices::post(app, due_notice(p));
        }
        n => {
            let names: Vec<&str> = due.iter().map(|t| t.title.as_str()).collect();
            log_line(app, &format!("tasks: {n} due now"));
            let p = Pill {
                icon: Icon::Check,
                dot: false,
                title: format!("{n} tasks due"),
                value: names.join(", "),
                action: Some(("Open".into(), PillAction::OpenTasks)),
                closable: true,
                ttl_ms: 8000,
            };
            notices::post(app, due_notice(p));
        }
    }
    panels::refresh(app);
    crate::tasks_window::refresh(app);
    schedule(app);
}

/// Due pills from one minute and the next merge while one waits: "3 tasks due" with Open.
fn due_notice(p: Pill) -> notices::Notice {
    notices::merging(
        notices::Notice::new(notices::Source::Tasks, Priority::Normal, p),
        "due",
        "tasks due",
        "",
        Some(("Open".into(), PillAction::OpenTasks)),
    )
}

fn due_pill(t: &Task) -> Pill {
    let at = t
        .due
        .and_then(|d| d.time)
        .map(|x| x.to_string())
        .unwrap_or_default();
    Pill {
        icon: Icon::Check,
        dot: false,
        title: t.title.clone(),
        value: format!("due {at}"),
        action: Some(("Done".into(), PillAction::TaskDone(t.id.clone()))),
        closable: true,
        ttl_ms: 8000,
    }
}

/// Done on the due pill.
pub fn done_from_pill(app: &mut App, id: &TaskId) {
    dismiss_pill(app);
    let c = clock();
    if let Some(t) = app.tasks.snap.tasks.get(id).cloned().filter(|t| !t.done) {
        let r = m::complete(&mut app.tasks.store, &t, &c);
        report(app, "finish the task", r);
        log_line(app, &format!("tasks: done from pill {}", t.title));
    }
    commit(app);
}

/// A checkbox in the Today panel: finish the task (a repeating one moves on), or reopen it.
pub fn toggle_done(app: &mut App, id: &TaskId) {
    let c = clock();
    let Some(t) = app.tasks.snap.tasks.get(id).cloned() else {
        return;
    };
    let (what, r) = if t.done {
        (
            "reopen the task",
            app.tasks.store.set_done(&t.id, false, c.now_ms),
        )
    } else {
        ("finish the task", m::complete(&mut app.tasks.store, &t, &c))
    };
    log_line(app, &format!("tasks: {what} from Today: {}", t.title));
    report(app, what, r);
    commit(app);
}

/// The dial's Tasks value: overdue first, else what is left today.
pub fn dial_value(app: &App) -> String {
    let c = clock();
    let today = app
        .tasks
        .snap
        .tasks
        .values()
        .filter(|t| !t.done && t.due.is_some_and(|d| d.date == c.today))
        .count();
    match (app.tasks.overdue, today) {
        (0, 0) => String::new(),
        (0, n) => format!("{n} today"),
        (n, _) => format!("{n} overdue"),
    }
}

/// The dial's Today value: what is still open in Today (due today or earlier).
pub fn dial_today_value(app: &App) -> String {
    let c = clock();
    match app
        .tasks
        .snap
        .tasks
        .values()
        .filter(|t| !t.done && t.due.is_some_and(|d| d.date <= c.today))
        .count()
    {
        0 => String::new(),
        n => format!("{n} left"),
    }
}

/// Clearly marked sample tasks for a test drive (tray), only into an empty store.
pub fn add_samples(app: &mut App) {
    let c = clock();
    let r = m::add_samples(&mut app.tasks.store, &app.tasks.snap, &app.tasks.inbox, &c);
    let n = r.as_ref().map_or(0, |n| *n);
    log_line(app, &format!("tasks: added {n} sample tasks"));
    commit(app);
}

// ---- the panel ----

pub fn open(app: &mut App, sel: Option<Selection>) {
    if let Some(s) = sel {
        select(app, s);
    }
    commit(app);
    panels::open_panel(app, Kind::Tasks);
}

/// Today's "Add task": Tasks opens on its Today list with the field ready, and Esc comes back.
pub fn add_from_today(app: &mut App) {
    select(app, Selection::Today);
    commit(app);
    panels::drill_in(app, Kind::Tasks);
}

/// The panel takes the keyboard while it is open (the other panels never do).
pub fn on_panel_shown(app: &mut App) {
    let ok = panels::take_keyboard(app);
    // WM_ACTIVATE arrives inside that call, while the App is borrowed, so read the result.
    app.tasks.focused = unsafe { GetForegroundWindow() } == app.panel_hwnd;
    log_line(
        app,
        &format!("tasks: panel takes the keyboard (foreground={ok})"),
    );
}

pub fn on_panel_hidden(app: &mut App) {
    app.tasks.focused = false;
    app.tasks.notice = None;
    panels::release_keyboard(app);
}

pub fn on_activate(app: &mut App, active: bool) {
    if app.tasks.focused != active {
        app.tasks.focused = active;
        panels::refresh(app);
    }
}

/// Esc clears the field (or leaves "New list") before it closes the panel.
pub fn on_esc(app: &mut App) {
    if app.tasks.mode == FieldMode::NewList || !app.tasks.field.is_empty() {
        app.tasks.mode = FieldMode::Task;
        app.tasks.field.clear();
        app.tasks.notice = None;
        panels::refresh(app);
    } else {
        panels::back_or_close(app);
    }
}

fn task_row(t: &Tasks, task: &Task, c: &Clock, smart: bool) -> RowView {
    let mut sub = Vec::new();
    if smart {
        sub.push(m::list_name(&t.snap, &task.list_id));
    }
    if let Some(line) = task.notes.lines().next().filter(|l| !l.trim().is_empty()) {
        sub.push(line.to_owned());
    }
    if task.created_by != t.me {
        sub.push(format!("Added by {}", t.display_name(&task.created_by)));
    }
    let (due, overdue) = match task.completed_at.filter(|_| task.done) {
        Some(ms) => {
            let (day, minute) = c.day_of_ms(ms);
            let when = if day == c.today {
                m::hhmm(minute)
            } else {
                m::day_phrase(day, c.today)
            };
            let by = task
                .completed_by
                .as_ref()
                .filter(|u| **u != t.me)
                .map(|u| format!(" by {}", t.display_name(u)))
                .unwrap_or_default();
            (format!("Done {when}{by}"), false)
        }
        None => m::due_label(task, c),
    };
    RowView::Task {
        title: task.title.clone(),
        sub: sub.join(" · "),
        due,
        overdue,
        repeat: task.repeat != app_store::Repeat::None && !task.done && !overdue,
        assignee: task.assignee.as_ref().map(|u| {
            t.display_name(u)
                .chars()
                .next()
                .unwrap_or('?')
                .to_uppercase()
                .collect()
        }),
        done: task.done,
    }
}

pub fn prayers_today(app: &App, c: &Clock) -> Vec<(String, u32)> {
    let (y, mo, d) = app.times_day;
    if (y, mo, d) != (c.today.year(), c.today.month() as u32, c.today.day() as u32) {
        return Vec::new();
    }
    let t = &app.times;
    [
        ("Subuh", t.subuh),
        ("Dzuhur", t.dzuhur),
        ("Ashar", t.ashar),
        ("Maghrib", t.maghrib),
        ("Isya", t.isya),
    ]
    .into_iter()
    .map(|(n, mi)| (n.to_string(), mi))
    .collect()
}

/// The rows the view shows for its tab, in the model's terms (index-aligned with the panel's).
fn rows(app: &App, c: &Clock) -> Vec<Row> {
    let t = &app.tasks;
    let (todo, done) = m::view_tasks(&t.snap, &t.sel, &t.inbox, c);
    match (t.tab, &t.sel) {
        (Tab::Todo, Selection::Today) => m::today_rows(todo, &prayers_today(app, c), c),
        (Tab::Todo, _) => todo.into_iter().map(Row::Task).collect(),
        (Tab::Done, _) => done.into_iter().map(Row::Task).collect(),
    }
}

pub fn view(app: &App) -> View {
    let t = &app.tasks;
    let c = clock();
    let side = m::sidebar(&t.snap, &t.inbox, &t.collapsed, &t.sel, &c);
    let (crumb, title) = m::header(&t.snap, &t.sel, &t.inbox, &c);
    let (todo, done) = m::view_tasks(&t.snap, &t.sel, &t.inbox, &c);
    let smart = matches!(
        t.sel,
        Selection::Today | Selection::Upcoming | Selection::Folder(_)
    );
    let rows: Vec<RowView> = rows(app, &c)
        .iter()
        .map(|r| match r {
            Row::Task(task) => task_row(t, task, &c, smart),
            Row::Prayer { name, minute } => RowView::Prayer {
                name: name.clone(),
                time: m::hhmm(*minute),
            },
        })
        .collect();

    let (tokens, understood, placeholder, action) = match t.mode {
        FieldMode::Task => {
            let q = t.parse(&c);
            let line = if t.field.text().trim().is_empty() {
                format!(
                    "Type a task, dates in Indonesian or English (besok 9am, jumat).  {GRAMMAR}"
                )
            } else if q.title.is_empty() {
                format!("Add a title too.  {GRAMMAR}")
            } else {
                format!(
                    "{}  {GRAMMAR}",
                    m::understood(&t.snap, &q, &t.sel, &t.inbox, &c, &t.me)
                )
            };
            (
                q.tokens.iter().map(|k| (k.range.clone(), k.kind)).collect(),
                line,
                "Add a task, e.g. beli galon besok 9am #list".to_string(),
                "Add",
            )
        }
        FieldMode::NewList => {
            let space = m::space_for_new_list(&t.snap, &t.sel)
                .and_then(|s| t.snap.spaces.get(&s))
                .map(|s| s.name.clone())
                .unwrap_or_default();
            (
                Vec::new(),
                format!("Enter creates the list in {space}. Esc cancels."),
                "New list name; Home/Groceries puts it in a folder".to_string(),
                "Create",
            )
        }
    };
    let sel = t.field.selection();
    let field = FieldView {
        text: t.field.text().to_owned(),
        caret: t.field.caret(),
        selection: sel,
        tokens,
        placeholder,
        focused: t.focused,
        action,
    };

    let empty = rows.is_empty().then(|| match (t.tab, &t.sel) {
        (Tab::Done, _) => (
            "Nothing done here yet".to_string(),
            "Ticked tasks move here.".to_string(),
        ),
        (_, Selection::Inbox) => (
            "Inbox is empty".into(),
            "A task with no #list lands here.".into(),
        ),
        (_, Selection::Today) => (
            "Nothing due today".into(),
            "Tasks due today, and overdue ones, show here.".into(),
        ),
        (_, Selection::Upcoming) => (
            "Nothing upcoming".into(),
            "Tasks due after today show here, earliest first.".into(),
        ),
        (_, Selection::Folder(_)) => (
            format!("{title} has no open tasks"),
            "A folder shows the tasks of every list inside it.".into(),
        ),
        (_, Selection::List(_)) => (
            format!("{title} is empty"),
            format!(
                "Add the first task above, or type #{} in quick add from any list.",
                title.to_lowercase().replace(' ', "-")
            ),
        ),
    });

    let done_today: Vec<&Task> = done
        .iter()
        .filter(|x| {
            x.completed_at
                .is_some_and(|ms| c.day_of_ms(ms).0 == c.today)
        })
        .collect();
    let footer = match done_today.first() {
        Some(last) => {
            let who = last
                .completed_by
                .as_ref()
                .map(|u| t.display_name(u))
                .unwrap_or_else(|| "You".into());
            format!(
                "{} done today · last: {who} finished {} at {}",
                done_today.len(),
                last.title,
                c.hhmm_of_ms(last.completed_at.unwrap_or(0))
            )
        }
        None => String::new(),
    };
    let sync = match &t.store_error {
        Some(_) => "Couldn't open app.db, changes aren't saved".to_string(),
        None => "On this PC only · sync is off".to_string(),
    };
    let understood = t.notice.clone().unwrap_or(understood);
    let side_scroll = t.side_scroll.clamp(0.0, tp::side_max_scroll(side.len()));
    View {
        side,
        crumb,
        title,
        field,
        understood,
        tab: t.tab,
        todo_n: todo.len(),
        done_n: done.len(),
        rows,
        scroll: t.scroll,
        side_scroll,
        empty,
        footer,
        sync,
    }
}

pub fn draw_panel(app: &App, dc: &ID2D1DeviceContext, hover: Option<tp::Hit>) {
    let v = view(app);
    tp::draw(&app.g, dc, &app.theme, &app.fonts, &v, hover);
}

pub fn hit(app: &App, x: f32, y: f32) -> Option<tp::Hit> {
    tp::hit(&app.g, &app.fonts, &view(app), x, y)
}

fn toggle_collapsed(app: &mut App, id: &str) {
    if !app.tasks.collapsed.remove(id) {
        app.tasks.collapsed.insert(id.to_owned());
    }
}

fn select(app: &mut App, sel: Selection) {
    app.tasks.sel = sel;
    app.tasks.tab = Tab::Todo;
    app.tasks.scroll = 0.0;
    let t = &app.tasks;
    let side = m::sidebar(&t.snap, &t.inbox, &t.collapsed, &t.sel, &clock());
    app.tasks.side_scroll = tp::reveal_selected(&side, t.side_scroll);
}

pub fn click(app: &mut App, h: tp::Hit) {
    app.tasks.notice = None;
    let c = clock();
    match h {
        tp::Hit::Side(i) | tp::Hit::Chevron(i) => {
            let side = m::sidebar(
                &app.tasks.snap,
                &app.tasks.inbox,
                &app.tasks.collapsed,
                &app.tasks.sel,
                &c,
            );
            let Some(item) = side.get(i) else { return };
            match (&item.key, h) {
                (SideKey::Space(id), _) => toggle_collapsed(app, &id.0.clone()),
                (SideKey::Folder(id), tp::Hit::Chevron(_)) => toggle_collapsed(app, &id.0.clone()),
                (SideKey::Folder(id), _) => select(app, Selection::Folder(id.clone())),
                (SideKey::List(id), _) => select(app, Selection::List(id.clone())),
                (SideKey::Inbox, _) => select(app, Selection::Inbox),
                (SideKey::Today, _) => select(app, Selection::Today),
                (SideKey::Upcoming, _) => select(app, Selection::Upcoming),
            }
        }
        tp::Hit::NewList => {
            app.tasks.mode = FieldMode::NewList;
            app.tasks.field.clear();
        }
        tp::Hit::Share => {
            app.tasks.notice = Some(
                "Sharing a space comes with sync, in a later update. Everything stays on this PC."
                    .into(),
            )
        }
        tp::Hit::Add => return submit(app),
        tp::Hit::AllTasks => return crate::tasks_window::show(app),
        tp::Hit::Field | tp::Hit::Row(_) => {}
        tp::Hit::Tab(t) => {
            app.tasks.tab = t;
            app.tasks.scroll = 0.0;
        }
        tp::Hit::Footer => {
            app.tasks.tab = Tab::Done;
            app.tasks.scroll = 0.0;
        }
        tp::Hit::Check(i) | tp::Hit::Delete(i) => {
            let Some(Row::Task(task)) = rows(app, &c).get(i).cloned() else {
                return;
            };
            let (what, r) = match h {
                tp::Hit::Delete(_) => (
                    "delete the task",
                    app.tasks
                        .store
                        .delete(&EntityRef::Task(task.id.clone()), c.now_ms),
                ),
                _ if task.done => (
                    "reopen the task",
                    app.tasks.store.set_done(&task.id, false, c.now_ms),
                ),
                _ => (
                    "finish the task",
                    m::complete(&mut app.tasks.store, &task, &c),
                ),
            };
            log_line(app, &format!("tasks: {what}: {}", task.title));
            report(app, what, r);
            return commit(app);
        }
    }
    panels::refresh(app);
}

/// A press in the field puts the caret there (Shift extends the selection).
pub fn press_field(app: &mut App, x: f32) {
    let v = view(app);
    let b = tp::field_byte_at(&app.g, &app.fonts, &v, x);
    let shift = unsafe { GetKeyState(VK_SHIFT.0 as i32) } < 0;
    app.tasks.field.place(b, shift);
    panels::refresh(app);
}

fn submit(app: &mut App) {
    let c = clock();
    let text = app.tasks.field.text().trim().to_owned();
    if text.is_empty() {
        return;
    }
    match app.tasks.mode {
        FieldMode::Task => {
            let q = app.tasks.parse(&c);
            if q.title.is_empty() {
                app.tasks.notice = Some("Add a title as well as the date.".into());
                return panels::refresh(app);
            }
            let t = &app.tasks;
            let list = m::target_list(&t.snap, &q, &t.sel, &t.inbox);
            let new = m::new_task(&t.snap, &q, &t.sel, &list, &c);
            let r = app
                .tasks
                .store
                .create_task(&list, new, c.now_ms)
                .map(|_| ());
            log_line(
                app,
                &format!(
                    "tasks: added \"{}\" to {}",
                    q.title,
                    m::list_name(&app.tasks.snap, &list)
                ),
            );
            report(app, "add the task", r);
            app.tasks.field.clear();
        }
        FieldMode::NewList => {
            let t = &app.tasks;
            let Some(space) = m::space_for_new_list(&t.snap, &t.sel) else {
                return;
            };
            let snap = t.snap.clone();
            match m::create_list_at(&mut app.tasks.store, &snap, &space, &text, c.now_ms) {
                Ok(Some(list)) => {
                    log_line(app, &format!("tasks: new list {text}"));
                    app.tasks.mode = FieldMode::Task;
                    app.tasks.field.clear();
                    app.tasks.store.snapshot().map(|s| app.tasks.snap = s).ok();
                    select(app, Selection::List(list));
                }
                Ok(None) => {}
                Err(e) => report(app, "create the list", Err(e)),
            }
        }
    }
    commit(app);
}

fn paste(app: &mut App) {
    if let Some(s) = read_clipboard(app.panel_hwnd) {
        app.tasks.field.insert(&s);
    }
}

/// The clipboard's text (CF_UNICODETEXT), up to 4096 characters.
pub fn read_clipboard(owner: HWND) -> Option<String> {
    unsafe {
        if OpenClipboard(Some(owner)).is_err() {
            return None;
        }
        let mut out = None;
        if let Ok(h) = GetClipboardData(13) {
            let hg = windows::Win32::Foundation::HGLOBAL(h.0);
            let p = GlobalLock(hg) as *const u16;
            if !p.is_null() {
                let mut n = 0;
                while *p.add(n) != 0 && n < 4096 {
                    n += 1;
                }
                out = Some(String::from_utf16_lossy(std::slice::from_raw_parts(p, n)));
                let _ = GlobalUnlock(hg);
            }
        }
        let _ = CloseClipboard();
        out
    }
}

/// WM_KEYDOWN on the panel: editing keys, Enter adds, Ctrl+A and Ctrl+V.
pub fn on_keydown(app: &mut App, vk: u16) -> bool {
    let ctrl = unsafe { GetKeyState(VK_CONTROL.0 as i32) } < 0;
    let shift = unsafe { GetKeyState(VK_SHIFT.0 as i32) } < 0;
    let key = match VIRTUAL_KEY(vk) {
        VK_LEFT => Some(Key::Left),
        VK_RIGHT => Some(Key::Right),
        VK_HOME => Some(Key::Home),
        VK_END => Some(Key::End),
        VK_BACK => Some(Key::Backspace),
        VK_DELETE => Some(Key::Delete),
        _ => None,
    };
    if let Some(k) = key {
        app.tasks.field.key(k, ctrl, shift);
    } else {
        match VIRTUAL_KEY(vk) {
            VK_RETURN => {
                submit(app);
                return true;
            }
            VK_ESCAPE => {
                on_esc(app);
                return true;
            }
            VIRTUAL_KEY(0x41) if ctrl => app.tasks.field.select_all(),
            VIRTUAL_KEY(0x56) if ctrl => paste(app),
            _ => return false,
        }
    }
    app.tasks.notice = None;
    panels::refresh(app);
    true
}

/// WM_CHAR on the panel.
pub fn on_char(app: &mut App, unit: u16) {
    if app.tasks.field.type_utf16(unit) {
        app.tasks.notice = None;
        panels::refresh(app);
    }
}

/// The wheel scrolls whichever side it is over: the sidebar's tree or the rows.
pub fn on_wheel(app: &mut App, x: f32, delta: i16) {
    let v = view(app);
    let step = delta as f32 / 120.0 * 48.0;
    let (cur, max) = if x < tp::SIDE {
        (v.side_scroll, tp::side_max_scroll(v.side.len()))
    } else {
        (
            app.tasks.scroll,
            (tp::content_height(&v) + 8.0 - (tp::ROWS_B - tp::ROWS_T)).max(0.0),
        )
    };
    let s = (cur - step).clamp(0.0, max);
    if s != cur {
        if x < tp::SIDE {
            app.tasks.side_scroll = s;
        } else {
            app.tasks.scroll = s;
        }
        panels::refresh(app);
    }
}
