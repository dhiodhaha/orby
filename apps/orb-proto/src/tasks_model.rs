//! What the Tasks panel shows and does, on an `app_store::Snapshot`: the sidebar tree, the rows
//! of the selected view, quick add (target list, due, repeat, assignee), ticking (a repeating task
//! moves to its next date instead), the overdue count for the orb's badge, and which timed tasks
//! fall due between two instants. No drawing and no OS calls; "now" comes in as a [`Clock`].

use std::collections::HashSet;

use app_quickadd as qa;
use app_store::{
    Date, Due, FolderId, ListId, ListParent, NewTask, Repeat, Snapshot, SpaceId, Store, Task,
    TaskEdit, TaskId, TimeOfDay, UserId,
};

/// "Now" for the panel, read once by the caller.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Clock {
    pub today: Date,
    /// Minutes after local midnight.
    pub minute: u32,
    /// Unix ms, for store writes.
    pub now_ms: u64,
    /// Local time minus UTC, for showing completion times.
    pub offset_secs: i64,
}

impl Clock {
    pub fn stamp(&self) -> Stamp {
        (self.today, self.minute)
    }

    /// The local (date, minute) of a Unix ms instant.
    pub fn day_of_ms(&self, ms: u64) -> (Date, u32) {
        let local = (ms / 1000) as i64 + self.offset_secs;
        let days = local.div_euclid(86_400);
        let epoch = qa::Date::new(1970, 1, 1).expect("epoch");
        (
            to_store(epoch.add_days(days)),
            (local.rem_euclid(86_400) / 60) as u32,
        )
    }

    pub fn hhmm_of_ms(&self, ms: u64) -> String {
        hhmm(self.day_of_ms(ms).1)
    }
}

/// A local instant to the minute: (date, minutes after midnight). Orders by time.
pub type Stamp = (Date, u32);

#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub enum Selection {
    Inbox,
    Today,
    Upcoming,
    List(ListId),
    Folder(FolderId),
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Tab {
    Todo,
    Done,
}

// ---- dates ----

pub fn to_qa(d: Date) -> qa::Date {
    qa::Date::new(d.year(), d.month(), d.day()).expect("a store date is valid")
}

pub fn to_store(d: qa::Date) -> Date {
    Date::new(d.year(), d.month(), d.day()).expect("a quickadd date is valid")
}

pub fn add_days(d: Date, n: i64) -> Date {
    to_store(to_qa(d).add_days(n))
}

const WEEKDAYS: [&str; 7] = ["Mon", "Tue", "Wed", "Thu", "Fri", "Sat", "Sun"];
const MONTHS: [&str; 12] = [
    "Jan", "Feb", "Mar", "Apr", "May", "Jun", "Jul", "Aug", "Sep", "Oct", "Nov", "Dec",
];

pub fn weekday_name(d: Date) -> &'static str {
    WEEKDAYS[to_qa(d).weekday().days_from_monday() as usize]
}

pub fn hhmm(minute: u32) -> String {
    format!("{:02}:{:02}", minute / 60, minute % 60)
}

fn minute_of(t: TimeOfDay) -> u32 {
    t.hour() as u32 * 60 + t.minute() as u32
}

fn days_between(a: Date, b: Date) -> i64 {
    let (a, b) = (to_qa(a), to_qa(b));
    // Dates are close together here; walk at most a year.
    for n in 0..=366 {
        if a.add_days(n) == b {
            return n;
        }
        if b.add_days(n) == a {
            return -n;
        }
    }
    i64::MAX
}

/// "Today", "Tomorrow", "Fri" (this week), "14 Nov" (later), with the time when there is one.
pub fn day_phrase(d: Date, today: Date) -> String {
    match days_between(today, d) {
        0 => "Today".into(),
        1 => "Tomorrow".into(),
        -1 => "Yesterday".into(),
        2..=6 => weekday_name(d).into(),
        _ => format!("{} {}", d.day(), MONTHS[d.month() as usize - 1]),
    }
}

fn due_stamp(due: Due) -> Stamp {
    (due.date, due.time.map_or(0, minute_of))
}

// ---- the task itself ----

/// Past due: an all-day task from before today, or a timed one whose minute has come.
pub fn is_overdue(t: &Task, c: &Clock) -> bool {
    match t.due {
        _ if t.done => false,
        None => false,
        Some(Due { date, time: None }) => date < c.today,
        Some(due @ Due { time: Some(_), .. }) => due_stamp(due) <= c.stamp(),
    }
}

/// What the row says about the date, and whether it reads as overdue (ink 500, never red).
pub fn due_label(t: &Task, c: &Clock) -> (String, bool) {
    let Some(due) = t.due else {
        return ("No date".into(), false);
    };
    let time = due.time.map(|x| hhmm(minute_of(x)));
    if is_overdue(t, c) {
        let when = if due.date == c.today {
            time.unwrap_or_default()
        } else {
            weekday_name(due.date).to_owned()
        };
        return (format!("Overdue · {when}"), true);
    }
    let with_time = |s: String| match &time {
        Some(t) => format!("{s} {t}"),
        None => s,
    };
    let when = with_time(day_phrase(due.date, c.today));
    // A repeating task names its next date, so ticking it visibly moves it on.
    let s = match t.repeat {
        Repeat::None => when,
        Repeat::Daily => format!("{when} · every day"),
        // Within the coming week the weekday alone already names the date.
        Repeat::Weekly if (2..=6).contains(&days_between(c.today, due.date)) => {
            with_time(format!("Every {}", weekday_name(due.date)))
        }
        Repeat::Weekly => format!("{when} · every {}", weekday_name(due.date)),
    };
    (s, false)
}

/// The next date of a repeating task that is ticked off: one step on, and past today if it was
/// left behind. `None` for a task that does not repeat.
pub fn next_repeat(due: Due, repeat: Repeat, today: Date) -> Option<Due> {
    let step = match repeat {
        Repeat::None => return None,
        Repeat::Daily => 1,
        Repeat::Weekly => 7,
    };
    let mut date = add_days(due.date, step);
    while date <= today {
        date = add_days(date, step);
    }
    Some(Due {
        date,
        time: due.time,
    })
}

/// Ticks a task off. A repeating one moves to its next date and stays open.
pub fn complete(store: &mut Store, t: &Task, c: &Clock) -> app_store::Result<()> {
    match t.due.and_then(|d| next_repeat(d, t.repeat, c.today)) {
        Some(next) => store.edit_task(&t.id, TaskEdit::Due(Some(next)), c.now_ms),
        None => store.set_done(&t.id, true, c.now_ms),
    }
}

pub fn overdue_count(s: &Snapshot, c: &Clock) -> usize {
    s.tasks.values().filter(|t| is_overdue(t, c)).count()
}

/// Open timed tasks that fall due after `from` and at or before `to`, earliest first.
pub fn due_between(s: &Snapshot, from: Stamp, to: Stamp) -> Vec<Task> {
    let mut v: Vec<Task> = s
        .tasks
        .values()
        .filter(|t| !t.done)
        .filter(|t| {
            t.due
                .is_some_and(|d| d.time.is_some() && due_stamp(d) > from && due_stamp(d) <= to)
        })
        .cloned()
        .collect();
    v.sort_by_key(|t| t.due);
    v
}

/// The next instant after `from` when a timed task falls due.
pub fn next_due_after(s: &Snapshot, from: Stamp) -> Option<Stamp> {
    s.tasks
        .values()
        .filter(|t| !t.done)
        .filter_map(|t| t.due.filter(|d| d.time.is_some()))
        .map(due_stamp)
        .filter(|st| *st > from)
        .min()
}

// ---- first run ----

/// The inbox: a list called "Inbox" directly in the first space. Creates the "Personal" space
/// and the inbox when the store has none (first run). Nothing else is added.
pub fn ensure_inbox(
    store: &mut Store,
    s: &Snapshot,
    saved: Option<&ListId>,
    now_ms: u64,
) -> app_store::Result<ListId> {
    if let Some(l) = saved.filter(|l| s.lists.contains_key(*l)) {
        return Ok(l.clone());
    }
    let space = match s.spaces.values().next() {
        Some(sp) => sp.id.clone(),
        None => store.create_space("Personal", now_ms)?,
    };
    if let Some(l) = s
        .lists
        .values()
        .find(|l| l.name == "Inbox" && l.parent == ListParent::Space(space.clone()))
    {
        return Ok(l.id.clone());
    }
    store.create_list(ListParent::Space(space), "Inbox", 0.0, now_ms)
}

/// Clearly marked sample tasks for test drives (tray "Test: add sample tasks"), only into a
/// store that has no tasks yet.
pub fn add_samples(
    store: &mut Store,
    s: &Snapshot,
    inbox: &ListId,
    c: &Clock,
) -> app_store::Result<usize> {
    if !s.tasks.is_empty() {
        return Ok(0);
    }
    let space = s.lists.get(inbox).and_then(|l| match &l.parent {
        ListParent::Space(sp) => Some(sp.clone()),
        ListParent::Folder(_) => None,
    });
    let Some(space) = space else { return Ok(0) };
    let home = store.create_folder(&space, None, "Home (sample)", 1.0, c.now_ms)?;
    let groceries = store.create_list(ListParent::Folder(home), "Groceries", 0.0, c.now_ms)?;
    let at = |days: i64, h: u8, m: u8| {
        Some(Due {
            date: add_days(c.today, days),
            time: TimeOfDay::new(h, m),
        })
    };
    let soon = {
        let m = (c.minute + 2).min(23 * 60 + 59);
        Some(Due {
            date: c.today,
            time: TimeOfDay::new((m / 60) as u8, (m % 60) as u8),
        })
    };
    let note = "Sample task".to_string();
    let samples = [
        (&groceries, "Beras 5 kg", soon, Repeat::None),
        (
            &groceries,
            "Minyak goreng 2 L",
            Some(Due {
                date: c.today,
                time: None,
            }),
            Repeat::None,
        ),
        (&groceries, "Galon Aqua", at(1, 9, 0), Repeat::Weekly),
        (&groceries, "Sabun cuci piring", None, Repeat::None),
        (inbox, "Kirim invoice", at(-2, 10, 0), Repeat::None),
    ];
    for (i, (list, title, due, repeat)) in samples.iter().enumerate() {
        store.create_task(
            list,
            NewTask {
                title: (*title).into(),
                notes: note.clone(),
                due: *due,
                repeat: *repeat,
                order: i as f64,
                ..NewTask::default()
            },
            c.now_ms,
        )?;
    }
    Ok(samples.len())
}

// ---- the sidebar ----

#[derive(Clone, Debug, PartialEq)]
pub enum SideKey {
    Inbox,
    Today,
    Upcoming,
    Space(SpaceId),
    Folder(FolderId),
    List(ListId),
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub enum SideIcon {
    Inbox,
    Today,
    Upcoming,
    Private,
    Shared,
    Folder,
    List,
}

#[derive(Clone, Debug, PartialEq)]
pub struct SideItem {
    pub key: SideKey,
    pub depth: u32,
    pub icon: SideIcon,
    pub name: String,
    /// Open tasks inside; 0 shows nothing.
    pub count: usize,
    /// Some(expanded) for a space or folder that has children.
    pub chevron: Option<bool>,
    /// "private" beside a private space.
    pub note: String,
    pub selected: bool,
}

fn open_in(s: &Snapshot, lists: &HashSet<&ListId>) -> usize {
    s.tasks
        .values()
        .filter(|t| !t.done && lists.contains(&t.list_id))
        .count()
}

/// Every list under a container (folders to any depth).
fn lists_under(s: &Snapshot, root: &ListParent) -> HashSet<ListId> {
    let mut folders: HashSet<FolderId> = HashSet::new();
    if let ListParent::Folder(f) = root {
        folders.insert(f.clone());
    }
    let space = match root {
        ListParent::Space(sp) => Some(sp),
        ListParent::Folder(_) => None,
    };
    let mut grew = true;
    while grew {
        grew = false;
        for f in s.folders.values() {
            let inside = match (&f.parent_id, space) {
                (None, Some(sp)) => &f.space_id == sp,
                (Some(p), _) => folders.contains(p),
                _ => false,
            };
            if inside && folders.insert(f.id.clone()) {
                grew = true;
            }
        }
    }
    s.lists
        .values()
        .filter(|l| match &l.parent {
            ListParent::Folder(f) => folders.contains(f),
            p => p == root,
        })
        .map(|l| l.id.clone())
        .collect()
}

fn ordered<T, K: Ord>(mut v: Vec<T>, key: impl Fn(&T) -> (f64, K)) -> Vec<T> {
    v.sort_by(|a, b| {
        let (oa, ka) = key(a);
        let (ob, kb) = key(b);
        oa.total_cmp(&ob).then(ka.cmp(&kb))
    });
    v
}

/// One level of a container: its folders first, then its lists, each by order. The inbox is
/// left out (it has its own row at the top).
fn level(
    s: &Snapshot,
    parent: &ListParent,
    inbox: &ListId,
    collapsed: &HashSet<String>,
    depth: u32,
    sel: &Selection,
    out: &mut Vec<SideItem>,
) {
    let folders: Vec<_> = s
        .folders
        .values()
        .filter(|f| match parent {
            ListParent::Space(sp) => f.parent_id.is_none() && &f.space_id == sp,
            ListParent::Folder(p) => f.parent_id.as_ref() == Some(p),
        })
        .collect();
    for f in ordered(folders, |f| (f.order, f.id.clone())) {
        let me = ListParent::Folder(f.id.clone());
        let lists = lists_under(s, &me);
        let has_children = s
            .folders
            .values()
            .any(|c| c.parent_id.as_ref() == Some(&f.id))
            || s.lists.values().any(|l| l.parent == me);
        let open = !collapsed.contains(f.id.as_str());
        out.push(SideItem {
            key: SideKey::Folder(f.id.clone()),
            depth,
            icon: SideIcon::Folder,
            name: f.name.clone(),
            count: open_in(s, &lists.iter().collect()),
            chevron: has_children.then_some(open),
            note: String::new(),
            selected: *sel == Selection::Folder(f.id.clone()),
        });
        if open {
            level(s, &me, inbox, collapsed, depth + 1, sel, out);
        }
    }
    let lists: Vec<_> = s
        .lists
        .values()
        .filter(|l| &l.parent == parent && &l.id != inbox)
        .collect();
    for l in ordered(lists, |l| (l.order, l.id.clone())) {
        out.push(SideItem {
            key: SideKey::List(l.id.clone()),
            depth,
            icon: SideIcon::List,
            name: l.name.clone(),
            count: open_in(s, &HashSet::from([&l.id])),
            chevron: None,
            note: String::new(),
            selected: *sel == Selection::List(l.id.clone()),
        });
    }
}

pub fn sidebar(
    s: &Snapshot,
    inbox: &ListId,
    collapsed: &HashSet<String>,
    sel: &Selection,
    c: &Clock,
) -> Vec<SideItem> {
    let agenda_today = s
        .tasks
        .values()
        .filter(|t| !t.done && t.due.is_some_and(|d| d.date <= c.today))
        .count();
    let upcoming = s
        .tasks
        .values()
        .filter(|t| !t.done && t.due.is_some_and(|d| d.date > c.today))
        .count();
    let fixed = |key: SideKey, icon, name: &str, count, selected| SideItem {
        key,
        depth: 0,
        icon,
        name: name.into(),
        count,
        chevron: None,
        note: String::new(),
        selected,
    };
    let mut out = vec![
        fixed(
            SideKey::Inbox,
            SideIcon::Inbox,
            "Inbox",
            open_in(s, &HashSet::from([inbox])),
            *sel == Selection::Inbox,
        ),
        fixed(
            SideKey::Today,
            SideIcon::Today,
            "Today",
            agenda_today,
            *sel == Selection::Today,
        ),
        fixed(
            SideKey::Upcoming,
            SideIcon::Upcoming,
            "Upcoming",
            upcoming,
            *sel == Selection::Upcoming,
        ),
    ];
    for sp in ordered(s.spaces.values().collect(), |sp| (0.0, sp.name.clone())) {
        let me = ListParent::Space(sp.id.clone());
        let has_children = s.folders.values().any(|f| f.space_id == sp.id)
            || s.lists.values().any(|l| l.parent == me && &l.id != inbox);
        let open = !collapsed.contains(sp.id.as_str());
        out.push(SideItem {
            key: SideKey::Space(sp.id.clone()),
            depth: 0,
            icon: if sp.shared {
                SideIcon::Shared
            } else {
                SideIcon::Private
            },
            name: sp.name.clone(),
            count: 0,
            chevron: has_children.then_some(open),
            note: if sp.shared {
                String::new()
            } else {
                "private".into()
            },
            selected: false,
        });
        if open {
            level(s, &me, inbox, collapsed, 1, sel, &mut out);
        }
    }
    out
}

// ---- the rows of a view ----

#[derive(Clone, Debug, PartialEq)]
pub enum Row {
    Task(Task),
    /// A dashed time marker in Today. Never a task: no checkbox, no state.
    Prayer {
        name: String,
        minute: u32,
    },
}

/// The tasks a view holds: (open, done). Open ones by order (lists) or by due (Today,
/// Upcoming); done ones latest first.
pub fn view_tasks(
    s: &Snapshot,
    sel: &Selection,
    inbox: &ListId,
    c: &Clock,
) -> (Vec<Task>, Vec<Task>) {
    let in_lists = |lists: HashSet<ListId>| {
        s.tasks
            .values()
            .filter(move |t| lists.contains(&t.list_id))
            .cloned()
            .collect::<Vec<_>>()
    };
    let (all, by_due): (Vec<Task>, bool) = match sel {
        Selection::Inbox => (in_lists(HashSet::from([inbox.clone()])), false),
        Selection::List(l) => (in_lists(HashSet::from([l.clone()])), false),
        Selection::Folder(f) => (
            in_lists(lists_under(s, &ListParent::Folder(f.clone()))),
            false,
        ),
        Selection::Today => {
            let day_start = c.today;
            let v = s.tasks.values().filter(|t| match t.done {
                false => t.due.is_some_and(|d| d.date <= c.today),
                true => t
                    .completed_at
                    .is_some_and(|ms| c.day_of_ms(ms).0 == day_start),
            });
            (v.cloned().collect(), true)
        }
        Selection::Upcoming => (
            s.tasks
                .values()
                .filter(|t| t.due.is_some_and(|d| d.date > c.today))
                .cloned()
                .collect(),
            true,
        ),
    };
    let (mut todo, mut done): (Vec<Task>, Vec<Task>) = all.into_iter().partition(|t| !t.done);
    if by_due {
        todo.sort_by(|a, b| {
            a.due
                .cmp(&b.due)
                .then(a.order.total_cmp(&b.order))
                .then(a.id.cmp(&b.id))
        });
    } else {
        todo.sort_by(|a, b| a.order.total_cmp(&b.order).then(a.id.cmp(&b.id)));
    }
    done.sort_by(|a, b| b.completed_at.cmp(&a.completed_at).then(a.id.cmp(&b.id)));
    (todo, done)
}

/// Today's open rows with the prayer times still ahead placed among them as markers: overdue
/// and all-day tasks first, then timed ones, a marker before the first task at or after it.
/// The minute Today's prayer markers next change: just after the next one still shown passes.
pub fn next_marker_change(prayers: &[(String, u32)], minute: u32) -> Option<u32> {
    prayers
        .iter()
        .map(|(_, m)| *m)
        .filter(|m| *m >= minute)
        .min()
        .map(|m| m + 1)
}

pub fn today_rows(todo: Vec<Task>, prayers: &[(String, u32)], c: &Clock) -> Vec<Row> {
    let mut markers: Vec<(u32, String)> = prayers
        .iter()
        .filter(|(_, m)| *m >= c.minute)
        .map(|(n, m)| (*m, n.clone()))
        .collect();
    markers.sort();
    let mut markers = markers.into_iter().peekable();
    let mut out = Vec::new();
    for t in todo {
        let timed_today = t
            .due
            .filter(|d| d.date == c.today)
            .and_then(|d| d.time)
            .map(minute_of);
        if let Some(m) = timed_today {
            while let Some((pm, name)) = markers.next_if(|(pm, _)| *pm <= m) {
                out.push(Row::Prayer { name, minute: pm });
            }
        }
        out.push(Row::Task(t));
    }
    out.extend(markers.map(|(minute, name)| Row::Prayer { name, minute }));
    out
}

// ---- names and paths ----

pub fn folder_path(s: &Snapshot, f: &FolderId) -> String {
    let mut parts = Vec::new();
    let mut cur = s.folders.get(f);
    let mut space = None;
    while let Some(folder) = cur {
        parts.push(folder.name.clone());
        space = Some(folder.space_id.clone());
        cur = folder.parent_id.as_ref().and_then(|p| s.folders.get(p));
    }
    if let Some(sp) = space.and_then(|sp| s.spaces.get(&sp)) {
        parts.push(sp.name.clone());
    }
    parts.reverse();
    parts.join("/")
}

pub fn container_path(s: &Snapshot, p: &ListParent) -> String {
    match p {
        ListParent::Space(sp) => s.spaces.get(sp).map(|x| x.name.clone()).unwrap_or_default(),
        ListParent::Folder(f) => folder_path(s, f),
    }
}

pub fn list_name(s: &Snapshot, l: &ListId) -> String {
    s.lists.get(l).map(|x| x.name.clone()).unwrap_or_default()
}

/// The breadcrumb ("Personal / Home") and title of the selected view.
pub fn header(s: &Snapshot, sel: &Selection, inbox: &ListId, c: &Clock) -> (String, String) {
    let crumb_of = |p: &ListParent| container_path(s, p).replace('/', " / ");
    match sel {
        Selection::Inbox => (
            s.lists
                .get(inbox)
                .map(|l| crumb_of(&l.parent))
                .unwrap_or_default(),
            "Inbox".into(),
        ),
        Selection::Today => (
            format!(
                "{} {} {}",
                weekday_name(c.today),
                c.today.day(),
                MONTHS[c.today.month() as usize - 1]
            ),
            "Today".into(),
        ),
        Selection::Upcoming => ("After today".into(), "Upcoming".into()),
        Selection::List(l) => (
            s.lists
                .get(l)
                .map(|x| crumb_of(&x.parent))
                .unwrap_or_default(),
            list_name(s, l),
        ),
        Selection::Folder(f) => {
            let path = folder_path(s, f);
            let (crumb, name) = path.rsplit_once('/').unwrap_or(("", &path));
            (crumb.replace('/', " / "), name.into())
        }
    }
}

/// The names quick add can resolve: every list, every folder path (spaces are its top level),
/// and the people in the spaces.
pub fn known(s: &Snapshot, people: &[String]) -> qa::Known {
    let mut lists: Vec<String> = s.lists.values().map(|l| l.name.clone()).collect();
    lists.sort();
    lists.dedup();
    let mut folders: Vec<String> = s.spaces.values().map(|sp| sp.name.clone()).collect();
    folders.extend(s.folders.keys().map(|f| folder_path(s, f)));
    qa::Known {
        lists,
        folders,
        people: people.to_vec(),
    }
}

/// The container a folder path names: a space, or a folder in it.
fn container_of_path(s: &Snapshot, path: &str) -> Option<ListParent> {
    if let Some(sp) = s.spaces.values().find(|sp| sp.name == path) {
        return Some(ListParent::Space(sp.id.clone()));
    }
    s.folders
        .keys()
        .find(|f| folder_path(s, f) == path)
        .map(|f| ListParent::Folder(f.clone()))
}

fn first_list_in(s: &Snapshot, p: &ListParent, inbox: &ListId) -> Option<ListId> {
    let direct: Vec<_> = s
        .lists
        .values()
        .filter(|l| &l.parent == p && &l.id != inbox)
        .collect();
    if let Some(l) = ordered(direct, |l| (l.order, l.id.clone())).first() {
        return Some(l.id.clone());
    }
    let mut under: Vec<_> = lists_under(s, p)
        .into_iter()
        .filter(|l| l != inbox)
        .collect();
    under.sort();
    under.into_iter().next()
}

/// Where a quick add lands: its `#list` (inside its `/folder` when both are given), else the
/// first list of its `/folder`, else the open list, else the inbox.
pub fn target_list(s: &Snapshot, q: &qa::QuickAdd, sel: &Selection, inbox: &ListId) -> ListId {
    let folder = q
        .folder_path
        .as_deref()
        .and_then(|p| container_of_path(s, p));
    if let Some(name) = &q.list {
        let named: Vec<_> = s.lists.values().filter(|l| &l.name == name).collect();
        let in_folder = folder
            .as_ref()
            .and_then(|f| named.iter().find(|l| lists_under(s, f).contains(&l.id)));
        let in_view = match sel {
            Selection::List(cur) => named.iter().find(|l| &l.id == cur),
            _ => None,
        };
        if let Some(l) = in_folder.or(in_view).or(named.first()) {
            return l.id.clone();
        }
    }
    if let Some(l) = folder.and_then(|f| first_list_in(s, &f, inbox)) {
        return l;
    }
    match sel {
        Selection::List(l) if s.lists.contains_key(l) => l.clone(),
        Selection::Folder(f) => {
            first_list_in(s, &ListParent::Folder(f.clone()), inbox).unwrap_or_else(|| inbox.clone())
        }
        _ => inbox.clone(),
    }
}

pub fn store_repeat(r: qa::Repeat) -> Repeat {
    match r {
        qa::Repeat::None => Repeat::None,
        qa::Repeat::Daily => Repeat::Daily,
        qa::Repeat::Weekly(_) => Repeat::Weekly,
    }
}

/// The due a quick add means. In Today a task with no date is due today.
pub fn quick_due(q: &qa::QuickAdd, sel: &Selection, c: &Clock) -> Option<Due> {
    match q.due {
        Some(d) => Some(Due {
            date: to_store(d.date),
            time: d.time.and_then(|t| TimeOfDay::new(t.hour(), t.minute())),
        }),
        None if *sel == Selection::Today => Some(Due {
            date: c.today,
            time: None,
        }),
        None => None,
    }
}

/// The new task a quick add makes, appended to the end of its list.
pub fn new_task(
    s: &Snapshot,
    q: &qa::QuickAdd,
    sel: &Selection,
    list: &ListId,
    c: &Clock,
) -> NewTask {
    let last = s
        .tasks
        .values()
        .filter(|t| &t.list_id == list)
        .map(|t| t.order)
        .fold(-1.0, f64::max);
    NewTask {
        title: q.title.clone(),
        notes: String::new(),
        due: quick_due(q, sel, c),
        repeat: store_repeat(q.repeat),
        assignee: q.assignee.as_ref().map(|p| UserId(p.clone())),
        order: last + 1.0,
    }
}

/// The muted line under the field: what quick add understood, in words.
pub fn understood(
    s: &Snapshot,
    q: &qa::QuickAdd,
    sel: &Selection,
    inbox: &ListId,
    c: &Clock,
    me: &UserId,
) -> String {
    let mut parts = Vec::new();
    if let Some(due) = quick_due(q, sel, c) {
        let day = match q.repeat {
            qa::Repeat::Daily => "Every day".to_string(),
            qa::Repeat::Weekly(_) => format!("Every {}", weekday_name(due.date)),
            qa::Repeat::None => day_phrase(due.date, c.today),
        };
        parts.push(match due.time {
            Some(t) => format!("{day} {}", hhmm(minute_of(t))),
            None => day,
        });
    }
    let list = target_list(s, q, sel, inbox);
    parts.push(format!("in {}", list_name(s, &list)));
    if let Some(p) = &q.assignee {
        parts.push(if p == me.as_str() {
            "for you".into()
        } else {
            format!("for {p}")
        });
    }
    let mut out = parts.join(", ");
    if let Some(first) = out.get(..1) {
        out = first.to_uppercase() + &out[1..];
    }
    out + "."
}

/// Creates a list from the "New list" field. "Home/Groceries" makes the folder Home (unless it
/// exists) in the space, then the list in it. Returns the new list.
pub fn create_list_at(
    store: &mut Store,
    s: &Snapshot,
    space: &SpaceId,
    name: &str,
    now_ms: u64,
) -> app_store::Result<Option<ListId>> {
    let parts: Vec<&str> = name
        .split('/')
        .map(str::trim)
        .filter(|p| !p.is_empty())
        .collect();
    let Some((list, folders)) = parts.split_last() else {
        return Ok(None);
    };
    let mut parent: Option<FolderId> = None;
    let mut known: Vec<app_store::Folder> = s.folders.values().cloned().collect();
    for f in folders {
        let found = known
            .iter()
            .find(|x| {
                &x.space_id == space && x.parent_id == parent && x.name.eq_ignore_ascii_case(f)
            })
            .map(|x| x.id.clone());
        let id = match found {
            Some(id) => id,
            None => {
                let order = known
                    .iter()
                    .filter(|x| &x.space_id == space && x.parent_id == parent)
                    .count() as f64;
                let id = store.create_folder(space, parent.as_ref(), f, order, now_ms)?;
                known.push(app_store::Folder {
                    id: id.clone(),
                    space_id: space.clone(),
                    parent_id: parent.clone(),
                    name: (*f).into(),
                    order,
                });
                id
            }
        };
        parent = Some(id);
    }
    let lp = match parent {
        Some(f) => ListParent::Folder(f),
        None => ListParent::Space(space.clone()),
    };
    let order = s.lists.values().filter(|l| l.parent == lp).count() as f64;
    Ok(Some(store.create_list(lp, list, order, now_ms)?))
}

/// The space a new list goes into: the one of the open list or folder, else the first.
pub fn space_for_new_list(s: &Snapshot, sel: &Selection) -> Option<SpaceId> {
    let of_parent = |p: &ListParent| match p {
        ListParent::Space(sp) => Some(sp.clone()),
        ListParent::Folder(f) => s.folders.get(f).map(|x| x.space_id.clone()),
    };
    let from_sel = match sel {
        Selection::List(l) => s.lists.get(l).and_then(|x| of_parent(&x.parent)),
        Selection::Folder(f) => s.folders.get(f).map(|x| x.space_id.clone()),
        _ => None,
    };
    from_sel.or_else(|| s.spaces.keys().next().cloned())
}

// ---- the Tasks window: every task, grouped by when ----

/// The Tasks window's groups, top to bottom.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum Group {
    /// Overdue and due today.
    Today,
    /// Due after today.
    Upcoming,
    NoDate,
    Done,
}

impl Group {
    pub fn name(self) -> &'static str {
        match self {
            Group::Today => "Today",
            Group::Upcoming => "Upcoming",
            Group::NoDate => "No date",
            Group::Done => "Done",
        }
    }
}

/// Open tasks in the order the panel's Today and Upcoming use: by due (all-day before timed on
/// a date), then by hand-set order.
fn by_due_then_order(a: &Task, b: &Task) -> std::cmp::Ordering {
    a.due
        .cmp(&b.due)
        .then(a.order.total_cmp(&b.order))
        .then(a.id.cmp(&b.id))
}

/// Every task in the window's four groups, each in its order: open ones by due then order, done
/// ones latest first. Empty groups are kept, so the caller decides what an empty one shows.
pub fn groups(s: &Snapshot, c: &Clock) -> Vec<(Group, Vec<Task>)> {
    let mut out: Vec<(Group, Vec<Task>)> =
        [Group::Today, Group::Upcoming, Group::NoDate, Group::Done]
            .into_iter()
            .map(|g| (g, Vec::new()))
            .collect();
    for t in s.tasks.values() {
        let g = match t.due {
            _ if t.done => 3,
            Some(d) if d.date <= c.today => 0,
            Some(_) => 1,
            None => 2,
        };
        out[g].1.push(t.clone());
    }
    for (g, v) in out.iter_mut() {
        match g {
            Group::Done => {
                v.sort_by(|a, b| b.completed_at.cmp(&a.completed_at).then(a.id.cmp(&b.id)))
            }
            _ => v.sort_by(by_due_then_order),
        }
    }
    out
}

/// One line of the Tasks window's list.
#[derive(Clone, Debug, PartialEq)]
pub enum WinItem {
    /// A group's heading with how many tasks it holds.
    Header(Group, usize),
    Task(Group, Box<Task>),
}

/// The window's list: each group that has tasks under its heading. Done shows only its heading
/// until it is unfolded.
pub fn window_items(groups: Vec<(Group, Vec<Task>)>, done_open: bool) -> Vec<WinItem> {
    let mut out = Vec::new();
    for (g, tasks) in groups {
        if tasks.is_empty() {
            continue;
        }
        out.push(WinItem::Header(g, tasks.len()));
        if g == Group::Done && !done_open {
            continue;
        }
        out.extend(tasks.into_iter().map(|t| WinItem::Task(g, Box::new(t))));
    }
    out
}

/// Alt+Up or Alt+Down on `group[i]`: the `order` writes that move it one place. It moves only
/// past a neighbour with the same due, since the date and time decide everything else. Two
/// different orders are swapped; equal ones (lists number their tasks on their own) renumber
/// that run of same-due tasks. `None` when it can't move.
pub fn reorder(group: &[Task], i: usize, up: bool) -> Option<Vec<(TaskId, f64)>> {
    let j = if up { i.checked_sub(1)? } else { i + 1 };
    let (a, b) = (group.get(i)?, group.get(j)?);
    if a.done || a.due != b.due {
        return None;
    }
    if a.order != b.order {
        return Some(vec![(a.id.clone(), b.order), (b.id.clone(), a.order)]);
    }
    // The run of tasks sharing this due, with the two swapped, numbered on from its first order.
    let start = (0..=i.min(j))
        .rev()
        .take_while(|k| group[*k].due == a.due)
        .last()
        .unwrap_or(i.min(j));
    let end = (i.max(j)..group.len())
        .take_while(|k| group[*k].due == a.due)
        .last()
        .unwrap_or(i.max(j));
    let mut run: Vec<&Task> = group[start..=end].iter().collect();
    run.swap(i - start, j - start);
    let base = group[start].order;
    Some(
        run.into_iter()
            .enumerate()
            .map(|(k, t)| (t.id.clone(), base + k as f64))
            .filter(|(id, o)| group.iter().any(|t| &t.id == id && t.order != *o))
            .collect(),
    )
}

/// An inline edit's title to save: trimmed, and `None` when it is empty or unchanged.
pub fn edited_title(old: &str, new: &str) -> Option<String> {
    let t = new.trim();
    (!t.is_empty() && t != old).then(|| t.to_owned())
}

/// The choices of the window's due menu.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum DuePick {
    Today,
    Tomorrow,
    NextWeek,
    Clear,
}

pub const DUE_PICKS: [DuePick; 4] = [
    DuePick::Today,
    DuePick::Tomorrow,
    DuePick::NextWeek,
    DuePick::Clear,
];

impl DuePick {
    pub fn label(self) -> &'static str {
        match self {
            DuePick::Today => "Today",
            DuePick::Tomorrow => "Tomorrow",
            DuePick::NextWeek => "Next week",
            DuePick::Clear => "No date",
        }
    }
}

/// The due a pick sets. A new date keeps the time of day the task had.
pub fn picked_due(pick: DuePick, old: Option<Due>, today: Date) -> Option<Due> {
    let days = match pick {
        DuePick::Today => 0,
        DuePick::Tomorrow => 1,
        DuePick::NextWeek => 7,
        DuePick::Clear => return None,
    };
    Some(Due {
        date: add_days(today, days),
        time: old.and_then(|d| d.time),
    })
}

/// What Undo puts back after a delete: the same task, made again in its list. The store keeps a
/// delete as a tombstone, so the task comes back under a new id.
pub fn restored(t: &Task) -> NewTask {
    NewTask {
        title: t.title.clone(),
        notes: t.notes.clone(),
        due: t.due,
        repeat: t.repeat,
        assignee: t.assignee.clone(),
        order: t.order,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use app_store::DeviceId;

    fn d(y: i32, m: u8, day: u8) -> Date {
        Date::new(y, m, day).unwrap()
    }

    /// Friday 6 November 2026, 14:30 WIB.
    fn clock() -> Clock {
        Clock {
            today: d(2026, 11, 6),
            minute: 14 * 60 + 30,
            now_ms: 1_793_950_200_000,
            offset_secs: 7 * 3600,
        }
    }

    fn me() -> UserId {
        UserId("dhio".into())
    }

    fn fresh() -> (Store, ListId) {
        let mut st = Store::open_in_memory(DeviceId("pc".into()), me()).unwrap();
        let snap = st.snapshot().unwrap();
        let inbox = ensure_inbox(&mut st, &snap, None, clock().now_ms).unwrap();
        (st, inbox)
    }

    fn parse(s: &Snapshot, text: &str) -> qa::QuickAdd {
        let c = clock();
        let now = qa::Now {
            date: to_qa(c.today),
            time: qa::Time::new(14, 30).unwrap(),
        };
        qa::parse(text, now, &known(s, &["dhio".into()]))
    }

    fn add(st: &mut Store, inbox: &ListId, sel: &Selection, text: &str) -> Task {
        let s = st.snapshot().unwrap();
        let q = parse(&s, text);
        let list = target_list(&s, &q, sel, inbox);
        let id = st
            .create_task(
                &list,
                new_task(&s, &q, sel, &list, &clock()),
                clock().now_ms,
            )
            .unwrap();
        st.task(&id).unwrap().unwrap()
    }

    #[test]
    fn the_first_run_makes_a_private_personal_space_with_an_inbox_and_nothing_else() {
        let (mut st, inbox) = fresh();
        let s = st.snapshot().unwrap();
        assert_eq!(s.spaces.len(), 1);
        let sp = s.spaces.values().next().unwrap();
        assert_eq!((sp.name.as_str(), sp.shared), ("Personal", false));
        assert_eq!(list_name(&s, &inbox), "Inbox");
        assert!(s.tasks.is_empty());
        // A second start finds the same inbox instead of making another.
        assert_eq!(
            ensure_inbox(&mut st, &s, None, clock().now_ms).unwrap(),
            inbox
        );
        assert_eq!(
            ensure_inbox(&mut st, &s, Some(&inbox), clock().now_ms).unwrap(),
            inbox
        );
        assert_eq!(st.snapshot().unwrap().lists.len(), 1);
    }

    #[test]
    fn quick_add_files_by_list_and_folder_and_falls_back_to_the_open_list_then_the_inbox() {
        let (mut st, inbox) = fresh();
        let s = st.snapshot().unwrap();
        let space = s.spaces.keys().next().unwrap().clone();
        let groceries = create_list_at(&mut st, &s, &space, "Home/Groceries", 1)
            .unwrap()
            .unwrap();
        let s = st.snapshot().unwrap();
        let work = create_list_at(&mut st, &s, &space, "Work", 2)
            .unwrap()
            .unwrap();

        let t = add(
            &mut st,
            &inbox,
            &Selection::Inbox,
            "beli galon besok 9am #groceries",
        );
        assert_eq!(t.list_id, groceries);
        assert_eq!(t.title, "beli galon");
        assert_eq!(
            t.due,
            Some(Due {
                date: d(2026, 11, 7),
                time: TimeOfDay::new(9, 0)
            })
        );

        let t = add(&mut st, &inbox, &Selection::Inbox, "sapu /personal/home");
        assert_eq!(t.list_id, groceries, "a folder alone means its first list");

        let t = add(
            &mut st,
            &inbox,
            &Selection::List(work.clone()),
            "rapat lusa jam 3 @dhio",
        );
        assert_eq!((t.list_id.clone(), t.assignee.clone()), (work, Some(me())));
        assert_eq!(
            t.due,
            Some(Due {
                date: d(2026, 11, 8),
                time: TimeOfDay::new(15, 0)
            })
        );

        let t = add(&mut st, &inbox, &Selection::Upcoming, "Kirim invoice");
        assert_eq!((t.list_id, t.due), (inbox.clone(), None));

        let t = add(&mut st, &inbox, &Selection::Today, "Telur 1 kg");
        assert_eq!(
            t.due,
            Some(Due {
                date: clock().today,
                time: None
            }),
            "in Today a task with no date is due today"
        );

        let t = add(
            &mut st,
            &inbox,
            &Selection::Inbox,
            "olahraga tiap senin 7 pagi",
        );
        assert_eq!(
            (t.repeat, t.due.unwrap().date),
            (Repeat::Weekly, d(2026, 11, 9))
        );
    }

    #[test]
    fn the_understood_line_restates_the_parse() {
        let (st, inbox) = fresh();
        let s = st.snapshot().unwrap();
        let q = parse(&s, "beli galon besok 9am @dhio");
        assert_eq!(
            understood(&s, &q, &Selection::Inbox, &inbox, &clock(), &me()),
            "Tomorrow 09:00, in Inbox, for you."
        );
        let q = parse(&s, "setor sampah tiap hari jam 7 malam");
        assert_eq!(
            understood(&s, &q, &Selection::Inbox, &inbox, &clock(), &me()),
            "Every day 19:00, in Inbox."
        );
    }

    #[test]
    fn ticking_a_repeating_task_moves_it_to_its_next_date_instead_of_closing_it() {
        let today = clock().today;
        let fri9 = Due {
            date: today,
            time: TimeOfDay::new(9, 0),
        };
        assert_eq!(
            next_repeat(fri9, Repeat::Weekly, today).unwrap().date,
            d(2026, 11, 13)
        );
        // Left behind for days: the next date is after today, not in the past.
        let old = Due {
            date: d(2026, 11, 1),
            time: None,
        };
        assert_eq!(
            next_repeat(old, Repeat::Daily, today).unwrap().date,
            d(2026, 11, 7)
        );
        assert_eq!(next_repeat(old, Repeat::None, today), None);

        let (mut st, inbox) = fresh();
        let t = add(
            &mut st,
            &inbox,
            &Selection::Inbox,
            "buang sampah setiap hari jam 7 malam",
        );
        complete(&mut st, &t, &clock()).unwrap();
        let after = st.task(&t.id).unwrap().unwrap();
        assert!(!after.done);
        assert_eq!(after.due.unwrap().date, d(2026, 11, 7));
        let once = add(&mut st, &inbox, &Selection::Inbox, "Kirim invoice");
        complete(&mut st, &once, &clock()).unwrap();
        let after = st.task(&once.id).unwrap().unwrap();
        assert!(after.done);
        assert_eq!(after.completed_by, Some(me()));
    }

    #[test]
    fn overdue_counts_past_dates_and_timed_tasks_whose_minute_has_come() {
        let (mut st, inbox) = fresh();
        let c = clock();
        let mk = |st: &mut Store, title: &str, due: Option<Due>| {
            st.create_task(
                &inbox,
                NewTask {
                    title: title.into(),
                    due,
                    ..NewTask::default()
                },
                c.now_ms,
            )
            .unwrap()
        };
        mk(
            &mut st,
            "yesterday all day",
            Some(Due {
                date: d(2026, 11, 5),
                time: None,
            }),
        );
        mk(
            &mut st,
            "today all day",
            Some(Due {
                date: c.today,
                time: None,
            }),
        );
        mk(
            &mut st,
            "today 14:30",
            Some(Due {
                date: c.today,
                time: TimeOfDay::new(14, 30),
            }),
        );
        mk(
            &mut st,
            "today 17:00",
            Some(Due {
                date: c.today,
                time: TimeOfDay::new(17, 0),
            }),
        );
        let done = mk(
            &mut st,
            "done last week",
            Some(Due {
                date: d(2026, 10, 30),
                time: None,
            }),
        );
        st.set_done(&done, true, c.now_ms).unwrap();
        mk(&mut st, "no date", None);
        let s = st.snapshot().unwrap();
        assert_eq!(overdue_count(&s, &c), 2);
        let t = s
            .tasks
            .values()
            .find(|t| t.title == "yesterday all day")
            .unwrap();
        assert_eq!(due_label(t, &c), ("Overdue · Thu".into(), true));
        let t = s.tasks.values().find(|t| t.title == "today 17:00").unwrap();
        assert_eq!(due_label(t, &c), ("Today 17:00".into(), false));
    }

    #[test]
    fn the_due_timer_sees_each_timed_task_once_in_order() {
        let (mut st, inbox) = fresh();
        let c = clock();
        for (title, h, m) in [("b", 17, 0), ("a", 15, 0), ("c", 17, 0)] {
            st.create_task(
                &inbox,
                NewTask {
                    title: title.into(),
                    due: Some(Due {
                        date: c.today,
                        time: TimeOfDay::new(h, m),
                    }),
                    ..NewTask::default()
                },
                c.now_ms,
            )
            .unwrap();
        }
        st.create_task(
            &inbox,
            NewTask {
                title: "all day".into(),
                due: Some(Due {
                    date: c.today,
                    time: None,
                }),
                ..NewTask::default()
            },
            c.now_ms,
        )
        .unwrap();
        let s = st.snapshot().unwrap();
        let now = c.stamp();
        assert_eq!(next_due_after(&s, now), Some((c.today, 15 * 60)));
        assert_eq!(
            next_due_after(&s, (c.today, 15 * 60)),
            Some((c.today, 17 * 60))
        );
        assert_eq!(next_due_after(&s, (c.today, 17 * 60)), None);
        let fired: Vec<String> = due_between(&s, now, (c.today, 17 * 60))
            .into_iter()
            .map(|t| t.title)
            .collect();
        assert_eq!(fired.len(), 3);
        assert_eq!(fired[0], "a");
        assert!(
            due_between(&s, (c.today, 17 * 60), (c.today, 18 * 60)).is_empty(),
            "nothing fires twice"
        );
    }

    #[test]
    fn today_places_the_prayers_still_ahead_as_markers_among_timed_tasks() {
        let (mut st, inbox) = fresh();
        let c = clock();
        for (title, due) in [
            (
                "all day",
                Due {
                    date: c.today,
                    time: None,
                },
            ),
            (
                "17:00",
                Due {
                    date: c.today,
                    time: TimeOfDay::new(17, 0),
                },
            ),
            (
                "old",
                Due {
                    date: d(2026, 11, 2),
                    time: None,
                },
            ),
        ] {
            st.create_task(
                &inbox,
                NewTask {
                    title: title.into(),
                    due: Some(due),
                    ..NewTask::default()
                },
                c.now_ms,
            )
            .unwrap();
        }
        let s = st.snapshot().unwrap();
        let (todo, _) = view_tasks(&s, &Selection::Today, &inbox, &c);
        let prayers = vec![
            ("Dzuhur".to_string(), 11 * 60 + 26),
            ("Ashar".to_string(), 14 * 60 + 40),
            ("Maghrib".to_string(), 17 * 60 + 38),
        ];
        let rows: Vec<String> = today_rows(todo, &prayers, &c)
            .into_iter()
            .map(|r| match r {
                Row::Task(t) => t.title,
                Row::Prayer { name, .. } => format!("[{name}]"),
            })
            .collect();
        assert_eq!(
            rows,
            vec!["old", "all day", "[Ashar]", "17:00", "[Maghrib]"]
        );
        // Ashar's marker stays through its minute and goes the minute after.
        assert_eq!(next_marker_change(&prayers, c.minute), Some(14 * 60 + 41));
        assert_eq!(
            next_marker_change(&prayers, 14 * 60 + 41),
            Some(17 * 60 + 39)
        );
        assert_eq!(next_marker_change(&prayers, 18 * 60), None);
    }

    #[test]
    fn a_repeating_task_names_its_next_date_so_ticking_it_shows_the_move() {
        let (mut st, inbox) = fresh();
        let c = clock();
        let mk = |st: &mut Store, title: &str, date: Date, repeat: Repeat| {
            let id = st
                .create_task(
                    &inbox,
                    NewTask {
                        title: title.into(),
                        due: Some(Due {
                            date,
                            time: TimeOfDay::new(9, 0),
                        }),
                        repeat,
                        ..NewTask::default()
                    },
                    c.now_ms,
                )
                .unwrap();
            st.task(&id).unwrap().unwrap()
        };
        let galon = mk(&mut st, "Galon Aqua", d(2026, 11, 7), Repeat::Weekly);
        assert_eq!(due_label(&galon, &c).0, "Tomorrow 09:00 · every Sat");
        complete(&mut st, &galon, &c).unwrap();
        let galon = st.task(&galon.id).unwrap().unwrap();
        assert_eq!(
            due_label(&galon, &c).0,
            "14 Nov 09:00 · every Sat",
            "this Saturday is skipped, and it says so"
        );
        let gym = mk(&mut st, "Gym", d(2026, 11, 11), Repeat::Weekly);
        assert_eq!(due_label(&gym, &c).0, "Every Wed 09:00");
        let pills = mk(&mut st, "Vitamin", d(2026, 11, 7), Repeat::Daily);
        assert_eq!(due_label(&pills, &c).0, "Tomorrow 09:00 · every day");
    }

    #[test]
    fn the_sidebar_nests_folders_and_lists_and_hides_the_inbox_and_collapsed_children() {
        let (mut st, inbox) = fresh();
        let s = st.snapshot().unwrap();
        let space = s.spaces.keys().next().unwrap().clone();
        let groceries = create_list_at(&mut st, &s, &space, "Home/Groceries", 1)
            .unwrap()
            .unwrap();
        let s = st.snapshot().unwrap();
        create_list_at(&mut st, &s, &space, "home/Bills", 2).unwrap();
        st.create_task(&groceries, NewTask::titled("Beras"), 3)
            .unwrap();
        let s = st.snapshot().unwrap();
        let c = clock();
        let sel = Selection::List(groceries.clone());
        let side = sidebar(&s, &inbox, &HashSet::new(), &sel, &c);
        let names: Vec<(u32, &str, usize)> = side
            .iter()
            .map(|i| (i.depth, i.name.as_str(), i.count))
            .collect();
        assert_eq!(
            names,
            vec![
                (0, "Inbox", 0),
                (0, "Today", 0),
                (0, "Upcoming", 0),
                (0, "Personal", 0),
                (1, "Home", 1),
                (2, "Groceries", 1),
                (2, "Bills", 0)
            ]
        );
        assert!(side[5].selected);
        assert_eq!(side[3].note, "private");
        let home = match &side[4].key {
            SideKey::Folder(f) => f.clone(),
            _ => panic!(),
        };
        let collapsed = HashSet::from([home.0.clone()]);
        assert_eq!(sidebar(&s, &inbox, &collapsed, &sel, &c).len(), 5);
        assert_eq!(
            header(&s, &sel, &inbox, &c),
            ("Personal / Home".into(), "Groceries".into())
        );
        assert_eq!(
            header(&s, &Selection::Folder(home), &inbox, &c),
            ("Personal".into(), "Home".into())
        );
    }

    #[test]
    fn samples_go_only_into_an_empty_store_and_say_so() {
        let (mut st, inbox) = fresh();
        let s = st.snapshot().unwrap();
        assert_eq!(add_samples(&mut st, &s, &inbox, &clock()).unwrap(), 5);
        let s = st.snapshot().unwrap();
        assert!(s.tasks.values().all(|t| t.notes == "Sample task"));
        assert_eq!(add_samples(&mut st, &s, &inbox, &clock()).unwrap(), 0);
    }

    fn titles(v: &[Task]) -> Vec<&str> {
        v.iter().map(|t| t.title.as_str()).collect()
    }

    #[test]
    fn the_window_groups_every_task_by_when_with_done_latest_first() {
        let (mut st, inbox) = fresh();
        let c = clock();
        let mk = |st: &mut Store, title: &str, due: Option<Due>, order: f64| {
            st.create_task(
                &inbox,
                NewTask {
                    title: title.into(),
                    due,
                    order,
                    ..NewTask::default()
                },
                c.now_ms,
            )
            .unwrap()
        };
        let on = |date: Date, time: Option<(u8, u8)>| {
            Some(Due {
                date,
                time: time.and_then(|(h, m)| TimeOfDay::new(h, m)),
            })
        };
        mk(&mut st, "today 17:00", on(c.today, Some((17, 0))), 0.0);
        mk(&mut st, "today all day", on(c.today, None), 5.0);
        mk(&mut st, "overdue", on(d(2026, 11, 2), None), 9.0);
        mk(&mut st, "tomorrow", on(d(2026, 11, 7), None), 0.0);
        mk(&mut st, "no date b", None, 2.0);
        mk(&mut st, "no date a", None, 1.0);
        let first = mk(&mut st, "done first", None, 3.0);
        let second = mk(&mut st, "done second", None, 4.0);
        st.set_done(&first, true, c.now_ms).unwrap();
        st.set_done(&second, true, c.now_ms + 60_000).unwrap();
        let g = groups(&st.snapshot().unwrap(), &c);
        let names: Vec<Group> = g.iter().map(|(g, _)| *g).collect();
        assert_eq!(
            names,
            vec![Group::Today, Group::Upcoming, Group::NoDate, Group::Done]
        );
        assert_eq!(
            titles(&g[0].1),
            vec!["overdue", "today all day", "today 17:00"],
            "overdue first, then all-day before timed"
        );
        assert_eq!(titles(&g[1].1), vec!["tomorrow"]);
        assert_eq!(titles(&g[2].1), vec!["no date a", "no date b"]);
        assert_eq!(titles(&g[3].1), vec!["done second", "done first"]);

        // Done is folded by default: its heading and count, no rows. Empty groups are left out.
        let folded = window_items(g.clone(), false);
        assert_eq!(folded.last(), Some(&WinItem::Header(Group::Done, 2)));
        assert_eq!(folded.len(), (1 + 3) + (1 + 1) + (1 + 2) + 1);
        assert_eq!(window_items(g.clone(), true).len(), folded.len() + 2);
        let only_inbox: Vec<(Group, Vec<Task>)> = g
            .into_iter()
            .map(|(gr, v)| (gr, if gr == Group::NoDate { v } else { Vec::new() }))
            .collect();
        assert_eq!(
            window_items(only_inbox, true)[0],
            WinItem::Header(Group::NoDate, 2)
        );
    }

    fn task(id: &str, order: f64, due: Option<Due>) -> Task {
        Task {
            id: TaskId(id.into()),
            list_id: ListId("l".into()),
            title: id.into(),
            notes: String::new(),
            due,
            repeat: Repeat::None,
            assignee: None,
            done: false,
            created_by: me(),
            completed_by: None,
            created_at: 0,
            completed_at: None,
            order,
        }
    }

    fn apply(group: &mut [Task], writes: &[(TaskId, f64)]) {
        for (id, o) in writes {
            group.iter_mut().find(|t| &t.id == id).unwrap().order = *o;
        }
        group.sort_by(by_due_then_order);
    }

    #[test]
    fn alt_arrows_move_a_task_one_place_among_tasks_with_the_same_due() {
        let mut g = vec![
            task("a", 0.0, None),
            task("b", 1.0, None),
            task("c", 4.0, None),
        ];
        let w = reorder(&g, 2, true).unwrap();
        assert_eq!(w.len(), 2, "two different orders are just swapped");
        apply(&mut g, &w);
        assert_eq!(titles(&g), vec!["a", "c", "b"]);
        assert_eq!(reorder(&g, 0, true), None, "the top can't go up");
        assert_eq!(reorder(&g, 2, false), None, "the bottom can't go down");

        // Lists number their own tasks, so two lists' tasks often share an order.
        let mut g = vec![
            task("a", 0.0, None),
            task("b", 0.0, None),
            task("c", 0.0, None),
        ];
        let w = reorder(&g, 0, false).unwrap();
        apply(&mut g, &w);
        assert_eq!(titles(&g), vec!["b", "a", "c"]);
        let w = reorder(&g, 2, true).unwrap();
        apply(&mut g, &w);
        assert_eq!(titles(&g), vec!["b", "c", "a"]);

        // A different due is a wall: the date decides there.
        let today = Some(Due {
            date: clock().today,
            time: None,
        });
        let at9 = Some(Due {
            date: clock().today,
            time: TimeOfDay::new(9, 0),
        });
        let g = vec![task("a", 0.0, today), task("b", 0.0, at9)];
        assert_eq!(reorder(&g, 1, true), None);
        assert_eq!(reorder(&g, 0, false), None);
    }

    #[test]
    fn reordering_writes_the_store_and_the_panel_sees_the_same_order() {
        let (mut st, inbox) = fresh();
        for t in ["Beras", "Telur", "Gula"] {
            add(&mut st, &inbox, &Selection::Inbox, t);
        }
        let c = clock();
        let s = st.snapshot().unwrap();
        let no_date = groups(&s, &c).remove(2).1;
        for (id, o) in reorder(&no_date, 2, true).unwrap() {
            st.edit_task(&id, TaskEdit::Order(o), c.now_ms).unwrap();
        }
        let s = st.snapshot().unwrap();
        assert_eq!(titles(&groups(&s, &c)[2].1), vec!["Beras", "Gula", "Telur"]);
        let (todo, _) = view_tasks(&s, &Selection::Inbox, &inbox, &c);
        assert_eq!(titles(&todo), vec!["Beras", "Gula", "Telur"]);
    }

    #[test]
    fn an_inline_edit_saves_a_trimmed_new_title_and_nothing_else() {
        assert_eq!(
            edited_title("Beras", "  Beras 5 kg "),
            Some("Beras 5 kg".into())
        );
        assert_eq!(edited_title("Beras", "Beras "), None, "unchanged");
        assert_eq!(edited_title("Beras", "   "), None, "a task keeps a title");
    }

    #[test]
    fn the_due_menu_sets_a_day_keeps_the_time_and_clears() {
        let today = clock().today;
        let at9 = Some(Due {
            date: d(2026, 11, 2),
            time: TimeOfDay::new(9, 0),
        });
        assert_eq!(
            picked_due(DuePick::Tomorrow, at9, today),
            Some(Due {
                date: d(2026, 11, 7),
                time: TimeOfDay::new(9, 0)
            })
        );
        assert_eq!(
            picked_due(DuePick::NextWeek, None, today),
            Some(Due {
                date: d(2026, 11, 13),
                time: None
            })
        );
        assert_eq!(picked_due(DuePick::Today, None, today).unwrap().date, today);
        assert_eq!(picked_due(DuePick::Clear, at9, today), None);
    }

    #[test]
    fn undo_after_a_delete_makes_the_same_task_again() {
        let (mut st, inbox) = fresh();
        let t = add(&mut st, &inbox, &Selection::Inbox, "Galon Aqua besok 9am");
        st.delete(&app_store::EntityRef::Task(t.id.clone()), 1)
            .unwrap();
        assert!(st.snapshot().unwrap().tasks.is_empty());
        let id = st.create_task(&t.list_id, restored(&t), 2).unwrap();
        let back = st.task(&id).unwrap().unwrap();
        assert_eq!(
            (back.title, back.due, back.order, back.list_id),
            (t.title, t.due, t.order, t.list_id)
        );
    }

    #[test]
    fn completion_times_show_in_local_time() {
        let c = clock();
        // 2026-11-06 07:30 UTC is 14:30 WIB.
        assert_eq!(c.hhmm_of_ms(c.now_ms), "14:30");
        assert_eq!(c.day_of_ms(c.now_ms).0, c.today);
    }
}
