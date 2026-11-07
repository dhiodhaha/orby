mod common;

use app_calendar::{occurrences, parse};
use common::*;

fn one_event(lines: &str) -> String {
    wrap(&format!("BEGIN:VEVENT\nUID:e\nSUMMARY:E\n{lines}END:VEVENT\n"))
}

#[test]
fn fallback_zones_without_a_vtimezone() {
    let mut body = String::new();
    for (i, tz) in ["Asia/Jakarta", "Asia/Makassar", "Asia/Jayapura", "Asia/Singapore", "UTC"].iter().enumerate() {
        body.push_str(&format!("BEGIN:VEVENT\nUID:{i}\nSUMMARY:{tz}\nDTSTART;TZID={tz}:20261106T090000\nEND:VEVENT\n"));
    }
    let cal = parse(&wrap(&body));
    let occ = occurrences(&cal, utc(2026, 11, 5, 0, 0), utc(2026, 11, 7, 0, 0), WIB);
    let start = |tz: &str| occ.iter().find(|o| o.title == tz).unwrap().start;
    assert_eq!(start("Asia/Jakarta"), utc(2026, 11, 6, 2, 0));
    assert_eq!(start("Asia/Makassar"), utc(2026, 11, 6, 1, 0));
    assert_eq!(start("Asia/Jayapura"), utc(2026, 11, 6, 0, 0));
    assert_eq!(start("Asia/Singapore"), utc(2026, 11, 6, 1, 0));
    assert_eq!(start("UTC"), utc(2026, 11, 6, 9, 0));
}

#[test]
fn unknown_zone_reads_as_the_display_offset() {
    let cal = parse(&one_event("DTSTART;TZID=Mars/Olympus:20261106T090000\n"));
    let occ = occurrences(&cal, utc(2026, 11, 5, 0, 0), utc(2026, 11, 7, 0, 0), WIB);
    assert_eq!(occ[0].start, wib(2026, 11, 6, 9, 0));
}

#[test]
fn quoted_tzid() {
    let cal = parse(&one_event("DTSTART;TZID=\"Asia/Jayapura\":20261106T090000\n"));
    let occ = occurrences(&cal, utc(2026, 11, 5, 0, 0), utc(2026, 11, 7, 0, 0), WIB);
    assert_eq!(occ[0].start, utc(2026, 11, 6, 0, 0));
}

#[test]
fn google_style_new_york_keeps_wall_time_across_dst() {
    let cal = parse(&wrap(
        "BEGIN:VTIMEZONE\nTZID:America/New_York\nX-LIC-LOCATION:America/New_York\n\
         BEGIN:DAYLIGHT\nTZOFFSETFROM:-0500\nTZOFFSETTO:-0400\nTZNAME:EDT\nDTSTART:19700308T020000\n\
         RRULE:FREQ=YEARLY;BYMONTH=3;BYDAY=2SU\nEND:DAYLIGHT\n\
         BEGIN:STANDARD\nTZOFFSETFROM:-0400\nTZOFFSETTO:-0500\nTZNAME:EST\nDTSTART:19701101T020000\n\
         RRULE:FREQ=YEARLY;BYMONTH=11;BYDAY=1SU\nEND:STANDARD\nEND:VTIMEZONE\n\
         BEGIN:VEVENT\nUID:ny\nSUMMARY:NY standup\nDTSTART;TZID=America/New_York:20261026T090000\n\
         DTEND;TZID=America/New_York:20261026T091500\nRRULE:FREQ=WEEKLY;COUNT=2\nEND:VEVENT\n",
    ));
    let occ = occurrences(&cal, utc(2026, 10, 1, 0, 0), utc(2026, 12, 1, 0, 0), WIB);
    let got: Vec<_> = occ.iter().map(|o| (o.start, o.end)).collect();
    assert_eq!(
        got,
        [(utc(2026, 10, 26, 13, 0), utc(2026, 10, 26, 13, 15)), (utc(2026, 11, 2, 14, 0), utc(2026, 11, 2, 14, 15))]
    );
}

#[test]
fn excluded_instances_still_use_up_the_count() {
    let cal = parse(&one_event(
        "DTSTART:20261101T010000Z\nDTEND:20261101T020000Z\nRRULE:FREQ=DAILY;COUNT=3\nEXDATE:20261102T010000Z\n",
    ));
    let got = starts(&cal, "E", utc(2026, 10, 1, 0, 0), utc(2027, 1, 1, 0, 0));
    assert_eq!(got, [utc(2026, 11, 1, 1, 0), utc(2026, 11, 3, 1, 0)]);
}

#[test]
fn exdate_lists_and_date_valued_exdates() {
    let cal = parse(&one_event(
        "DTSTART;VALUE=DATE:20261101\nRRULE:FREQ=DAILY;COUNT=5\nEXDATE;VALUE=DATE:20261102,20261104\n",
    ));
    let got = starts(&cal, "E", wib(2026, 10, 1, 0, 0), wib(2027, 1, 1, 0, 0));
    assert_eq!(got, [wib(2026, 11, 1, 0, 0), wib(2026, 11, 3, 0, 0), wib(2026, 11, 5, 0, 0)]);
}

#[test]
fn missing_end_means_one_day_or_one_moment() {
    let all_day = parse(&one_event("DTSTART;VALUE=DATE:20261106\n"));
    let occ = occurrences(&all_day, wib(2026, 11, 6, 0, 0), wib(2026, 11, 7, 0, 0), WIB);
    assert_eq!((occ[0].start, occ[0].end), (wib(2026, 11, 6, 0, 0), wib(2026, 11, 7, 0, 0)));

    let moment = parse(&one_event("DTSTART:20261106T030000Z\n"));
    let occ = occurrences(&moment, utc(2026, 11, 6, 3, 0), utc(2026, 11, 6, 4, 0), WIB);
    assert_eq!((occ[0].start, occ[0].end), (utc(2026, 11, 6, 3, 0), utc(2026, 11, 6, 3, 0)));
    assert!(
        occurrences(&moment, utc(2026, 11, 6, 2, 0), utc(2026, 11, 6, 3, 0), WIB).is_empty(),
        "window end is exclusive"
    );
}

#[test]
fn all_day_duration_form() {
    let cal = parse(&one_event("DTSTART;VALUE=DATE:20261106\nDURATION:P2D\n"));
    let occ = occurrences(&cal, wib(2026, 11, 7, 0, 0), wib(2026, 11, 8, 0, 0), WIB);
    assert_eq!(occ[0].end, wib(2026, 11, 8, 0, 0));
}

#[test]
fn an_event_in_progress_at_the_window_start_is_included() {
    let cal = parse(&one_event("DTSTART:20261106T030000Z\nDTEND:20261106T050000Z\n"));
    assert_eq!(occurrences(&cal, utc(2026, 11, 6, 4, 0), utc(2026, 11, 6, 6, 0), WIB).len(), 1);
    assert!(occurrences(&cal, utc(2026, 11, 6, 5, 0), utc(2026, 11, 6, 6, 0), WIB).is_empty());
}

#[test]
fn unsupported_rule_shows_the_event_once() {
    let cal = parse(&one_event("DTSTART:20261106T030000Z\nRRULE:FREQ=HOURLY;COUNT=5\n"));
    assert_eq!(starts(&cal, "E", utc(2026, 11, 1, 0, 0), utc(2026, 12, 1, 0, 0)), [utc(2026, 11, 6, 3, 0)]);
}

#[test]
fn cancelled_master_hides_every_instance() {
    let cal = parse(&one_event("DTSTART:20261101T030000Z\nRRULE:FREQ=DAILY\nSTATUS:CANCELLED\n"));
    assert!(occurrences(&cal, utc(2026, 11, 1, 0, 0), utc(2026, 12, 1, 0, 0), WIB).is_empty());
}

#[test]
fn endless_rule_far_from_its_start_is_bounded_by_the_window() {
    let cal = parse(&one_event("DTSTART;TZID=Asia/Jakarta:20100104T080000\nRRULE:FREQ=DAILY\n"));
    let got = starts(&cal, "E", wib(2026, 11, 6, 0, 0), wib(2026, 11, 8, 0, 0));
    assert_eq!(got, [wib(2026, 11, 6, 8, 0), wib(2026, 11, 7, 8, 0)]);
}

#[test]
fn garbage_input_gives_an_empty_calendar() {
    assert!(parse("").events.is_empty());
    assert!(parse("not an ics file\n:::\n;;;").events.is_empty());
    assert!(parse(&one_event("DTSTART:yesterday\n")).events.is_empty());
}
