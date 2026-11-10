//! Fixed Drink, Focus and History screens for the snapshot mode: an empty day, a partial day and a
//! met goal with a full log; Focus set up, running, paused (by you and for prayer) and on a break;
//! History's month and year on both tabs, the day detail above a month square and beside a year
//! square, and the empty, first-week and loading layouts. Hand-made data on 10 Nov 2026, no
//! clock, so Windows and macOS draw the same views and a change to the drawing can be checked
//! pixel for pixel against the last run. History is drink and focus only, never prayer.

use app_habits::Date;

use crate::drink_panel::{self as dp, Bar, LogRow};
use crate::focus_panel::{self as fp, Run};
use crate::history_model::{self as hm, Ask, DrinkIn, Log, SessionIn, Span, Tab};
use crate::history_panel as hp;
use crate::snapshot::Scene;
use crate::ui::Fonts;

const GOAL: u32 = 2000;

fn today() -> Date {
    Date::new(2026, 11, 10).unwrap()
}

// ---- Drink ----

fn week(today_ml: u32) -> Vec<Bar> {
    [
        ("Wed", 2100),
        ("Thu", 1800),
        ("Fri", 2250),
        ("Sat", 1250),
        ("Sun", 2000),
        ("Mon", 1650),
    ]
    .iter()
    .map(|(d, ml)| Bar {
        label: (*d).into(),
        ml: *ml,
        today: false,
    })
    .chain(std::iter::once(Bar {
        label: "Today".into(),
        ml: today_ml,
        today: true,
    }))
    .collect()
}

fn log_row(time: &str, ml: u32, source: &str) -> LogRow {
    LogRow {
        time: time.into(),
        ml,
        source: source.into(),
    }
}

fn drink_view(total: u32, rows: Vec<LogRow>, to_go: &str, pace: &str, next: &str) -> dp::View {
    dp::View {
        total,
        goal: GOAL,
        to_go: to_go.into(),
        pace: pace.into(),
        glasses: vec![150, 250, 500],
        default_glass: 250,
        rows,
        week: week(total),
        next: next.into(),
    }
}

fn drink_partial() -> dp::View {
    drink_view(
        750,
        vec![
            log_row("14:20", 250, "glass"),
            log_row("11:05", 250, "from your phone"),
            log_row("09:12", 250, "glass"),
        ],
        "1250 ml to go.",
        "On pace would be 1100 ml by now.",
        "Next reminder 14:45 · every 40 min",
    )
}

/// The Drink panel: (name, view, hover).
pub fn drink_cases() -> Vec<(&'static str, dp::View, Option<dp::Hit>)> {
    let empty = drink_view(
        0,
        Vec::new(),
        "2000 ml to go.",
        "On pace would be 1100 ml by now.",
        "Next reminder 14:45 · every 20 min",
    );
    let met = drink_view(
        2250,
        vec![
            log_row("21:40", 500, "glass"),
            log_row("19:02", 250, "glass"),
            log_row("16:48", 150, "from your phone"),
            log_row("14:20", 350, "glass"),
            log_row("11:05", 500, "glass"),
            log_row("09:12", 500, "glass"),
        ],
        "Goal met for today.",
        "",
        "No more reminders today",
    );
    let away = dp::View {
        next: "Reminders wait while you are away".into(),
        ..drink_partial()
    };
    vec![
        ("drink-empty", empty, None),
        ("drink-partial", drink_partial(), None),
        ("drink-goal-met", met, None),
        ("drink-hover-undo", drink_partial(), Some(dp::Hit::Undo)),
        (
            "drink-hover-glass",
            drink_partial(),
            Some(dp::Hit::Glass(150)),
        ),
        (
            "drink-hover-default-glass",
            drink_partial(),
            Some(dp::Hit::Glass(250)),
        ),
        ("drink-hover-week", away, Some(dp::Hit::Week)),
    ]
}

// ---- Focus ----

fn running(run: Run, left: u64, length: u64, session: u32, done: u32, held: usize) -> fp::View {
    fp::View::Running {
        run,
        left_secs: left,
        length_secs: length,
        progress: 1.0 - left as f32 / length as f32,
        session,
        of: 4,
        done,
        held,
        task: None,
    }
}

/// The Focus panel: (name, view, hover).
pub fn focus_cases() -> Vec<(&'static str, fp::View, Option<fp::Hit>)> {
    let setup = fp::View::Setup {
        minutes: 25,
        short_break: 5,
        long_break: 15,
        every: 4,
        auto_start: false,
        task: None,
    };
    let custom = fp::View::Setup {
        minutes: 40,
        short_break: 5,
        long_break: 15,
        every: 4,
        auto_start: true,
        task: Some("Draft the release notes".into()),
    };
    let with_task = match running(Run::Focus, 1112, 1500, 2, 1, 3) {
        fp::View::Running {
            run,
            left_secs,
            length_secs,
            progress,
            session,
            of,
            done,
            held,
            ..
        } => fp::View::Running {
            run,
            left_secs,
            length_secs,
            progress,
            session,
            of,
            done,
            held,
            task: Some("Draft the release notes".into()),
        },
        v => v,
    };
    vec![
        ("focus-setup", setup.clone(), None),
        (
            "focus-setup-hover-start",
            setup.clone(),
            Some(fp::Hit::Start),
        ),
        ("focus-setup-hover-preset", setup, Some(fp::Hit::Preset(50))),
        ("focus-setup-custom", custom, Some(fp::Hit::Breaks)),
        (
            "focus-running",
            running(Run::Focus, 1112, 1500, 2, 1, 0),
            None,
        ),
        ("focus-running-task", with_task, Some(fp::Hit::Primary)),
        (
            "focus-paused",
            running(Run::Paused { prayer: None }, 640, 1500, 3, 2, 1),
            Some(fp::Hit::Skip),
        ),
        (
            "focus-paused-prayer",
            running(
                Run::Paused {
                    prayer: Some("Ashar".into()),
                },
                640,
                1500,
                3,
                2,
                0,
            ),
            None,
        ),
        (
            "focus-break",
            running(Run::Break { long: false }, 212, 300, 2, 2, 2),
            Some(fp::Hit::Stop),
        ),
        (
            "focus-break-long",
            running(Run::Break { long: true }, 845, 900, 4, 4, 0),
            None,
        ),
    ]
}

// ---- History ----

const TASKS: [Option<&str>; 4] = [
    Some("Write the quarterly report for the board meeting next week"),
    Some("Review pull requests"),
    None,
    Some("Design the History panel"),
];

/// About five months of drinks (some from the phone) and three of focus sessions.
fn full_log() -> Log {
    let t = today();
    let mut drinks = Vec::new();
    for i in 0..140i64 {
        let day = hm::add_days(t, -i);
        let ml = if i == 0 {
            750
        } else {
            [2100, 1800, 2250, 900, 2000, 1500, 2600, 0, 1200, 2050, 1750][i as usize % 11]
        };
        let glasses = ml / 250;
        for k in 0..glasses {
            drinks.push(DrinkIn {
                at: day.at(8 + k, 15 + (i as u32 % 4) * 10),
                ml: 250,
                device: (i % 5 == 2 && k == 0).then(|| "phone".to_string()),
            });
        }
    }
    let mut sessions = Vec::new();
    for i in 0..90i64 {
        if i % 7 == 5 {
            continue;
        }
        let day = hm::add_days(t, -i);
        for k in 0..(1 + i as u32 % 4) {
            sessions.push(SessionIn {
                ended: day.at(9 + k, 30),
                minutes: [25, 50, 25, 15][(i as usize + k as usize) % 4],
                task: TASKS[(i as usize + k as usize) % 4].map(str::to_string),
            });
        }
    }
    Log::new(&drinks, &sessions, GOAL)
}

/// Three days since the first drink.
fn first_week_log() -> Log {
    let t = today();
    let drinks: Vec<DrinkIn> = (0..3i64)
        .flat_map(|i| {
            (0..(4 + i as u32)).map(move |k| DrinkIn {
                at: hm::add_days(t, -i).at(9 + k, 0),
                ml: 250,
                device: None,
            })
        })
        .collect();
    Log::new(&drinks, &[], GOAL)
}

fn ask(tab: Tab, span: Span) -> Ask {
    Ask {
        tab,
        span,
        anchor: hm::this_anchor(today()),
        today: today(),
        focus: None,
        loading: false,
        glass_ml: 250,
        focus_min: 25,
    }
}

pub struct HistoryCase {
    pub name: &'static str,
    pub view: hm::View,
    pub detail: Option<(usize, hm::Detail)>,
    pub hover: Option<hp::Hit>,
}

/// The view, and the day detail the way history.rs picks it: the pointer's day, else the
/// keyboard's.
fn case(name: &'static str, log: &Log, ask: &Ask, hover: Option<hp::Hit>) -> HistoryCase {
    let v = hm::view(log, ask);
    let shown = match hover {
        Some(hp::Hit::Day(i)) => Some(i),
        _ => v.focus,
    };
    let detail = shown.and_then(|i| {
        let c = v.cells.get(i)?;
        let pace = if c.today && v.tab == Tab::Drink {
            "On pace would be 1100 ml by now."
        } else {
            ""
        };
        let span = if v.body.compact() {
            Span::Month
        } else {
            v.span
        };
        hm::detail(log, v.tab, span, c, pace).map(|d| (i, d))
    });
    HistoryCase {
        name,
        view: v,
        detail,
        hover,
    }
}

fn index_of(log: &Log, a: &Ask, d: Date) -> usize {
    hm::view(log, a)
        .cells
        .iter()
        .position(|c| c.date == d)
        .unwrap_or(0)
}

/// The History panel.
pub fn history_cases() -> Vec<HistoryCase> {
    let log = full_log();
    let t = today();
    let mut out = Vec::new();
    out.push(case(
        "history-drink-month",
        &log,
        &ask(Tab::Drink, Span::Month),
        None,
    ));
    // The keyboard's day: its ring and its detail above the square.
    let mut a = ask(Tab::Drink, Span::Month);
    a.focus = Some(hm::add_days(t, -9));
    out.push(case("history-drink-month-detail", &log, &a, None));
    // The pointer on today: the detail carries the Drink panel's pace.
    let a = ask(Tab::Drink, Span::Month);
    let i = index_of(&log, &a, t);
    out.push(case(
        "history-drink-month-hover-today",
        &log,
        &a,
        Some(hp::Hit::Day(i)),
    ));
    // A page back: Next is on, and hovered.
    let mut a = ask(Tab::Drink, Span::Month);
    a.anchor = hm::add_days(a.anchor, -35);
    out.push(case(
        "history-drink-month-prev",
        &log,
        &a,
        Some(hp::Hit::Next),
    ));
    out.push(case(
        "history-focus-month",
        &log,
        &ask(Tab::Focus, Span::Month),
        Some(hp::Hit::Span(Span::Year)),
    ));
    out.push(case(
        "history-drink-year",
        &log,
        &ask(Tab::Drink, Span::Year),
        Some(hp::Hit::Prev),
    ));
    // Beside a year square: to its left near the end, to its right near the start.
    let a = ask(Tab::Focus, Span::Year);
    let i = index_of(&log, &a, hm::add_days(t, -20));
    out.push(case(
        "history-focus-year-detail",
        &log,
        &a,
        Some(hp::Hit::Day(i)),
    ));
    let a = ask(Tab::Drink, Span::Year);
    let i = index_of(&log, &a, hm::add_days(t, -340));
    out.push(case(
        "history-drink-year-detail-early",
        &log,
        &a,
        Some(hp::Hit::Day(i)),
    ));
    let empty = Log::new(&[], &[], GOAL);
    out.push(case(
        "history-empty",
        &empty,
        &ask(Tab::Drink, Span::Month),
        Some(hp::Hit::Action),
    ));
    out.push(case(
        "history-empty-focus",
        &empty,
        &ask(Tab::Focus, Span::Year),
        Some(hp::Hit::Tab(Tab::Drink)),
    ));
    let fw = first_week_log();
    let a = ask(Tab::Drink, Span::Month);
    let i = index_of(&fw, &a, hm::add_days(t, -1));
    out.push(case("history-first-week", &fw, &a, Some(hp::Hit::Day(i))));
    let mut a = ask(Tab::Drink, Span::Month);
    a.loading = true;
    out.push(case("history-loading", &log, &a, None));
    out
}

pub fn scenes(_f: &Fonts) -> Vec<Scene> {
    let mut out = Vec::new();
    for (name, v, hover) in drink_cases() {
        out.push(Scene::new(name, dp::W, dp::height(&v), move |cv, t, f| {
            dp::draw(cv, t, f, &v, hover)
        }));
    }
    for (name, v, hover) in focus_cases() {
        out.push(Scene::new(name, fp::W, fp::height(&v), move |cv, t, f| {
            fp::draw(cv, t, f, &v, hover)
        }));
    }
    for c in history_cases() {
        let (w, h) = hp::size(&c.view);
        out.push(Scene::new(c.name, w, h, move |cv, t, f| {
            hp::draw(cv, t, f, &c.view, c.detail.clone(), c.hover)
        }));
    }
    out
}
