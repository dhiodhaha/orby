//! Drink pacing (ORB_SPEC 6.5, DESIGN.md "Drink (v6.2)"): the log, pace against the goal, and
//! when the next reminder is due.
//!
//! The reminder clock counts active minutes since the last reset (a drink, a shown reminder, a
//! screen unlock). Idle time over the threshold pauses it; a screen lock resets it. The caller
//! passes presence, focus, meeting and today's prayer times in a [`ReminderContext`].

use std::time::Duration;

use crate::time::{mins, Date, Moment};

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Glass {
    pub name: String,
    pub ml: u32,
}

/// Local minutes after midnight, `start_min` inclusive, `end_min` exclusive.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct ActiveHours {
    pub start_min: u32,
    pub end_min: u32,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct DrinkConfig {
    pub goal_ml: u32,
    pub glasses_ml: Vec<u32>,
    /// The user's own bottle, e.g. "Tumbler" 600 ml.
    pub custom_glass: Option<Glass>,
    /// The pill's quick button.
    pub default_glass_ml: u32,
    pub active_hours: ActiveHours,
    pub interval: Duration,
    /// Shorter interval while behind pace; `None` turns catch-up off.
    pub catch_up: Option<Duration>,
    /// "Habis sholat, minum dulu" this long after each prayer time; `None` is off (the default).
    pub after_prayer: Option<Duration>,
    /// The sahur pill comes this long before Imsak in puasa mode.
    pub sahur_lead: Duration,
    /// Idle longer than this pauses the reminder clock.
    pub idle_after: Duration,
    pub snooze: Duration,
}

impl Default for DrinkConfig {
    fn default() -> Self {
        DrinkConfig {
            goal_ml: 2000,
            glasses_ml: vec![150, 250, 500],
            custom_glass: None,
            default_glass_ml: 250,
            active_hours: ActiveHours { start_min: 8 * 60, end_min: 21 * 60 },
            interval: mins(60),
            catch_up: Some(mins(45)),
            after_prayer: None,
            sahur_lead: mins(30),
            idle_after: mins(5),
            snooze: mins(15),
        }
    }
}

impl DrinkConfig {
    /// The reminder interval for this pace: the catch-up interval while behind, else the normal one.
    pub fn interval_for(&self, consumed_ml: u32, target_ml: u32) -> Duration {
        match self.catch_up {
            Some(short) if consumed_ml < target_ml => short,
            _ => self.interval,
        }
    }
}

/// The suggested daily goal range from body weight, 30 to 35 ml per kg.
pub fn suggest_goal_ml(weight_kg: u32) -> (u32, u32) {
    (weight_kg * 30, weight_kg * 35)
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Source {
    /// Logged on this device.
    Local,
    /// Synced from another of the user's devices ("from your phone").
    Device(String),
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Entry {
    pub ml: u32,
    pub at: Moment,
    pub source: Source,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Prayer {
    Subuh,
    Dzuhur,
    Ashar,
    Maghrib,
    Isya,
}

impl Prayer {
    pub const ALL: [Prayer; 5] = [Prayer::Subuh, Prayer::Dzuhur, Prayer::Ashar, Prayer::Maghrib, Prayer::Isya];
}

/// Today's times in local minutes after midnight, as `app_core::prayer::Times` gives them.
/// They are reused for the neighbouring night, which is off by a minute at most.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct PrayerTimes {
    pub imsak: u32,
    pub subuh: u32,
    pub dzuhur: u32,
    pub ashar: u32,
    pub maghrib: u32,
    pub isya: u32,
}

impl PrayerTimes {
    pub fn of(&self, p: Prayer) -> u32 {
        match p {
            Prayer::Subuh => self.subuh,
            Prayer::Dzuhur => self.dzuhur,
            Prayer::Ashar => self.ashar,
            Prayer::Maghrib => self.maghrib,
            Prayer::Isya => self.isya,
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Presence {
    Active,
    /// No input since `since` (GetLastInputInfo). Counts as away once past `idle_after`.
    Idle {
        since: Moment,
    },
    Locked,
}

/// Which window the day's goal is paced over.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum DayMode {
    /// Active hours.
    Normal,
    /// Puasa: the goal is spread over the night, Maghrib to Imsak.
    Fasting { imsak: u32, maghrib: u32 },
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct ReminderContext {
    pub presence: Presence,
    /// When the running focus session hands over to its break (`FocusTimer::break_at`).
    pub focus_until: Option<Moment>,
    /// A recorded meeting is on; its end is unknown, so reminders wait until it ends.
    pub meeting_recording: bool,
    pub prayers: PrayerTimes,
    /// Puasa today (Ramadan, Senin-Kamis or manual), decided by the caller.
    pub fasting: bool,
}

impl ReminderContext {
    pub fn mode(&self) -> DayMode {
        if self.fasting {
            DayMode::Fasting { imsak: self.prayers.imsak, maghrib: self.prayers.maghrib }
        } else {
            DayMode::Normal
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ReminderKind {
    /// "Minum dulu" on the normal interval.
    Interval,
    /// "Minum dulu" on the shorter interval because the user is behind pace.
    CatchUp,
    AfterPrayer(Prayer),
    /// At Maghrib in puasa mode.
    Buka,
    /// `sahur_lead` before Imsak in puasa mode; `imsak` is in minutes after midnight.
    Sahur {
        imsak: u32,
    },
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Reminder {
    /// Never earlier than the `now` it was asked at; equal to `now` means show it now.
    pub at: Moment,
    pub kind: ReminderKind,
    /// Ahead of pace: show it without a sound.
    pub quiet: bool,
    /// It waited for the focus break: the break pill carries it ("Break 5 min · minum dulu?").
    pub at_break: bool,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum DrinkError {
    ZeroAmount,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Clock {
    /// Active since `since`, plus `banked` active time from before the last idle stretch.
    /// `origin` is the last reset, used to drop time banked before today's window opened.
    Running { origin: Moment, since: Moment, banked: Duration },
    /// Away: idle past the threshold, or locked (which also zeroes `banked`).
    Paused { origin: Moment, banked: Duration },
}

#[derive(Clone, Copy, Debug)]
struct Window {
    start: Moment,
    end: Moment,
}

#[derive(Clone, Debug)]
pub struct Drink {
    config: DrinkConfig,
    entries: Vec<Entry>,
    clock: Clock,
    snoozed_until: Option<Moment>,
    last_shown: Option<Moment>,
}

impl Drink {
    pub fn new(config: DrinkConfig) -> Self {
        let epoch = Moment::from_unix(0, 0);
        Drink {
            config,
            entries: Vec::new(),
            clock: Clock::Running { origin: epoch, since: epoch, banked: Duration::ZERO },
            snoozed_until: None,
            last_shown: None,
        }
    }

    pub fn config(&self) -> &DrinkConfig {
        &self.config
    }

    /// Every entry, in the order logged.
    pub fn entries(&self) -> &[Entry] {
        &self.entries
    }

    /// Logs a drink. A new drink restarts the reminder clock and clears a snooze; a synced entry
    /// older than the last reset does not.
    pub fn log(&mut self, ml: u32, at: Moment, source: Source) -> Result<(), DrinkError> {
        if ml == 0 {
            return Err(DrinkError::ZeroAmount);
        }
        self.entries.push(Entry { ml, at, source });
        let origin = match self.clock {
            Clock::Running { origin, .. } | Clock::Paused { origin, .. } => origin,
        };
        if at >= origin {
            self.reset_clock(at);
            self.snoozed_until = None;
        }
        Ok(())
    }

    /// Removes the latest entry (by time), the only one the panel offers Undo on.
    pub fn undo_last(&mut self) -> Option<Entry> {
        let (i, _) = self.entries.iter().enumerate().max_by_key(|(i, e)| (e.at, *i))?;
        Some(self.entries.remove(i))
    }

    pub fn total_for(&self, date: Date) -> u32 {
        self.entries.iter().filter(|e| e.at.date() == date).map(|e| e.ml).sum()
    }

    /// Where intake should be by `now`, linear over today's active hours.
    pub fn pace_target(&self, now: Moment) -> u32 {
        self.pace_target_in(now, DayMode::Normal)
    }

    pub fn pace_target_in(&self, now: Moment, mode: DayMode) -> u32 {
        let w = self.window(now, mode);
        if now <= w.start {
            return 0;
        }
        let frac = (now.since(w.start).as_secs_f64() / w.end.since(w.start).as_secs_f64()).min(1.0);
        (f64::from(self.config.goal_ml) * frac) as u32
    }

    /// Intake that counts toward the current pace window: the calendar day, or the night in puasa.
    pub fn consumed_in(&self, now: Moment, mode: DayMode) -> u32 {
        match mode {
            DayMode::Normal => {
                let day = now.date();
                self.entries.iter().filter(|e| e.at <= now && e.at.date() == day).map(|e| e.ml).sum()
            }
            DayMode::Fasting { .. } => {
                let w = self.window(now, mode);
                self.entries.iter().filter(|e| e.at >= w.start && e.at <= now).map(|e| e.ml).sum()
            }
        }
    }

    pub fn is_behind(&self, now: Moment) -> bool {
        self.is_behind_in(now, DayMode::Normal)
    }

    pub fn is_behind_in(&self, now: Moment, mode: DayMode) -> bool {
        self.consumed_in(now, mode) < self.pace_target_in(now, mode)
    }

    /// The next reminder, or `None` while away, during a recorded meeting, or when nothing is left
    /// in the current window. Takes `&mut self` because it folds `ctx.presence` into the reminder
    /// clock; call it on every poll.
    pub fn next_reminder(&mut self, now: Moment, ctx: &ReminderContext) -> Option<Reminder> {
        let mode = ctx.mode();
        let w = self.window(now, mode);
        self.observe(now, ctx.presence);
        if matches!(self.clock, Clock::Paused { .. }) || ctx.meeting_recording {
            return None;
        }
        let consumed = self.consumed_in(now, mode);
        let target = self.pace_target_in(now, mode);

        let interval = self.interval_due(w, consumed, target).map(|at| {
            let kind = if self.config.interval_for(consumed, target) == self.config.interval {
                ReminderKind::Interval
            } else {
                ReminderKind::CatchUp
            };
            (at, kind)
        });
        let (at, kind) = self.fixed_due(now, w, ctx).into_iter().chain(interval).min_by_key(|(at, _)| *at)?;

        let quiet = matches!(kind, ReminderKind::Interval | ReminderKind::CatchUp) && consumed > target;
        let mut at = at.max(now);
        let mut at_break = false;
        if let Some(until) = ctx.focus_until.filter(|u| *u > at) {
            at = until;
            at_break = true;
        }
        // An overdue reminder moves to `now`, and a waiting one to the break: neither may leave
        // the window, or a late start or a late break would remind outside active hours.
        (at < w.end).then_some(Reminder { at, kind, quiet, at_break })
    }

    /// The app starts watching presence at `now` (launch). Time before it is unknown (the PC was
    /// off or the app closed), so it counts as away: the clock runs from `now`, with nothing banked.
    pub fn start_watching(&mut self, now: Moment) {
        let origin = match self.clock {
            Clock::Running { origin, .. } | Clock::Paused { origin, .. } => origin,
        };
        self.clock = Clock::Running { origin, since: now.max(origin), banked: Duration::ZERO };
    }

    /// The pill was shown (and dismissed or acted on): restart the clock.
    pub fn reminder_shown(&mut self, at: Moment) {
        self.last_shown = Some(at);
        self.snoozed_until = None;
        self.reset_clock(at);
    }

    /// Snooze 15m on the pill.
    pub fn snooze(&mut self, now: Moment) {
        self.reminder_shown(now);
        self.snoozed_until = Some(now + self.config.snooze);
    }

    fn reset_clock(&mut self, at: Moment) {
        self.clock = Clock::Running { origin: at, since: at, banked: Duration::ZERO };
    }

    fn observe(&mut self, now: Moment, presence: Presence) {
        let away_since = match presence {
            Presence::Locked => {
                self.clock = Clock::Paused { origin: now, banked: Duration::ZERO };
                return;
            }
            Presence::Idle { since } if now.since(since) > self.config.idle_after => Some(since),
            _ => None,
        };
        self.clock = match (self.clock, away_since) {
            (Clock::Running { origin, since, banked }, Some(idle_from)) => {
                Clock::Paused { origin, banked: banked + idle_from.since(since) }
            }
            (Clock::Paused { origin, banked }, None) => Clock::Running { origin, since: now, banked },
            (clock, _) => clock,
        };
    }

    fn interval_due(&self, w: Window, consumed: u32, target: u32) -> Option<Moment> {
        let due = match (self.snoozed_until, self.clock) {
            (Some(s), _) => s,
            (None, Clock::Running { origin, since, banked }) => {
                let (since, banked) =
                    if origin < w.start { (since.max(w.start), Duration::ZERO) } else { (since, banked) };
                since + self.config.interval_for(consumed, target).saturating_sub(banked)
            }
            (None, Clock::Paused { .. }) => return None,
        };
        (due < w.end).then_some(due)
    }

    /// Reminders tied to the clock face: after each prayer, buka and sahur. One is pending from its
    /// time until an interval later, unless a reminder was shown after its time. Only the current
    /// window counts, so after active hours there is nothing until tomorrow's first poll.
    fn fixed_due(&self, now: Moment, w: Window, ctx: &ReminderContext) -> Vec<(Moment, ReminderKind)> {
        let p = ctx.prayers;
        let mut out = Vec::new();
        for date in [now.date(), now.date().next()] {
            if let Some(off) = self.config.after_prayer {
                for prayer in Prayer::ALL {
                    // In puasa the buka pill at Maghrib already covers it.
                    if !(ctx.fasting && prayer == Prayer::Maghrib) {
                        out.push((date.at_minute(p.of(prayer)) + off, ReminderKind::AfterPrayer(prayer)));
                    }
                }
            }
            if ctx.fasting {
                out.push((date.at_minute(p.maghrib), ReminderKind::Buka));
                out.push((date.at_minute(p.imsak) - self.config.sahur_lead, ReminderKind::Sahur { imsak: p.imsak }));
            }
        }
        let stale = self.config.interval;
        out.retain(|(at, _)| {
            now < *at + stale && *at < w.end && self.last_shown.is_none_or(|l| *at > l) && self.allowed(*at, ctx)
        });
        out
    }

    fn allowed(&self, at: Moment, ctx: &ReminderContext) -> bool {
        let m = at.minute_of_day();
        if ctx.fasting {
            !(ctx.prayers.imsak <= m && m < ctx.prayers.maghrib)
        } else {
            let h = self.config.active_hours;
            h.start_min <= m && m < h.end_min
        }
    }

    /// The pace window for `now`: today's active hours, or in puasa the night that holds `now`
    /// (the coming night during the fast).
    fn window(&self, now: Moment, mode: DayMode) -> Window {
        let d = now.date();
        match mode {
            DayMode::Normal => {
                let h = self.config.active_hours;
                Window { start: d.at_minute(h.start_min), end: d.at_minute(h.end_min) }
            }
            DayMode::Fasting { imsak, maghrib } if now.minute_of_day() < imsak => {
                Window { start: d.prev().at_minute(maghrib), end: d.at_minute(imsak) }
            }
            DayMode::Fasting { imsak, maghrib } => {
                Window { start: d.at_minute(maghrib), end: d.next().at_minute(imsak) }
            }
        }
    }
}
