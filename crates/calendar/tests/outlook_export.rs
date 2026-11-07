mod common;

use app_calendar::{occurrences, parse, Date, LinkKind, LocalDateTime};
use common::*;

#[test]
fn reads_names_long_folded_uids_and_both_zones() {
    let cal = parse(OUTLOOK);
    assert_eq!(cal.name.as_deref(), Some("Calendar"));
    assert_eq!(cal.events.len(), 7);
    assert_eq!(
        cal.events[0].uid,
        "040000008200E00074C5B7101A82E00800000000A1B2C3D4E5F6A701000000000000000010000000F1E2D3C4B5A69788"
    );
    let ids: Vec<&str> = cal.timezones.iter().map(|t| t.tzid.as_str()).collect();
    assert_eq!(ids, ["SE Asia Standard Time", "Eastern Standard Time"]);
}

#[test]
fn eastern_time_follows_its_dst_rules() {
    let cal = parse(OUTLOOK);
    let est = |y, m, d, h| {
        cal.zone_offset_minutes(
            "Eastern Standard Time",
            LocalDateTime::new(Date::new(y, m, d).unwrap(), h, 0, 0).unwrap(),
        )
    };
    // 2026: DST ends Sun 1 Nov at 02:00. 2027: it starts Sun 14 Mar at 02:00.
    assert_eq!(est(2026, 7, 1, 12), Some(-240));
    assert_eq!(est(2026, 11, 1, 1), Some(-240));
    assert_eq!(est(2026, 11, 1, 3), Some(-300));
    assert_eq!(est(2027, 1, 15, 12), Some(-300));
    assert_eq!(est(2027, 3, 14, 1), Some(-300));
    assert_eq!(est(2027, 3, 14, 3), Some(-240));
    let wall = LocalDateTime::new(Date::new(2026, 11, 6).unwrap(), 12, 0, 0).unwrap();
    assert_eq!(cal.zone_offset_minutes("SE Asia Standard Time", wall), Some(420));
}

#[test]
fn monthly_last_friday_in_new_york_converts_across_dst() {
    let cal = parse(OUTLOOK);
    // 09:00 in New York each time: 13:00Z while on EDT, 14:00Z after DST ends.
    // UNTIL=20270301 drops the March one.
    let got = starts(&cal, "Client sync (NY)", wib(2026, 10, 1, 0, 0), wib(2027, 6, 1, 0, 0));
    assert_eq!(
        got,
        [
            utc(2026, 10, 30, 13, 0),
            utc(2026, 11, 27, 14, 0),
            utc(2026, 12, 25, 14, 0),
            utc(2027, 1, 29, 14, 0),
            utc(2027, 2, 26, 14, 0),
        ]
    );
    assert_eq!(got[0], wib(2026, 10, 30, 20, 0));
    assert_eq!(got[1], wib(2026, 11, 27, 21, 0));
}

#[test]
fn teams_link_from_an_outlook_description() {
    let cal = parse(OUTLOOK);
    let occ = occurrences(&cal, wib(2026, 11, 27, 0, 0), wib(2026, 11, 28, 0, 0), WIB);
    let sync = occ.iter().find(|o| o.title == "Client sync (NY)").unwrap();
    assert_eq!(sync.end.unix_seconds() - sync.start.unix_seconds(), 30 * 60);
    assert_eq!(sync.location.as_deref(), Some("Microsoft Teams Meeting"));
    let link = sync.link.clone().unwrap();
    assert_eq!(link.kind, LinkKind::Teams);
    assert_eq!(
        link.url,
        "https://teams.microsoft.com/l/meetup-join/19%3ameeting_ZmFrZS1tZWV0aW5n%40thread.v2/0?context=%7b%22Tid%22%3a%2211111111%22%7d"
    );
    let event = cal.events.iter().find(|e| e.summary == "Client sync (NY)").unwrap();
    assert!(event.description.as_deref().unwrap().contains("Join on your computer, mobile app or room device"));
    assert!(!event.description.as_deref().unwrap().contains("REMINDER"), "VALARM text stays out");
}

#[test]
fn weekly_count_with_an_override() {
    let cal = parse(OUTLOOK);
    let (from, to) = (wib(2026, 11, 1, 0, 0), wib(2026, 12, 31, 0, 0));
    assert_eq!(starts(&cal, "Team lunch", from, to), [wib(2026, 11, 6, 12, 0), wib(2026, 11, 20, 12, 0)]);
    let late: Vec<_> =
        occurrences(&cal, from, to, WIB).into_iter().filter(|o| o.title == "Team lunch (late)").collect();
    assert_eq!(late.len(), 1);
    assert_eq!((late[0].start, late[0].end), (wib(2026, 11, 13, 12, 30), wib(2026, 11, 13, 13, 30)));
    assert_eq!(late[0].location.as_deref(), Some("Warung Bu Rini"));
}

#[test]
fn biweekly_two_days_with_until_and_exdate() {
    let cal = parse(OUTLOOK);
    // Weeks of 2 Nov and 16 Nov; Thu 5 Nov is excluded; Tue 1 Dec 15:00 WIB is past UNTIL.
    let got = starts(&cal, "1:1 with Ibu", wib(2026, 11, 1, 0, 0), wib(2027, 1, 1, 0, 0));
    assert_eq!(got, [wib(2026, 11, 3, 15, 0), wib(2026, 11, 17, 15, 0), wib(2026, 11, 19, 15, 0)]);
    let event = cal.events.iter().find(|e| e.summary == "1:1 with Ibu").unwrap();
    assert_eq!(event.link.as_ref().map(|l| l.kind), Some(LinkKind::Zoom));
    assert_eq!(event.location, None);
    assert_eq!(
        event.description.as_deref(),
        Some("Zoom: https://us06web.zoom.us/j/9876543210?pwd=QwErTy\nAgenda, goals; blockers\n")
    );
}

#[test]
fn all_day_outlook_event() {
    let cal = parse(OUTLOOK);
    let occ = occurrences(&cal, wib(2026, 11, 10, 0, 0), wib(2026, 11, 11, 0, 0), WIB);
    let pahlawan = occ.iter().find(|o| o.title == "Hari Pahlawan").unwrap();
    assert!(pahlawan.all_day);
    assert_eq!((pahlawan.start, pahlawan.end), (wib(2026, 11, 10, 0, 0), wib(2026, 11, 11, 0, 0)));
    assert_eq!(titles(&occ)[0], "Hari Pahlawan", "all-day sorts first");
}

#[test]
fn yearly_by_month_day_and_yearly_nth_weekday() {
    let cal = parse(OUTLOOK);
    let (from, to) = (wib(2026, 1, 1, 0, 0), wib(2028, 1, 1, 0, 0));
    assert_eq!(starts(&cal, "Anniversary dinner", from, to), [wib(2026, 11, 12, 19, 0), wib(2027, 11, 12, 19, 0)]);
    // Second Tuesday of November.
    assert_eq!(starts(&cal, "Board review", from, to), [wib(2026, 11, 10, 10, 0), wib(2027, 11, 9, 10, 0)]);
}

#[test]
fn links_from_url_and_location() {
    let cal = parse(OUTLOOK);
    let link = |title: &str| cal.events.iter().find(|e| e.summary == title).unwrap().link.clone().unwrap();
    assert_eq!(link("Board review").kind, LinkKind::Meet, "URL property");
    assert_eq!(link("Anniversary dinner").url, "https://teams.live.com/meet/9312345678901", "LOCATION");
}
