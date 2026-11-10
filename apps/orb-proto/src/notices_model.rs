//! The notice queue (docs/design/notices.md, frames `Y_alKzOsBY` and `IcwMYj4XTv`). Every pill
//! from every source comes through here: one pill in front, the rest wait by priority, then age.
//! Quiet times (focus, full screen, a pause, the orb hidden) hold normal notices; urgent ones
//! still show. Earlier today lists what showed or waited, and never holds a prayer notice.
//!
//! Pure: no window, no clock. `Now` carries the wall clock (for days, staleness and cooldowns) and
//! a monotonic millisecond count (for pacing).

use std::collections::BTreeMap;
use std::time::Duration;

pub use app_habits::held::Priority;
use app_habits::Moment;
use serde::{Deserialize, Serialize};

use crate::ui::{Pill, PillAction};

/// A new pill waits at least this long after the last one appeared.
pub const PACING_MS: u64 = 1500;
/// Clear all can be undone this long.
pub const UNDO_CLEAR: Duration = Duration::from_secs(6);
/// A pill that stays until you close it ("10 menit lagi Isya", the adzan playing). This is the
/// longest a Windows timer waits (USER_TIMER_MAXIMUM, about 24 days), so the pill's own clock
/// never ends it; the prayer time does.
pub const STICKY_MS: u32 = 0x7FFF_FFFF;
/// A prayer reminder held this long past its moment says the wrong thing ("30 menit lagi" with
/// 18 minutes left), so it is dropped instead of shown late.
pub const PRAYER_GRACE: Duration = Duration::from_secs(120);
/// The countdown's words change this many seconds after each whole minute before the prayer, so
/// a reminder fired a moment late at −10 still says "10 menit lagi".
const COUNTDOWN_SLACK_SECS: i64 = 2;

/// The sticky reminder's words with `secs_left` until the prayer: "9 menit lagi Maghrib" down to
/// "1 menit lagi Maghrib", then "Maghrib sebentar lagi" in the last minute.
pub fn reminder_title(prayer: &str, secs_left: i64) -> String {
    match (secs_left.max(0) + COUNTDOWN_SLACK_SECS) / 60 {
        0 => format!("{prayer} sebentar lagi"),
        m => format!("{m} menit lagi {prayer}"),
    }
}

/// Seconds until `reminder_title` next says something else, or `None` once it reads "sebentar
/// lagi" ("Memasuki waktu" replaces it at the time).
pub fn countdown_next_secs(secs_left: i64) -> Option<i64> {
    let s = secs_left.max(0) + COUNTDOWN_SLACK_SECS;
    (s >= 60).then_some(s % 60 + 1)
}

#[derive(Clone, Copy, Debug)]
pub struct Now {
    pub at: Moment,
    pub ms: u64,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
pub enum Source {
    Prayer,
    Calendar,
    Tasks,
    Drink,
    Focus,
    Downloads,
    Disk,
    /// The app's own words (summaries, confirmations): never muted, never listed.
    App,
}

impl Source {
    pub fn name(self) -> &'static str {
        match self {
            Source::Prayer => "Prayer",
            Source::Calendar => "Calendar",
            Source::Tasks => "Tasks",
            Source::Drink => "Drink",
            Source::Focus => "Focus",
            Source::Downloads => "Downloads",
            Source::Disk => "Disk space",
            Source::App => "Notices",
        }
    }

    /// "Don't show downloads again".
    pub fn noun(self) -> &'static str {
        match self {
            Source::Calendar => "calendar",
            Source::Tasks => "tasks",
            Source::Drink => "drink",
            Source::Focus => "focus",
            Source::Downloads => "downloads",
            Source::Disk => "disk space",
            Source::Prayer | Source::App => "",
        }
    }

    /// Prayer at the time can't be muted (only its adzan sound can), and the app's own words
    /// answer something you did.
    pub fn mutable(self) -> bool {
        !matches!(self, Source::Prayer | Source::App)
    }
}

/// What happens to a notice that cannot show now.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Keep {
    /// It waits in Earlier today (and on the badge) until its turn, or until you open it.
    List,
    /// Never listed (prayer reminders, focus phases): it waits unlisted and comes as a pill once
    /// the quiet time ends, unless its moment has passed.
    Moment,
    /// Never listed and never waits for a quiet time: it is dropped (the clipboard offer).
    Drop,
}

/// Notices of one kind from one source merge: "3 downloads done", with one action for all.
#[derive(Clone, Debug, PartialEq)]
pub struct Merge {
    pub key: &'static str,
    /// The words after the count: "downloads done". Empty: the newest simply replaces the one
    /// waiting (a second "Minum dulu" says nothing new).
    pub many: String,
    pub value: String,
    pub action: Option<(String, PillAction)>,
}

#[derive(Clone, Debug)]
pub struct Notice {
    pub source: Source,
    pub priority: Priority,
    pub keep: Keep,
    pub pill: Pill,
    pub merge: Option<Merge>,
    /// Gone once this passes ("10 menit lagi Maghrib" at Maghrib, "Standup in 10 min" at its start).
    pub stale_at: Option<Moment>,
    /// Dropped if it has not shown by then (cleared once it shows): a held "30 menit lagi" is
    /// never shown late.
    pub show_by: Option<Moment>,
    /// A reminder ahead of this prayer: "Memasuki waktu" drops it wherever it waits.
    pub prayer: Option<String>,
    /// After a pill of this kind shows, the next one waits this long ("Disk space: once a day").
    pub cooldown: Option<(&'static str, Duration)>,
    /// A prayer reminder: only presenting holds it. Focus, full screen, a pause and the orb
    /// hidden for a full-screen app let it through (silently in full screen).
    pub through: bool,
    /// Stays until you close it. Covered by a panel or the dial, it comes back; a normal notice
    /// arriving takes the front for its turn, then this one comes back.
    pub sticky: bool,
    /// Its title counts the minutes to `prayer` at `stale_at` ("9 menit lagi Maghrib"), set
    /// whenever it shows and once a minute while it is on screen.
    pub countdown: bool,
}

impl Notice {
    pub fn new(source: Source, priority: Priority, pill: Pill) -> Notice {
        // Prayer notices never stay in a list, so they can never read as a prayer log.
        let keep = if matches!(source, Source::Prayer | Source::App) {
            Keep::Moment
        } else {
            Keep::List
        };
        Notice {
            source,
            priority,
            keep,
            pill,
            merge: None,
            stale_at: None,
            show_by: None,
            prayer: None,
            cooldown: None,
            through: false,
            sticky: false,
            countdown: false,
        }
    }

    pub fn keep(self, keep: Keep) -> Notice {
        Notice { keep, ..self }
    }

    pub fn merging(self, merge: Merge) -> Notice {
        Notice {
            merge: Some(merge),
            ..self
        }
    }

    pub fn stale_at(self, at: Moment) -> Notice {
        Notice {
            stale_at: Some(at),
            ..self
        }
    }

    /// "10 menit lagi Maghrib" replaces a waiting "30 menit lagi Maghrib". `prayer_at` is the
    /// prayer's time and `now` when the reminder fired: it is gone at the prayer time, and if it
    /// has not shown within `PRAYER_GRACE` of firing it is dropped, so it never shows late.
    pub fn before_prayer(self, prayer: &str, prayer_at: Moment, now: Moment) -> Notice {
        Notice {
            prayer: Some(prayer.to_string()),
            stale_at: Some(prayer_at),
            show_by: Some(now + PRAYER_GRACE),
            through: true,
            ..self
        }
        .replacing("before-prayer")
    }

    /// Stays until you close it.
    pub fn sticky(self) -> Notice {
        Notice {
            sticky: true,
            ..self
        }
    }

    /// Its title counts down the minutes to the prayer (needs `before_prayer`).
    pub fn counting_down(self) -> Notice {
        Notice {
            countdown: true,
            ..self
        }
    }

    /// Seconds from `now` to its prayer, for a countdown notice.
    fn countdown_left(&self, now: Moment) -> Option<i64> {
        match (&self.prayer, self.stale_at) {
            (Some(_), Some(at)) if self.countdown => Some(at.since(now).as_secs() as i64),
            _ => None,
        }
    }

    /// The newest of this kind replaces the one waiting.
    pub fn replacing(self, key: &'static str) -> Notice {
        Notice {
            merge: Some(Merge {
                key,
                many: String::new(),
                value: String::new(),
                action: None,
            }),
            ..self
        }
    }

    pub fn cooldown(self, kind: &'static str, d: Duration) -> Notice {
        Notice {
            cooldown: Some((kind, d)),
            ..self
        }
    }

    fn listed(&self) -> bool {
        self.keep == Keep::List
    }

    fn merges_with(&self, other: &Notice) -> bool {
        matches!((&self.merge, &other.merge), (Some(a), Some(b)) if a.key == b.key)
            && self.source == other.source
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum State {
    /// Comes as a pill when its turn comes.
    Queued,
    /// Arrived in a quiet time: waits in the list (and on the badge) until you open it.
    Held,
    /// Showed, or you marked it seen. Earlier today.
    Seen,
}

#[derive(Clone, Debug)]
pub struct Entry {
    pub id: u64,
    pub notice: Notice,
    pub at: Moment,
    pub count: u32,
    pub state: State,
}

impl Entry {
    /// The pill as it shows: a merged entry says how many.
    pub fn pill(&self) -> Pill {
        match (&self.notice.merge, self.count) {
            (Some(m), n) if n > 1 && !m.many.is_empty() => Pill {
                title: format!("{n} {}", m.many),
                value: m.value.clone(),
                action: m.action.clone(),
                ..self.notice.pill.clone()
            },
            _ => self.notice.pill.clone(),
        }
    }

    fn waiting(&self) -> bool {
        matches!(self.state, State::Queued | State::Held)
    }

    fn fresh(&self, now: Moment) -> bool {
        self.notice.stale_at.is_none_or(|s| now < s) && self.notice.show_by.is_none_or(|s| now < s)
    }

    /// It is on screen: from now on only `stale_at` ends it. A countdown says the minutes left
    /// now, not when it fired (one held by presenting shows "8 menit lagi", never "10").
    fn shown(&mut self, now: Moment) {
        self.notice.show_by = None;
        self.retitle(now);
    }

    /// A countdown's words for `now`. Returns whether they changed.
    fn retitle(&mut self, now: Moment) -> bool {
        let n = &mut self.notice;
        let (Some(left), Some(prayer)) = (n.countdown_left(now), n.prayer.as_deref()) else {
            return false;
        };
        let title = reminder_title(prayer, left);
        if n.pill.title == title {
            return false;
        }
        n.pill.title = title;
        true
    }
}

/// The quiet times that hold notices.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Quiet {
    /// The orb is hidden: what waits adds to the badge.
    pub hidden: bool,
    /// Full screen or presenting on the orb's monitor: no sound, no shake.
    pub full_screen: bool,
    /// A focus session holds normal notices until the break.
    pub focus: bool,
    /// Pause notices (30 min, 1 hour, until tomorrow).
    pub paused: bool,
    /// Presenting (Windows presentation mode): the one quiet time that holds prayer reminders.
    pub presenting: bool,
}

impl Quiet {
    pub fn holds(&self) -> bool {
        self.hidden || self.full_screen || self.focus || self.paused || self.presenting
    }

    /// Whether this quiet time holds `n`. Prayer reminders wait only while you present.
    pub fn holds_notice(&self, n: &Notice) -> bool {
        if n.through {
            self.presenting
        } else {
            self.holds()
        }
    }

    /// After these, what waited comes back as one summary. Being hidden ends with the badge only.
    fn summarizes(&self) -> bool {
        self.full_screen || self.focus || self.paused || self.presenting
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum Mute {
    /// Unix seconds.
    Until(i64),
    Always,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum MuteFor {
    Hour,
    Tomorrow,
    Always,
}

/// What `offer` did with a notice.
#[derive(Clone, Debug, PartialEq)]
pub enum Offered {
    /// Put it on screen now.
    Show(Pill),
    /// It merged into the pill on screen: new words in place.
    Update(Pill),
    /// It waits its turn as a pill.
    Queued,
    /// A quiet time holds it.
    Held,
    /// Quiet priority while something else shows or a quiet time: straight to the list.
    Listed,
    Muted,
    Dropped,
}

/// What `next` says when the slot is free.
#[derive(Clone, Debug, PartialEq)]
pub enum Next {
    Show(Pill, Priority),
    /// Nothing may show yet: ask again after this many milliseconds.
    Wait(u64),
    Idle,
}

/// A quiet time that summarizes just ended.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Release {
    /// Listed notices that waited through it.
    pub waited: usize,
    /// Focus was one of the quiet times that ended (its break pill carries the count).
    pub focus: bool,
}

#[derive(Clone, Debug)]
enum Front {
    Entry(Box<Entry>),
    /// An answer to something you just did: shows at once, never listed.
    Reply,
}

#[derive(Default)]
pub struct Queue {
    entries: Vec<Entry>,
    next_id: u64,
    front: Option<Front>,
    last_shown_ms: Option<u64>,
    /// When a kind with a cooldown last showed (unix seconds, saved).
    pub cooldowns: BTreeMap<String, i64>,
    pub mutes: BTreeMap<Source, Mute>,
    quiet: Quiet,
    quiet_since: Option<Moment>,
    cleared: Option<(Vec<Entry>, Moment)>,
}

fn rank(p: Priority) -> u8 {
    match p {
        Priority::Urgent => 0,
        Priority::Normal => 1,
        Priority::Quiet => 2,
    }
}

impl Queue {
    pub fn new() -> Queue {
        Queue::default()
    }

    fn add(&mut self, notice: Notice, at: Moment, state: State) -> u64 {
        self.next_id += 1;
        self.entries.push(Entry {
            id: self.next_id,
            notice,
            at,
            count: 1,
            state,
        });
        self.next_id
    }

    pub fn muted(&self, s: Source, unix_now: i64) -> bool {
        match self.mutes.get(&s) {
            Some(Mute::Always) => true,
            Some(Mute::Until(t)) => unix_now < *t,
            None => false,
        }
    }

    /// Seconds until a cooldown kind may show again (0: now).
    fn cooldown_left(&self, n: &Notice, unix_now: i64) -> i64 {
        match n.cooldown {
            Some((kind, d)) => self
                .cooldowns
                .get(kind)
                .map_or(0, |last| (last + d.as_secs() as i64 - unix_now).max(0)),
            None => 0,
        }
    }

    fn mark_shown(&mut self, n: &Notice, now: Now, unix_now: i64) {
        self.last_shown_ms = Some(now.ms);
        if let Some((kind, _)) = n.cooldown {
            self.cooldowns.insert(kind.to_string(), unix_now);
        }
    }

    /// The pill on screen goes back in line (it was interrupted before it was read).
    fn requeue_front(&mut self) {
        if let Some(Front::Entry(e)) = self.front.take() {
            let mut e = *e;
            if e.notice.keep != Keep::Drop {
                e.state = State::Queued;
                self.entries.push(e);
            }
        }
    }

    fn front_entry(&mut self) -> Option<&mut Entry> {
        match &mut self.front {
            Some(Front::Entry(e)) => Some(&mut **e),
            _ => None,
        }
    }

    pub fn front_is_urgent(&self) -> bool {
        matches!(&self.front, Some(Front::Entry(e)) if e.notice.priority == Priority::Urgent)
    }

    /// The pill in front shows even while the orb is hidden (urgent, or a prayer reminder).
    pub fn front_breaks_through(&self) -> bool {
        matches!(&self.front, Some(Front::Entry(e))
            if e.notice.priority == Priority::Urgent || e.notice.through)
    }

    /// The countdown pill in front, its words for `now`: the new pill if they changed. Only the
    /// title changes; the pill keeps its place in front.
    pub fn countdown_tick(&mut self, now: Moment) -> Option<Pill> {
        let e = self.front_entry()?;
        e.retitle(now).then(|| e.pill())
    }

    /// How long until the countdown pill in front says something else (`None`: no countdown in
    /// front, or it reads "sebentar lagi" until "Memasuki waktu" replaces it).
    pub fn countdown_wait(&self, now: Moment) -> Option<Duration> {
        match &self.front {
            Some(Front::Entry(e)) => e
                .notice
                .countdown_left(now)
                .and_then(countdown_next_secs)
                .map(|s| Duration::from_secs(s as u64)),
            _ => None,
        }
    }

    /// A sticky normal pill in front gives the slot to a new notice and comes back after it.
    fn front_yields(&self) -> bool {
        matches!(&self.front, Some(Front::Entry(e))
            if e.notice.sticky && e.notice.priority != Priority::Urgent)
    }

    /// A notice arrives. `q` is the quiet time right now; `slot_busy` says a pill (perhaps one
    /// closing) is on screen.
    pub fn offer(
        &mut self,
        n: Notice,
        now: Now,
        unix_now: i64,
        q: Quiet,
        slot_busy: bool,
    ) -> Offered {
        if n.source.mutable() && self.muted(n.source, unix_now) {
            return Offered::Muted;
        }
        if n.priority != Priority::Urgent && n.merge.is_some() {
            if let Some(e) = self.front_entry().filter(|e| e.notice.merges_with(&n)) {
                e.count += 1;
                e.notice = n;
                e.shown(now.at);
                return Offered::Update(e.pill());
            }
            // Into one waiting its turn, or one held by the quiet time that still holds. One left
            // in the list by an earlier quiet time stays as it was: the new one must still show.
            let joins = |e: &Entry| {
                (e.state == State::Queued || (e.state == State::Held && q.holds_notice(&n)))
                    && e.notice.merges_with(&n)
            };
            if let Some(e) = self.entries.iter_mut().find(|e| joins(e)) {
                e.count += 1;
                e.notice = n;
                // It reads as the newest (and counts in this quiet time's summary).
                e.at = now.at;
                return if e.state == State::Held {
                    Offered::Held
                } else {
                    Offered::Queued
                };
            }
            // A newer reminder supersedes an older one still waiting in the list.
            if n.merge.as_ref().is_some_and(|m| m.many.is_empty()) {
                for e in self
                    .entries
                    .iter_mut()
                    .filter(|e| e.state == State::Held && e.notice.merges_with(&n))
                {
                    e.state = State::Seen;
                }
            }
        }
        match n.priority {
            // Urgent jumps to the front and never merges; it waits only behind another urgent.
            Priority::Urgent => {
                if self.front_is_urgent() {
                    self.add(n, now.at, State::Queued);
                    return Offered::Queued;
                }
                self.requeue_front();
                self.show(n, now, unix_now)
            }
            Priority::Quiet => {
                if slot_busy || q.holds() {
                    self.add(n, now.at, State::Seen);
                    Offered::Listed
                } else {
                    self.show(n, now, unix_now)
                }
            }
            Priority::Normal => {
                if q.holds_notice(&n) {
                    return self.hold(n, now);
                }
                if slot_busy && self.front_yields() && !n.sticky {
                    // The sticky pill waits behind this one for one turn, then comes back (it is
                    // older, so it is next in line).
                    self.requeue_front();
                    return self.show(n, now, unix_now);
                }
                let paced = self.last_shown_ms.is_some_and(|l| now.ms < l + PACING_MS);
                if slot_busy
                    || paced
                    || self.cooldown_left(&n, unix_now) > 0
                    || self.entries.iter().any(|e| e.state == State::Queued)
                {
                    self.add(n, now.at, State::Queued);
                    Offered::Queued
                } else {
                    self.show(n, now, unix_now)
                }
            }
        }
    }

    fn hold(&mut self, n: Notice, now: Now) -> Offered {
        if n.keep == Keep::Drop {
            return Offered::Dropped;
        }
        self.add(n, now.at, State::Held);
        Offered::Held
    }

    fn show(&mut self, n: Notice, now: Now, unix_now: i64) -> Offered {
        self.mark_shown(&n, now, unix_now);
        self.next_id += 1;
        let mut e = Entry {
            id: self.next_id,
            notice: n,
            at: now.at,
            count: 1,
            state: State::Seen,
        };
        e.shown(now.at);
        let pill = e.pill();
        self.front = Some(Front::Entry(Box::new(e)));
        Offered::Show(pill)
    }

    /// An answer to something you just did shows at once; what it covers goes back in line.
    pub fn reply(&mut self, now: Now) {
        self.requeue_front();
        self.front = Some(Front::Reply);
        self.last_shown_ms = Some(now.ms);
    }

    /// The pill on screen starts to leave. `acted`: its action was taken, so it is done with.
    /// A sticky pill that you did not close (a panel or the dial covered it) goes back in line.
    pub fn close_front(&mut self, acted: bool) {
        self.close_front_as(acted, false);
    }

    /// The pill on screen leaves because you closed it (its X): a sticky one is done with too.
    pub fn close_front_by_you(&mut self) {
        self.close_front_as(false, true);
    }

    fn close_front_as(&mut self, acted: bool, by_you: bool) {
        if matches!(&self.front, Some(Front::Entry(e)) if e.notice.sticky && !acted && !by_you) {
            self.requeue_front();
            return;
        }
        if let Some(Front::Entry(mut e)) = self.front.take() {
            if e.notice.listed() && !acted {
                e.state = State::Seen;
                self.entries.push(*e);
            }
        }
    }

    /// Drops what went stale or belongs to an earlier day.
    fn tidy(&mut self, now: Moment) {
        let today = now.date();
        self.entries.retain(|e| e.at.date() == today);
        for e in &mut self.entries {
            if e.waiting() && !e.fresh(now) {
                e.state = State::Seen;
            }
        }
        // An unlisted notice that is no longer waiting has nowhere to be.
        self.entries.retain(|e| e.notice.listed() || e.waiting());
    }

    /// The slot is free: the next pill, or how long until one may show.
    pub fn next(&mut self, now: Now, unix_now: i64, q: Quiet) -> Next {
        self.tidy(now.at);
        if q.holds() {
            // A quiet time began while these waited their turn: they wait for it to end instead.
            for e in &mut self.entries {
                if e.state == State::Queued
                    && e.notice.priority != Priority::Urgent
                    && q.holds_notice(&e.notice)
                {
                    e.state = State::Held;
                }
            }
            self.entries
                .retain(|e| !(e.state == State::Held && e.notice.keep == Keep::Drop));
        }
        let mut order: Vec<usize> = (0..self.entries.len())
            .filter(|&i| self.entries[i].state == State::Queued)
            .collect();
        order.sort_by_key(|&i| {
            (
                rank(self.entries[i].notice.priority),
                self.entries[i].at,
                self.entries[i].id,
            )
        });
        let mut wait: Option<u64> = None;
        for i in order {
            let left = self.ready_in(&self.entries[i], now, unix_now);
            if left == 0 {
                let mut e = self.entries.remove(i);
                let p = e.notice.priority;
                self.mark_shown(&e.notice, now, unix_now);
                e.state = State::Seen;
                e.shown(now.at);
                let pill = e.pill();
                self.front = Some(Front::Entry(Box::new(e)));
                return Next::Show(pill, p);
            }
            wait = Some(wait.map_or(left, |w: u64| w.min(left)));
        }
        match wait {
            Some(ms) => Next::Wait(ms),
            None => Next::Idle,
        }
    }

    /// Milliseconds until a queued entry may show: pacing and its cooldown (urgent: none).
    fn ready_in(&self, e: &Entry, now: Now, unix_now: i64) -> u64 {
        if e.notice.priority == Priority::Urgent {
            return 0;
        }
        let paced = self
            .last_shown_ms
            .map_or(0, |l| (l + PACING_MS).saturating_sub(now.ms));
        paced.max(self.cooldown_left(&e.notice, unix_now) as u64 * 1000)
    }

    /// How long until the first queued pill may show, without taking it.
    pub fn wait_ms(&self, now: Now, unix_now: i64) -> Option<u64> {
        self.entries
            .iter()
            .filter(|e| e.state == State::Queued)
            .map(|e| self.ready_in(e, now, unix_now))
            .min()
    }

    /// Records the quiet time. When one that summarizes ends, says how many listed notices
    /// waited through it; unlisted ones that are still current go back in line as pills.
    pub fn set_quiet(&mut self, q: Quiet, now: Moment) -> Option<Release> {
        let was = self.quiet;
        self.quiet = q;
        if q.summarizes() && !was.summarizes() {
            self.quiet_since = Some(now);
        }
        // Unlisted notices still current go back in line once nothing holds them (a prayer
        // reminder as soon as presenting ends, even while focus runs).
        let mut released = false;
        for e in &mut self.entries {
            if e.state == State::Held && !e.notice.listed() && !q.holds_notice(&e.notice) {
                e.state = State::Queued;
                released = true;
            }
        }
        if released || !q.holds() {
            self.tidy(now);
        }
        if was.summarizes() && !q.summarizes() {
            let since = self.quiet_since.take();
            let waited = self
                .entries
                .iter()
                .filter(|e| {
                    e.state == State::Held && e.notice.listed() && since.is_none_or(|s| e.at >= s)
                })
                .count();
            return Some(Release {
                waited,
                focus: was.focus && !q.focus,
            });
        }
        None
    }

    /// Listed notices held since the current quiet time began ("2 held").
    pub fn held_now(&self) -> usize {
        match self.quiet_since {
            Some(s) => self
                .entries
                .iter()
                .filter(|e| e.state == State::Held && e.notice.listed() && e.at >= s)
                .count(),
            None => 0,
        }
    }

    /// The orb's count: listed notices waiting behind the front pill.
    pub fn waiting(&self) -> usize {
        self.entries
            .iter()
            .filter(|e| e.waiting() && e.notice.listed())
            .count()
    }

    /// What waits, in the order it would show (for the stack's spread).
    pub fn waiting_entries(&self) -> Vec<&Entry> {
        let mut v: Vec<&Entry> = self
            .entries
            .iter()
            .filter(|e| e.waiting() && e.notice.listed())
            .collect();
        v.sort_by_key(|e| {
            (
                e.state != State::Queued,
                rank(e.notice.priority),
                e.at,
                e.id,
            )
        });
        v
    }

    /// Earlier today, newest first: (waiting, seen).
    pub fn list(&self) -> (Vec<&Entry>, Vec<&Entry>) {
        let mut listed: Vec<&Entry> = self.entries.iter().filter(|e| e.notice.listed()).collect();
        listed.sort_by_key(|e| std::cmp::Reverse((e.at, e.id)));
        listed.into_iter().partition(|e| e.waiting())
    }

    /// Opens a listed notice: it becomes the pill in front (what was there goes back in line).
    pub fn open(&mut self, id: u64, now: Now) -> Option<Pill> {
        let i = self.entries.iter().position(|e| e.id == id)?;
        let mut e = self.entries.remove(i);
        self.requeue_front();
        self.last_shown_ms = Some(now.ms);
        e.state = State::Seen;
        e.shown(now.at);
        let pill = e.pill();
        self.front = Some(Front::Entry(Box::new(e)));
        Some(pill)
    }

    /// Takes a waiting notice out of line once its action ran from the stack.
    pub fn acted(&mut self, id: u64) {
        self.entries.retain(|e| e.id != id);
    }

    pub fn mark_seen(&mut self, id: u64) {
        if let Some(e) = self.entries.iter_mut().find(|e| e.id == id) {
            e.state = State::Seen;
        }
    }

    pub fn source_of(&self, id: u64) -> Option<Source> {
        self.entries
            .iter()
            .find(|e| e.id == id)
            .map(|e| e.notice.source)
    }

    /// Empties the list and the badge (unlisted reminders still come). Undo for 6 s.
    pub fn clear_all(&mut self, now: Moment) {
        let (gone, keep): (Vec<Entry>, Vec<Entry>) =
            self.entries.drain(..).partition(|e| e.notice.listed());
        self.entries = keep;
        self.cleared = Some((gone, now));
    }

    pub fn can_undo_clear(&self, now: Moment) -> bool {
        self.cleared
            .as_ref()
            .is_some_and(|(_, at)| now.since(*at) < UNDO_CLEAR)
    }

    pub fn undo_clear(&mut self, now: Moment) -> bool {
        if !self.can_undo_clear(now) {
            return false;
        }
        let (gone, _) = self.cleared.take().unwrap();
        self.entries.extend(gone);
        true
    }

    /// Mutes a source; what of it still waits is marked seen. Returns the mute it replaced.
    pub fn mute(&mut self, s: Source, m: Mute) -> Option<Mute> {
        for e in &mut self.entries {
            if e.notice.source == s && e.waiting() {
                e.state = State::Seen;
            }
        }
        self.entries
            .retain(|e| e.notice.listed() || e.notice.source != s);
        self.mutes.insert(s, m)
    }

    pub fn set_mute(&mut self, s: Source, m: Option<Mute>) {
        match m {
            Some(m) => self.mutes.insert(s, m),
            None => self.mutes.remove(&s),
        };
    }

    /// Mutes still in force, with their words ("for good", "until 15:20", "until tomorrow").
    pub fn active_mutes(&self, unix_now: i64) -> Vec<(Source, Mute)> {
        self.mutes
            .iter()
            .filter(|(s, _)| self.muted(**s, unix_now))
            .map(|(s, m)| (*s, *m))
            .collect()
    }

    /// "Memasuki waktu Maghrib": every reminder ahead of Maghrib is stale now.
    pub fn drop_prayer(&mut self, prayer: &str) {
        self.entries
            .retain(|e| e.notice.prayer.as_deref() != Some(prayer));
        // The one on screen is done with too, so an urgent pill covering it cannot put it back.
        if matches!(&self.front, Some(Front::Entry(e)) if e.notice.prayer.as_deref() == Some(prayer))
        {
            self.front = None;
        }
    }

    /// You hid the orb: prayer reminders are skipped while it is hidden by you, so the ones
    /// waiting (and the one on screen) are done with. Returns whether the front was one.
    pub fn drop_through(&mut self) -> bool {
        self.entries.retain(|e| !(e.waiting() && e.notice.through));
        let front = matches!(&self.front, Some(Front::Entry(e)) if e.notice.through);
        if front {
            self.front = None;
        }
        front
    }

    /// Takes out what matches (the clipboard offer once its link joined the queue).
    pub fn drop_where(&mut self, f: impl Fn(&Pill) -> bool) {
        self.entries.retain(|e| !(e.waiting() && f(&e.notice.pill)));
    }

    /// The next moment the list or the badge changes without anything arriving: a waiting
    /// notice going stale, or midnight with something listed.
    pub fn next_change(&self, now: Moment) -> Option<Moment> {
        let stale = self
            .entries
            .iter()
            .filter(|e| e.waiting())
            .flat_map(|e| [e.notice.stale_at, e.notice.show_by])
            .flatten()
            .filter(|s| *s > now)
            .min();
        let midnight = (!self.entries.is_empty()).then(|| now.date().next().at_minute(0));
        [stale, midnight].into_iter().flatten().min()
    }
}

/// When a mute set now ends, in unix seconds (`None` for good). `tomorrow` is local midnight.
pub fn mute_until(f: MuteFor, unix_now: i64, unix_tomorrow: i64) -> Mute {
    match f {
        MuteFor::Hour => Mute::Until(unix_now + 3600),
        MuteFor::Tomorrow => Mute::Until(unix_tomorrow),
        MuteFor::Always => Mute::Always,
    }
}

/// "for good", "for 1 hour" (or "until 15:20" once part of it passed), "until tomorrow".
pub fn mute_words(
    m: Mute,
    unix_now: i64,
    hhmm_of: impl Fn(i64) -> String,
    tomorrow: i64,
) -> String {
    match m {
        Mute::Always => "for good".into(),
        Mute::Until(t) if t >= tomorrow => "until tomorrow".into(),
        Mute::Until(t) if t - unix_now > 3540 => "for 1 hour".into(),
        Mute::Until(t) => format!("until {}", hhmm_of(t)),
    }
}

/// The summary after a quiet time: "3 notices waited".
pub fn summary_words(n: usize) -> (String, String) {
    (
        if n == 1 {
            "1 notice".into()
        } else {
            format!("{n} notices")
        },
        "waited".into(),
    )
}

pub fn hhmm(m: Moment) -> String {
    let t = m.minute_of_day();
    format!("{:02}:{:02}", t / 60, t % 60)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::ui::Icon;
    use app_habits::Date;

    fn day() -> Date {
        Date::new(2026, 11, 6).unwrap()
    }

    fn at(h: u32, m: u32, ms: u64) -> Now {
        Now {
            at: day().at(h, m),
            ms,
        }
    }

    fn pill(title: &str) -> Pill {
        Pill {
            icon: Icon::Bell,
            dot: false,
            title: title.into(),
            value: String::new(),
            action: None,
            closable: true,
            ttl_ms: 6000,
        }
    }

    fn normal(s: Source, title: &str) -> Notice {
        Notice::new(s, Priority::Normal, pill(title))
    }

    const CALM: Quiet = Quiet {
        hidden: false,
        full_screen: false,
        focus: false,
        paused: false,
        presenting: false,
    };

    fn titles(q: &Queue) -> Vec<String> {
        q.waiting_entries().iter().map(|e| e.pill().title).collect()
    }

    #[test]
    fn one_pill_at_a_time_then_priority_then_age_with_pacing() {
        let mut q = Queue::new();
        let t = at(14, 0, 10_000);
        assert_eq!(
            q.offer(normal(Source::Tasks, "Kirim invoice"), t, 0, CALM, false),
            Offered::Show(pill("Kirim invoice"))
        );
        assert_eq!(
            q.offer(normal(Source::Drink, "Minum dulu"), t, 0, CALM, true),
            Offered::Queued
        );
        assert_eq!(
            q.offer(
                Notice::new(Source::Calendar, Priority::Quiet, pill("Ayah")),
                t,
                0,
                CALM,
                true
            ),
            Offered::Listed
        );
        q.close_front(false);
        // The slot frees 1 s after the first appeared: pacing holds the next for 0.5 s more.
        assert_eq!(q.next(at(14, 0, 11_000), 0, CALM), Next::Wait(500));
        assert_eq!(
            q.next(at(14, 0, 11_500), 0, CALM),
            Next::Show(pill("Minum dulu"), Priority::Normal)
        );
        let (waiting, earlier) = q.list();
        assert!(waiting.is_empty());
        assert_eq!(
            earlier.len(),
            2,
            "the dismissed invoice and the quiet notice"
        );
    }

    #[test]
    fn urgent_jumps_the_front_and_the_interrupted_pill_comes_back() {
        let mut q = Queue::new();
        q.offer(
            normal(Source::Tasks, "Kirim invoice"),
            at(14, 0, 0),
            0,
            CALM,
            false,
        );
        let urgent = Notice::new(Source::Calendar, Priority::Urgent, pill("Standup"));
        assert_eq!(
            q.offer(urgent, at(14, 1, 100), 0, CALM, true),
            Offered::Show(pill("Standup"))
        );
        q.close_front(false);
        assert_eq!(
            q.next(at(14, 1, 5000), 0, CALM),
            Next::Show(pill("Kirim invoice"), Priority::Normal)
        );
    }

    #[test]
    fn same_source_and_kind_merge_and_urgent_never_merges() {
        let mut q = Queue::new();
        let done = |t: &str| {
            normal(Source::Downloads, t).merging(Merge {
                key: "done",
                many: "downloads done".into(),
                value: "Videos".into(),
                action: Some(("Show".into(), PillAction::OpenDownloads)),
            })
        };
        assert!(matches!(
            q.offer(done("Saved a"), at(9, 0, 0), 0, CALM, false),
            Offered::Show(_)
        ));
        match q.offer(done("Saved b"), at(9, 0, 100), 0, CALM, true) {
            Offered::Update(p) => assert_eq!(
                (p.title.as_str(), p.value.as_str()),
                ("2 downloads done", "Videos")
            ),
            o => panic!("{o:?}"),
        }
        q.close_front(false);
        q.offer(
            normal(Source::Tasks, "Kirim invoice"),
            at(9, 1, 200),
            0,
            CALM,
            true,
        );
        q.offer(done("Saved c"), at(9, 1, 300), 0, CALM, true);
        q.offer(done("Saved d"), at(9, 1, 400), 0, CALM, true);
        assert_eq!(titles(&q), vec!["Kirim invoice", "2 downloads done"]);
    }

    #[test]
    fn focus_holds_normal_and_lets_urgent_through_then_releases_one_count() {
        let mut q = Queue::new();
        let focus = Quiet {
            focus: true,
            ..CALM
        };
        assert_eq!(q.set_quiet(focus, day().at(14, 0)), None);
        assert_eq!(
            q.offer(
                normal(Source::Drink, "Minum dulu"),
                at(14, 5, 0),
                0,
                focus,
                false
            ),
            Offered::Held
        );
        let offer = normal(Source::Downloads, "Save this video?").keep(Keep::Drop);
        assert_eq!(
            q.offer(offer, at(14, 7, 0), 0, focus, false),
            Offered::Dropped
        );
        assert!(matches!(
            q.offer(
                Notice::new(Source::Prayer, Priority::Urgent, pill("Memasuki waktu")),
                at(14, 8, 0),
                0,
                focus,
                false
            ),
            Offered::Show(_)
        ));
        q.close_front(false);
        assert_eq!(q.held_now(), 1);
        assert_eq!(q.waiting(), 1);
        assert_eq!(
            q.set_quiet(CALM, day().at(14, 25)),
            Some(Release {
                waited: 1,
                focus: true
            })
        );
        // The drink waits in the list for you.
        assert_eq!(q.next(at(14, 25, 60_000), 0, CALM), Next::Idle);
        let (waiting, _) = q.list();
        assert_eq!(waiting.len(), 1);
    }

    /// "10 menit lagi Isya" fired at 18:41 for Isya at 18:51 (the user's log, 7 Nov 2026).
    fn ten_before(fired: Moment) -> Notice {
        normal(Source::Prayer, "10 menit lagi Isya")
            .before_prayer("Isya", day().at(18, 51), fired)
            .sticky()
    }

    fn thirty_before(fired: Moment) -> Notice {
        normal(Source::Prayer, "30 menit lagi Isya").before_prayer("Isya", day().at(18, 51), fired)
    }

    #[test]
    fn prayer_reminders_pass_focus_full_screen_pause_and_an_fs_hidden_orb() {
        // The bug: full screen held "30 menit lagi Isya" for 20 minutes, then it showed late.
        for quiet in [
            Quiet {
                full_screen: true,
                hidden: true,
                ..CALM
            },
            Quiet {
                focus: true,
                ..CALM
            },
            Quiet {
                paused: true,
                ..CALM
            },
        ] {
            let mut q = Queue::new();
            q.set_quiet(quiet, day().at(18, 0));
            assert_eq!(
                q.offer(
                    thirty_before(day().at(18, 21)),
                    at(18, 21, 0),
                    0,
                    quiet,
                    false
                ),
                Offered::Show(pill("30 menit lagi Isya")),
                "{quiet:?}"
            );
            assert!(q.front_breaks_through(), "it shows over a hidden orb");
        }
    }

    #[test]
    fn presenting_holds_a_prayer_reminder_and_drops_it_once_its_moment_passed() {
        let mut q = Queue::new();
        let present = Quiet {
            presenting: true,
            full_screen: true,
            ..CALM
        };
        q.set_quiet(present, day().at(18, 0));
        assert_eq!(
            q.offer(
                thirty_before(day().at(18, 21)),
                at(18, 21, 0),
                0,
                present,
                false
            ),
            Offered::Held
        );
        // Presenting ends 20 minutes later: "30 menit lagi" would be wrong now, so it is gone.
        q.set_quiet(CALM, day().at(18, 41));
        assert_eq!(q.next(at(18, 41, 1_000_000), 0, CALM), Next::Idle);

        // Presenting ends within the grace: it still shows, right away.
        let mut q = Queue::new();
        q.set_quiet(present, day().at(18, 0));
        q.offer(
            thirty_before(day().at(18, 21)),
            at(18, 21, 0),
            0,
            present,
            false,
        );
        q.set_quiet(CALM, day().at(18, 22));
        assert_eq!(
            q.next(at(18, 22, 60_000), 0, CALM),
            Next::Show(pill("30 menit lagi Isya"), Priority::Normal)
        );
    }

    #[test]
    fn a_newer_reminder_for_the_same_prayer_supersedes_the_held_one() {
        let mut q = Queue::new();
        let present = Quiet {
            presenting: true,
            ..CALM
        };
        q.set_quiet(present, day().at(18, 0));
        q.offer(
            thirty_before(day().at(18, 21)),
            at(18, 21, 0),
            0,
            present,
            false,
        );
        q.offer(
            ten_before(day().at(18, 41)),
            at(18, 41, 0),
            0,
            present,
            false,
        );
        q.set_quiet(CALM, day().at(18, 42));
        let mut shown = Vec::new();
        let mut ms = 100_000;
        while let Next::Show(p, _) = q.next(at(18, 42, ms), 0, CALM) {
            shown.push(p.title);
            q.close_front_by_you();
            ms += 10_000;
        }
        assert_eq!(
            shown,
            vec!["10 menit lagi Isya"],
            "never the 30 after the 10"
        );

        // Calm: the 30 is on screen when the 10 comes, and the 10 takes its place.
        let mut q = Queue::new();
        q.offer(
            thirty_before(day().at(18, 21)),
            at(18, 21, 0),
            0,
            CALM,
            false,
        );
        q.close_front(false);
        match q.offer(
            ten_before(day().at(18, 41)),
            at(18, 41, 9_000),
            0,
            CALM,
            false,
        ) {
            Offered::Show(p) => assert_eq!(p.title, "10 menit lagi Isya"),
            o => panic!("{o:?}"),
        }
    }

    #[test]
    fn ten_minutes_before_stays_until_you_close_it() {
        let mut q = Queue::new();
        assert!(matches!(
            q.offer(ten_before(day().at(18, 41)), at(18, 41, 0), 0, CALM, false),
            Offered::Show(_)
        ));
        // A panel or the dial covers it: it comes back.
        q.close_front(false);
        assert_eq!(
            q.next(at(18, 42, 5_000), 0, CALM),
            Next::Show(pill("10 menit lagi Isya"), Priority::Normal)
        );
        // Another notice takes the front for its turn, then the reminder comes back.
        assert_eq!(
            q.offer(
                normal(Source::Drink, "Minum dulu"),
                at(18, 43, 9_000),
                0,
                CALM,
                true
            ),
            Offered::Show(pill("Minum dulu"))
        );
        q.close_front(false);
        assert_eq!(
            q.next(at(18, 43, 20_000), 0, CALM),
            Next::Show(pill("10 menit lagi Isya"), Priority::Normal)
        );
        // Your X ends it.
        q.close_front_by_you();
        assert_eq!(q.next(at(18, 44, 30_000), 0, CALM), Next::Idle);
    }

    #[test]
    fn hiding_the_orb_yourself_ends_a_waiting_or_shown_prayer_reminder() {
        let mut q = Queue::new();
        q.offer(ten_before(day().at(18, 41)), at(18, 41, 0), 0, CALM, false);
        assert!(q.drop_through(), "the one on screen");
        q.close_front(false);
        assert_eq!(q.next(at(18, 42, 9_000), 0, CALM), Next::Idle);
    }

    #[test]
    fn reminder_title_counts_whole_minutes_then_sebentar_lagi() {
        let t = |s| reminder_title("Maghrib", s);
        assert_eq!(t(1800), "30 menit lagi Maghrib");
        assert_eq!(t(600), "10 menit lagi Maghrib");
        // A reminder fired a moment late at −10 still says 10.
        assert_eq!(t(598), "10 menit lagi Maghrib");
        assert_eq!(t(597), "9 menit lagi Maghrib");
        assert_eq!(t(538), "9 menit lagi Maghrib");
        assert_eq!(t(537), "8 menit lagi Maghrib");
        assert_eq!(t(60), "1 menit lagi Maghrib");
        assert_eq!(t(58), "1 menit lagi Maghrib");
        assert_eq!(t(57), "Maghrib sebentar lagi");
        assert_eq!(t(0), "Maghrib sebentar lagi");
        assert_eq!(t(-5), "Maghrib sebentar lagi");
    }

    #[test]
    fn the_countdown_wakes_exactly_when_its_words_change() {
        assert_eq!(countdown_next_secs(600), Some(3));
        assert_eq!(countdown_next_secs(597), Some(60));
        for s in 58..=660 {
            let w = countdown_next_secs(s).unwrap();
            let (now, at, before) = (
                reminder_title("Isya", s),
                reminder_title("Isya", s - w),
                reminder_title("Isya", s - w + 1),
            );
            assert_ne!(now, at, "changed by the wake ({s} s left)");
            assert_eq!(now, before, "not a second earlier ({s} s left)");
        }
        // "sebentar lagi" stays until "Memasuki waktu" replaces it: no more wakes.
        assert_eq!(countdown_next_secs(57), None);
        assert_eq!(countdown_next_secs(0), None);
    }

    fn counting(fired: Moment) -> Notice {
        Notice {
            pill: Pill {
                value: "18:51".into(),
                ..pill("10 menit lagi Isya")
            },
            ..ten_before(fired).counting_down()
        }
    }

    fn secs(h: u32, m: u32, s: u64, ms: u64) -> Now {
        Now {
            at: day().at(h, m) + Duration::from_secs(s),
            ms,
        }
    }

    #[test]
    fn the_ten_minute_pill_counts_down_in_place() {
        let mut q = Queue::new();
        match q.offer(counting(day().at(18, 41)), at(18, 41, 0), 0, CALM, false) {
            Offered::Show(p) => assert_eq!(p.title, "10 menit lagi Isya"),
            o => panic!("{o:?}"),
        }
        assert_eq!(
            q.countdown_wait(day().at(18, 41)),
            Some(Duration::from_secs(3))
        );
        let p = q.countdown_tick(secs(18, 41, 3, 0).at).expect("new words");
        assert_eq!(
            (p.title.as_str(), p.value.as_str()),
            ("9 menit lagi Isya", "18:51")
        );
        assert_eq!(q.countdown_tick(secs(18, 41, 4, 0).at), None, "same words");
        assert_eq!(
            q.countdown_wait(secs(18, 41, 3, 0).at),
            Some(Duration::from_secs(60))
        );
        assert_eq!(
            q.countdown_tick(secs(18, 50, 3, 0).at).map(|p| p.title),
            Some("Isya sebentar lagi".into())
        );
        assert_eq!(q.countdown_wait(secs(18, 50, 3, 0).at), None);
        // Your X ends it, and nothing counts any more.
        q.close_front_by_you();
        assert_eq!(q.countdown_wait(day().at(18, 46)), None);
        assert_eq!(q.countdown_tick(day().at(18, 46)), None);
        assert_eq!(q.next(at(18, 46, 9_000), 0, CALM), Next::Idle);
    }

    #[test]
    fn a_countdown_shown_late_or_again_says_the_minutes_left_then() {
        // Presenting held it at 18:41; it shows at 18:42:30 (within the grace).
        let mut q = Queue::new();
        let present = Quiet {
            presenting: true,
            ..CALM
        };
        q.set_quiet(present, day().at(18, 0));
        assert_eq!(
            q.offer(counting(day().at(18, 41)), at(18, 41, 0), 0, present, false),
            Offered::Held
        );
        q.set_quiet(CALM, secs(18, 42, 30, 0).at);
        match q.next(secs(18, 42, 30, 60_000), 0, CALM) {
            Next::Show(p, _) => assert_eq!(p.title, "8 menit lagi Isya"),
            n => panic!("{n:?}"),
        }
        // Covered by a panel at 18:44, back at 18:47: the words are current again.
        q.close_front(false);
        assert_eq!(q.countdown_wait(day().at(18, 45)), None, "not on screen");
        match q.next(at(18, 47, 90_000), 0, CALM) {
            Next::Show(p, _) => assert_eq!(p.title, "4 menit lagi Isya"),
            n => panic!("{n:?}"),
        }
        // The −30 one keeps its words.
        let mut q = Queue::new();
        q.offer(
            thirty_before(day().at(18, 21)),
            at(18, 21, 0),
            0,
            CALM,
            false,
        );
        assert_eq!(q.countdown_tick(day().at(18, 25)), None);
        assert_eq!(q.countdown_wait(day().at(18, 25)), None);
    }

    #[test]
    fn memasuki_waktu_replaces_the_ten_minute_pill_for_good() {
        let mut q = Queue::new();
        q.offer(ten_before(day().at(18, 41)), at(18, 41, 0), 0, CALM, false);
        q.drop_prayer("Isya");
        assert!(matches!(
            q.offer(
                Notice::new(
                    Source::Prayer,
                    Priority::Urgent,
                    pill("Memasuki waktu Isya")
                ),
                at(18, 51, 9_000),
                0,
                CALM,
                true
            ),
            Offered::Show(_)
        ));
        q.close_front_by_you();
        assert_eq!(q.next(at(18, 52, 20_000), 0, CALM), Next::Idle);
        // And it goes stale at Isya even if nothing dropped it.
        let mut q = Queue::new();
        q.offer(ten_before(day().at(18, 41)), at(18, 41, 0), 0, CALM, false);
        q.close_front(false);
        assert_eq!(q.next(at(18, 51, 20_000), 0, CALM), Next::Idle);
        assert!(
            q.list().0.is_empty() && q.list().1.is_empty(),
            "prayer never stays in the list"
        );
    }

    #[test]
    fn hidden_holds_without_a_summary_and_quiet_time_moves_queued_into_the_list() {
        let mut q = Queue::new();
        q.offer(normal(Source::Tasks, "a"), at(9, 0, 0), 0, CALM, false);
        q.offer(normal(Source::Tasks, "b"), at(9, 0, 10), 0, CALM, true);
        let hidden = Quiet {
            hidden: true,
            ..CALM
        };
        q.close_front(false);
        assert_eq!(q.set_quiet(hidden, day().at(9, 1)), None);
        assert_eq!(q.next(at(9, 1, 9000), 0, hidden), Next::Idle);
        assert_eq!(q.waiting(), 1, "b waits on the badge");
        assert_eq!(
            q.set_quiet(CALM, day().at(9, 2)),
            None,
            "being hidden ends with the badge only"
        );
        assert_eq!(q.next(at(9, 2, 99_000), 0, CALM), Next::Idle);
        assert_eq!(q.waiting(), 1);
    }

    #[test]
    fn mutes_and_cooldowns() {
        let mut q = Queue::new();
        q.mute(Source::Downloads, Mute::Until(1000));
        assert_eq!(
            q.offer(
                normal(Source::Downloads, "Saved"),
                at(9, 0, 0),
                999,
                CALM,
                false
            ),
            Offered::Muted
        );
        assert!(matches!(
            q.offer(
                normal(Source::Downloads, "Saved"),
                at(9, 0, 0),
                1000,
                CALM,
                false
            ),
            Offered::Show(_)
        ));
        q.close_front(false);
        let low =
            |t: &str| normal(Source::Disk, t).cooldown("low-space", Duration::from_secs(86_400));
        assert!(matches!(
            q.offer(low("C: 8%"), at(9, 1, 10_000), 5000, CALM, false),
            Offered::Show(_)
        ));
        q.close_front(false);
        assert_eq!(
            q.offer(low("C: 2%"), at(9, 2, 20_000), 6000, CALM, false),
            Offered::Queued
        );
        assert_eq!(q.next(at(9, 2, 20_000), 6000, CALM), Next::Wait(85_400_000));
        assert!(!q.muted(Source::Prayer, 0));
        q.mute(Source::Prayer, Mute::Always);
        assert!(
            matches!(
                q.offer(
                    Notice::new(Source::Prayer, Priority::Urgent, pill("Memasuki waktu")),
                    at(9, 3, 30_000),
                    6000,
                    CALM,
                    true
                ),
                Offered::Show(_)
            ),
            "prayer at the time can't be muted"
        );
    }

    #[test]
    fn clear_all_and_undo_and_midnight() {
        let mut q = Queue::new();
        let hidden = Quiet {
            hidden: true,
            ..CALM
        };
        q.offer(normal(Source::Tasks, "a"), at(23, 0, 0), 0, hidden, false);
        q.offer(normal(Source::Drink, "b"), at(23, 1, 0), 0, hidden, false);
        assert_eq!(q.waiting(), 2);
        q.clear_all(day().at(23, 2));
        assert_eq!(q.waiting(), 0);
        assert!(q.undo_clear(day().at(23, 2) + Duration::from_secs(5)));
        assert_eq!(q.waiting(), 2);
        q.clear_all(day().at(23, 3));
        assert!(!q.undo_clear(day().at(23, 3) + Duration::from_secs(7)));
        q.offer(normal(Source::Tasks, "c"), at(23, 30, 0), 0, hidden, false);
        q.mark_seen(q.list().0[0].id);
        assert_eq!(
            q.next_change(day().at(23, 31)),
            Some(day().next().at_minute(0))
        );
        q.next(
            Now {
                at: day().next().at(0, 1),
                ms: 0,
            },
            0,
            hidden,
        );
        assert_eq!(q.list().1.len(), 0, "the list clears at midnight");
    }

    #[test]
    fn opening_a_row_puts_it_in_front_and_a_reply_requeues_the_front() {
        let mut q = Queue::new();
        let hidden = Quiet {
            hidden: true,
            ..CALM
        };
        q.offer(
            normal(Source::Tasks, "Kirim invoice"),
            at(9, 0, 0),
            0,
            hidden,
            false,
        );
        let id = q.list().0[0].id;
        assert_eq!(q.open(id, at(9, 1, 0)), Some(pill("Kirim invoice")));
        q.reply(at(9, 1, 100));
        assert_eq!(titles(&q), vec!["Kirim invoice"]);
        q.close_front(false);
        assert_eq!(
            q.next(at(9, 2, 5000), 0, CALM),
            Next::Show(pill("Kirim invoice"), Priority::Normal)
        );
        q.close_front(true);
        assert!(
            q.list().0.is_empty() && q.list().1.is_empty(),
            "an acted notice is done with"
        );
    }

    #[test]
    fn a_newer_reminder_replaces_the_waiting_one() {
        let mut q = Queue::new();
        let hidden = Quiet {
            hidden: true,
            ..CALM
        };
        let drink = |v: &str| {
            Notice::new(
                Source::Drink,
                Priority::Normal,
                Pill {
                    value: v.into(),
                    ..pill("Minum dulu")
                },
            )
            .replacing("remind")
        };
        q.offer(drink("500 / 2000 ml"), at(10, 0, 0), 0, hidden, false);
        assert_eq!(
            q.offer(drink("500 / 2000 ml"), at(11, 0, 0), 0, hidden, false),
            Offered::Held
        );
        assert_eq!(q.waiting(), 1);
        assert_eq!(
            q.list().0[0].pill().title,
            "Minum dulu",
            "no count for a replacement"
        );
    }

    #[test]
    fn a_new_reminder_after_a_quiet_time_shows_and_supersedes_the_listed_one() {
        let mut q = Queue::new();
        let hidden = Quiet {
            hidden: true,
            ..CALM
        };
        let drink =
            || Notice::new(Source::Drink, Priority::Normal, pill("Minum dulu")).replacing("remind");
        q.offer(drink(), at(10, 0, 0), 0, hidden, false);
        assert!(matches!(
            q.offer(drink(), at(11, 0, 9000), 0, CALM, false),
            Offered::Show(_)
        ));
        assert_eq!(q.waiting(), 0, "the older reminder is no longer waiting");
    }

    #[test]
    fn mute_words_read_naturally() {
        let tomorrow = 100_000;
        assert_eq!(
            mute_words(Mute::Always, 0, |_| String::new(), tomorrow),
            "for good"
        );
        assert_eq!(
            mute_words(Mute::Until(3600), 0, |_| String::new(), tomorrow),
            "for 1 hour"
        );
        assert_eq!(
            mute_words(Mute::Until(1800), 0, |_| "15:20".into(), tomorrow),
            "until 15:20"
        );
        assert_eq!(
            mute_words(Mute::Until(tomorrow), 0, |_| String::new(), tomorrow),
            "until tomorrow"
        );
        assert_eq!(summary_words(3), ("3 notices".into(), "waited".into()));
    }
}
