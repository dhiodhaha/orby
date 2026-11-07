//! History (frames "v6.2 · 23" Bm7GaZ8U3r and "v6.2 · 24" s1JLm1o0lZ): drink and focus by day,
//! never prayer. Pure: the days, the five levels, the ranges, the summary numbers and every word
//! on the panel. `history_panel.rs` draws what this decides.

use std::collections::BTreeMap;

use app_habits::{Date, Moment};

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Tab {
    Drink,
    Focus,
}

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Span {
    Month,
    Year,
}

impl Span {
    /// Days on screen: 5 weeks, or 53 weeks so a whole year fits with today's week.
    pub fn days(self) -> i64 {
        match self {
            Span::Month => 35,
            Span::Year => 371,
        }
    }

    /// Back or forward one page: 5 weeks, or 52 weeks so a year back lands on the same week.
    pub fn step(self) -> i64 {
        match self {
            Span::Month => 35,
            Span::Year => 364,
        }
    }
}

/// One drink, as History needs it.
pub struct DrinkIn {
    pub at: Moment,
    pub ml: u32,
    /// `Some("phone")` for a drink logged on another device.
    pub device: Option<String>,
}

/// One finished focus session; `task` is already the task's name.
pub struct SessionIn {
    pub ended: Moment,
    pub minutes: u32,
    pub task: Option<String>,
}

#[derive(Clone, Debug, Default, PartialEq)]
pub struct Day {
    pub ml: u32,
    pub drinks: u32,
    pub from_device: u32,
    pub device: Option<String>,
    pub last_drink: Option<Moment>,
    pub focus_min: u32,
    pub sessions: u32,
    /// Minutes per task (`None` is "No task"), in first-seen order.
    pub tasks: Vec<(Option<String>, u32)>,
}

/// The whole log by day. `started` is the first day anything was logged, so days before it are
/// outlines ("before you started"), never "nothing logged".
pub struct Log {
    days: BTreeMap<Date, Day>,
    pub goal: u32,
    pub started: Option<Date>,
}

impl Log {
    pub fn new(drinks: &[DrinkIn], sessions: &[SessionIn], goal: u32) -> Log {
        let mut days: BTreeMap<Date, Day> = BTreeMap::new();
        for d in drinks {
            let day = days.entry(d.at.date()).or_default();
            day.ml += d.ml;
            day.drinks += 1;
            if let Some(dev) = &d.device {
                day.from_device += 1;
                day.device.get_or_insert_with(|| dev.clone());
            }
            day.last_drink = Some(day.last_drink.map_or(d.at, |l| l.max(d.at)));
        }
        for s in sessions {
            let day = days.entry(s.ended.date()).or_default();
            day.focus_min += s.minutes;
            day.sessions += 1;
            match day.tasks.iter_mut().find(|(t, _)| *t == s.task) {
                Some((_, m)) => *m += s.minutes,
                None => day.tasks.push((s.task.clone(), s.minutes)),
            }
        }
        let started = days.keys().next().copied();
        Log {
            days,
            goal: goal.max(1),
            started,
        }
    }

    pub fn day(&self, d: Date) -> Option<&Day> {
        self.days.get(&d)
    }

    pub fn has(&self, tab: Tab) -> bool {
        self.days.values().any(|d| amount(tab, d) > 0)
    }

    fn total(&self, tab: Tab, from: Date, to: Date) -> u32 {
        self.days
            .range(from..=to)
            .map(|(_, d)| amount(tab, d))
            .sum()
    }
}

fn amount(tab: Tab, d: &Day) -> u32 {
    match tab {
        Tab::Drink => d.ml,
        Tab::Focus => d.focus_min,
    }
}

// ---- dates ----

fn epoch() -> Moment {
    Moment::from_unix(0, 0)
}

fn day_no(d: Date) -> i64 {
    (d.at_minute(0).since(epoch()).as_secs() / 86_400) as i64
}

pub fn add_days(d: Date, n: i64) -> Date {
    Moment::from_unix((day_no(d) + n) * 86_400, 0).date()
}

pub fn days_between(from: Date, to: Date) -> i64 {
    day_no(to) - day_no(from)
}

/// Monday is 0.
pub fn weekday(d: Date) -> usize {
    (day_no(d) + 3).rem_euclid(7) as usize
}

pub fn monday(d: Date) -> Date {
    add_days(d, -(weekday(d) as i64))
}

pub fn sunday(d: Date) -> Date {
    add_days(monday(d), 6)
}

/// The same day `k` months away, held to the month's last day (31 Mar back one is 28 Feb).
pub fn shift_month(d: Date, k: i32) -> Date {
    let m0 = d.year() * 12 + d.month() as i32 - 1 + k;
    let (y, m) = (m0.div_euclid(12), m0.rem_euclid(12) as u32 + 1);
    (1..=d.day())
        .rev()
        .find_map(|day| Date::new(y, m, day))
        .unwrap_or(d)
}

const MONTHS: [&str; 12] = [
    "Jan", "Feb", "Mar", "Apr", "May", "Jun", "Jul", "Aug", "Sep", "Oct", "Nov", "Dec",
];
const WEEKDAYS: [&str; 7] = ["Mon", "Tue", "Wed", "Thu", "Fri", "Sat", "Sun"];

pub fn month_name(d: Date) -> &'static str {
    MONTHS[d.month() as usize - 1]
}

/// "5 Oct".
pub fn d_mon(d: Date) -> String {
    format!("{} {}", d.day(), month_name(d))
}

/// "Thu 5 Nov".
pub fn wd_d_mon(d: Date) -> String {
    format!("{} {}", WEEKDAYS[weekday(d)], d_mon(d))
}

fn hhmm(m: Moment) -> String {
    let t = m.minute_of_day();
    format!("{:02}:{:02}", t / 60, t % 60)
}

// ---- words and numbers ----

/// A number with small units between its digits: ("6", false), ("h", true), ("40", false)...
pub type Parts = Vec<(String, bool)>;

pub fn minutes_parts(m: u32) -> Parts {
    let (h, mm) = (m / 60, m % 60);
    match (h, mm) {
        (0, mm) => vec![(mm.to_string(), false), ("min".into(), true)],
        (h, 0) => vec![(h.to_string(), false), ("h".into(), true)],
        (h, mm) => vec![
            (h.to_string(), false),
            ("h".into(), true),
            (mm.to_string(), false),
            ("min".into(), true),
        ],
    }
}

/// "1 h 42 min", "37 min", "2 h".
pub fn minutes_text(m: u32) -> String {
    minutes_parts(m)
        .into_iter()
        .map(|(s, _)| s)
        .collect::<Vec<_>>()
        .join(" ")
}

fn litres(ml: u32) -> String {
    format!("{:.1}", ml as f64 / 1000.0)
}

fn plural(n: u32, one: &str, many: &str) -> String {
    format!("{n} {}", if n == 1 { one } else { many })
}

fn count_word(n: i64) -> String {
    ["one", "two", "three", "four", "five", "six"]
        .get((n - 1).max(0) as usize)
        .map_or_else(|| n.to_string(), |w| w.to_string())
}

// ---- levels ----

/// Five levels, the same greys for both tabs. Drink is the share of the goal; focus is time.
pub fn level(tab: Tab, d: Option<&Day>, goal: u32) -> u8 {
    let Some(d) = d else { return 0 };
    match tab {
        Tab::Drink => match d.ml {
            0 => 0,
            ml if ml * 2 < goal => 1,
            ml if ml * 4 < goal * 3 => 2,
            ml if ml < goal => 3,
            _ => 4,
        },
        Tab::Focus => match d.focus_min {
            0 => 0,
            m if m < 30 => 1,
            m if m < 60 => 2,
            m if m < 120 => 3,
            _ => 4,
        },
    }
}

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Cell {
    /// Before the first day anything was logged: an outline.
    Before,
    Level(u8),
    /// Still ahead: a dotted outline.
    Ahead,
    /// A day still loading: a quiet block (two tones so the grid does not read as one slab).
    Loading(bool),
}

#[derive(Clone, Debug, PartialEq)]
pub struct CellView {
    pub date: Date,
    pub kind: Cell,
    pub today: bool,
}

#[derive(Clone, Debug, PartialEq)]
pub struct Stat {
    pub value: Parts,
    pub key: String,
    pub sub: String,
}

#[derive(Clone, Debug, PartialEq)]
pub struct TaskRow {
    /// `None` is "No task".
    pub name: Option<String>,
    pub minutes: u32,
    /// Against the biggest row, 0 to 1.
    pub share: f32,
}

#[derive(Clone, Debug, PartialEq)]
pub struct ByTask {
    pub label: String,
    pub total: String,
    pub rows: Vec<TaskRow>,
}

#[derive(Clone, Debug, PartialEq)]
pub enum Body {
    /// Days are still arriving (sync); this PC's days show, the rest are quiet blocks.
    Loading { done: u32, total: u32 },
    /// Nothing logged on this tab yet.
    Empty {
        title: String,
        text: String,
        action: String,
    },
    /// Under 7 days since the first log: one total, no averages.
    FirstWeek {
        value: Parts,
        since: String,
        note: String,
    },
    Full {
        stats: Vec<Stat>,
        by_task: Option<ByTask>,
    },
}

impl Body {
    /// The small layout (text beside a small month grid, no range controls).
    pub fn compact(&self) -> bool {
        !matches!(self, Body::Full { .. })
    }
}

/// The day detail, on hover or the keyboard.
#[derive(Clone, Debug, PartialEq)]
pub struct Detail {
    pub title: String,
    pub value: String,
    /// After the value, muted: "/ 2000 ml", "· goal met", "· 4 sessions".
    pub note: String,
    /// The whole value is muted ("Nothing logged").
    pub quiet: bool,
    pub line: String,
    /// Task splits: name, time.
    pub rows: Vec<(String, String)>,
}

#[derive(Clone, Debug, PartialEq)]
pub struct View {
    pub tab: Tab,
    pub span: Span,
    pub range: String,
    pub can_prev: bool,
    pub can_next: bool,
    pub body: Body,
    pub cells: Vec<CellView>,
    /// Year view: (week column, month name) along the top.
    pub months: Vec<(usize, &'static str)>,
    pub legend: String,
    pub legend_top: &'static str,
    /// The keyboard's day, as an index into `cells`.
    pub focus: Option<usize>,
}

/// What the panel asks for. `anchor` is the Sunday that ends the page.
pub struct Ask {
    pub tab: Tab,
    pub span: Span,
    pub anchor: Date,
    pub today: Date,
    pub focus: Option<Date>,
    /// While days are still arriving.
    pub loading: bool,
    /// The empty state's one action: the default glass, or the focus length.
    pub glass_ml: u32,
    pub focus_min: u32,
}

pub fn this_anchor(today: Date) -> Date {
    sunday(today)
}

/// The anchor that keeps `d` on screen: unchanged if it already shows, else the page that brings
/// it in from the side it left by (never past this week).
pub fn anchor_for(d: Date, span: Span, anchor: Date, today: Date) -> Date {
    let n = span.days();
    let start = add_days(anchor, -(n - 1));
    let a = if d > anchor {
        sunday(d)
    } else if d < start {
        add_days(monday(d), n - 1)
    } else {
        anchor
    };
    a.min(this_anchor(today))
}

fn started(log: &Log, today: Date) -> Date {
    log.started.unwrap_or(today).min(today)
}

fn range_label(span: Span, from: Date, to: Date, today: Date) -> String {
    match span {
        Span::Month if to.year() != today.year() => {
            format!("{} to {} {}", d_mon(from), d_mon(to), to.year())
        }
        Span::Month => format!("{} to {}", d_mon(from), d_mon(to)),
        Span::Year => format!(
            "{} {} to {} {}",
            month_name(from),
            from.year(),
            month_name(to),
            to.year()
        ),
    }
}

pub fn view(log: &Log, ask: &Ask) -> View {
    let (tab, span, today) = (ask.tab, ask.span, ask.today);
    let anchor = ask.anchor.min(this_anchor(today));
    let current = anchor == this_anchor(today);
    let from = add_days(anchor, -(span.days() - 1));
    let start = started(log, today);
    let body = if ask.loading {
        Body::Loading { done: 0, total: 0 }
    } else if !log.has(tab) {
        empty(tab, ask)
    } else if days_between(start, today) < 6 {
        first_week(log, tab, start, today)
    } else {
        Body::Full {
            stats: stats(log, tab, span, from, anchor, start, today, current),
            by_task: (tab == Tab::Focus && span == Span::Month)
                .then(|| by_task(log, from, anchor, start, today, current))
                .flatten(),
        }
    };
    // The small layout always shows this month's grid.
    let (span, from, anchor) = if body.compact() {
        (
            Span::Month,
            add_days(this_anchor(today), -34),
            this_anchor(today),
        )
    } else {
        (span, from, anchor)
    };
    let mut cells: Vec<CellView> = (0..span.days())
        .map(|i| {
            let date = add_days(from, i);
            let kind = if date > today {
                Cell::Ahead
            } else if date < start {
                Cell::Before
            } else {
                Cell::Level(level(tab, log.day(date), log.goal))
            };
            CellView {
                date,
                kind,
                today: date == today,
            }
        })
        .collect();
    let body = match body {
        Body::Loading { .. } => {
            // This PC's days are here; the others are still on their way.
            let mut done = 0;
            for (i, c) in cells.iter_mut().enumerate() {
                if let Cell::Level(l) = c.kind {
                    if l > 0 || c.today {
                        done += 1;
                    } else {
                        c.kind = Cell::Loading(i % 3 == 0);
                    }
                }
            }
            Body::Loading {
                done,
                total: cells.iter().filter(|c| c.kind != Cell::Ahead).count() as u32,
            }
        }
        b => b,
    };
    let months = if span == Span::Year {
        month_labels(from)
    } else {
        Vec::new()
    };
    let any_before = cells.iter().any(|c| c.kind == Cell::Before);
    let legend = match (tab, span) {
        (Tab::Drink, Span::Year) if any_before => "Outlined days are before you started",
        (Tab::Drink, _) => "Darker is closer to your goal",
        (Tab::Focus, Span::Month) => "Arrow keys move a day",
        (Tab::Focus, Span::Year) => "Darker is more focus time",
    };
    let focus = ask
        .focus
        .and_then(|d| cells.iter().position(|c| c.date == d));
    View {
        tab,
        span: ask.span,
        range: range_label(ask.span, from, anchor.min(today), today),
        can_prev: from > start,
        can_next: !current,
        body,
        cells,
        months,
        legend: legend.into(),
        legend_top: match tab {
            Tab::Drink => "Goal met",
            Tab::Focus => "2 h or more",
        },
        focus,
    }
}

fn empty(tab: Tab, ask: &Ask) -> Body {
    let title = "Nothing logged yet".to_string();
    match tab {
        Tab::Drink => Body::Empty {
            title,
            text: "Log a drink and today fills in. A darker square is a day closer to your goal."
                .into(),
            action: format!("+{} ml", ask.glass_ml),
        },
        Tab::Focus => Body::Empty {
            title,
            text: "Finish a focus session and today fills in. A darker square is more focus time."
                .into(),
            action: format!("Start {} min", ask.focus_min),
        },
    }
}

fn first_week(log: &Log, tab: Tab, start: Date, today: Date) -> Body {
    let total = log.total(tab, start, today);
    let value = match tab {
        Tab::Drink if total < 1000 => vec![(total.to_string(), false), ("ml".into(), true)],
        Tab::Drink => vec![(litres(total), false), ("L".into(), true)],
        Tab::Focus => minutes_parts(total),
    };
    let days = days_between(start, today) + 1;
    let since = if days == 1 {
        "Today so far".into()
    } else {
        format!("Since {}", wd_d_mon(start))
    };
    let verb = if days == 1 { "doesn't" } else { "don't" };
    let what = if days == 1 {
        "one day".to_string()
    } else {
        format!("{} days", count_word(days))
    };
    let note = format!(
        "Averages and best week show once you have 7 days, so {what} {verb} paint a false picture."
    );
    Body::FirstWeek { value, since, note }
}

#[allow(clippy::too_many_arguments)] // the page and the log's bounds, all needed for the words
fn stats(
    log: &Log,
    tab: Tab,
    span: Span,
    from: Date,
    to: Date,
    start: Date,
    today: Date,
    current: bool,
) -> Vec<Stat> {
    let (a, b) = (from.max(start), to.min(today));
    let known = (days_between(a, b) + 1).max(1) as u32;
    let total = if a <= b { log.total(tab, a, b) } else { 0 };
    let sub = if start > from {
        format!("since {}", d_mon(start))
    } else {
        match (span, current) {
            (Span::Month, true) => "last 5 weeks".into(),
            (Span::Month, false) => "these 5 weeks".into(),
            (Span::Year, true) => "last 12 months".into(),
            (Span::Year, false) => "these 12 months".into(),
        }
    };
    let best = (0..span.days() / 7)
        .map(|w| {
            let mon = add_days(from, w * 7);
            (log.total(tab, mon, add_days(mon, 6)), mon)
        })
        .fold((0, from), |best, w| if w.0 > best.0 { w } else { best });
    let best_sub = if best.0 == 0 {
        "nothing logged".to_string()
    } else {
        format!("from {}", d_mon(best.1))
    };
    let avg = total / known;
    match tab {
        Tab::Drink => {
            let met = (0..=days_between(a, b))
                .filter(|i| log.day(add_days(a, *i)).is_some_and(|d| d.ml >= log.goal))
                .count() as u32;
            vec![
                Stat {
                    value: vec![(avg.to_string(), false), ("ml".into(), true)],
                    key: "Average a day".into(),
                    sub: sub.clone(),
                },
                Stat {
                    value: vec![
                        (met.to_string(), false),
                        ((if met == 1 { "day" } else { "days" }).into(), true),
                    ],
                    key: "Goal met".into(),
                    sub,
                },
                Stat {
                    value: vec![(litres(best.0), false), ("L".into(), true)],
                    key: "Best week".into(),
                    sub: best_sub,
                },
            ]
        }
        Tab::Focus => {
            let first = if current {
                Stat {
                    value: minutes_parts(log.total(tab, monday(today), today)),
                    key: "This week".into(),
                    sub: "Mon to today".into(),
                }
            } else {
                Stat {
                    value: minutes_parts(total),
                    key: "In all".into(),
                    sub: sub.clone(),
                }
            };
            vec![
                first,
                Stat {
                    value: minutes_parts(avg),
                    key: "Average a day".into(),
                    sub,
                },
                Stat {
                    value: minutes_parts(best.0),
                    key: "Best week".into(),
                    sub: best_sub,
                },
            ]
        }
    }
}

fn by_task(
    log: &Log,
    from: Date,
    to: Date,
    start: Date,
    today: Date,
    current: bool,
) -> Option<ByTask> {
    let mut groups: Vec<(Option<String>, u32)> = Vec::new();
    for (_, d) in log.days.range(from..=to.min(today)) {
        for (t, m) in &d.tasks {
            match groups.iter_mut().find(|(g, _)| g == t) {
                Some((_, gm)) => *gm += m,
                None => groups.push((t.clone(), *m)),
            }
        }
    }
    let total: u32 = groups.iter().map(|g| g.1).sum();
    if total == 0 {
        return None;
    }
    groups.sort_by(|x, y| y.1.cmp(&x.1).then(x.0.is_none().cmp(&y.0.is_none())));
    let top = groups[0].1.max(1) as f32;
    let label = if start > from {
        format!("By task, since {}", d_mon(start))
    } else if current {
        "By task, last 5 weeks".into()
    } else {
        "By task, these 5 weeks".into()
    };
    Some(ByTask {
        label,
        total: minutes_text(total),
        rows: groups
            .into_iter()
            .take(4)
            .map(|(name, minutes)| TaskRow {
                name,
                minutes,
                share: minutes as f32 / top,
            })
            .collect(),
    })
}

/// A month's name over the first week column that holds its 1st, skipping one that would crowd
/// the label before it.
fn month_labels(from: Date) -> Vec<(usize, &'static str)> {
    let mut out: Vec<(usize, &'static str)> = Vec::new();
    for c in 0..53usize {
        let mon = add_days(from, c as i64 * 7);
        let new_month = c == 0 || (0..7).any(|i| add_days(mon, i).day() == 1);
        let at = if c == 0 { mon } else { add_days(mon, 6) };
        if new_month && out.last().is_none_or(|(lc, _)| c - lc >= 3) {
            out.push((c, month_name(at)));
        }
    }
    // The first column's month is often a few days of it; drop it if the next comes soon.
    if out.len() > 1 && out[1].0 < 3 {
        out.remove(0);
    }
    out
}

/// The day detail for one cell. `pace` is the Drink panel's own pace line, for today.
pub fn detail(log: &Log, tab: Tab, span: Span, c: &CellView, pace: &str) -> Option<Detail> {
    let title = if c.today {
        "Today, so far".to_string()
    } else {
        wd_d_mon(c.date)
    };
    let quiet = |value: &str| Detail {
        title: title.clone(),
        value: value.into(),
        note: String::new(),
        quiet: true,
        line: String::new(),
        rows: Vec::new(),
    };
    match c.kind {
        Cell::Ahead | Cell::Loading(_) => return None,
        Cell::Before => return Some(quiet("Before you started")),
        Cell::Level(_) => {}
    }
    let Some(d) = log.day(c.date).filter(|d| amount(tab, d) > 0) else {
        return Some(quiet("Nothing logged"));
    };
    Some(match tab {
        Tab::Drink => {
            let note = if d.ml >= log.goal {
                "· goal met".to_string()
            } else {
                format!("/ {} ml", log.goal)
            };
            let drinks = match (d.drinks, d.from_device) {
                (1, 0) => format!("1 drink at {}", d.last_drink.map(hhmm).unwrap_or_default()),
                (n, 0) => format!(
                    "{n} drinks, last at {}",
                    d.last_drink.map(hhmm).unwrap_or_default()
                ),
                (n, k) => format!(
                    "{}, {k} from your {}",
                    plural(n, "drink", "drinks"),
                    d.device.as_deref().unwrap_or("phone")
                ),
            };
            let line = if c.today && !pace.is_empty() && d.ml < log.goal {
                pace.trim_end_matches('.').to_string()
            } else {
                drinks
            };
            Detail {
                title,
                value: format!("{} ml", d.ml),
                note,
                quiet: false,
                line,
                rows: Vec::new(),
            }
        }
        Tab::Focus => {
            let mut tasks = d.tasks.clone();
            tasks.sort_by_key(|t| std::cmp::Reverse(t.1));
            let named = tasks.iter().any(|t| t.0.is_some());
            let (line, rows) = match span {
                Span::Year => (
                    tasks
                        .iter()
                        .find_map(|(t, m)| {
                            t.as_ref()
                                .map(|t| format!("Most on {t}, {}", minutes_text(*m)))
                        })
                        .unwrap_or_default(),
                    Vec::new(),
                ),
                Span::Month if named => (
                    String::new(),
                    tasks
                        .iter()
                        .take(2)
                        .map(|(t, m)| {
                            (
                                t.clone().unwrap_or_else(|| "No task".into()),
                                minutes_text(*m),
                            )
                        })
                        .collect(),
                ),
                Span::Month => (String::new(), Vec::new()),
            };
            Detail {
                title,
                value: minutes_text(d.focus_min),
                note: format!("· {}", plural(d.sessions, "session", "sessions")),
                quiet: false,
                line,
                rows,
            }
        }
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn date(y: i32, m: u32, d: u32) -> Date {
        Date::new(y, m, d).unwrap()
    }

    fn drink(d: Date, h: u32, ml: u32) -> DrinkIn {
        DrinkIn {
            at: d.at(h, 0),
            ml,
            device: None,
        }
    }

    fn session(d: Date, minutes: u32, task: Option<&str>) -> SessionIn {
        SessionIn {
            ended: d.at(15, 0),
            minutes,
            task: task.map(String::from),
        }
    }

    fn ask(tab: Tab, span: Span, today: Date) -> Ask {
        Ask {
            tab,
            span,
            anchor: this_anchor(today),
            today,
            focus: None,
            loading: false,
            glass_ml: 250,
            focus_min: 25,
        }
    }

    /// Fri 6 Nov 2026, as in the frames.
    fn friday() -> Date {
        date(2026, 11, 6)
    }

    #[test]
    fn weeks_start_on_monday() {
        assert_eq!(weekday(friday()), 4);
        assert_eq!(monday(friday()), date(2026, 11, 2));
        assert_eq!(sunday(friday()), date(2026, 11, 8));
        assert_eq!(add_days(date(2026, 1, 1), -1), date(2025, 12, 31));
        assert_eq!(shift_month(date(2026, 3, 31), -1), date(2026, 2, 28));
        assert_eq!(shift_month(date(2026, 1, 15), -1), date(2025, 12, 15));
    }

    #[test]
    fn five_levels_for_drink_and_focus() {
        let day = |ml, focus_min| Day {
            ml,
            focus_min,
            ..Day::default()
        };
        let levels: Vec<u8> = [0, 900, 1000, 1500, 1999, 2000]
            .iter()
            .map(|ml| level(Tab::Drink, Some(&day(*ml, 0)), 2000))
            .collect();
        assert_eq!(levels, [0, 1, 2, 3, 3, 4]);
        let levels: Vec<u8> = [0, 29, 30, 60, 119, 120]
            .iter()
            .map(|m| level(Tab::Focus, Some(&day(0, *m)), 2000))
            .collect();
        assert_eq!(levels, [0, 1, 2, 3, 3, 4]);
        assert_eq!(level(Tab::Drink, None, 2000), 0);
    }

    #[test]
    fn the_month_view_is_five_weeks_to_this_sunday() {
        let today = friday();
        let log = Log::new(
            &(0..40)
                .map(|i| drink(add_days(today, -i), 10, 2000))
                .collect::<Vec<_>>(),
            &[],
            2000,
        );
        let v = view(&log, &ask(Tab::Drink, Span::Month, today));
        assert_eq!(v.cells.len(), 35);
        assert_eq!(v.cells[0].date, date(2026, 10, 5));
        assert_eq!(v.range, "5 Oct to 6 Nov");
        assert!(v.cells[32].today);
        assert_eq!(v.cells[33].kind, Cell::Ahead);
        assert_eq!(v.cells[0].kind, Cell::Level(4));
        assert!(v.can_prev && !v.can_next);
        let Body::Full { stats, .. } = &v.body else {
            panic!("{:?}", v.body)
        };
        assert_eq!(stats[0].value[0].0, "2000");
        assert_eq!(stats[0].sub, "last 5 weeks");
        assert_eq!(stats[1].value[0].0, "33");
        assert_eq!(stats[2].value[0].0, "14.0");
        assert_eq!(stats[2].sub, "from 5 Oct");
    }

    #[test]
    fn days_before_the_first_log_are_outlines_and_averages_count_from_it() {
        let today = friday();
        let first = date(2026, 10, 20);
        let log = Log::new(&[drink(first, 9, 1000), drink(today, 9, 2000)], &[], 2000);
        let v = view(&log, &ask(Tab::Drink, Span::Month, today));
        assert_eq!(v.cells[0].kind, Cell::Before);
        assert_eq!(v.cells[15].kind, Cell::Level(2));
        assert_eq!(
            v.cells[16].kind,
            Cell::Level(0),
            "a day after the start with nothing is plain, not missed"
        );
        assert!(!v.can_prev, "nothing before the first log to page back to");
        let Body::Full { stats, .. } = &v.body else {
            panic!()
        };
        // 3000 ml over the 18 days from 20 Oct to 6 Nov.
        assert_eq!(stats[0].value[0].0, "166");
        assert_eq!(stats[0].sub, "since 20 Oct");
        assert_eq!(
            stats[1].value,
            vec![("1".to_string(), false), ("day".to_string(), true)]
        );
    }

    #[test]
    fn an_empty_tab_and_the_first_week_say_so_plainly() {
        let today = friday();
        let log = Log::new(&[], &[], 2000);
        let v = view(&log, &ask(Tab::Drink, Span::Year, today));
        let Body::Empty { action, .. } = &v.body else {
            panic!("{:?}", v.body)
        };
        assert_eq!(action, "+250 ml");
        assert_eq!(v.cells.len(), 35, "the small layout shows this month");
        assert!(v
            .cells
            .iter()
            .filter(|c| c.date < today)
            .all(|c| c.kind == Cell::Before));
        assert_eq!(v.cells[32].kind, Cell::Level(0));

        let log = Log::new(
            &[],
            &[
                session(date(2026, 11, 4), 50, None),
                session(today, 176, Some("Kirim invoice")),
            ],
            2000,
        );
        let v = view(&log, &ask(Tab::Focus, Span::Month, today));
        let Body::FirstWeek { value, since, note } = &v.body else {
            panic!("{:?}", v.body)
        };
        assert_eq!(minutes_text(226), "3 h 46 min");
        assert_eq!(value, &minutes_parts(226));
        assert_eq!(since, "Since Wed 4 Nov");
        assert!(note.contains("three days don't"), "{note}");
        // Drink has nothing on its own tab: empty, even though focus has days.
        assert!(matches!(
            view(&log, &ask(Tab::Drink, Span::Month, today)).body,
            Body::Empty { .. }
        ));
    }

    #[test]
    fn no_guilt_words_anywhere() {
        let today = friday();
        let log = Log::new(
            &(0..20)
                .map(|i| drink(add_days(today, -i * 2), 10, 300))
                .collect::<Vec<_>>(),
            &(0..20)
                .map(|i| session(add_days(today, -i * 3), 25, None))
                .collect::<Vec<_>>(),
            2000,
        );
        for tab in [Tab::Drink, Tab::Focus] {
            for span in [Span::Month, Span::Year] {
                let v = view(&log, &ask(tab, span, today));
                let mut words = format!("{:?}", v.body) + &v.legend + &v.range;
                for c in &v.cells {
                    if let Some(d) = detail(&log, tab, span, c, "") {
                        words += &format!("{d:?}");
                    }
                }
                let lower = words.to_lowercase();
                for bad in [
                    "missed", "streak", "failed", "behind", "broke", "prayer", "shalat", "sholat",
                ] {
                    assert!(!lower.contains(bad), "{tab:?} {span:?} says {bad}");
                }
            }
        }
    }

    #[test]
    fn the_day_detail_reads_like_the_frame() {
        let today = friday();
        let thu = date(2026, 11, 5);
        let wed = date(2026, 11, 4);
        let mut drinks: Vec<DrinkIn> = (0..7)
            .map(|i| drink(thu, 9 + i * 2, if i == 6 { 300 } else { 250 }))
            .collect();
        drinks[6].at = thu.at(21, 10);
        drinks.extend((0..8).map(|i| DrinkIn {
            at: wed.at(8 + i, 0),
            ml: if i < 2 { 400 } else { 225 },
            device: (i < 3).then(|| "phone".to_string()),
        }));
        drinks.push(drink(today, 9, 750));
        let log = Log::new(&drinks, &[], 2000);
        let cell = |d: Date| CellView {
            date: d,
            kind: Cell::Level(1),
            today: d == today,
        };
        let thu_d = detail(&log, Tab::Drink, Span::Month, &cell(thu), "").unwrap();
        assert_eq!(
            (
                thu_d.title.as_str(),
                thu_d.value.as_str(),
                thu_d.note.as_str(),
                thu_d.line.as_str()
            ),
            (
                "Thu 5 Nov",
                "1800 ml",
                "/ 2000 ml",
                "7 drinks, last at 21:10"
            )
        );
        let wed_d = detail(&log, Tab::Drink, Span::Month, &cell(wed), "").unwrap();
        assert_eq!(
            (
                wed_d.value.as_str(),
                wed_d.note.as_str(),
                wed_d.line.as_str()
            ),
            ("2150 ml", "· goal met", "8 drinks, 3 from your phone")
        );
        let t = detail(
            &log,
            Tab::Drink,
            Span::Month,
            &cell(today),
            "Ahead of pace for 11:00.",
        )
        .unwrap();
        assert_eq!(
            (t.title.as_str(), t.line.as_str()),
            ("Today, so far", "Ahead of pace for 11:00")
        );
        let quiet = detail(&log, Tab::Drink, Span::Month, &cell(date(2026, 11, 3)), "").unwrap();
        assert!(quiet.quiet && quiet.value == "Nothing logged");

        let tue = date(2026, 11, 3);
        let log = Log::new(
            &[],
            &[
                session(tue, 40, Some("Kirim invoice")),
                session(tue, 25, Some("Kirim invoice")),
                session(tue, 37, Some("Belajar Rust")),
                session(add_days(tue, -20), 5, None),
            ],
            2000,
        );
        let d = detail(&log, Tab::Focus, Span::Month, &cell(tue), "").unwrap();
        assert_eq!(
            (d.value.as_str(), d.note.as_str()),
            ("1 h 42 min", "· 3 sessions")
        );
        assert_eq!(
            d.rows,
            vec![
                ("Kirim invoice".to_string(), "1 h 5 min".to_string()),
                ("Belajar Rust".to_string(), "37 min".to_string())
            ]
        );
        let y = detail(&log, Tab::Focus, Span::Year, &cell(tue), "").unwrap();
        assert_eq!(y.line, "Most on Kirim invoice, 1 h 5 min");
    }

    #[test]
    fn focus_month_splits_time_by_task() {
        let today = friday();
        let log = Log::new(
            &[],
            &[
                session(today, 60, Some("A")),
                session(add_days(today, -1), 30, None),
                session(add_days(today, -40), 25, Some("B")),
                session(add_days(today, -8), 90, Some("C")),
            ],
            2000,
        );
        let v = view(&log, &ask(Tab::Focus, Span::Month, today));
        let Body::Full {
            by_task: Some(b),
            stats,
        } = &v.body
        else {
            panic!("{:?}", v.body)
        };
        assert_eq!(b.total, "3 h", "B is outside these 5 weeks");
        assert_eq!(
            b.rows
                .iter()
                .map(|r| r.name.as_deref().unwrap_or("No task"))
                .collect::<Vec<_>>(),
            ["C", "A", "No task"]
        );
        assert_eq!(b.rows[1].share, 60.0 / 90.0);
        assert_eq!(stats[0].key, "This week");
        assert_eq!(minutes_text(90), "1 h 30 min");
    }

    #[test]
    fn paging_keeps_the_focused_day_on_screen() {
        let today = friday();
        let a = this_anchor(today);
        assert_eq!(anchor_for(today, Span::Month, a, today), a);
        let back = anchor_for(date(2026, 9, 30), Span::Month, a, today);
        assert_eq!(back, add_days(monday(date(2026, 9, 30)), 34));
        assert!(add_days(back, -34) <= date(2026, 9, 30) && date(2026, 9, 30) <= back);
        assert_eq!(
            anchor_for(date(2026, 12, 25), Span::Month, back, today),
            a,
            "never past this week"
        );
        let log = Log::new(
            &[drink(date(2025, 1, 1), 9, 500), drink(today, 9, 500)],
            &[],
            2000,
        );
        let v = view(
            &log,
            &Ask {
                tab: Tab::Drink,
                span: Span::Month,
                anchor: add_days(a, -35),
                today,
                focus: Some(date(2026, 10, 1)),
                ..ask(Tab::Drink, Span::Month, today)
            },
        );
        assert_eq!(v.range, "31 Aug to 4 Oct");
        assert!(v.can_next);
        assert_eq!(v.cells[v.focus.unwrap()].date, date(2026, 10, 1));
        let Body::Full { stats, .. } = &v.body else {
            panic!()
        };
        assert_eq!(stats[0].sub, "these 5 weeks");
    }

    #[test]
    fn the_year_view_has_53_weeks_and_month_names() {
        let today = friday();
        let log = Log::new(
            &[drink(date(2026, 1, 5), 9, 2000), drink(today, 9, 500)],
            &[],
            2000,
        );
        let v = view(&log, &ask(Tab::Drink, Span::Year, today));
        assert_eq!(v.cells.len(), 371);
        assert_eq!(v.range, "Nov 2025 to Nov 2026");
        assert_eq!(v.legend, "Outlined days are before you started");
        let names: Vec<&str> = v.months.iter().map(|m| m.1).collect();
        assert_eq!(
            names,
            [
                "Nov", "Dec", "Jan", "Feb", "Mar", "Apr", "May", "Jun", "Jul", "Aug", "Sep", "Oct",
                "Nov"
            ][..names.len()]
                .to_vec()
        );
        assert!(v.months.windows(2).all(|w| w[1].0 - w[0].0 >= 3));
    }

    #[test]
    fn loading_shows_this_pcs_days_and_counts_them() {
        let today = friday();
        let log = Log::new(
            &[
                drink(date(2026, 10, 7), 9, 2000),
                drink(date(2026, 11, 4), 9, 2000),
            ],
            &[],
            2000,
        );
        let v = view(
            &log,
            &Ask {
                loading: true,
                ..ask(Tab::Drink, Span::Month, today)
            },
        );
        let Body::Loading { done, total } = v.body else {
            panic!()
        };
        // 7 Oct, 4 Nov and today; 33 days up to today on screen, 2 before the start.
        assert_eq!((done, total), (3, 33));
        assert!(matches!(v.cells[3].kind, Cell::Loading(_)));
        assert_eq!(v.cells[2].kind, Cell::Level(4));
    }
}
