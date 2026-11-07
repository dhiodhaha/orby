//! Civil dates, clock times and weekdays, with the day arithmetic the parser needs.

/// ISO order: the week starts on Monday.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub enum Weekday {
    Monday,
    Tuesday,
    Wednesday,
    Thursday,
    Friday,
    Saturday,
    Sunday,
}

impl Weekday {
    const ALL: [Weekday; 7] = [
        Weekday::Monday,
        Weekday::Tuesday,
        Weekday::Wednesday,
        Weekday::Thursday,
        Weekday::Friday,
        Weekday::Saturday,
        Weekday::Sunday,
    ];

    /// 0 for Monday up to 6 for Sunday.
    pub fn days_from_monday(self) -> i64 {
        self as i64
    }

    fn from_days_from_monday(n: i64) -> Weekday {
        Weekday::ALL[n.rem_euclid(7) as usize]
    }
}

/// A valid proleptic Gregorian date. Build it with [`Date::new`].
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct Date {
    year: i32,
    month: u8,
    day: u8,
}

impl Date {
    pub fn new(year: i32, month: u8, day: u8) -> Option<Date> {
        if (1..=12).contains(&month) && day >= 1 && day <= days_in_month(year, month) {
            Some(Date { year, month, day })
        } else {
            None
        }
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

    pub fn weekday(self) -> Weekday {
        // 1970-01-01 was a Thursday, three days after a Monday.
        Weekday::from_days_from_monday(self.days_since_epoch() + 3)
    }

    pub fn add_days(self, n: i64) -> Date {
        Date::from_days_since_epoch(self.days_since_epoch() + n)
    }

    /// Days since 1970-01-01 (Howard Hinnant's days_from_civil).
    fn days_since_epoch(self) -> i64 {
        let m = self.month as i64;
        let d = self.day as i64;
        let y = self.year as i64 - i64::from(m <= 2);
        let era = y.div_euclid(400);
        let yoe = y - era * 400;
        let doy = (153 * (m + if m > 2 { -3 } else { 9 }) + 2) / 5 + d - 1;
        let doe = yoe * 365 + yoe / 4 - yoe / 100 + doy;
        era * 146_097 + doe - 719_468
    }

    fn from_days_since_epoch(z: i64) -> Date {
        let z = z + 719_468;
        let era = z.div_euclid(146_097);
        let doe = z - era * 146_097;
        let yoe = (doe - doe / 1460 + doe / 36_524 - doe / 146_096) / 365;
        let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
        let mp = (5 * doy + 2) / 153;
        let day = (doy - (153 * mp + 2) / 5 + 1) as u8;
        let month = if mp < 10 { mp + 3 } else { mp - 9 } as u8;
        let year = (yoe + era * 400 + i64::from(month <= 2)) as i32;
        Date { year, month, day }
    }
}

pub fn days_in_month(year: i32, month: u8) -> u8 {
    match month {
        1 | 3 | 5 | 7 | 8 | 10 | 12 => 31,
        4 | 6 | 9 | 11 => 30,
        2 if (year % 4 == 0 && year % 100 != 0) || year % 400 == 0 => 29,
        2 => 28,
        _ => 0,
    }
}

/// A wall-clock time of day, minute precision. Build it with [`Time::new`].
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct Time {
    hour: u8,
    minute: u8,
}

impl Time {
    pub fn new(hour: u8, minute: u8) -> Option<Time> {
        (hour < 24 && minute < 60).then_some(Time { hour, minute })
    }
    pub fn hour(self) -> u8 {
        self.hour
    }
    pub fn minute(self) -> u8 {
        self.minute
    }
}

/// The caller's local "now". The weekday is derived from the date so the two can never disagree.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Now {
    pub date: Date,
    pub time: Time,
}

impl Now {
    pub fn weekday(self) -> Weekday {
        self.date.weekday()
    }
}

/// First `wd` strictly after `today` (1 to 7 days ahead).
pub(crate) fn next_weekday_after(today: Date, wd: Weekday) -> Date {
    let diff = (wd.days_from_monday() - today.weekday().days_from_monday()).rem_euclid(7);
    today.add_days(if diff == 0 { 7 } else { diff })
}

/// First `wd` on or after `today` (0 to 6 days ahead).
pub(crate) fn next_weekday_from(today: Date, wd: Weekday) -> Date {
    let diff = (wd.days_from_monday() - today.weekday().days_from_monday()).rem_euclid(7);
    today.add_days(diff)
}

/// `wd` in the calendar week (Monday to Sunday) after the one holding `today`.
pub(crate) fn weekday_of_next_week(today: Date, wd: Weekday) -> Date {
    let next_monday = today.add_days(7 - today.weekday().days_from_monday());
    next_monday.add_days(wd.days_from_monday())
}

/// The first date on or after `today` whose day of month is `day`, skipping months too short for it.
pub(crate) fn next_day_of_month(today: Date, day: u8) -> Option<Date> {
    let (mut y, mut m) = (today.year(), today.month());
    for _ in 0..13 {
        if let Some(d) = Date::new(y, m, day) {
            if d >= today {
                return Some(d);
            }
        }
        if m == 12 {
            y += 1;
            m = 1;
        } else {
            m += 1;
        }
    }
    None
}

/// The first `day`/`month` on or after `today`. Up to 8 years ahead so 29 Feb finds a leap year.
pub(crate) fn next_day_and_month(today: Date, day: u8, month: u8) -> Option<Date> {
    (0..=8).filter_map(|k| Date::new(today.year() + k, month, day)).find(|d| *d >= today)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn d(y: i32, m: u8, day: u8) -> Date {
        Date::new(y, m, day).unwrap()
    }

    #[test]
    fn weekdays_of_known_dates() {
        assert_eq!(d(2026, 11, 6).weekday(), Weekday::Friday);
        assert_eq!(d(1970, 1, 1).weekday(), Weekday::Thursday);
        assert_eq!(d(2000, 2, 29).weekday(), Weekday::Tuesday);
        assert_eq!(d(2027, 1, 1).weekday(), Weekday::Friday);
    }

    #[test]
    fn day_arithmetic_crosses_months_years_and_leap_days() {
        assert_eq!(d(2026, 12, 31).add_days(1), d(2027, 1, 1));
        assert_eq!(d(2028, 2, 28).add_days(1), d(2028, 2, 29));
        assert_eq!(d(2027, 2, 28).add_days(1), d(2027, 3, 1));
        assert_eq!(d(2026, 3, 1).add_days(-1), d(2026, 2, 28));
        assert_eq!(d(1969, 12, 31).add_days(366), d(1971, 1, 1));
    }

    #[test]
    fn invalid_dates_and_times_cannot_be_built() {
        assert!(Date::new(2026, 11, 31).is_none());
        assert!(Date::new(2027, 2, 29).is_none());
        assert!(Date::new(2026, 13, 1).is_none());
        assert!(Date::new(2026, 1, 0).is_none());
        assert!(Time::new(24, 0).is_none());
        assert!(Time::new(9, 60).is_none());
    }

    #[test]
    fn next_29_february_finds_a_leap_year() {
        assert_eq!(next_day_and_month(d(2026, 11, 6), 29, 2), Some(d(2028, 2, 29)));
    }
}
