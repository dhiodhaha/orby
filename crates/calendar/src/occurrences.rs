//! Expanding a calendar into concrete occurrences inside a time window.

use std::collections::HashMap;

use crate::link::MeetingLink;
use crate::model::{Calendar, DateTimeValue, Event, EventEnd, Status};
use crate::rrule::{expand, RecurEnd};
use crate::time::{Instant, LocalDateTime, DAY};

/// One concrete appearance of an event.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Occurrence {
    pub uid: String,
    pub title: String,
    pub start: Instant,
    /// Exclusive. Equal to `start` for an event with no end or duration.
    pub end: Instant,
    /// All-day occurrences start and end at local midnight of the display offset.
    pub all_day: bool,
    pub link: Option<MeetingLink>,
    pub location: Option<String>,
}

/// Every occurrence overlapping `[from, to)`, sorted by start (all-day first among equal starts).
///
/// `display_offset_minutes` is the viewer's UTC offset: it places all-day dates, floating times
/// and times in a zone that is neither in the file nor in the fallback table.
///
/// CANCELLED events are dropped. An override (same UID plus RECURRENCE-ID) replaces the instance
/// it names, wherever it moved to; a cancelled override only removes it. EXDATE removes instances.
pub fn occurrences(cal: &Calendar, from: Instant, to: Instant, display_offset_minutes: i32) -> Vec<Occurrence> {
    let r = Resolver { cal, display_offset: display_offset_minutes };
    let mut overrides: HashMap<&str, Vec<&Event>> = HashMap::new();
    for ev in cal.events.iter().filter(|e| e.recurrence_id.is_some()) {
        overrides.entry(ev.uid.as_str()).or_default().push(ev);
    }

    let mut out = Vec::new();
    for master in cal.events.iter().filter(|e| e.recurrence_id.is_none() && e.status != Status::Cancelled) {
        let replaced = overrides.get(master.uid.as_str()).map(Vec::as_slice).unwrap_or_default();
        let length = r.length_seconds(master);
        for wall in instance_walls(master, &r, to) {
            let start = r.instant(&master.start.with_wall(wall));
            let removed = master.exdates.iter().any(|x| r.names_instance(x, wall, start))
                || replaced
                    .iter()
                    .any(|o| o.recurrence_id.as_ref().is_some_and(|id| r.names_instance(id, wall, start)));
            let end = start.add_seconds(length);
            if !removed && overlaps(start, end, from, to) {
                out.push(occurrence(master, start, end));
            }
        }
    }
    for ov in overrides.values().flatten().filter(|o| o.status != Status::Cancelled) {
        let start = r.instant(&ov.start);
        let end = start.add_seconds(r.length_seconds(ov));
        if overlaps(start, end, from, to) {
            out.push(occurrence(ov, start, end));
        }
    }

    out.sort_by(|a, b| {
        (a.start, !a.all_day, a.end, &a.title, &a.uid).cmp(&(b.start, !b.all_day, b.end, &b.title, &b.uid))
    });
    out
}

fn occurrence(ev: &Event, start: Instant, end: Instant) -> Occurrence {
    Occurrence {
        uid: ev.uid.clone(),
        title: ev.summary.clone(),
        start,
        end,
        all_day: ev.start.is_date(),
        link: ev.link.clone(),
        location: ev.location.clone(),
    }
}

/// A zero-length occurrence counts when it starts inside the window.
fn overlaps(start: Instant, end: Instant, from: Instant, to: Instant) -> bool {
    if start == end {
        from <= start && start < to
    } else {
        start < to && end > from
    }
}

/// Wall-clock starts in the event's own zone, so a weekly 09:00 stays at 09:00 across a DST change.
fn instance_walls(ev: &Event, r: &Resolver, to: Instant) -> Vec<LocalDateTime> {
    let start = ev.start.wall();
    let Some(rule) = &ev.rrule else { return vec![start] };
    // Two days of slack cover any gap between the event's zone and UTC.
    let stop_after = to.to_local(0).add_seconds(2 * DAY);
    expand(rule, start, stop_after, |t| match &rule.end {
        RecurEnd::Until(DateTimeValue::Date(d)) => t.date() <= *d,
        RecurEnd::Until(until) => r.instant(&ev.start.with_wall(t)) <= r.instant(until),
        _ => true,
    })
}

struct Resolver<'a> {
    cal: &'a Calendar,
    display_offset: i32,
}

impl Resolver<'_> {
    fn offset(&self, v: &DateTimeValue) -> i32 {
        match v {
            DateTimeValue::Utc(_) => 0,
            DateTimeValue::Date(_) | DateTimeValue::Floating(_) => self.display_offset,
            DateTimeValue::Zoned { wall, tzid } => {
                self.cal.zone_offset_minutes(tzid, *wall).unwrap_or(self.display_offset)
            }
        }
    }

    fn instant(&self, v: &DateTimeValue) -> Instant {
        v.wall().to_instant(self.offset(v))
    }

    /// Does an EXDATE or RECURRENCE-ID value name this instance? A date names the whole day.
    fn names_instance(&self, id: &DateTimeValue, wall: LocalDateTime, start: Instant) -> bool {
        match id {
            DateTimeValue::Date(d) => wall.date() == *d,
            _ => self.instant(id) == start,
        }
    }

    /// Elapsed seconds of the event as written, reused for every instance. An instance that itself
    /// spans a DST change keeps this elapsed length rather than its wall-clock end.
    fn length_seconds(&self, ev: &Event) -> i64 {
        let secs = match &ev.end {
            EventEnd::At(end) => self.instant(end).unix_seconds() - self.instant(&ev.start).unix_seconds(),
            EventEnd::After(secs) => *secs,
            EventEnd::Unspecified if ev.start.is_date() => DAY,
            EventEnd::Unspecified => 0,
        };
        secs.max(0)
    }
}
