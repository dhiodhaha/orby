// Shared by several test binaries; each uses a different subset.
#![allow(dead_code)]

use app_calendar::{occurrences, Calendar, Date, Instant, LocalDateTime, Occurrence};

pub const WIB: i32 = 7 * 60;

pub const GOOGLE: &str = include_str!("../fixtures/google.ics");
pub const OUTLOOK: &str = include_str!("../fixtures/outlook.ics");

pub fn at(y: i32, m: u8, d: u8, h: u8, mi: u8, offset_minutes: i32) -> Instant {
    LocalDateTime::new(Date::new(y, m, d).unwrap(), h, mi, 0).unwrap().to_instant(offset_minutes)
}

pub fn wib(y: i32, m: u8, d: u8, h: u8, mi: u8) -> Instant {
    at(y, m, d, h, mi, WIB)
}

pub fn utc(y: i32, m: u8, d: u8, h: u8, mi: u8) -> Instant {
    at(y, m, d, h, mi, 0)
}

/// Starts of the occurrences titled `title` in [from, to), viewed from WIB.
pub fn starts(cal: &Calendar, title: &str, from: Instant, to: Instant) -> Vec<Instant> {
    occurrences(cal, from, to, WIB).into_iter().filter(|o| o.title == title).map(|o| o.start).collect()
}

pub fn titles(occ: &[Occurrence]) -> Vec<&str> {
    occ.iter().map(|o| o.title.as_str()).collect()
}

pub fn wrap(body: &str) -> String {
    format!("BEGIN:VCALENDAR\nVERSION:2.0\nPRODID:test\n{body}END:VCALENDAR\n")
}
