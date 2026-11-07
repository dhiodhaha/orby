//! RRULE: parsing and expansion into wall-clock start times.
//!
//! Supported: FREQ DAILY, WEEKLY, MONTHLY, YEARLY; INTERVAL; COUNT; UNTIL; BYDAY (with ordinals
//! such as 1MO or -1FR inside a month); BYMONTHDAY (negative counts from the month's end);
//! BYMONTH. Weeks start on Monday. BYSETPOS, BYWEEKNO, BYYEARDAY and sub-daily rules are not.

use crate::model::{parse_date_time, DateTimeValue};
use crate::time::{days_in_month, Date, LocalDateTime, Weekday};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Freq {
    Daily,
    Weekly,
    Monthly,
    Yearly,
}

/// A BYDAY entry: "MO" (every Monday) or "1MO" / "-1FR" (the first Monday, the last Friday).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct WeekdayNum {
    /// Never zero.
    pub ordinal: Option<i8>,
    pub weekday: Weekday,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum RecurEnd {
    Never,
    Count(u32),
    /// Inclusive.
    Until(DateTimeValue),
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct RRule {
    pub freq: Freq,
    /// At least 1.
    pub interval: u32,
    pub end: RecurEnd,
    pub by_day: Vec<WeekdayNum>,
    /// 1..=31 or -31..=-1.
    pub by_month_day: Vec<i8>,
    /// 1..=12.
    pub by_month: Vec<u8>,
}

impl RRule {
    /// None when FREQ is missing or unsupported, or a part is malformed. The event then shows once.
    pub fn parse(value: &str) -> Option<RRule> {
        let mut freq = None;
        let mut rule = RRule {
            freq: Freq::Daily,
            interval: 1,
            end: RecurEnd::Never,
            by_day: vec![],
            by_month_day: vec![],
            by_month: vec![],
        };
        for part in value.trim().split(';') {
            let Some((key, v)) = part.split_once('=') else { continue };
            let v = v.trim();
            match key.trim().to_ascii_uppercase().as_str() {
                "FREQ" => {
                    freq = Some(match v.to_ascii_uppercase().as_str() {
                        "DAILY" => Freq::Daily,
                        "WEEKLY" => Freq::Weekly,
                        "MONTHLY" => Freq::Monthly,
                        "YEARLY" => Freq::Yearly,
                        _ => return None,
                    })
                }
                "INTERVAL" => rule.interval = v.parse().ok().filter(|n| *n >= 1)?,
                "COUNT" => rule.end = RecurEnd::Count(v.parse().ok()?),
                "UNTIL" => rule.end = RecurEnd::Until(parse_date_time(v, None, false)?),
                "BYDAY" => rule.by_day = v.split(',').map(parse_weekday_num).collect::<Option<_>>()?,
                "BYMONTHDAY" => {
                    rule.by_month_day = v
                        .split(',')
                        .map(|d| d.trim().parse::<i8>().ok().filter(|d| *d != 0 && (-31..=31).contains(d)))
                        .collect::<Option<_>>()?
                }
                "BYMONTH" => {
                    rule.by_month = v
                        .split(',')
                        .map(|m| m.trim().parse::<u8>().ok().filter(|m| (1..=12).contains(m)))
                        .collect::<Option<_>>()?
                }
                _ => {}
            }
        }
        rule.freq = freq?;
        Some(rule)
    }
}

fn parse_weekday_num(s: &str) -> Option<WeekdayNum> {
    let s = s.trim();
    let split = s.len().checked_sub(2)?;
    let (num, code) = (s.get(..split)?, s.get(split..)?);
    let weekday = Weekday::from_ics(code)?;
    let ordinal = match num {
        "" => None,
        n => Some(n.parse::<i8>().ok().filter(|n| *n != 0 && (-53..=53).contains(n))?),
    };
    Some(WeekdayNum { ordinal, weekday })
}

/// Bounds the walk so a rule that can never match (BYMONTHDAY=30;BYMONTH=2) still ends.
const MAX_PERIODS: i64 = 200_000;

/// Start times of a recurring event, from `dtstart` (always the first, as RFC 5545 says) up to
/// `stop_after`, honouring COUNT. `within_until` answers the UNTIL check, which needs zone
/// resolution the caller owns.
pub(crate) fn expand(
    rule: &RRule,
    dtstart: LocalDateTime,
    stop_after: LocalDateTime,
    within_until: impl Fn(LocalDateTime) -> bool,
) -> Vec<LocalDateTime> {
    let limit = match rule.end {
        RecurEnd::Count(n) => n as usize,
        _ => usize::MAX,
    };
    let accept = |t: LocalDateTime| t <= stop_after && within_until(t);
    let mut out = Vec::new();
    if limit == 0 || !accept(dtstart) {
        return out;
    }
    out.push(dtstart);
    if out.len() >= limit {
        return out;
    }
    let anchor = dtstart.date();
    for period in 0..MAX_PERIODS {
        let Some((first_day, dates)) = period_dates(rule, anchor, period * i64::from(rule.interval)) else { break };
        if first_day > stop_after.date() {
            break;
        }
        for date in dates {
            let t = LocalDateTime::at(date, dtstart.seconds_of_day());
            if t <= dtstart {
                continue;
            }
            if !accept(t) {
                return out;
            }
            out.push(t);
            if out.len() >= limit {
                return out;
            }
        }
    }
    out
}

/// The first day of the period `step` periods after the anchor's, and the matching dates in it, sorted.
fn period_dates(rule: &RRule, anchor: Date, step: i64) -> Option<(Date, Vec<Date>)> {
    Some(match rule.freq {
        Freq::Daily => {
            let d = anchor.add_days(step);
            (d, if day_matches(rule, d) { vec![d] } else { vec![] })
        }
        Freq::Weekly => {
            let monday = anchor.add_days(7 * step - i64::from(anchor.weekday().monday_index()));
            let weekdays: Vec<Weekday> = if rule.by_day.is_empty() {
                vec![anchor.weekday()]
            } else {
                rule.by_day.iter().map(|w| w.weekday).collect()
            };
            let mut days: Vec<Date> = weekdays
                .into_iter()
                .map(|w| monday.add_days(i64::from(w.monday_index())))
                .filter(|d| month_allowed(rule, *d))
                .collect();
            days.sort();
            days.dedup();
            (monday, days)
        }
        Freq::Monthly => {
            let total = i64::from(anchor.month()) - 1 + step;
            let year = i32::try_from(i64::from(anchor.year()) + total.div_euclid(12)).ok()?;
            let month = (total.rem_euclid(12) + 1) as u8;
            let days =
                if month_allowed_number(rule, month) { month_days(rule, year, month, anchor.day()) } else { vec![] };
            (Date::new(year, month, 1)?, days)
        }
        Freq::Yearly => {
            let year = i32::try_from(i64::from(anchor.year()) + step).ok()?;
            (Date::new(year, 1, 1)?, dates_in_year(rule, anchor, year))
        }
    })
}

/// The dates a YEARLY rule gives in one year: BYMONTH (or the anchor's month), then the days in each.
pub(crate) fn dates_in_year(rule: &RRule, anchor: Date, year: i32) -> Vec<Date> {
    let mut months = if rule.by_month.is_empty() { vec![anchor.month()] } else { rule.by_month.clone() };
    months.sort_unstable();
    months.dedup();
    months.into_iter().flat_map(|m| month_days(rule, year, m, anchor.day())).collect()
}

fn month_allowed(rule: &RRule, d: Date) -> bool {
    month_allowed_number(rule, d.month())
}

fn month_allowed_number(rule: &RRule, month: u8) -> bool {
    rule.by_month.is_empty() || rule.by_month.contains(&month)
}

fn day_matches(rule: &RRule, d: Date) -> bool {
    month_allowed(rule, d)
        && (rule.by_month_day.is_empty()
            || rule.by_month_day.iter().any(|&md| resolve_month_day(d.year(), d.month(), md) == Some(d.day())))
        && (rule.by_day.is_empty() || rule.by_day.iter().any(|w| w.weekday == d.weekday()))
}

fn resolve_month_day(year: i32, month: u8, md: i8) -> Option<u8> {
    let len = i16::from(days_in_month(year, month));
    let day = if md > 0 { i16::from(md) } else { len + 1 + i16::from(md) };
    (1..=len).contains(&day).then_some(day as u8)
}

/// Days of one month a MONTHLY rule (or one month of a YEARLY rule) picks. With both BYMONTHDAY
/// and BYDAY, a day must match both (Friday the 13th). With neither, the anchor's day of month,
/// skipped in months too short for it.
fn month_days(rule: &RRule, year: i32, month: u8, default_day: u8) -> Vec<Date> {
    let by_month_day: Vec<u8> = rule.by_month_day.iter().filter_map(|&md| resolve_month_day(year, month, md)).collect();
    let by_day: Vec<u8> = rule.by_day.iter().flat_map(|w| weekday_days(year, month, *w)).collect();
    let mut days = match (rule.by_month_day.is_empty(), rule.by_day.is_empty()) {
        (true, true) => vec![default_day],
        (false, true) => by_month_day,
        (true, false) => by_day,
        (false, false) => by_month_day.into_iter().filter(|d| by_day.contains(d)).collect(),
    };
    days.sort_unstable();
    days.dedup();
    days.into_iter().filter_map(|d| Date::new(year, month, d)).collect()
}

/// Days of the month falling on a weekday: all of them, or the nth from the start or the end.
fn weekday_days(year: i32, month: u8, w: WeekdayNum) -> Vec<u8> {
    let Some(first) = Date::new(year, month, 1) else { return vec![] };
    let offset = (7 + w.weekday.monday_index() - first.weekday().monday_index()) % 7;
    let all: Vec<u8> = (1 + offset..=days_in_month(year, month)).step_by(7).collect();
    let pick = match w.ordinal {
        None => return all,
        Some(n) if n > 0 => all.get(n as usize - 1),
        Some(n) => all.len().checked_sub(n.unsigned_abs() as usize).and_then(|i| all.get(i)),
    };
    pick.copied().into_iter().collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn d(y: i32, m: u8, day: u8) -> Date {
        Date::new(y, m, day).unwrap()
    }

    fn dates(rule: &str, start: Date, stop: Date) -> Vec<Date> {
        let rule = RRule::parse(rule).unwrap();
        expand(&rule, LocalDateTime::midnight(start), LocalDateTime::midnight(stop), |_| true)
            .into_iter()
            .map(|t| t.date())
            .collect()
    }

    #[test]
    fn parses_parts() {
        let r = RRule::parse("FREQ=MONTHLY;INTERVAL=2;COUNT=4;BYDAY=1MO,-1FR,+2TU,WE;BYMONTHDAY=13,-1;BYMONTH=1,12")
            .unwrap();
        assert_eq!(r.freq, Freq::Monthly);
        assert_eq!(r.interval, 2);
        assert_eq!(r.end, RecurEnd::Count(4));
        assert_eq!(
            r.by_day,
            vec![
                WeekdayNum { ordinal: Some(1), weekday: Weekday::Mon },
                WeekdayNum { ordinal: Some(-1), weekday: Weekday::Fri },
                WeekdayNum { ordinal: Some(2), weekday: Weekday::Tue },
                WeekdayNum { ordinal: None, weekday: Weekday::Wed },
            ]
        );
        assert_eq!(r.by_month_day, vec![13, -1]);
        assert_eq!(r.by_month, vec![1, 12]);
    }

    #[test]
    fn rejects_unsupported_or_broken_rules() {
        assert!(RRule::parse("FREQ=HOURLY").is_none());
        assert!(RRule::parse("INTERVAL=2").is_none());
        assert!(RRule::parse("FREQ=DAILY;INTERVAL=0").is_none());
        assert!(RRule::parse("FREQ=WEEKLY;BYDAY=XX").is_none());
        assert!(RRule::parse("FREQ=MONTHLY;BYDAY=0MO").is_none());
        assert!(RRule::parse("FREQ=MONTHLY;BYMONTHDAY=32").is_none());
    }

    #[test]
    fn friday_the_13th_intersects_bymonthday_and_byday() {
        assert_eq!(
            dates("FREQ=MONTHLY;BYDAY=FR;BYMONTHDAY=13;COUNT=3", d(2026, 2, 13), d(2030, 1, 1)),
            vec![d(2026, 2, 13), d(2026, 3, 13), d(2026, 11, 13)]
        );
    }

    #[test]
    fn monthly_on_the_31st_skips_short_months() {
        assert_eq!(
            dates("FREQ=MONTHLY;COUNT=4", d(2026, 10, 31), d(2030, 1, 1)),
            vec![d(2026, 10, 31), d(2026, 12, 31), d(2027, 1, 31), d(2027, 3, 31)]
        );
    }

    #[test]
    fn yearly_on_feb_29_waits_for_leap_years() {
        assert_eq!(
            dates("FREQ=YEARLY;COUNT=3", d(2024, 2, 29), d(2040, 1, 1)),
            vec![d(2024, 2, 29), d(2028, 2, 29), d(2032, 2, 29)]
        );
    }

    #[test]
    fn dtstart_counts_even_when_it_does_not_match() {
        // 3 Nov 2026 is a Tuesday; the rule asks for Mondays.
        assert_eq!(
            dates("FREQ=WEEKLY;BYDAY=MO;COUNT=3", d(2026, 11, 3), d(2030, 1, 1)),
            vec![d(2026, 11, 3), d(2026, 11, 9), d(2026, 11, 16)]
        );
    }

    #[test]
    fn impossible_rule_ends() {
        assert_eq!(dates("FREQ=MONTHLY;BYMONTHDAY=30;BYMONTH=2", d(2026, 1, 30), d(2100, 1, 1)), vec![d(2026, 1, 30)]);
    }

    #[test]
    fn daily_with_filters() {
        assert_eq!(
            dates("FREQ=DAILY;BYDAY=SA,SU;BYMONTH=11", d(2026, 10, 31), d(2026, 11, 9)),
            vec![d(2026, 10, 31), d(2026, 11, 1), d(2026, 11, 7), d(2026, 11, 8)]
        );
    }
}
