mod common;

use app_calendar::{occurrences, parse, LinkKind};
use common::*;

#[test]
fn reads_the_calendar_name_and_events_with_lf_or_crlf() {
    let lf = parse(GOOGLE);
    assert_eq!(lf.name.as_deref(), Some("Work"));
    assert_eq!(lf.events.len(), 14);
    assert_eq!(lf.timezones.len(), 1);
    assert_eq!(parse(&GOOGLE.replace('\n', "\r\n")), lf);
}

#[test]
fn friday_6_nov_in_order() {
    let cal = parse(GOOGLE);
    let day = occurrences(&cal, wib(2026, 11, 6, 0, 0), wib(2026, 11, 7, 0, 0), WIB);
    assert_eq!(titles(&day), ["Cuti bersama", "Standup", "Design review, orb v6", "Jemput Raka", "Gym"]);

    let cuti = &day[0];
    assert!(cuti.all_day);
    assert_eq!((cuti.start, cuti.end), (wib(2026, 11, 6, 0, 0), wib(2026, 11, 7, 0, 0)));

    let standup = &day[1];
    assert!(!standup.all_day);
    assert_eq!((standup.start, standup.end), (wib(2026, 11, 6, 10, 30), wib(2026, 11, 6, 11, 0)));
    assert_eq!(standup.uid, "standup-7f3a@google.com");

    // Written in UTC (07:00Z), shown at 14:00 WIB.
    assert_eq!(day[2].start, utc(2026, 11, 6, 7, 0));
    assert_eq!(day[2].start, wib(2026, 11, 6, 14, 0));
}

#[test]
fn weekly_byday_with_exdate_cancelled_instance_and_moved_override() {
    let cal = parse(GOOGLE);
    let (from, to) = (wib(2026, 11, 2, 0, 0), wib(2026, 11, 14, 0, 0));
    // Tue 3 is cancelled by an override, Wed 4 is an EXDATE, Thu 5 moved to 13:30.
    assert_eq!(
        starts(&cal, "Standup", from, to),
        [
            wib(2026, 11, 2, 10, 30),
            wib(2026, 11, 6, 10, 30),
            wib(2026, 11, 9, 10, 30),
            wib(2026, 11, 10, 10, 30),
            wib(2026, 11, 11, 10, 30),
            wib(2026, 11, 12, 10, 30),
            wib(2026, 11, 13, 10, 30),
        ]
    );
    let moved: Vec<_> = occurrences(&cal, from, to, WIB).into_iter().filter(|o| o.title == "Standup (moved)").collect();
    assert_eq!(moved.len(), 1);
    assert_eq!((moved[0].start, moved[0].end), (wib(2026, 11, 5, 13, 30), wib(2026, 11, 5, 14, 0)));
    assert_eq!(moved[0].uid, "standup-7f3a@google.com");
}

#[test]
fn override_is_found_even_when_the_original_slot_is_outside_the_window() {
    let cal = parse(GOOGLE);
    // The original 10:30 slot is before the window; the moved 13:30 one is inside.
    let occ = occurrences(&cal, wib(2026, 11, 5, 12, 0), wib(2026, 11, 5, 18, 0), WIB);
    assert_eq!(titles(&occ), ["Standup (moved)"]);
}

#[test]
fn daily_count() {
    let cal = parse(GOOGLE);
    let all = starts(&cal, "Ngaji pagi", wib(2026, 10, 1, 0, 0), wib(2027, 1, 1, 0, 0));
    assert_eq!(all, (1..=5).map(|d| wib(2026, 11, d, 5, 0)).collect::<Vec<_>>());
    let occ = occurrences(&cal, wib(2026, 11, 1, 0, 0), wib(2026, 11, 2, 0, 0), WIB);
    let ngaji = occ.iter().find(|o| o.title == "Ngaji pagi").unwrap();
    assert_eq!(ngaji.end, wib(2026, 11, 1, 5, 30), "DURATION:PT30M");
}

#[test]
fn daily_interval_until_is_inclusive() {
    let cal = parse(GOOGLE);
    // UNTIL=20261108T100000Z is exactly 8 Nov 17:00 WIB, so that one is kept.
    assert_eq!(
        starts(&cal, "Gym", wib(2026, 10, 1, 0, 0), wib(2027, 1, 1, 0, 0)),
        [2, 4, 6, 8].map(|d| wib(2026, 11, d, 17, 0))
    );
}

#[test]
fn monthly_first_monday_with_count() {
    let cal = parse(GOOGLE);
    assert_eq!(
        starts(&cal, "Sprint planning", wib(2026, 10, 1, 0, 0), wib(2028, 1, 1, 0, 0)),
        [wib(2026, 11, 2, 14, 0), wib(2026, 12, 7, 14, 0), wib(2027, 1, 4, 14, 0), wib(2027, 2, 1, 14, 0)]
    );
}

#[test]
fn monthly_bymonthday_with_until() {
    let cal = parse(GOOGLE);
    assert_eq!(
        starts(&cal, "Bayar listrik", wib(2026, 1, 1, 0, 0), wib(2028, 1, 1, 0, 0)),
        [wib(2026, 10, 20, 9, 0), wib(2026, 11, 20, 9, 0), wib(2026, 12, 20, 9, 0)]
    );
}

#[test]
fn monthly_last_day_of_month_all_day() {
    let cal = parse(GOOGLE);
    assert_eq!(
        starts(&cal, "Tutup buku", wib(2026, 11, 1, 0, 0), wib(2027, 4, 1, 0, 0)),
        [
            wib(2026, 11, 30, 0, 0),
            wib(2026, 12, 31, 0, 0),
            wib(2027, 1, 31, 0, 0),
            wib(2027, 2, 28, 0, 0),
            wib(2027, 3, 31, 0, 0),
        ]
    );
}

#[test]
fn yearly_all_day_from_an_old_start() {
    let cal = parse(GOOGLE);
    assert_eq!(
        starts(&cal, "Ulang tahun Ibu", wib(2026, 1, 1, 0, 0), wib(2028, 1, 1, 0, 0)),
        [wib(2026, 11, 8, 0, 0), wib(2027, 11, 8, 0, 0)]
    );
}

#[test]
fn multi_day_all_day_event_covers_each_day() {
    let cal = parse(GOOGLE);
    let on = |d: u8| {
        titles(&occurrences(&cal, wib(2026, 11, d, 0, 0), wib(2026, 11, d + 1, 0, 0), WIB)).contains(&"Offsite Bandung")
    };
    assert!(!on(8));
    assert!(on(9));
    assert!(on(10));
    assert!(!on(11), "DTEND of an all-day event is exclusive");
}

#[test]
fn cancelled_event_never_shows() {
    let cal = parse(GOOGLE);
    let occ = occurrences(&cal, wib(2026, 1, 1, 0, 0), wib(2028, 1, 1, 0, 0), WIB);
    assert!(!titles(&occ).contains(&"Old sync"));
}

#[test]
fn escaped_text_and_folded_lines() {
    let cal = parse(GOOGLE);
    let review = cal.events.iter().find(|e| e.uid == "design-review-91c2@google.com").unwrap();
    assert_eq!(review.summary, "Design review, orb v6");
    assert_eq!(review.description.as_deref(), Some("Agenda: pills; dock; today panel\nNotes, please."));
    let standup = cal.events.iter().find(|e| e.summary == "Standup").unwrap();
    // The link was folded mid-word across two lines.
    assert!(standup
        .description
        .as_deref()
        .unwrap()
        .contains("Join with Google Meet: https://meet.google.com/abc-defg-hij\n"));
    assert_eq!(standup.location, None, "an empty LOCATION is no location");
}

#[test]
fn meeting_links() {
    let cal = parse(GOOGLE);
    let day = occurrences(&cal, wib(2026, 11, 6, 0, 0), wib(2026, 11, 7, 0, 0), WIB);
    let link = |title: &str| day.iter().find(|o| o.title == title).unwrap().link.clone();
    let meet = link("Standup").unwrap();
    assert_eq!((meet.kind, meet.url.as_str()), (LinkKind::Meet, "https://meet.google.com/abc-defg-hij"));
    let zoom = link("Design review, orb v6").unwrap();
    assert_eq!((zoom.kind, zoom.url.as_str()), (LinkKind::Zoom, "https://us02web.zoom.us/j/81234567890?pwd=AbCdEf123"));
    assert_eq!(link("Gym"), None);
    assert_eq!(link("Cuti bersama"), None);
}

#[test]
fn display_offset_moves_all_day_and_floating_but_not_zoned_or_utc() {
    let cal = parse(GOOGLE);
    let wita = 8 * 60;
    let day = occurrences(&cal, at(2026, 11, 6, 0, 0, wita), at(2026, 11, 7, 0, 0, wita), wita);
    let start = |title: &str| day.iter().find(|o| o.title == title).unwrap().start;
    assert_eq!(start("Cuti bersama"), at(2026, 11, 6, 0, 0, wita));
    assert_eq!(start("Jemput Raka"), at(2026, 11, 6, 15, 30, wita));
    assert_eq!(start("Standup"), wib(2026, 11, 6, 10, 30));
    assert_eq!(start("Design review, orb v6"), utc(2026, 11, 6, 7, 0));
}

#[test]
fn results_are_sorted_and_inside_the_window() {
    let cal = parse(GOOGLE);
    let (from, to) = (wib(2026, 10, 1, 0, 0), wib(2027, 3, 1, 0, 0));
    let occ = occurrences(&cal, from, to, WIB);
    assert!(occ.windows(2).all(|w| w[0].start <= w[1].start));
    assert!(occ.iter().all(|o| o.start < to && (o.end > from || o.start >= from)));
}
