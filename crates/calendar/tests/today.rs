mod common;

use app_calendar::{occurrences, parse, today_timeline, Item, PrayerTime, TaskDue};
use common::*;

fn prayer(name: &str, h: u8, m: u8) -> PrayerTime {
    PrayerTime { name: name.into(), at: wib(2026, 11, 6, h, m) }
}

fn task(id: &str, title: &str, h: u8, m: u8, done: bool) -> TaskDue {
    TaskDue { id: id.into(), title: title.into(), due: wib(2026, 11, 6, h, m), done }
}

fn label(item: &Item) -> String {
    match item {
        Item::Event(o) => format!("event {}", o.title),
        Item::Task(t) => format!("task {}", t.title),
        Item::PrayerMarker(p) => format!("prayer {}", p.name),
    }
}

/// Friday 6 Nov 2026, the design's sample day: two calendars, two tasks, five prayer times
/// (Dzuhur shown as Jumat).
fn sample_day() -> Vec<Item> {
    let (from, to) = (wib(2026, 11, 6, 0, 0), wib(2026, 11, 7, 0, 0));
    let mut events = occurrences(&parse(GOOGLE), from, to, WIB);
    events.extend(occurrences(&parse(OUTLOOK), from, to, WIB));
    let tasks = vec![task("t1", "Kirim invoice", 9, 0, true), task("t2", "Beli galon", 16, 0, false)];
    let prayers = vec![
        prayer("Subuh", 3, 59),
        prayer("Jumat", 11, 26),
        prayer("Ashar", 14, 41),
        prayer("Maghrib", 17, 43),
        prayer("Isya", 18, 55),
    ];
    today_timeline(events, tasks, prayers)
}

#[test]
fn one_timeline_in_time_order() {
    let labels: Vec<String> = sample_day().iter().map(label).collect();
    assert_eq!(
        labels,
        [
            "event Cuti bersama",
            "prayer Subuh",
            "task Kirim invoice",
            "event Standup",
            "prayer Jumat",
            "event Team lunch",
            "event Design review, orb v6",
            "prayer Ashar",
            "event Jemput Raka",
            "task Beli galon",
            "event Gym",
            "prayer Maghrib",
            "prayer Isya",
        ]
    );
    let items = sample_day();
    assert!(items.windows(2).all(|w| w[0].at() <= w[1].at()));
}

#[test]
fn only_tasks_are_checkable_and_prayers_carry_no_done_state() {
    let items = sample_day();
    for item in &items {
        match item {
            Item::Task(_) => assert!(item.is_checkable()),
            Item::Event(_) => assert!(!item.is_checkable()),
            Item::PrayerMarker(p) => {
                assert!(!item.is_checkable(), "{} must never be checkable", p.name);
                // The whole of a marker is its name and time.
                let PrayerTime { name: _, at: _ } = p;
            }
        }
    }
    assert_eq!(items.iter().filter(|i| matches!(i, Item::PrayerMarker(_))).count(), 5);
    let done: Vec<bool> =
        items.iter().filter_map(|i| if let Item::Task(t) = i { Some(t.done) } else { None }).collect();
    assert_eq!(done, [true, false]);
}

#[test]
fn ties_put_all_day_first_then_the_prayer_then_events_then_tasks() {
    let (from, to) = (wib(2026, 11, 6, 0, 0), wib(2026, 11, 7, 0, 0));
    let cal = parse(&wrap(
        "BEGIN:VEVENT\nUID:a\nSUMMARY:Midday call\nDTSTART;TZID=Asia/Jakarta:20261106T112600\nEND:VEVENT\n\
         BEGIN:VEVENT\nUID:b\nSUMMARY:Holiday\nDTSTART;VALUE=DATE:20261106\nEND:VEVENT\n",
    ));
    let items = today_timeline(
        occurrences(&cal, from, to, WIB),
        vec![task("t", "Reply", 11, 26, false), task("m", "Midnight", 0, 0, false)],
        vec![prayer("Jumat", 11, 26)],
    );
    let labels: Vec<String> = items.iter().map(label).collect();
    assert_eq!(labels, ["event Holiday", "task Midnight", "prayer Jumat", "event Midday call", "task Reply"]);
}

#[test]
fn a_day_with_nothing_else_still_has_its_prayer_markers() {
    let items = today_timeline(Vec::new(), Vec::new(), vec![prayer("Maghrib", 17, 43), prayer("Isya", 18, 55)]);
    let labels: Vec<String> = items.iter().map(label).collect();
    assert_eq!(labels, ["prayer Maghrib", "prayer Isya"]);
}
