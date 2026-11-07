//! Proleptic Gregorian date arithmetic. Nothing here reads the system clock.

use std::fmt;

pub(crate) const DAY: i64 = 86_400;

/// A calendar date. Only valid dates can be built.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct Date {
    year: i32,
    month: u8,
    day: u8,
}

impl Date {
    pub fn new(year: i32, month: u8, day: u8) -> Option<Date> {
        let valid = (1..=12).contains(&month) && day >= 1 && day <= days_in_month(year, month);
        valid.then_some(Date { year, month, day })
    }

    pub fn year(self) -> i32 {
        self.year
    }
    pub fn month(self) -> u8 {
        self.month
    }
    pub fn day(self) -> u8 {
        self.day
    }

    /// Days since 1970-01-01 (Howard Hinnant's days_from_civil).
    pub fn days_since_epoch(self) -> i64 {
        let y = i64::from(self.year) - i64::from(self.month <= 2);
        let era = y.div_euclid(400);
        let yoe = y - era * 400;
        let m = i64::from(self.month);
        let mp = if m > 2 { m - 3 } else { m + 9 };
        let doy = (153 * mp + 2) / 5 + i64::from(self.day) - 1;
        let doe = yoe * 365 + yoe / 4 - yoe / 100 + doy;
        era * 146_097 + doe - 719_468
    }

    pub fn from_days_since_epoch(days: i64) -> Date {
        let z = days + 719_468;
        let era = z.div_euclid(146_097);
        let doe = z - era * 146_097;
        let yoe = (doe - doe / 1460 + doe / 36_524 - doe / 146_096) / 365;
        let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
        let mp = (5 * doy + 2) / 153;
        let day = (doy - (153 * mp + 2) / 5 + 1) as u8;
        let month = (if mp < 10 { mp + 3 } else { mp - 9 }) as u8;
        let year = (yoe + era * 400 + i64::from(month <= 2)) as i32;
        Date { year, month, day }
    }

    pub fn add_days(self, n: i64) -> Date {
        Date::from_days_since_epoch(self.days_since_epoch() + n)
    }

    pub fn weekday(self) -> Weekday {
        // 1970-01-01 was a Thursday, index 3 when Monday is 0.
        Weekday::from_monday_index((self.days_since_epoch() + 3).rem_euclid(7) as u8)
    }
}

impl fmt::Display for Date {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{:04}-{:02}-{:02}", self.year, self.month, self.day)
    }
}

pub fn is_leap_year(year: i32) -> bool {
    (year % 4 == 0 && year % 100 != 0) || year % 400 == 0
}

pub fn days_in_month(year: i32, month: u8) -> u8 {
    match month {
        1 | 3 | 5 | 7 | 8 | 10 | 12 => 31,
        4 | 6 | 9 | 11 => 30,
        2 if is_leap_year(year) => 29,
        2 => 28,
        _ => 0,
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum Weekday {
    Mon,
    Tue,
    Wed,
    Thu,
    Fri,
    Sat,
    Sun,
}

impl Weekday {
    const ALL: [Weekday; 7] =
        [Weekday::Mon, Weekday::Tue, Weekday::Wed, Weekday::Thu, Weekday::Fri, Weekday::Sat, Weekday::Sun];

    /// Monday is 0, Sunday is 6.
    pub fn monday_index(self) -> u8 {
        self as u8
    }

    fn from_monday_index(i: u8) -> Weekday {
        Self::ALL[usize::from(i % 7)]
    }

    /// The two-letter RFC 5545 code: MO, TU, WE, TH, FR, SA, SU.
    pub(crate) fn from_ics(code: &str) -> Option<Weekday> {
        let i = ["MO", "TU", "WE", "TH", "FR", "SA", "SU"].iter().position(|c| code.eq_ignore_ascii_case(c))?;
        Some(Self::ALL[i])
    }
}

/// A wall-clock date and time with no zone attached.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct LocalDateTime {
    date: Date,
    seconds: u32,
}

impl LocalDateTime {
    pub fn new(date: Date, hour: u8, minute: u8, second: u8) -> Option<LocalDateTime> {
        (hour < 24 && minute < 60 && second < 60).then(|| LocalDateTime {
            date,
            seconds: u32::from(hour) * 3600 + u32::from(minute) * 60 + u32::from(second),
        })
    }

    pub fn midnight(date: Date) -> LocalDateTime {
        LocalDateTime { date, seconds: 0 }
    }

    /// `seconds_of_day` is taken from another valid LocalDateTime, so it is below 86 400.
    pub(crate) fn at(date: Date, seconds_of_day: u32) -> LocalDateTime {
        LocalDateTime { date, seconds: seconds_of_day }
    }

    pub fn date(self) -> Date {
        self.date
    }
    pub fn seconds_of_day(self) -> u32 {
        self.seconds
    }
    pub fn hour(self) -> u8 {
        (self.seconds / 3600) as u8
    }
    pub fn minute(self) -> u8 {
        (self.seconds / 60 % 60) as u8
    }

    fn linear(self) -> i64 {
        self.date.days_since_epoch() * DAY + i64::from(self.seconds)
    }

    fn from_linear(s: i64) -> LocalDateTime {
        LocalDateTime { date: Date::from_days_since_epoch(s.div_euclid(DAY)), seconds: s.rem_euclid(DAY) as u32 }
    }

    pub fn add_seconds(self, s: i64) -> LocalDateTime {
        LocalDateTime::from_linear(self.linear() + s)
    }

    /// The instant this wall time names in a zone `offset_minutes` east of UTC.
    pub fn to_instant(self, offset_minutes: i32) -> Instant {
        Instant(self.linear() - i64::from(offset_minutes) * 60)
    }
}

impl fmt::Display for LocalDateTime {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let s = self.seconds;
        write!(f, "{}T{:02}:{:02}:{:02}", self.date, s / 3600, s / 60 % 60, s % 60)
    }
}

/// A point in time: whole seconds since 1970-01-01T00:00:00Z.
#[derive(Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct Instant(i64);

impl Instant {
    pub const fn from_unix_seconds(s: i64) -> Instant {
        Instant(s)
    }
    pub const fn unix_seconds(self) -> i64 {
        self.0
    }
    pub fn add_seconds(self, s: i64) -> Instant {
        Instant(self.0 + s)
    }
    /// The wall time at this instant in a zone `offset_minutes` east of UTC.
    pub fn to_local(self, offset_minutes: i32) -> LocalDateTime {
        LocalDateTime::from_linear(self.0 + i64::from(offset_minutes) * 60)
    }
}

impl fmt::Debug for Instant {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}Z", self.to_local(0))
    }
}

/// An RFC 5545 DURATION ("P1D", "PT1H30M", "P2W", "-PT15M") in seconds. Days count as 24 h.
pub(crate) fn parse_duration(s: &str) -> Option<i64> {
    let s = s.trim();
    let (sign, s) = match s.as_bytes().first()? {
        b'-' => (-1, &s[1..]),
        b'+' => (1, &s[1..]),
        _ => (1, s),
    };
    let s = s.strip_prefix(['P', 'p'])?;
    let (mut total, mut num, mut in_time, mut any) = (0i64, String::new(), false, false);
    for c in s.chars() {
        match c.to_ascii_uppercase() {
            '0'..='9' => num.push(c),
            'T' if !in_time && num.is_empty() => in_time = true,
            unit => {
                let n: i64 = num.parse().ok()?;
                num.clear();
                let secs = match (unit, in_time) {
                    ('W', false) => 7 * DAY,
                    ('D', false) => DAY,
                    ('H', true) => 3600,
                    ('M', true) => 60,
                    ('S', true) => 1,
                    _ => return None,
                };
                total += n * secs;
                any = true;
            }
        }
    }
    (any && num.is_empty()).then_some(sign * total)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn d(y: i32, m: u8, day: u8) -> Date {
        Date::new(y, m, day).unwrap()
    }

    #[test]
    fn epoch_round_trip_and_weekdays() {
        assert_eq!(d(1970, 1, 1).days_since_epoch(), 0);
        assert_eq!(d(1970, 1, 1).weekday(), Weekday::Thu);
        assert_eq!(d(2026, 11, 6).weekday(), Weekday::Fri);
        assert_eq!(d(1601, 1, 1).weekday(), Weekday::Mon);
        for days in [-800_000, -1, 0, 59, 20_398, 400_000] {
            assert_eq!(Date::from_days_since_epoch(days).days_since_epoch(), days);
        }
        assert_eq!(d(2028, 2, 28).add_days(1), d(2028, 2, 29));
        assert_eq!(d(2026, 12, 31).add_days(1), d(2027, 1, 1));
    }

    #[test]
    fn invalid_dates_are_rejected() {
        assert!(Date::new(2027, 2, 29).is_none());
        assert!(Date::new(2028, 2, 29).is_some());
        assert!(Date::new(1900, 2, 29).is_none());
        assert!(Date::new(2000, 2, 29).is_some());
        assert!(Date::new(2026, 13, 1).is_none());
        assert!(Date::new(2026, 4, 31).is_none());
    }

    #[test]
    fn local_time_and_instant_conversion() {
        let wall = LocalDateTime::new(d(2026, 11, 6), 10, 20, 0).unwrap();
        let utc = wall.to_instant(7 * 60);
        assert_eq!(format!("{utc:?}"), "2026-11-06T03:20:00Z");
        assert_eq!(utc.to_local(7 * 60), wall);
        assert_eq!(wall.add_seconds(14 * 3600).date(), d(2026, 11, 7));
        assert!(LocalDateTime::new(d(2026, 1, 1), 24, 0, 0).is_none());
    }

    #[test]
    fn durations() {
        assert_eq!(parse_duration("PT30M"), Some(1800));
        assert_eq!(parse_duration("PT1H30M"), Some(5400));
        assert_eq!(parse_duration("P1D"), Some(DAY));
        assert_eq!(parse_duration("P2W"), Some(14 * DAY));
        assert_eq!(parse_duration("P1DT2H"), Some(DAY + 7200));
        assert_eq!(parse_duration("-PT15M"), Some(-900));
        assert_eq!(parse_duration("-P0DT0H10M0S"), Some(-600));
        assert_eq!(parse_duration("P"), None);
        assert_eq!(parse_duration("PT5"), None);
        assert_eq!(parse_duration("1H"), None);
    }
}
