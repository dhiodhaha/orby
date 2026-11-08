//! Hijri month starts: officially announced dates, not astronomy.
//!
//! Two sources, merged per calendar in [`Months`]:
//! - a small built-in table (below), the offline fallback;
//! - for Muhammadiyah, the official KHGT calendar API (parsed in [`crate::khgt`], fetched and
//!   cached by the app). Fetched months win over built-in ones.
//!
//! A date outside the known months is `Unknown`, never guessed.
//!
//! BUILT-IN TABLE, refresh each year after each announcement (see docs/followups.md):
//! - Kemenag (MABIMS criteria, set by the sidang isbat): 1 Ramadan 1447 H = Thu 19 Feb 2026 and
//!   1 Syawal 1447 H = Sat 21 Mar 2026 (Menteri Agama, sidang isbat of 17 Feb and 19 Mar 2026).
//!   1448 H is not set yet: its sidang isbat is in Feb 2027.
//! - Muhammadiyah (KHGT, Kalender Hijriah Global Tunggal, in force since 1 Muharram 1447 H =
//!   26 Jun 2025): 1 Ramadan 1447 H = Wed 18 Feb 2026, 1 Syawal 1447 H = Fri 20 Mar 2026
//!   (Maklumat PP Muhammadiyah); 1 Ramadan 1448 H = Mon 8 Feb 2027, 1 Syawal 1448 H = Tue
//!   9 Mar 2027 (the published KHGT calendar; the API gives the same dates).
//!   KHGT: a month begins when, before 24:00 UTC, somewhere on land the moon is at least 5° high
//!   at sunset with an elongation of at least 8°; or, when that happens only after 24:00 UTC, on
//!   land in the Americas, provided the conjunction falls before dawn in New Zealand.

use crate::prayer::julian;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum HijriCalendar {
    /// The government's: MABIMS criteria and the sidang isbat.
    Kemenag,
    /// Muhammadiyah's KHGT.
    Muhammadiyah,
}

/// (year, month, day), Gregorian.
pub type Date = (i32, u32, u32);

/// Month names, KBBI spelling, Muharam first.
pub const MONTHS: [&str; 12] = [
    "Muharam",
    "Safar",
    "Rabiulawal",
    "Rabiulakhir",
    "Jumadilawal",
    "Jumadilakhir",
    "Rajab",
    "Syakban",
    "Ramadan",
    "Syawal",
    "Zulkaidah",
    "Zulhijah",
];

/// The first day of one Hijri month.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct MonthStart {
    pub year: u32,
    /// 1 (Muharam) to 12 (Zulhijah).
    pub month: u32,
    pub start: Date,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct HijriDate {
    pub year: u32,
    pub month: u32,
    pub day: u32,
}

impl HijriDate {
    /// "28 Jumadilawal 1448 H".
    pub fn label(&self) -> String {
        let name = MONTHS
            .get(self.month.wrapping_sub(1) as usize)
            .copied()
            .unwrap_or("?");
        format!("{} {} {} H", self.day, name, self.year)
    }
}

/// (Hijri year, 1 Ramadan, 1 Syawal).
const KEMENAG: &[(u32, Date, Date)] = &[(1447, (2026, 2, 19), (2026, 3, 21))];
const MUHAMMADIYAH: &[(u32, Date, Date)] = &[
    (1447, (2026, 2, 18), (2026, 3, 20)),
    (1448, (2027, 2, 8), (2027, 3, 9)),
];

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Ramadan {
    /// The day of Ramadan, 1 to 30.
    Day(u32),
    NotRamadan,
    /// Past the known months: no announcement for this year yet.
    Unknown,
}

pub(crate) fn jd((y, m, d): Date) -> i64 {
    julian(y, m, d).round() as i64
}

fn next_month(year: u32, month: u32) -> (u32, u32) {
    if month >= 12 {
        (year + 1, 1)
    } else {
        (year, month + 1)
    }
}

/// The Hijri year a date falls in, give or take one near the year's edges. Enough to pick
/// which years to ask for.
pub fn approx_year(date: Date) -> u32 {
    // 1 Muharram 1 H is about JD 1948439.5; a mean year is 10631/30 days.
    let days = julian(date.0, date.1, date.2) - 1948439.5;
    ((days * 30.0 / 10631.0).floor() + 1.0).max(1.0) as u32
}

/// Known month starts for one calendar, sorted, one per (year, month).
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Months {
    starts: Vec<MonthStart>,
}

impl Months {
    /// The built-in table: only Ramadan and Syawal of each listed year.
    pub fn builtin(c: HijriCalendar) -> Months {
        let t = match c {
            HijriCalendar::Kemenag => KEMENAG,
            HijriCalendar::Muhammadiyah => MUHAMMADIYAH,
        };
        let mut starts = Vec::with_capacity(t.len() * 2);
        for &(year, ramadan, syawal) in t {
            starts.push(MonthStart {
                year,
                month: 9,
                start: ramadan,
            });
            starts.push(MonthStart {
                year,
                month: 10,
                start: syawal,
            });
        }
        Months { starts }
    }

    /// These months plus `fetched`; for a month in both, the fetched start wins.
    pub fn merged(&self, fetched: &[MonthStart]) -> Months {
        let mut starts: Vec<MonthStart> = fetched.to_vec();
        for m in &self.starts {
            if !starts
                .iter()
                .any(|f| f.year == m.year && f.month == m.month)
            {
                starts.push(*m);
            }
        }
        starts.sort_by_key(|m| (m.year, m.month));
        starts.dedup_by_key(|m| (m.year, m.month));
        Months { starts }
    }

    pub fn len(&self) -> usize {
        self.starts.len()
    }

    pub fn is_empty(&self) -> bool {
        self.starts.is_empty()
    }

    fn find(&self, year: u32, month: u32) -> Option<&MonthStart> {
        self.starts
            .iter()
            .find(|m| m.year == year && m.month == month)
    }

    /// The Hijri date of `date`, when both its month's start and the next month's start are
    /// known (so the month's length is known too).
    pub fn hijri_date(&self, date: Date) -> Option<HijriDate> {
        let day = jd(date);
        let m = self
            .starts
            .iter()
            .filter(|m| jd(m.start) <= day)
            .max_by_key(|m| jd(m.start))?;
        let (ny, nm) = next_month(m.year, m.month);
        let next = self.find(ny, nm)?;
        let (s, e) = (jd(m.start), jd(next.start));
        if day >= e || !(29..=30).contains(&(e - s)) {
            return None;
        }
        Some(HijriDate {
            year: m.year,
            month: m.month,
            day: (day - s + 1) as u32,
        })
    }

    /// Whether `date` is in Ramadan. Dates in a known month are known; so are those from a
    /// listed Syawal up to the next listed Ramadan, and about 11 months before the first one.
    pub fn ramadan(&self, date: Date) -> Ramadan {
        if let Some(h) = self.hijri_date(date) {
            return if h.month == 9 {
                Ramadan::Day(h.day)
            } else {
                Ramadan::NotRamadan
            };
        }
        let day = jd(date);
        let ramadans: Vec<&MonthStart> = self.starts.iter().filter(|m| m.month == 9).collect();
        for (i, r) in ramadans.iter().enumerate() {
            let Some(syawal) = self.find(r.year, 10) else {
                continue;
            };
            let (s, e) = (jd(r.start), jd(syawal.start));
            if day >= s && day < e {
                return Ramadan::Day((day - s + 1) as u32);
            }
            if day >= e {
                if let Some(next) = ramadans.get(i + 1) {
                    if day < jd(next.start) {
                        return Ramadan::NotRamadan;
                    }
                }
            }
        }
        match ramadans.first() {
            // About 11 months before the first listed Ramadan is still the year before it.
            Some(first) if day < jd(first.start) && day >= jd(first.start) - 330 => {
                Ramadan::NotRamadan
            }
            _ => Ramadan::Unknown,
        }
    }

    /// The last known month start: answers stop there, so refresh before then.
    pub fn known_until(&self) -> Option<Date> {
        self.starts.last().map(|m| m.start)
    }
}

/// Whether `date` is in Ramadan by the built-in table alone.
pub fn ramadan(c: HijriCalendar, date: Date) -> Ramadan {
    Months::builtin(c).ramadan(date)
}

/// The last day the built-in table can answer for: refresh it before then.
pub fn known_until(c: HijriCalendar) -> Option<Date> {
    Months::builtin(c).known_until()
}

#[cfg(test)]
mod tests {
    use super::*;
    use HijriCalendar::*;

    #[test]
    fn ramadan_1447_starts_a_day_apart_and_both_last_30_days() {
        assert_eq!(ramadan(Muhammadiyah, (2026, 2, 18)), Ramadan::Day(1));
        assert_eq!(ramadan(Kemenag, (2026, 2, 18)), Ramadan::NotRamadan);
        assert_eq!(ramadan(Kemenag, (2026, 2, 19)), Ramadan::Day(1));
        assert_eq!(ramadan(Muhammadiyah, (2026, 3, 19)), Ramadan::Day(30));
        assert_eq!(
            ramadan(Muhammadiyah, (2026, 3, 20)),
            Ramadan::NotRamadan,
            "Idulfitri"
        );
        assert_eq!(ramadan(Kemenag, (2026, 3, 20)), Ramadan::Day(30));
        assert_eq!(
            ramadan(Kemenag, (2026, 3, 21)),
            Ramadan::Unknown,
            "1448 is not set yet"
        );
    }

    #[test]
    fn muhammadiyah_knows_ramadan_1448_and_the_months_between() {
        assert_eq!(ramadan(Muhammadiyah, (2026, 11, 7)), Ramadan::NotRamadan);
        assert_eq!(ramadan(Muhammadiyah, (2027, 2, 7)), Ramadan::NotRamadan);
        assert_eq!(ramadan(Muhammadiyah, (2027, 2, 8)), Ramadan::Day(1));
        assert_eq!(ramadan(Muhammadiyah, (2027, 3, 8)), Ramadan::Day(29));
        assert_eq!(ramadan(Muhammadiyah, (2027, 3, 9)), Ramadan::Unknown);
        assert_eq!(known_until(Muhammadiyah), Some((2027, 3, 9)));
    }

    #[test]
    fn every_listed_month_is_29_or_30_days() {
        for c in [Kemenag, Muhammadiyah] {
            let m = Months::builtin(c);
            for pair in m.starts.chunks(2) {
                let n = jd(pair[1].start) - jd(pair[0].start);
                assert!(n == 29 || n == 30, "{c:?} {}: {n}", pair[0].year);
            }
        }
    }

    #[test]
    fn the_builtin_table_only_dates_ramadan() {
        let m = Months::builtin(Muhammadiyah);
        assert_eq!(
            m.hijri_date((2026, 3, 1)),
            Some(HijriDate {
                year: 1447,
                month: 9,
                day: 12
            })
        );
        assert_eq!(m.hijri_date((2026, 11, 8)), None);
    }

    #[test]
    fn fetched_months_win_and_builtin_ones_stay() {
        let base = Months::builtin(Muhammadiyah);
        // A made-up correction: Syakban, Ramadan and Syawal 1448 each a day later.
        let fetched = [
            MonthStart {
                year: 1448,
                month: 8,
                start: (2027, 1, 10),
            },
            MonthStart {
                year: 1448,
                month: 9,
                start: (2027, 2, 9),
            },
            MonthStart {
                year: 1448,
                month: 10,
                start: (2027, 3, 10),
            },
        ];
        let m = base.merged(&fetched);
        assert_eq!(m.len(), 5);
        assert_eq!(
            m.hijri_date((2027, 2, 8)).map(|h| (h.month, h.day)),
            Some((8, 30))
        );
        assert_eq!(m.ramadan((2027, 2, 8)), Ramadan::NotRamadan);
        assert_eq!(m.ramadan((2027, 3, 9)), Ramadan::Day(29));
        assert_eq!(m.ramadan((2027, 2, 9)), Ramadan::Day(1));
        assert_eq!(m.ramadan((2026, 2, 18)), Ramadan::Day(1), "1447 kept");
    }

    #[test]
    fn approx_year_is_close() {
        assert_eq!(approx_year((2026, 11, 8)), 1448);
        assert_eq!(approx_year((2026, 2, 18)), 1447);
        let y = approx_year((2025, 6, 26));
        assert!(y == 1446 || y == 1447, "{y}");
    }

    #[test]
    fn label_uses_kbbi_names() {
        let h = HijriDate {
            year: 1448,
            month: 5,
            day: 28,
        };
        assert_eq!(h.label(), "28 Jumadilawal 1448 H");
    }
}
