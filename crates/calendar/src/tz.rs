//! Time zones: VTIMEZONE observances, plus a small fallback table for zones the app's users live in.

use crate::model::DateTimeValue;
use crate::rrule::{dates_in_year, RRule, RecurEnd};
use crate::time::LocalDateTime;

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct VTimezone {
    pub tzid: String,
    pub observances: Vec<Observance>,
}

/// A STANDARD or DAYLIGHT block. Which of the two it is does not matter for the offset.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Observance {
    /// DTSTART: the first onset, in local wall time.
    pub onset: LocalDateTime,
    pub offset_from_minutes: i32,
    pub offset_to_minutes: i32,
    /// The yearly rule for later onsets, such as BYMONTH=3;BYDAY=2SU.
    pub rule: Option<RRule>,
}

impl VTimezone {
    /// The UTC offset in force at a wall time: the TZOFFSETTO of the latest onset at or before it.
    /// Simplified: onsets are compared as wall times, so the hour at a transition may land on
    /// either side.
    pub fn offset_minutes_at(&self, wall: LocalDateTime) -> Option<i32> {
        let mut latest: Option<(LocalDateTime, i32)> = None;
        for obs in &self.observances {
            for onset in obs.onsets_near(wall.date().year()) {
                if onset <= wall && latest.is_none_or(|(t, _)| onset > t) {
                    latest = Some((onset, obs.offset_to_minutes));
                }
            }
        }
        latest
            .map(|(_, offset)| offset)
            .or_else(|| self.observances.iter().min_by_key(|o| o.onset).map(|o| o.offset_from_minutes))
    }
}

impl Observance {
    /// Onsets in the year before and the year of `year`, enough to find the one in force.
    fn onsets_near(&self, year: i32) -> Vec<LocalDateTime> {
        let Some(rule) = &self.rule else { return vec![self.onset] };
        [year - 1, year]
            .into_iter()
            .flat_map(|y| dates_in_year(rule, self.onset.date(), y))
            .map(|d| LocalDateTime::at(d, self.onset.seconds_of_day()))
            .filter(|t| *t >= self.onset && until_allows(rule, *t))
            .collect()
    }
}

fn until_allows(rule: &RRule, t: LocalDateTime) -> bool {
    match &rule.end {
        RecurEnd::Until(DateTimeValue::Date(d)) => t.date() <= *d,
        RecurEnd::Until(until) => t <= until.wall(),
        _ => true,
    }
}

/// Offsets for zones used without a VTIMEZONE. None of these observe daylight saving.
/// Outlook's Windows names for WIB and WITA are included because Outlook writes them as TZID.
pub fn fallback_offset_minutes(tzid: &str) -> Option<i32> {
    match tzid {
        "Asia/Jakarta" | "SE Asia Standard Time" => Some(7 * 60),
        "Asia/Makassar" | "Asia/Singapore" | "Singapore Standard Time" => Some(8 * 60),
        "Asia/Jayapura" => Some(9 * 60),
        "UTC" | "Etc/UTC" | "GMT" | "Etc/GMT" => Some(0),
        _ => None,
    }
}

/// "+0700", "-0500" or "+053000" as minutes east of UTC. Seconds are dropped.
pub(crate) fn parse_utc_offset(s: &str) -> Option<i32> {
    let s = s.trim();
    let (sign, digits) = match s.as_bytes().first()? {
        b'+' => (1, &s[1..]),
        b'-' => (-1, &s[1..]),
        _ => return None,
    };
    if !(digits.len() == 4 || digits.len() == 6) || !digits.bytes().all(|b| b.is_ascii_digit()) {
        return None;
    }
    let hours: i32 = digits[0..2].parse().ok()?;
    let minutes: i32 = digits[2..4].parse().ok()?;
    Some(sign * (hours * 60 + minutes))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn utc_offsets() {
        assert_eq!(parse_utc_offset("+0700"), Some(420));
        assert_eq!(parse_utc_offset("-0500"), Some(-300));
        assert_eq!(parse_utc_offset("+0530"), Some(330));
        assert_eq!(parse_utc_offset("+053000"), Some(330));
        assert_eq!(parse_utc_offset("0700"), None);
        assert_eq!(parse_utc_offset("+7"), None);
    }

    #[test]
    fn fallback_table() {
        assert_eq!(fallback_offset_minutes("Asia/Jakarta"), Some(420));
        assert_eq!(fallback_offset_minutes("Asia/Makassar"), Some(480));
        assert_eq!(fallback_offset_minutes("Asia/Jayapura"), Some(540));
        assert_eq!(fallback_offset_minutes("Asia/Singapore"), Some(480));
        assert_eq!(fallback_offset_minutes("UTC"), Some(0));
        assert_eq!(fallback_offset_minutes("Europe/Paris"), None);
    }
}
