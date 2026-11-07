//! The calendar side of the Today panel, decided without the OS: what a pasted calendar link
//! is, how a read went, what the day looks like around now, and which event pill comes next.
//! "Now" and the zone offset come in from the caller.

use std::path::PathBuf;

use app_calendar::{Date, Instant, Item, Occurrence};

/// The first event pill comes this long before the start.
pub const SOON_SECS: i64 = 600;

/// Where the ICS text comes from.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Source {
    Http(String),
    File(PathBuf),
}

impl Source {
    /// A pasted link or path: `https://`, `http://`, `webcal://` (read over https), `file:///`,
    /// or a Windows path to an `.ics` file. Anything else (a sentence, two lines) is refused.
    pub fn parse(text: &str) -> Option<Source> {
        let s = text
            .trim()
            .trim_matches(|c| matches!(c, '"' | '\'' | '<' | '>'));
        if s.is_empty() || s.contains(char::is_whitespace) && !looks_like_path(s) {
            return None;
        }
        if s.contains(['\n', '\r']) {
            return None;
        }
        let lower = s.to_ascii_lowercase();
        for scheme in ["webcals://", "webcal://"] {
            if lower.starts_with(scheme) {
                return host_ok(&s[scheme.len()..])
                    .then(|| Source::Http(format!("https://{}", &s[scheme.len()..])));
            }
        }
        for scheme in ["https://", "http://"] {
            if lower.starts_with(scheme) {
                return host_ok(&s[scheme.len()..]).then(|| Source::Http(s.to_string()));
            }
        }
        if lower.starts_with("file:///") {
            let path = percent_decode(&s["file:///".len()..]).replace('/', "\\");
            return looks_like_path(&path).then(|| Source::File(PathBuf::from(path)));
        }
        if looks_like_path(s) && lower.ends_with(".ics") {
            return Some(Source::File(PathBuf::from(s)));
        }
        None
    }

    /// What the log and the panel may show: the host or the file name. A private ICS link
    /// carries its secret in the path, so the path is never shown.
    pub fn label(&self) -> String {
        match self {
            Source::Http(url) => {
                let rest = url.split_once("://").map_or(url.as_str(), |(_, r)| r);
                rest.split(['/', '?', '#'])
                    .next()
                    .unwrap_or(rest)
                    .to_string()
            }
            Source::File(p) => p
                .file_name()
                .map(|n| n.to_string_lossy().into_owned())
                .unwrap_or_else(|| "file".into()),
        }
    }
}

fn host_ok(rest: &str) -> bool {
    let host = rest.split(['/', '?', '#']).next().unwrap_or("");
    !host.is_empty() && !host.starts_with(':')
}

fn looks_like_path(s: &str) -> bool {
    let b = s.as_bytes();
    (b.len() > 3 && b[0].is_ascii_alphabetic() && b[1] == b':' && (b[2] == b'\\' || b[2] == b'/'))
        || s.starts_with("\\\\")
}

fn percent_decode(s: &str) -> String {
    let mut out = Vec::with_capacity(s.len());
    let b = s.as_bytes();
    let mut i = 0;
    while i < b.len() {
        if b[i] == b'%' && i + 2 < b.len() {
            let hex = |c: u8| (c as char).to_digit(16);
            if let (Some(h), Some(l)) = (hex(b[i + 1]), hex(b[i + 2])) {
                out.push((h * 16 + l) as u8);
                i += 3;
                continue;
            }
        }
        out.push(b[i]);
        i += 1;
    }
    String::from_utf8_lossy(&out).into_owned()
}

/// Why a read failed: the network (try again later) or the link itself (paste a new one).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Fail {
    Offline,
    Broken,
}

/// How the calendar link stands.
#[derive(Clone, Debug, PartialEq)]
pub enum Sync {
    NotConnected,
    /// The first read of a new link is on its way and nothing is cached.
    Loading,
    /// `read_at` is the Unix second of the last good read.
    Ready {
        read_at: i64,
    },
    Failed {
        why: Fail,
        detail: String,
        last_ok: Option<i64>,
    },
}

impl Sync {
    /// After a failed read: what was good before stays on screen.
    pub fn failed(&self, why: Fail, detail: String) -> Sync {
        let last_ok = match self {
            Sync::Ready { read_at } => Some(*read_at),
            Sync::Failed { last_ok, .. } => *last_ok,
            _ => None,
        };
        Sync::Failed {
            why,
            detail,
            last_ok,
        }
    }
}

/// Checks the text a link returned before it replaces the cached calendar.
pub fn check_ics(text: &str) -> Result<(), String> {
    if text.contains("BEGIN:VCALENDAR") {
        Ok(())
    } else if text.trim().is_empty() {
        Err("The link returned nothing".into())
    } else {
        Err("That is not a calendar (ICS) link".into())
    }
}

// ---- event pills ----

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Cue {
    /// 10 min before: normal priority, waits for a focus break.
    Soon,
    /// At the start: urgent, shows even while the orb is hidden.
    Start,
}

fn cues(o: &Occurrence) -> [(Instant, Cue); 2] {
    [
        (o.start.add_seconds(-SOON_SECS), Cue::Soon),
        (o.start, Cue::Start),
    ]
}

/// When the next event pill is due after `after`. All-day events get no pill.
pub fn next_cue(occ: &[Occurrence], after: Instant) -> Option<Instant> {
    occ.iter()
        .filter(|o| !o.all_day)
        .flat_map(cues)
        .map(|(at, _)| at)
        .filter(|at| *at > after)
        .min()
}

/// The pills due in `(from, to]`, by event. An event whose start is also in the window gets
/// only its start pill (after a sleep, "in 10 min" would be wrong).
pub fn cues_between(occ: &[Occurrence], from: Instant, to: Instant) -> Vec<(usize, Cue)> {
    let mut out: Vec<(usize, Cue, Instant)> = Vec::new();
    for (i, o) in occ.iter().enumerate().filter(|(_, o)| !o.all_day) {
        let due: Vec<(Instant, Cue)> = cues(o)
            .into_iter()
            .filter(|(at, _)| *at > from && *at <= to)
            .collect();
        if let Some((at, cue)) = due.last() {
            out.push((i, *cue, *at));
        }
    }
    out.sort_by_key(|(i, _, at)| (*at, *i));
    out.into_iter().map(|(i, c, _)| (i, c)).collect()
}

/// Join shows from 10 min before the start until the end, and only with a call link.
pub fn join_open(o: &Occurrence, now: Instant) -> bool {
    o.link.is_some()
        && !o.all_day
        && now >= o.start.add_seconds(-SOON_SECS)
        && now < o.end.max(o.start.add_seconds(60))
}

// ---- the day ----

/// The day as the Today panel lays it out around now.
#[derive(Debug, Default, PartialEq)]
pub struct Day {
    pub all_day: Vec<Occurrence>,
    /// Events that have ended and tasks done, folded into "Earlier today".
    pub earlier: Vec<Item>,
    /// Running and coming events, open tasks (overdue first), and the prayer times still ahead.
    pub ahead: Vec<Item>,
}

/// Splits the merged timeline at `now`. A prayer time that has passed leaves the panel; it is
/// never folded into "Earlier today", so nothing about a prayer reads as done.
pub fn split_day(items: Vec<Item>, now: Instant) -> Day {
    let mut d = Day::default();
    for item in items {
        match item {
            Item::Event(o) if o.all_day => d.all_day.push(o),
            Item::Event(ref o) if o.end.max(o.start.add_seconds(60)) <= now => d.earlier.push(item),
            Item::Task(ref t) if t.done => d.earlier.push(item),
            Item::PrayerMarker(ref p) if p.at.add_seconds(60) <= now => {}
            other => d.ahead.push(other),
        }
    }
    d
}

/// The muted part of the "Earlier today" row. It counts tasks only.
pub fn earlier_label(earlier: &[Item]) -> String {
    match earlier
        .iter()
        .filter(|i| matches!(i, Item::Task(t) if t.done))
        .count()
    {
        0 => String::new(),
        1 => "1 task done".into(),
        n => format!("{n} tasks done"),
    }
}

/// The occurrences that touch the local day starting at `day_start`.
pub fn on_day(occ: &[Occurrence], day_start: Instant) -> Vec<Occurrence> {
    let day_end = day_start.add_seconds(86_400);
    occ.iter()
        .filter(|o| o.start < day_end && o.end.max(o.start.add_seconds(1)) > day_start)
        .cloned()
        .collect()
}

// ---- words ----

/// "in 25 min", "in 1 h 6 min", "in 2 h". Rounds up, so it never says "in 0 min".
pub fn in_words(secs: i64) -> String {
    let m = (secs.max(0) + 59) / 60;
    match (m / 60, m % 60) {
        (0, 0) => "now".into(),
        (0, m) => format!("in {m} min"),
        (h, 0) => format!("in {h} h"),
        (h, m) => format!("in {h} h {m} min"),
    }
}

pub fn hhmm(at: Instant, offset_min: i32) -> String {
    let l = at.to_local(offset_min);
    format!("{:02}:{:02}", l.hour(), l.minute())
}

/// "Saturday, 7 Nov".
pub fn day_title(d: Date) -> String {
    const DAYS: [&str; 7] = [
        "Monday",
        "Tuesday",
        "Wednesday",
        "Thursday",
        "Friday",
        "Saturday",
        "Sunday",
    ];
    const MONTHS: [&str; 12] = [
        "Jan", "Feb", "Mar", "Apr", "May", "Jun", "Jul", "Aug", "Sep", "Oct", "Nov", "Dec",
    ];
    format!(
        "{}, {} {}",
        DAYS[d.weekday().monday_index() as usize],
        d.day(),
        MONTHS[usize::from(d.month() - 1)]
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use app_calendar::{LinkKind, LocalDateTime, MeetingLink, PrayerTime, TaskDue};

    const WIB: i32 = 7 * 60;

    fn at(h: u8, m: u8) -> Instant {
        LocalDateTime::new(Date::new(2026, 11, 6).unwrap(), h, m, 0)
            .unwrap()
            .to_instant(WIB)
    }

    fn event(title: &str, start: Instant, mins: i64, link: bool) -> Occurrence {
        Occurrence {
            uid: title.to_lowercase(),
            title: title.into(),
            start,
            end: start.add_seconds(mins * 60),
            all_day: false,
            link: link.then(|| MeetingLink {
                kind: LinkKind::Meet,
                url: "https://meet.google.com/abc-defg-hij".into(),
            }),
            location: None,
        }
    }

    #[test]
    fn links_and_paths_are_recognised_and_other_text_is_not() {
        assert_eq!(
            Source::parse("  https://calendar.example.com/ical/x/basic.ics \r\n"),
            Some(Source::Http(
                "https://calendar.example.com/ical/x/basic.ics".into()
            ))
        );
        assert_eq!(
            Source::parse("webcal://outlook.office365.com/owa/calendar/a/b/reachcalendar.ics"),
            Some(Source::Http(
                "https://outlook.office365.com/owa/calendar/a/b/reachcalendar.ics".into()
            ))
        );
        assert_eq!(
            Source::parse("http://127.0.0.1:8765/cal.ics"),
            Some(Source::Http("http://127.0.0.1:8765/cal.ics".into()))
        );
        assert_eq!(
            Source::parse("file:///C:/Users/me/My%20Cal/work.ics"),
            Some(Source::File(PathBuf::from(
                "C:\\Users\\me\\My Cal\\work.ics"
            )))
        );
        assert_eq!(
            Source::parse("\"C:\\Users\\me\\My Cal\\work.ics\""),
            Some(Source::File(PathBuf::from(
                "C:\\Users\\me\\My Cal\\work.ics"
            )))
        );
        for bad in [
            "",
            "beli galon besok",
            "https://",
            "C:\\notes.txt",
            "ftp://x/y.ics",
            "https://a.com/x\nhttps://b.com/y",
        ] {
            assert_eq!(Source::parse(bad), None, "{bad:?}");
        }
    }

    #[test]
    fn the_label_never_shows_the_secret_part_of_a_link() {
        let s = Source::parse("https://calendar.google.com/calendar/ical/me%40gmail.com/private-0123456789abcdef/basic.ics").unwrap();
        assert_eq!(s.label(), "calendar.google.com");
        assert_eq!(
            Source::parse("C:\\cal\\work.ics").unwrap().label(),
            "work.ics"
        );
    }

    #[test]
    fn a_failed_read_keeps_the_last_good_time() {
        let s = Sync::Ready { read_at: 100 }.failed(Fail::Offline, "x".into());
        assert_eq!(
            s,
            Sync::Failed {
                why: Fail::Offline,
                detail: "x".into(),
                last_ok: Some(100)
            }
        );
        let s = s.failed(Fail::Broken, "y".into());
        assert_eq!(
            s,
            Sync::Failed {
                why: Fail::Broken,
                detail: "y".into(),
                last_ok: Some(100)
            }
        );
        assert_eq!(
            Sync::Loading.failed(Fail::Broken, "z".into()),
            Sync::Failed {
                why: Fail::Broken,
                detail: "z".into(),
                last_ok: None
            }
        );
        assert!(check_ics("BEGIN:VCALENDAR\r\nEND:VCALENDAR").is_ok());
        assert!(check_ics("<html>sign in</html>").is_err());
        assert!(check_ics("").is_err());
    }

    #[test]
    fn pills_come_ten_minutes_before_and_at_the_start() {
        let occ = vec![
            event("Standup", at(10, 30), 30, true),
            event("Review", at(13, 0), 60, false),
        ];
        assert_eq!(next_cue(&occ, at(10, 0)), Some(at(10, 20)));
        assert_eq!(next_cue(&occ, at(10, 20)), Some(at(10, 30)));
        assert_eq!(next_cue(&occ, at(10, 30)), Some(at(12, 50)));
        assert_eq!(next_cue(&occ, at(13, 0)), None);
        assert_eq!(
            cues_between(&occ, at(10, 0), at(10, 20)),
            vec![(0, Cue::Soon)]
        );
        assert_eq!(
            cues_between(&occ, at(10, 20), at(10, 30)),
            vec![(0, Cue::Start)]
        );
        // Woken after both moments: only "starting now".
        assert_eq!(
            cues_between(&occ, at(10, 0), at(10, 31)),
            vec![(0, Cue::Start)]
        );
        assert_eq!(
            cues_between(&occ, at(10, 31), at(12, 55)),
            vec![(1, Cue::Soon)]
        );
    }

    #[test]
    fn all_day_events_get_no_pill() {
        let mut o = event("Ayah di Solo", at(0, 0), 24 * 60, false);
        o.all_day = true;
        assert_eq!(next_cue(&[o.clone()], at(0, 0).add_seconds(-3600)), None);
        assert!(cues_between(&[o], at(0, 0).add_seconds(-3600), at(1, 0)).is_empty());
    }

    #[test]
    fn join_shows_from_ten_minutes_before_until_the_end_and_only_with_a_link() {
        let o = event("Standup", at(10, 30), 30, true);
        assert!(!join_open(&o, at(10, 19)));
        assert!(join_open(&o, at(10, 20)));
        assert!(join_open(&o, at(10, 59)));
        assert!(!join_open(&o, at(11, 0)));
        assert!(!join_open(
            &event("Jemput Raka", at(16, 0), 45, false),
            at(16, 0)
        ));
    }

    #[test]
    fn the_day_splits_at_now_and_passed_prayers_leave_without_a_trace() {
        let task = |id: &str, h: u8, done: bool| {
            Item::Task(TaskDue {
                id: id.into(),
                title: id.into(),
                due: at(h, 0),
                done,
            })
        };
        let marker = |n: &str, h: u8, m: u8| {
            Item::PrayerMarker(PrayerTime {
                name: n.into(),
                at: at(h, m),
            })
        };
        let mut all_day = event("Ayah di Solo", at(0, 0), 24 * 60, false);
        all_day.all_day = true;
        let items = vec![
            Item::Event(all_day.clone()),
            marker("Subuh", 4, 1),
            Item::Event(event("Gym", at(6, 0), 60, false)),
            task("Bayar listrik", 8, true),
            task("Kirim invoice", 9, false),
            Item::Event(event("Standup", at(10, 0), 30, true)),
            marker("Dzuhur", 11, 26),
            task("Beras 5 kg", 17, true),
        ];
        let d = split_day(items, at(10, 20));
        assert_eq!(d.all_day, vec![all_day]);
        let names = |v: &[Item]| {
            v.iter()
                .map(|i| match i {
                    Item::Event(o) => o.title.clone(),
                    Item::Task(t) => t.title.clone(),
                    Item::PrayerMarker(p) => p.name.clone(),
                })
                .collect::<Vec<_>>()
        };
        assert_eq!(names(&d.earlier), ["Gym", "Bayar listrik", "Beras 5 kg"]);
        // Overdue open task, the running meeting, then the prayer ahead.
        assert_eq!(names(&d.ahead), ["Kirim invoice", "Standup", "Dzuhur"]);
        assert!(d
            .earlier
            .iter()
            .all(|i| !matches!(i, Item::PrayerMarker(_))));
        assert_eq!(earlier_label(&d.earlier), "2 tasks done");
    }

    #[test]
    fn a_prayer_marker_stays_through_its_own_minute() {
        let m = || {
            vec![Item::PrayerMarker(PrayerTime {
                name: "Ashar".into(),
                at: at(14, 41),
            })]
        };
        assert_eq!(split_day(m(), at(14, 41)).ahead.len(), 1);
        assert_eq!(split_day(m(), at(14, 41).add_seconds(59)).ahead.len(), 1);
        assert!(split_day(m(), at(14, 42)).ahead.is_empty());
    }

    #[test]
    fn earlier_counts_tasks_only() {
        assert_eq!(
            earlier_label(&[Item::Event(event("Gym", at(6, 0), 60, false))]),
            ""
        );
        assert_eq!(
            earlier_label(&[Item::Task(TaskDue {
                id: "a".into(),
                title: "a".into(),
                due: at(8, 0),
                done: true
            })]),
            "1 task done"
        );
    }

    #[test]
    fn only_occurrences_that_touch_the_day_are_kept() {
        let day = at(0, 0);
        let occ = vec![
            event("Yesterday late", day.add_seconds(-7200), 60, false),
            event("Across midnight", day.add_seconds(-1800), 60, false),
            event("Today", at(9, 0), 30, false),
            event("Tomorrow", day.add_seconds(86_400), 30, false),
        ];
        let kept: Vec<String> = on_day(&occ, day).into_iter().map(|o| o.title).collect();
        assert_eq!(kept, ["Across midnight", "Today"]);
    }

    #[test]
    fn words_for_times() {
        assert_eq!(in_words(66 * 60), "in 1 h 6 min");
        assert_eq!(in_words(25 * 60 - 30), "in 25 min");
        assert_eq!(in_words(120 * 60), "in 2 h");
        assert_eq!(in_words(1), "in 1 min");
        assert_eq!(in_words(0), "now");
        assert_eq!(hhmm(at(10, 30), WIB), "10:30");
        assert_eq!(day_title(Date::new(2026, 11, 6).unwrap()), "Friday, 6 Nov");
        assert_eq!(
            day_title(Date::new(2026, 11, 7).unwrap()),
            "Saturday, 7 Nov"
        );
    }
}
