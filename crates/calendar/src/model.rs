//! The parsed calendar: what an ICS file says, before any expansion.

use std::str::FromStr;

use crate::link::MeetingLink;
use crate::rrule::RRule;
use crate::time::{Date, LocalDateTime};
use crate::tz::{fallback_offset_minutes, VTimezone};

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Calendar {
    /// X-WR-CALNAME, as Google and Outlook exports carry it.
    pub name: Option<String>,
    pub events: Vec<Event>,
    pub timezones: Vec<VTimezone>,
}

impl Calendar {
    /// The UTC offset of `tzid` at a wall time: the calendar's own VTIMEZONE first, then the
    /// built-in fallback table. None means the zone is unknown.
    pub fn zone_offset_minutes(&self, tzid: &str, wall: LocalDateTime) -> Option<i32> {
        self.timezones
            .iter()
            .find(|tz| tz.tzid == tzid)
            .and_then(|tz| tz.offset_minutes_at(wall))
            .or_else(|| fallback_offset_minutes(tzid))
    }
}

/// A DTSTART, DTEND, EXDATE, RECURRENCE-ID or UNTIL value in one of its four forms.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum DateTimeValue {
    /// VALUE=DATE: an all-day date.
    Date(Date),
    /// A time ending in Z.
    Utc(LocalDateTime),
    /// No zone: the wall time wherever the viewer is.
    Floating(LocalDateTime),
    /// TZID=...: a wall time in a named zone.
    Zoned { wall: LocalDateTime, tzid: String },
}

impl DateTimeValue {
    /// The wall time as written. A date reads as its midnight.
    pub fn wall(&self) -> LocalDateTime {
        match self {
            DateTimeValue::Date(d) => LocalDateTime::midnight(*d),
            DateTimeValue::Utc(w) | DateTimeValue::Floating(w) | DateTimeValue::Zoned { wall: w, .. } => *w,
        }
    }

    pub fn is_date(&self) -> bool {
        matches!(self, DateTimeValue::Date(_))
    }

    /// The same form (and zone) at another wall time.
    pub(crate) fn with_wall(&self, wall: LocalDateTime) -> DateTimeValue {
        match self {
            DateTimeValue::Date(_) => DateTimeValue::Date(wall.date()),
            DateTimeValue::Utc(_) => DateTimeValue::Utc(wall),
            DateTimeValue::Floating(_) => DateTimeValue::Floating(wall),
            DateTimeValue::Zoned { tzid, .. } => DateTimeValue::Zoned { wall, tzid: tzid.clone() },
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum EventEnd {
    /// DTEND.
    At(DateTimeValue),
    /// DURATION, in seconds.
    After(i64),
    /// Neither: an all-day event lasts its day, a timed one is a moment.
    Unspecified,
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum Status {
    #[default]
    Confirmed,
    Tentative,
    Cancelled,
}

impl Status {
    pub(crate) fn parse(value: &str) -> Status {
        match value.trim().to_ascii_uppercase().as_str() {
            "CANCELLED" => Status::Cancelled,
            "TENTATIVE" => Status::Tentative,
            _ => Status::Confirmed,
        }
    }
}

/// One VEVENT. A master carries the RRULE; an override shares its UID and has `recurrence_id`.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Event {
    pub uid: String,
    pub summary: String,
    pub description: Option<String>,
    pub location: Option<String>,
    pub url: Option<String>,
    /// The first Meet, Zoom or Teams join link found in the event.
    pub link: Option<MeetingLink>,
    pub status: Status,
    pub start: DateTimeValue,
    pub end: EventEnd,
    pub rrule: Option<RRule>,
    pub exdates: Vec<DateTimeValue>,
    pub recurrence_id: Option<DateTimeValue>,
}

/// Parses "20261106", "20261106T103000", "20261106T033000Z" with an optional TZID.
pub(crate) fn parse_date_time(value: &str, tzid: Option<&str>, date_only: bool) -> Option<DateTimeValue> {
    let v = value.trim();
    if !v.is_ascii() {
        return None;
    }
    if date_only || !v.contains(['T', 't']) {
        return parse_date(v).map(DateTimeValue::Date);
    }
    let (body, utc) = match v.strip_suffix(['Z', 'z']) {
        Some(b) => (b, true),
        None => (v, false),
    };
    let (d, t) = body.split_once(['T', 't'])?;
    if t.len() != 6 {
        return None;
    }
    // A leap second (60) is folded into :59.
    let second: u8 = digits::<u8>(&t[4..6])?.min(59);
    let wall = LocalDateTime::new(parse_date(d)?, digits(&t[0..2])?, digits(&t[2..4])?, second)?;
    Some(match tzid {
        _ if utc => DateTimeValue::Utc(wall),
        Some(tz) if !tz.is_empty() => DateTimeValue::Zoned { wall, tzid: tz.to_string() },
        _ => DateTimeValue::Floating(wall),
    })
}

fn parse_date(s: &str) -> Option<Date> {
    if s.len() != 8 || !s.is_ascii() {
        return None;
    }
    Date::new(digits(&s[0..4])?, digits(&s[4..6])?, digits(&s[6..8])?)
}

fn digits<T: FromStr>(s: &str) -> Option<T> {
    if s.is_empty() || !s.bytes().all(|b| b.is_ascii_digit()) {
        return None;
    }
    s.parse().ok()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn wall(y: i32, m: u8, d: u8, h: u8, mi: u8) -> LocalDateTime {
        LocalDateTime::new(Date::new(y, m, d).unwrap(), h, mi, 0).unwrap()
    }

    #[test]
    fn all_four_forms() {
        assert_eq!(
            parse_date_time("20261106", None, false),
            Some(DateTimeValue::Date(Date::new(2026, 11, 6).unwrap()))
        );
        assert_eq!(parse_date_time("20261106", None, true), Some(DateTimeValue::Date(Date::new(2026, 11, 6).unwrap())));
        assert_eq!(
            parse_date_time("20261106T033000Z", None, false),
            Some(DateTimeValue::Utc(wall(2026, 11, 6, 3, 30)))
        );
        assert_eq!(
            parse_date_time("20261106T033000Z", Some("Asia/Jakarta"), false),
            Some(DateTimeValue::Utc(wall(2026, 11, 6, 3, 30)))
        );
        assert_eq!(
            parse_date_time("20261106T103000", None, false),
            Some(DateTimeValue::Floating(wall(2026, 11, 6, 10, 30)))
        );
        assert_eq!(
            parse_date_time("20261106T103000", Some("Asia/Jakarta"), false),
            Some(DateTimeValue::Zoned { wall: wall(2026, 11, 6, 10, 30), tzid: "Asia/Jakarta".into() })
        );
    }

    #[test]
    fn garbage_is_rejected() {
        for bad in ["", "2026116", "20261332", "20261106T25", "20261106T250000", "2026-11-06", "20261106T10300é"] {
            assert_eq!(parse_date_time(bad, None, false), None, "{bad}");
        }
    }
}
