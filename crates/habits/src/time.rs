//! Local wall-clock time without a time zone database.
//!
//! A [`Moment`] counts seconds from 1970-01-01 00:00 in local time. The app converts once with
//! [`Moment::from_unix`] and the UTC offset it already knows; everything below works in local time
//! because reminders, active hours and prayer times are all local.

use std::ops::{Add, Sub};
use std::time::Duration;

/// A duration of whole minutes.
pub const fn mins(n: u64) -> Duration {
    Duration::from_secs(n * 60)
}

/// A calendar date (proleptic Gregorian).
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct Date {
    year: i32,
    month: u32,
    day: u32,
}

impl Date {
    /// `None` for a date that does not exist, such as 31 April.
    pub fn new(year: i32, month: u32, day: u32) -> Option<Date> {
        if !(1..=12).contains(&month) || day == 0 {
            return None;
        }
        let d = Date { year, month, day };
        (Date::from_days(d.days()) == d).then_some(d)
    }

    pub fn year(self) -> i32 {
        self.year
    }
    pub fn month(self) -> u32 {
        self.month
    }
    pub fn day(self) -> u32 {
        self.day
    }

    /// The moment `minute` minutes after this date's midnight (may run into the next day).
    pub fn at_minute(self, minute: u32) -> Moment {
        Moment(self.days() * 86_400 + i64::from(minute) * 60)
    }

    pub fn at(self, hour: u32, minute: u32) -> Moment {
        self.at_minute(hour * 60 + minute)
    }

    pub fn next(self) -> Date {
        Date::from_days(self.days() + 1)
    }

    pub fn prev(self) -> Date {
        Date::from_days(self.days() - 1)
    }

    /// Days since 1970-01-01 (H. Hinnant's days_from_civil).
    fn days(self) -> i64 {
        let y = i64::from(if self.month <= 2 { self.year - 1 } else { self.year });
        let era = if y >= 0 { y } else { y - 399 } / 400;
        let yoe = y - era * 400;
        let mp = i64::from((self.month + 9) % 12);
        let doy = (153 * mp + 2) / 5 + i64::from(self.day) - 1;
        let doe = yoe * 365 + yoe / 4 - yoe / 100 + doy;
        era * 146_097 + doe - 719_468
    }

    fn from_days(z: i64) -> Date {
        let z = z + 719_468;
        let era = if z >= 0 { z } else { z - 146_096 } / 146_097;
        let doe = z - era * 146_097;
        let yoe = (doe - doe / 1460 + doe / 36_524 - doe / 146_096) / 365;
        let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
        let mp = (5 * doy + 2) / 153;
        let day = (doy - (153 * mp + 2) / 5 + 1) as u32;
        let month = if mp < 10 { mp + 3 } else { mp - 9 } as u32;
        let year = (yoe + era * 400 + i64::from(month <= 2)) as i32;
        Date { year, month, day }
    }
}

/// A local wall-clock instant, to the second.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct Moment(i64);

impl Moment {
    /// From Unix seconds and the local UTC offset in seconds (WIB is 7 * 3600).
    pub fn from_unix(unix_secs: i64, utc_offset_secs: i64) -> Moment {
        Moment(unix_secs + utc_offset_secs)
    }

    pub fn date(self) -> Date {
        Date::from_days(self.0.div_euclid(86_400))
    }

    /// Whole minutes after local midnight, 0 to 1439.
    pub fn minute_of_day(self) -> u32 {
        (self.0.rem_euclid(86_400) / 60) as u32
    }

    /// Time from `earlier` to `self`, or zero if `earlier` is later.
    pub fn since(self, earlier: Moment) -> Duration {
        Duration::from_secs(u64::try_from(self.0 - earlier.0).unwrap_or(0))
    }
}

impl Add<Duration> for Moment {
    type Output = Moment;
    fn add(self, d: Duration) -> Moment {
        Moment(self.0 + d.as_secs() as i64)
    }
}

impl Sub<Duration> for Moment {
    type Output = Moment;
    fn sub(self, d: Duration) -> Moment {
        Moment(self.0 - d.as_secs() as i64)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn dates_round_trip_through_moments() {
        let d = Date::new(2026, 11, 6).unwrap();
        let m = d.at(14, 20);
        assert_eq!(m.date(), d);
        assert_eq!(m.minute_of_day(), 14 * 60 + 20);
        assert_eq!(d.next(), Date::new(2026, 11, 7).unwrap());
        assert_eq!(Date::new(2024, 3, 1).unwrap().prev(), Date::new(2024, 2, 29).unwrap());
        assert_eq!(Date::new(2026, 1, 1).unwrap().prev(), Date::new(2025, 12, 31).unwrap());
    }

    #[test]
    fn rejects_dates_that_do_not_exist() {
        assert!(Date::new(2026, 4, 31).is_none());
        assert!(Date::new(2026, 2, 29).is_none());
        assert!(Date::new(2026, 13, 1).is_none());
        assert!(Date::new(2024, 2, 29).is_some());
    }

    #[test]
    fn unix_seconds_convert_with_the_offset() {
        // 2026-11-06 07:20 UTC is 14:20 WIB.
        let m = Moment::from_unix(1_793_949_600, 7 * 3600);
        assert_eq!(m.date(), Date::new(2026, 11, 6).unwrap());
        assert_eq!(m.minute_of_day(), 14 * 60 + 20);
    }

    #[test]
    fn since_saturates_at_zero() {
        let a = Date::new(2026, 11, 6).unwrap().at(10, 0);
        assert_eq!((a + mins(5)).since(a), mins(5));
        assert_eq!(a.since(a + mins(5)), Duration::ZERO);
    }
}
