//! The History panel on the orb: which tab, span and page show, the keyboard's day, and the
//! glue from the drink log and focus sessions (habits.rs) to `history_model`. Prayer is never
//! part of it.

use app_habits::drink::Source;
use app_habits::Date;

use crate::canvas::Canvas;
use crate::history_model::{self as m, Ask, DrinkIn, Log, SessionIn, Span, Tab, View};
use crate::history_panel::{self, Hit};
use crate::panels::{self, Kind};
use crate::sys::vk::*;
use crate::ui::{DialChild, Icon};
use crate::{habits, log_line, store, App};

pub struct History {
    /// The last tab used; the dial's History opens on it.
    pub tab: Tab,
    pub span: Span,
    /// The Sunday that ends the page; `None` follows this week, across midnight too.
    anchor: Option<Date>,
    /// The day the arrow keys are on.
    focus: Option<Date>,
    /// Days still arriving. Only the tray's test item sets it until sync exists.
    loading: bool,
}

impl History {
    pub fn new() -> History {
        History {
            tab: Tab::Drink,
            span: Span::Month,
            anchor: None,
            focus: None,
            loading: false,
        }
    }
}

fn today() -> Date {
    habits::now().date()
}

/// The drink log and the finished sessions, by day. A session's task id becomes its name when
/// the task is still in the store.
fn log(app: &App) -> Log {
    let drinks: Vec<DrinkIn> = app
        .habits
        .drink
        .entries()
        .iter()
        .map(|e| DrinkIn {
            at: e.at,
            ml: e.ml,
            device: match &e.source {
                Source::Local => None,
                Source::Device(d) => Some(d.clone()),
            },
        })
        .collect();
    let sessions: Vec<SessionIn> = app
        .habits
        .sessions
        .iter()
        .map(|s| SessionIn {
            ended: store::local(s.ended),
            minutes: s.minutes,
            task: s
                .task
                .as_ref()
                .map(|id| task_title(app, id).unwrap_or_else(|| id.clone())),
        })
        .collect();
    Log::new(&drinks, &sessions, app.habits.drink.config().goal_ml)
}

/// A task's title while it is still in the store.
#[cfg(windows)]
fn task_title(app: &App, id: &str) -> Option<String> {
    app.tasks
        .snap
        .tasks
        .get(&app_store::TaskId(id.to_string()))
        .map(|t| t.title.clone())
}

/// The Mac has no task store until tasks.rs is shared (the tasks stage drops this cfg pair):
/// the session shows its task id.
#[cfg(not(windows))]
fn task_title(_app: &App, _id: &str) -> Option<String> {
    None
}

fn ask(app: &App) -> Ask {
    let h = &app.history;
    let t = today();
    Ask {
        tab: h.tab,
        span: h.span,
        anchor: h.anchor.unwrap_or(m::this_anchor(t)),
        today: t,
        focus: h.focus,
        loading: h.loading,
        glass_ml: app.habits.drink.config().default_glass_ml,
        focus_min: app.habits.prefs.length_min,
    }
}

pub fn view(app: &App) -> View {
    m::view(&log(app), &ask(app))
}

pub fn size(app: &App) -> (f32, f32) {
    history_panel::size(&view(app))
}

pub fn hit(app: &App, x: f32, y: f32) -> Option<Hit> {
    history_panel::hit(&app.fonts, &view(app), x, y)
}

pub fn draw_panel(app: &App, cv: &mut dyn Canvas, hover: Option<Hit>) {
    let log = log(app);
    let v = m::view(&log, &ask(app));
    // The pointer's day wins over the keyboard's.
    let shown = match hover {
        Some(Hit::Day(i)) => Some(i),
        _ => v.focus,
    };
    let detail = shown.and_then(|i| {
        let c = v.cells.get(i)?;
        let pace = if c.today && v.tab == Tab::Drink {
            habits::drink_view(app).pace
        } else {
            String::new()
        };
        let span = if v.body.compact() {
            Span::Month
        } else {
            v.span
        };
        m::detail(&log, v.tab, span, c, &pace).map(|d| (i, d))
    });
    history_panel::draw(cv, &app.theme, &app.fonts, &v, detail, hover);
}

/// Opens History, on `tab` or the last tab used, on this week's page.
pub fn open(app: &mut App, tab: Option<Tab>) {
    if let Some(t) = tab {
        app.history.tab = t;
    }
    app.history.anchor = None;
    app.history.focus = None;
    panels::open_panel(app, Kind::History);
    // Already open (another tab from the tray): show the change.
    panels::refresh(app);
    log_line(
        app,
        &format!("history: {:?} {:?}", app.history.tab, app.history.span),
    );
}

/// The panel's timer: today's square moves on at midnight.
pub fn ms_to_midnight() -> u32 {
    let n = habits::now();
    habits::ms_until(n, n.date().next().at_minute(0))
}

/// Tray test item: the loading layout (sync is not in this wave, so nothing else reaches it).
pub fn test_loading(app: &mut App) {
    app.history.loading = true;
    open(app, None);
}

pub fn on_closed(app: &mut App) {
    app.history.loading = false;
}

pub fn dial_child(app: &App) -> DialChild {
    let value = match app.history.tab {
        Tab::Drink => "Drink",
        Tab::Focus => "Focus",
    };
    DialChild {
        text: String::new(),
        icon: Some(Icon::Grid),
        name: "History".into(),
        value: value.into(),
    }
}

fn set_tab(app: &mut App, t: Tab) {
    app.history.tab = t;
    app.history.loading = false;
    panels::refresh(app);
}

/// Back or forward one page; the keyboard's day stays only if it is still on screen.
fn page(app: &mut App, dir: i64) {
    let t = today();
    let now = m::this_anchor(t);
    let span = app.history.span;
    let a = m::add_days(app.history.anchor.unwrap_or(now), dir * span.step()).min(now);
    app.history.anchor = (a != now).then_some(a);
    let from = m::add_days(a, -(span.days() - 1));
    if app.history.focus.is_some_and(|d| d < from || d > a) {
        app.history.focus = None;
    }
    panels::refresh(app);
}

pub fn click(app: &mut App, hit: Hit) {
    let v = view(app);
    match hit {
        Hit::Tab(t) => set_tab(app, t),
        Hit::Span(s) if s != app.history.span => {
            app.history.span = s;
            app.history.anchor = None;
            if let Some(d) = app.history.focus {
                move_to(app, d);
            }
            panels::refresh(app);
        }
        Hit::Span(_) => {}
        Hit::Prev if v.can_prev => page(app, -1),
        Hit::Next if v.can_next => page(app, 1),
        Hit::Prev | Hit::Next => {}
        Hit::Day(i) => {
            if let Some(c) = v.cells.get(i).filter(|c| c.date <= today()) {
                app.history.focus = Some(c.date);
                panels::refresh(app);
            }
        }
        Hit::Action => match app.history.tab {
            Tab::Drink => {
                let ml = app.habits.drink.config().default_glass_ml;
                let _ = habits::log_drink(app, ml);
            }
            Tab::Focus => {
                let mins = app.habits.prefs.length_min;
                habits::start_focus(app, mins);
                panels::open_panel(app, Kind::Focus);
            }
        },
    }
}

/// Puts the keyboard's day on `d` (never past today), paging so it shows.
fn move_to(app: &mut App, d: Date) {
    let t = today();
    let d = d.min(t);
    app.history.focus = Some(d);
    let span = app.history.span;
    let now = m::this_anchor(t);
    let a = m::anchor_for(d, span, app.history.anchor.unwrap_or(now), t);
    app.history.anchor = (a != now).then_some(a);
}

/// Arrows a day or a week, Page Up and Down a month, Home today, Tab the other tab.
pub fn on_keydown(app: &mut App, vk: u16) -> bool {
    let t = today();
    let v = view(app);
    let from = v.focus.and_then(|i| v.cells.get(i)).map(|c| c.date);
    let cur = from.or(app.history.focus).unwrap_or(t);
    let target = match vk {
        VK_LEFT => m::add_days(cur, -1),
        VK_RIGHT => m::add_days(cur, 1),
        VK_UP => m::add_days(cur, -7),
        VK_DOWN => m::add_days(cur, 7),
        VK_PRIOR => m::shift_month(cur, -1),
        VK_NEXT => m::shift_month(cur, 1),
        VK_HOME => t,
        VK_TAB => {
            let other = if app.history.tab == Tab::Drink {
                Tab::Focus
            } else {
                Tab::Drink
            };
            set_tab(app, other);
            return true;
        }
        _ => return false,
    };
    // The first key only shows where it is.
    let target = if app.history.focus.is_none() && vk != VK_HOME {
        cur
    } else {
        target
    };
    if v.body.compact() {
        // The small grid is this month only.
        let (first, last) = (v.cells[0].date, t);
        app.history.focus = Some(target.clamp(first, last));
    } else {
        // Back as far as the week the log starts in (or what already shows).
        let lower = log(app).started.map_or(t, m::monday).min(v.cells[0].date);
        move_to(app, target.max(lower));
    }
    panels::refresh(app);
    true
}
