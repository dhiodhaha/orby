//! Word tables and small token readers. Every input here is already lowercased.

use crate::calendar::{Time, Weekday};

/// Indonesian 3-letter forms "sen", "sel" and "min" are left out on purpose: they are everyday
/// words (cent, cell, minus or admin) and would turn plain titles into dates.
pub(crate) fn weekday(word: &str) -> Option<Weekday> {
    Some(match word {
        "senin" | "monday" | "mon" => Weekday::Monday,
        "selasa" | "tuesday" | "tue" | "tues" => Weekday::Tuesday,
        "rabu" | "rab" | "wednesday" | "wed" => Weekday::Wednesday,
        "kamis" | "kam" | "thursday" | "thu" | "thur" | "thurs" => Weekday::Thursday,
        "jumat" | "jum'at" | "jum\u{2019}at" | "jum" | "friday" | "fri" => Weekday::Friday,
        "sabtu" | "sab" | "saturday" | "sat" => Weekday::Saturday,
        "minggu" | "ahad" | "sunday" | "sun" => Weekday::Sunday,
        _ => return None,
    })
}

pub(crate) fn month(word: &str) -> Option<u8> {
    Some(match word {
        "januari" | "january" | "jan" => 1,
        "februari" | "february" | "feb" => 2,
        "maret" | "march" | "mar" => 3,
        "april" | "apr" => 4,
        "mei" | "may" => 5,
        "juni" | "june" | "jun" => 6,
        "juli" | "july" | "jul" => 7,
        "agustus" | "august" | "agu" | "agt" | "ags" | "aug" => 8,
        "september" | "sep" | "sept" => 9,
        "oktober" | "october" | "okt" | "oct" => 10,
        "november" | "nov" => 11,
        "desember" | "december" | "des" | "dec" => 12,
        _ => return None,
    })
}

/// A plain 1 or 2 digit number.
pub(crate) fn small_number(s: &str) -> Option<u8> {
    digits(s, 1, 2)
}

pub(crate) fn day_of_month(s: &str) -> Option<u8> {
    small_number(s).filter(|d| (1..=31).contains(d))
}

/// "15/11": day then month, the Indonesian order.
pub(crate) fn day_slash_month(s: &str) -> Option<(u8, u8)> {
    let (d, m) = s.split_once('/')?;
    Some((day_of_month(d)?, small_number(m).filter(|m| (1..=12).contains(m))?))
}

fn digits(s: &str, min_len: usize, max_len: usize) -> Option<u8> {
    if (min_len..=max_len).contains(&s.len()) && s.bytes().all(|b| b.is_ascii_digit()) {
        s.parse().ok()
    } else {
        None
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Meridiem {
    Am,
    Pm,
}

pub(crate) fn meridiem(word: &str) -> Option<Meridiem> {
    match word {
        "am" => Some(Meridiem::Am),
        "pm" => Some(Meridiem::Pm),
        _ => None,
    }
}

/// Indonesian parts of the day that qualify an hour: pagi, siang, sore, malam.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum DayPart {
    Morning,
    Midday,
    Afternoon,
    Night,
}

pub(crate) fn day_part(word: &str) -> Option<DayPart> {
    match word {
        "pagi" => Some(DayPart::Morning),
        "siang" => Some(DayPart::Midday),
        "sore" => Some(DayPart::Afternoon),
        "malam" => Some(DayPart::Night),
        _ => None,
    }
}

/// A clock reading as typed: "9", "15:30", "15.30", "9am", "9:30pm".
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) struct Clock {
    pub hour: u8,
    pub minute: Option<u8>,
    pub meridiem: Option<Meridiem>,
}

pub(crate) fn clock(s: &str) -> Option<Clock> {
    let (body, meridiem) = match (s.strip_suffix("am"), s.strip_suffix("pm")) {
        (Some(b), _) => (b, Some(Meridiem::Am)),
        (_, Some(b)) => (b, Some(Meridiem::Pm)),
        _ => (s, None),
    };
    let (h, m) = match body.split_once([':', '.']) {
        Some((h, m)) => (h, Some(m)),
        None => (body, None),
    };
    let hour = small_number(h).filter(|h| *h < 24)?;
    let minute = match m {
        Some(m) => Some(digits(m, 2, 2).filter(|m| *m < 60)?),
        None => None,
    };
    Some(Clock { hour, minute, meridiem })
}

impl Clock {
    fn at(self, hour: u8) -> Option<Time> {
        Time::new(hour, self.minute.unwrap_or(0))
    }

    /// 12-hour reading; the hour must be 1 to 12.
    pub fn with_meridiem(self, m: Meridiem) -> Option<Time> {
        let h = self.hour;
        if !(1..=12).contains(&h) {
            return None;
        }
        self.at(match (m, h) {
            (Meridiem::Am, 12) => 0,
            (Meridiem::Am, h) => h,
            (Meridiem::Pm, 12) => 12,
            (Meridiem::Pm, h) => h + 12,
        })
    }

    pub fn with_day_part(self, part: DayPart) -> Option<Time> {
        let h = self.hour;
        self.at(match part {
            DayPart::Morning => h,
            DayPart::Midday if h <= 6 => h + 12,
            DayPart::Midday => h,
            DayPart::Afternoon if h < 12 => h + 12,
            DayPart::Afternoon => h,
            DayPart::Night if (6..=11).contains(&h) => h + 12,
            DayPart::Night if h == 12 => 0,
            DayPart::Night => h,
        })
    }

    /// "jam 3" with nothing after it: nobody sets a task for 3 at night, so 1 to 6 is afternoon.
    pub fn after_jam(self) -> Option<Time> {
        let h = self.hour;
        self.at(if (1..=6).contains(&h) { h + 12 } else { h })
    }

    /// A bare reading taken as a 24-hour clock.
    pub fn literal(self) -> Option<Time> {
        self.at(self.hour)
    }
}

/// Lowercase, with spaces and underscores read as hyphens, so "#client-a" finds "Client A".
fn slug(s: &str) -> String {
    s.chars()
        .flat_map(char::to_lowercase)
        .map(|c| if c.is_whitespace() || c == '_' { '-' } else { c })
        .collect()
}

/// An exact name wins; otherwise a prefix counts only when it fits exactly one name.
pub(crate) fn match_name<'k>(typed: &str, names: &'k [String]) -> Option<&'k str> {
    let t = slug(typed);
    if t.is_empty() {
        return None;
    }
    if let Some(n) = names.iter().find(|n| slug(n) == t) {
        return Some(n);
    }
    unique(names.iter().filter(|n| slug(n).starts_with(&t))).map(String::as_str)
}

/// Same rule as [`match_name`], segment by segment, on paths of the same depth.
pub(crate) fn match_folder<'k>(typed: &str, folders: &'k [String]) -> Option<&'k str> {
    let segments = |p: &str| p.split('/').filter(|s| !s.is_empty()).map(slug).collect::<Vec<_>>();
    let want = segments(typed);
    if want.is_empty() {
        return None;
    }
    let same_depth =
        || folders.iter().map(move |f| (f, segments(f))).filter(|(_, segs)| segs.len() == want.len());
    if let Some((f, _)) = same_depth().find(|(_, segs)| *segs == want) {
        return Some(f);
    }
    unique(
        same_depth()
            .filter(|(_, segs)| segs.iter().zip(&want).all(|(have, w)| have.starts_with(w.as_str())))
            .map(|(f, _)| f),
    )
    .map(String::as_str)
}

fn unique<T>(mut it: impl Iterator<Item = T>) -> Option<T> {
    let first = it.next()?;
    it.next().is_none().then_some(first)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn clock_readings() {
        assert_eq!(clock("9am"), Some(Clock { hour: 9, minute: None, meridiem: Some(Meridiem::Am) }));
        assert_eq!(clock("15.30"), Some(Clock { hour: 15, minute: Some(30), meridiem: None }));
        assert_eq!(clock("9:05pm").unwrap().with_meridiem(Meridiem::Pm), Time::new(21, 5));
        assert_eq!(clock("15.000"), None);
        assert_eq!(clock("1.5"), None);
        assert_eq!(clock("25"), None);
        assert_eq!(clock("am"), None);
        assert_eq!(clock("13pm").unwrap().with_meridiem(Meridiem::Pm), None);
    }
}
