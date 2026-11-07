//! The Today panel's one combined timeline: calendar events, tasks due and prayer markers.
//!
//! Prayer is not a task (DESIGN.md). A prayer marker has a name and a time and nothing else:
//! there is no done state to set and no way to check it off.

use crate::occurrences::Occurrence;
use crate::time::Instant;

/// A to-do due today, as the tasks module hands it over.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct TaskDue {
    pub id: String,
    pub title: String,
    pub due: Instant,
    pub done: bool,
}

/// A prayer time, already named for display ("Dzuhur", or "Jumat" on Friday when that setting is on).
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct PrayerTime {
    pub name: String,
    pub at: Instant,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Item {
    Event(Occurrence),
    Task(TaskDue),
    /// Informs only. Never a row, never a checkbox.
    PrayerMarker(PrayerTime),
}

impl Item {
    pub fn at(&self) -> Instant {
        match self {
            Item::Event(o) => o.start,
            Item::Task(t) => t.due,
            Item::PrayerMarker(p) => p.at,
        }
    }

    /// Only tasks can be ticked.
    pub fn is_checkable(&self) -> bool {
        matches!(self, Item::Task(_))
    }

    /// Order at the same minute: all-day events, then the prayer time (it marks the moment),
    /// then timed events, then tasks.
    fn rank(&self) -> u8 {
        match self {
            Item::Event(o) if o.all_day => 0,
            Item::PrayerMarker(_) => 1,
            Item::Event(_) => 2,
            Item::Task(_) => 3,
        }
    }
}

/// Merges the day's occurrences, tasks due and prayer times into one list sorted by time.
/// The caller picks the day: everything passed in is kept.
pub fn today_timeline(
    occurrences: impl IntoIterator<Item = Occurrence>,
    tasks_due: impl IntoIterator<Item = TaskDue>,
    prayer_times: impl IntoIterator<Item = PrayerTime>,
) -> Vec<Item> {
    let mut items: Vec<Item> = occurrences
        .into_iter()
        .map(Item::Event)
        .chain(tasks_due.into_iter().map(Item::Task))
        .chain(prayer_times.into_iter().map(Item::PrayerMarker))
        .collect();
    items.sort_by_key(|item| (item.at(), item.rank()));
    items
}
