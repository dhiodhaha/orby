//! Parses Muhammadiyah's official KHGT calendar API into Hijri month starts.
//!
//! Source: `GET https://api-khgt.muhammadiyah.or.id/api/calendar/{H}?type=hijriah&end_year={H2}`
//! (documented at https://api-khgt.muhammadiyah.or.id/docs, Majelis Tarjih dan Tajdid PP
//! Muhammadiyah). The answer, trimmed to the fields read here:
//!
//! ```json
//! {"success": true,
//!  "data": [{"hijri_month": "Ramadhan", "hijri_year": 1447, "tanggal_masehi": "2026-02-18",
//!            "status": "KHGT", "is_primary": true, "days": [...]}, ...],
//!  "meta": {"calendar_mode": "khgt_default", "source": "official_database"}}
//! ```
//!
//! Everything is checked; a bad answer is an `Err`, never a panic or a partial table.

use serde_json::Value;

use crate::hijri::{jd, Date, MonthStart};

/// The API's month names (and common spellings) to 1..=12.
fn month_number(name: &str) -> Option<u32> {
    let n: String = name
        .chars()
        .filter(|c| c.is_ascii_alphabetic())
        .map(|c| c.to_ascii_lowercase())
        .collect();
    let m = match n.as_str() {
        "muharam" | "muharram" => 1,
        "safar" | "shafar" => 2,
        "rabiulawal" | "rabiulawwal" => 3,
        "rabiulakhir" | "rabiulakhar" | "rabiultsani" => 4,
        "jumadilawal" | "jumadalula" | "jumadilula" => 5,
        "jumadilakhir" | "jumadalakhirah" | "jumadilakhirah" => 6,
        "rajab" => 7,
        "syaban" | "syakban" | "shaban" | "syaaban" => 8,
        "ramadhan" | "ramadan" => 9,
        "syawal" | "syawwal" | "shawwal" => 10,
        "zulkaidah" | "zulqaidah" | "dzulqaidah" | "dzulqadah" => 11,
        "zulhijah" | "zulhijjah" | "dzulhijjah" | "dzulhijah" => 12,
        _ => return None,
    };
    Some(m)
}

fn parse_date(s: &str) -> Option<Date> {
    let mut it = s.trim().splitn(3, '-');
    let y: i32 = it.next()?.parse().ok()?;
    let m: u32 = it.next()?.parse().ok()?;
    let d: u32 = it.next()?.parse().ok()?;
    if !(1900..=2200).contains(&y) || !(1..=12).contains(&m) || !(1..=31).contains(&d) {
        return None;
    }
    Some((y, m, d))
}

/// The month starts in one API answer, sorted. Rejects the whole answer when any month is
/// unreadable or two consecutive months are not 29 or 30 days apart.
pub fn parse(json: &str) -> Result<Vec<MonthStart>, String> {
    let v: Value = serde_json::from_str(json).map_err(|e| format!("not JSON ({e})"))?;
    if v.get("success").and_then(Value::as_bool) != Some(true) {
        let msg = v
            .get("message")
            .and_then(Value::as_str)
            .unwrap_or("no success flag");
        return Err(format!("the API said: {msg}"));
    }
    let data = v
        .get("data")
        .and_then(Value::as_array)
        .ok_or("no data list")?;
    let mut out = Vec::with_capacity(data.len());
    for (i, m) in data.iter().enumerate() {
        if m.get("is_primary").and_then(Value::as_bool) == Some(false) {
            continue;
        }
        let year = m
            .get("hijri_year")
            .and_then(Value::as_u64)
            .filter(|y| (1300..=1700).contains(y))
            .ok_or_else(|| format!("month {i}: no hijri_year"))? as u32;
        let name = m
            .get("hijri_month")
            .or_else(|| m.get("hijri_month_name"))
            .and_then(Value::as_str)
            .unwrap_or("");
        let month =
            month_number(name).ok_or_else(|| format!("month {i}: unknown name {name:?}"))?;
        let start = m
            .get("tanggal_masehi")
            .and_then(Value::as_str)
            .and_then(parse_date)
            .ok_or_else(|| format!("month {i}: no tanggal_masehi"))?;
        out.push(MonthStart { year, month, start });
    }
    if out.is_empty() {
        return Err("no months".into());
    }
    out.sort_by_key(|m| (m.year, m.month));
    out.dedup_by_key(|m| (m.year, m.month));
    for w in out.windows(2) {
        let (a, b) = (w[0], w[1]);
        let consecutive = (b.year, b.month)
            == if a.month == 12 {
                (a.year + 1, 1)
            } else {
                (a.year, a.month + 1)
            };
        let gap = jd(b.start) - jd(a.start);
        if consecutive && !(29..=30).contains(&gap) {
            return Err(format!("{}-{} lasts {gap} days", a.year, a.month));
        }
        if gap <= 0 {
            return Err(format!("{}-{} starts out of order", b.year, b.month));
        }
    }
    Ok(out)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::hijri::{HijriCalendar, Months, Ramadan};

    const SAMPLE: &str = include_str!("../tests/fixtures/khgt-calendar-1447-1449.json");

    #[test]
    fn parses_the_saved_answer() {
        let months = parse(SAMPLE).unwrap();
        assert_eq!(months.len(), 36);
        assert_eq!(
            months[0],
            MonthStart {
                year: 1447,
                month: 1,
                start: (2025, 6, 26)
            }
        );
        let find = |y, m| {
            months
                .iter()
                .find(|s| s.year == y && s.month == m)
                .unwrap()
                .start
        };
        // The announced dates, also in the built-in table.
        assert_eq!(find(1447, 9), (2026, 2, 18));
        assert_eq!(find(1447, 10), (2026, 3, 20));
        assert_eq!(find(1448, 9), (2027, 2, 8));
        assert_eq!(find(1448, 10), (2027, 3, 9));
        assert_eq!(find(1449, 9), (2028, 1, 28));
    }

    #[test]
    fn the_fetched_table_agrees_with_the_builtin_one_and_dates_every_day() {
        let fetched = parse(SAMPLE).unwrap();
        let m = Months::builtin(HijriCalendar::Muhammadiyah).merged(&fetched);
        assert_eq!(m.len(), 36);
        // The API's own answer for this day: "28 Jumadil Awal 1448 H KHGT".
        assert_eq!(
            m.hijri_date((2026, 11, 8)).map(|h| h.label()),
            Some("28 Jumadilawal 1448 H".into())
        );
        assert_eq!(m.ramadan((2026, 2, 18)), Ramadan::Day(1));
        assert_eq!(m.ramadan((2027, 3, 8)), Ramadan::Day(29));
        assert_eq!(m.ramadan((2027, 3, 9)), Ramadan::NotRamadan, "now known");
        assert_eq!(m.ramadan((2028, 1, 28)), Ramadan::Day(1));
        // Zulhijah 1449 has no next month in the sample.
        assert_eq!(m.hijri_date((2028, 5, 1)), None);
    }

    #[test]
    fn bad_answers_are_errors() {
        assert!(parse("").is_err());
        assert!(parse("<html>").is_err());
        assert!(parse(r#"{"success":false,"message":"Parameter tahun tidak valid."}"#).is_err());
        assert!(parse(r#"{"success":true,"data":[]}"#).is_err());
        assert!(parse(r#"{"success":true,"data":[{"hijri_year":1448}]}"#).is_err());
        let bad_name = r#"{"success":true,"data":[
            {"hijri_month":"Bulan","hijri_year":1448,"tanggal_masehi":"2026-06-16"}]}"#;
        assert!(parse(bad_name).is_err());
        let bad_date = r#"{"success":true,"data":[
            {"hijri_month":"Muharam","hijri_year":1448,"tanggal_masehi":"2026-13-16"}]}"#;
        assert!(parse(bad_date).is_err());
        let too_long = r#"{"success":true,"data":[
            {"hijri_month":"Muharam","hijri_year":1448,"tanggal_masehi":"2026-06-16"},
            {"hijri_month":"Safar","hijri_year":1448,"tanggal_masehi":"2026-07-20"}]}"#;
        assert!(parse(too_long).is_err());
    }

    #[test]
    fn skips_months_that_are_not_primary() {
        let j = r#"{"success":true,"data":[
            {"hijri_month":"Muharam","hijri_year":1448,"tanggal_masehi":"2026-06-16","is_primary":true},
            {"hijri_month":"Muharam","hijri_year":1448,"tanggal_masehi":"2026-06-17","is_primary":false}]}"#;
        assert_eq!(parse(j).unwrap().len(), 1);
        assert_eq!(parse(j).unwrap()[0].start, (2026, 6, 16));
    }
}
