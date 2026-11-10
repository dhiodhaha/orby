//! Fixed Today, Notices and Downloads screens for the snapshot mode: an empty day and a busy one
//! (events with and without a call link, tasks, prayer markers around the now line, hover and
//! focus, a scrolled timeline, the state cards), the Notices list (empty, rows, the row menu, muted
//! sources, the mute note) and the Downloads queue (empty, the engine card, running, paused, done,
//! failed, the refused link, the format menu). Hand-made data, no clock, no files.

use app_downloads::{ErrorKind, Format, JobId, MediaLink, Queue, Tone};

use crate::downloads_model::{Btn, Chip, Field, Row as DlRow, Section};
use crate::downloads_panel::{self as dp, EngineCard};
use crate::notices_model::Source;
use crate::notices_panel::{self as np, MenuItem};
use crate::snapshot::Scene;
use crate::today_panel::{self as tp, Card, Line};
use crate::ui::{Fonts, Icon};

// ---- Today ----

fn event(title: &str, sub: &str, time: &str, join: bool, past: bool) -> Line {
    Line::Event {
        title: title.into(),
        sub: sub.into(),
        time: time.into(),
        join,
        past,
    }
}

fn task(title: &str, sub: &str, time: &str, done: bool, overdue: bool) -> Line {
    Line::Task {
        title: title.into(),
        sub: sub.into(),
        time: time.into(),
        done,
        overdue,
    }
}

fn marker(name: &str, time: &str) -> Line {
    Line::Marker {
        name: name.into(),
        time: time.into(),
    }
}

fn today_view(lines: Vec<Line>, card: Option<Card>) -> tp::View {
    tp::View {
        date: "Tuesday, 10 Nov".into(),
        all_day: Vec::new(),
        banner: ("Ashar".into(), "in 18 min".into(), "14:55".into()),
        card,
        lines,
        footer: "Work · read 14:30".into(),
        scroll: 0.0,
        focus: None,
    }
}

fn busy_lines(earlier_open: bool) -> Vec<Line> {
    let mut lines = vec![Line::Earlier {
        label: "2 tasks done".into(),
        open: earlier_open,
    }];
    if earlier_open {
        lines.extend([
            event("Standup", "Work · Meet", "09:30", false, true),
            marker("Dzuhur", "11:41"),
            task("Reply to Rani", "Inbox", "Done 12:10", true, false),
            task(
                "Book the flight to Jakarta",
                "Personal",
                "Done 13:05",
                true,
                false,
            ),
        ]);
    }
    lines.extend([
        Line::Now("Now 14:37".into()),
        event("Design review", "Work · Zoom", "14:45", true, false),
        marker("Ashar", "14:55"),
        task("Kirim invoice", "Work", "Overdue 2 h", false, true),
        task(
            "Draft the quarterly letter to the board about the new office and the budget",
            "Work",
            "16:00",
            false,
            false,
        ),
        event(
            "1:1 with Dimas",
            "Work · Kopi Klotok, Jl. Kaliurang",
            "16:30",
            false,
            false,
        ),
        task("Water the plants", "Home", "Today", false, false),
        marker("Maghrib", "17:41"),
        event("Futsal", "Personal", "19:00", false, false),
        marker("Isya", "18:53"),
    ]);
    lines
}

/// (name, view, hover)
pub fn today_views() -> Vec<(&'static str, tp::View, Option<tp::Hit>)> {
    let mut out = Vec::new();
    // Nothing left today, no calendar yet.
    out.push((
        "today-empty",
        tp::View {
            footer: String::new(),
            ..today_view(
                vec![
                    Line::Now("Now 14:37".into()),
                    Line::Quiet("No events or tasks left today".into()),
                    marker("Ashar", "14:55"),
                    marker("Maghrib", "17:41"),
                    marker("Isya", "18:53"),
                ],
                Some(Card {
                    icon: Icon::Today,
                    title: "No calendar connected".into(),
                    body: vec![
                        "Copy an ICS link from Outlook, Google or any app,".into(),
                        "then paste it here to see meetings beside tasks.".into(),
                    ],
                    buttons: vec![
                        (Some(tp::Hit::Paste), "Paste ICS link".into()),
                        (None, "Google · coming soon".into()),
                    ],
                }),
            )
        },
        Some(tp::Hit::Paste),
    ));
    // A busy day with all-day chips; the Join button hovered.
    let busy = tp::View {
        all_day: vec![
            ("Hari Pahlawan".into(), "Holidays".into()),
            ("Mum's birthday".into(), String::new()),
            ("Offsite planning week in Bali".into(), "Work".into()),
        ],
        ..today_view(busy_lines(false), None)
    };
    let join = busy
        .lines
        .iter()
        .position(|l| matches!(l, Line::Event { join: true, .. }));
    out.push(("today-busy", busy, join.map(tp::Hit::Join)));
    // Earlier opened, a task's checkbox hovered, the keyboard on another row.
    let mut open = today_view(busy_lines(true), None);
    let check = open
        .lines
        .iter()
        .position(|l| matches!(l, Line::Task { overdue: true, .. }));
    open.focus = open.lines.iter().position(|l| {
        matches!(
            l,
            Line::Event {
                join: false,
                past: false,
                ..
            }
        )
    });
    out.push(("today-earlier-open", open, check.map(tp::Hit::Check)));
    // Scrolled to the bottom: the thumb, rows cut by the list's edge; Add task hovered.
    let mut scrolled = today_view(busy_lines(true), None);
    scrolled.lines.extend([
        task("Call the landlord", "Home", "20:00", false, false),
        task("Read two chapters", "Personal", "21:00", false, false),
        event(
            "Late sync with Berlin",
            "Work · Teams",
            "21:30",
            false,
            false,
        ),
    ]);
    scrolled.scroll = tp::max_scroll(&scrolled) - 10.0;
    out.push(("today-scrolled", scrolled, Some(tp::Hit::AddTask)));
    // The calendar is unreachable: the Offline card with Try again hovered.
    out.push((
        "today-offline",
        today_view(
            busy_lines(false),
            Some(Card {
                icon: Icon::CloudOff,
                title: "Offline".into(),
                body: vec![
                    "Showing events from 13:10.".into(),
                    "Tasks and prayer times work as usual.".into(),
                ],
                buttons: vec![(Some(tp::Hit::Retry), "Try again".into())],
            }),
        ),
        Some(tp::Hit::Retry),
    ));
    // A broken link while it reads again: the quiet "Retrying…" button.
    out.push((
        "today-broken",
        tp::View {
            footer: "Reading Work…".into(),
            ..today_view(
                busy_lines(false),
                Some(Card {
                    icon: Icon::Info,
                    title: "Couldn't read the calendar link".into(),
                    body: vec![
                        "The server answered 404 Not Found for this link.".into(),
                        "Paste a new link, or try again later.".into(),
                    ],
                    buttons: vec![(None, "Retrying…".into())],
                }),
            )
        },
        None,
    ));
    out
}

// ---- Notices ----

fn notice(icon: Icon, title: &str, value: &str, time: &str, unseen: bool, s: Source) -> np::Row {
    np::Row {
        id: 0,
        icon,
        title: title.into(),
        value: value.into(),
        time: time.into(),
        unseen,
        source: s,
    }
}

fn notice_rows() -> Vec<np::Row> {
    vec![
        notice(
            Icon::Tasks,
            "Kirim invoice",
            "due 14:00",
            "14:00",
            true,
            Source::Tasks,
        ),
        notice(
            Icon::Today,
            "Design review with the whole product team",
            "in 10 min · Zoom",
            "14:35",
            true,
            Source::Calendar,
        ),
        notice(
            Icon::Drink,
            "Minum dulu",
            "750 / 2000 ml",
            "13:30",
            false,
            Source::Drink,
        ),
        notice(
            Icon::Prayer,
            "Dzuhur",
            "11:41",
            "11:41",
            false,
            Source::Prayer,
        ),
        notice(
            Icon::Download,
            "Lo-fi beats to study to",
            "saved · 48 MB",
            "10:12",
            false,
            Source::Downloads,
        ),
        notice(
            Icon::Drive,
            "Disk full on D:",
            "2 downloads paused",
            "09:58",
            false,
            Source::Downloads,
        ),
        notice(
            Icon::Focus,
            "Focus done",
            "50 min",
            "09:20",
            false,
            Source::Focus,
        ),
    ]
}

fn notices_view(rows: Vec<np::Row>, n_waiting: usize) -> np::View {
    np::View {
        rows,
        n_waiting,
        muted: Vec::new(),
        undo_clear: false,
        focus: None,
        menu: None,
        menu_item: 0,
        note: None,
    }
}

pub fn notices_views() -> Vec<(&'static str, np::View, Option<np::Hit>)> {
    let mut out = Vec::new();
    out.push(("notices-empty", notices_view(Vec::new(), 0), None));
    out.push((
        "notices-rows",
        notices_view(notice_rows(), 2),
        Some(np::Hit::Row(2)),
    ));
    // Twelve rows: eight shown, "and 4 earlier"; the keyboard on one, Clear all hovered.
    let mut many: Vec<np::Row> = notice_rows();
    many.extend(notice_rows().into_iter().skip(2).take(5));
    let mut v = notices_view(many, 2);
    v.focus = Some(3);
    out.push(("notices-many", v, Some(np::Hit::ClearAll)));
    // The row menu, opened from "···", with the keyboard on Mute until tomorrow.
    let mut v = notices_view(notice_rows(), 2);
    v.menu = Some(1);
    v.menu_item = 2;
    out.push(("notices-menu", v, Some(np::Hit::Menu(MenuItem::Always))));
    // A menu on the last row opens above it.
    let mut v = notices_view(notice_rows(), 0);
    v.menu = Some(6);
    out.push(("notices-menu-up", v, Some(np::Hit::More(6))));
    // Muted sources, with Unmute hovered, and Undo after Clear all.
    let mut v = notices_view(notice_rows().into_iter().take(3).collect(), 1);
    v.muted = vec![
        (Source::Drink, "for 1 hour".into()),
        (Source::Downloads, "until tomorrow".into()),
    ];
    v.undo_clear = true;
    out.push(("notices-muted", v, Some(np::Hit::Unmute(Source::Downloads))));
    // Just muted: the note with Undo stands in for its own muted row.
    let mut v = notices_view(notice_rows().into_iter().take(2).collect(), 0);
    v.muted = vec![
        (Source::Downloads, "for good".into()),
        (Source::Focus, "for 1 hour".into()),
    ];
    v.note = Some(("Downloads muted".into(), "for good".into()));
    out.push(("notices-note", v, Some(np::Hit::UndoMute)));
    out
}

// ---- Downloads ----

fn ids(n: usize) -> Vec<JobId> {
    let mut q = Queue::new();
    (0..n)
        .map(|i| {
            q.add(
                MediaLink::parse(&format!("https://youtu.be/abcdefghij{i}")).unwrap(),
                Format::default(),
                "D:\\Downloads".into(),
            )
        })
        .collect()
}

#[allow(clippy::too_many_arguments)] // one row, every field
fn dl(
    id: JobId,
    section: Section,
    title: &str,
    line: &str,
    chip: Chip,
    audio: bool,
    buttons: Vec<Btn>,
    format_chip: &str,
) -> DlRow {
    DlRow {
        id,
        section,
        title: title.into(),
        line: line.into(),
        chip,
        audio,
        buttons,
        format_chip: format_chip.into(),
    }
}

fn queue_rows() -> Vec<DlRow> {
    let id = ids(9);
    vec![
        dl(
            id[0],
            Section::NeedsYou,
            "This video is private",
            "instagram.com/reel/C9xT2 · Private video",
            Chip::Failed(ErrorKind::Private),
            false,
            vec![Btn::Remove],
            "",
        ),
        dl(
            id[1],
            Section::NeedsYou,
            "Tutorial: building a desk from reclaimed teak, part 3 of 5",
            "Disk full on D: · 61% kept",
            Chip::Ring {
                fraction: 0.61,
                tone: Tone::Ember,
            },
            false,
            vec![Btn::ChangeFolder, Btn::Cancel],
            "",
        ),
        dl(
            id[2],
            Section::Now,
            "Lo-fi beats to study to",
            "190 of 412 MB · 4.2 MB/s · 53 s left",
            Chip::Ring {
                fraction: 0.46,
                tone: Tone::Ink,
            },
            false,
            vec![Btn::Pause, Btn::Cancel],
            "",
        ),
        dl(
            id[3],
            Section::Now,
            "Podcast: Ngobrol santai episode 112",
            "Finding formats…",
            Chip::Reading,
            true,
            vec![Btn::Cancel],
            "",
        ),
        dl(
            id[4],
            Section::Now,
            "youtu.be/q7Lm3kV",
            "Paused · 34%",
            Chip::Paused { fraction: 0.34 },
            false,
            vec![Btn::Resume, Btn::Cancel],
            "",
        ),
        dl(
            id[5],
            Section::Now,
            "tiktok.com/@kopi/video/72",
            "Waiting · offline",
            Chip::Ring {
                fraction: 0.2,
                tone: Tone::Grey,
            },
            false,
            vec![Btn::Cancel],
            "",
        ),
        dl(
            id[6],
            Section::Now,
            "x.com/rani/status/1855",
            "Waiting",
            Chip::Waiting,
            false,
            vec![Btn::Format, Btn::Cancel],
            "720p",
        ),
        dl(
            id[7],
            Section::Done,
            "Gamelan at the kraton",
            "48 MB · 10:42",
            Chip::Done,
            true,
            vec![Btn::Open, Btn::Folder],
            "",
        ),
        dl(
            id[8],
            Section::Done,
            "Couldn't read this link",
            "soundcloud.com/x/y · Retry?",
            Chip::Failed(ErrorKind::Other),
            true,
            vec![Btn::Retry, Btn::Remove],
            "",
        ),
    ]
}

fn dl_view(rows: Vec<DlRow>, engine: EngineCard, field: Field) -> dp::View {
    dp::View {
        count: "2 running · 1 waiting".into(),
        engine,
        field,
        rows,
        folder: "D:\\Downloads\\Orby".into(),
        free: "12.4 GB free on D:".into(),
        scroll: 0.0,
    }
}

pub fn downloads_views() -> Vec<(&'static str, dp::View, Option<dp::Hit>)> {
    let mut out = Vec::new();
    let empty = |engine, field| dp::View {
        count: "nothing yet".into(),
        ..dl_view(Vec::new(), engine, field)
    };
    out.push((
        "downloads-empty",
        empty(EngineCard::None, Field::Empty),
        Some(dp::Hit::Paste),
    ));
    out.push((
        "downloads-nolink",
        empty(EngineCard::Missing { waiting: false }, Field::NoLink),
        Some(dp::Hit::Fetch),
    ));
    out.push((
        "downloads-fetching",
        empty(
            EngineCard::Fetching {
                done: 7_400_000,
                total: Some(18_200_000),
                waiting: true,
            },
            Field::Empty,
        ),
        None,
    ));
    out.push((
        "downloads-engine-failed",
        dl_view(
            Vec::new(),
            EngineCard::Failed("The download stopped halfway".into()),
            Field::Unsupported("shopee.co.id/product/123456789/a-very-long-product-name".into()),
        ),
        None,
    ));
    let rows = queue_rows();
    out.push((
        "downloads-queue",
        dl_view(rows[2..5].to_vec(), EngineCard::None, Field::Empty),
        Some(dp::Hit::Btn(0, Btn::Pause)),
    ));
    let mut all = dl_view(rows.clone(), EngineCard::None, Field::Empty);
    all.scroll = 0.0;
    out.push(("downloads-all", all, Some(dp::Hit::Row(1))));
    let mut scrolled = dl_view(rows, EngineCard::None, Field::Empty);
    scrolled.scroll = dp::max_scroll(&scrolled);
    out.push(("downloads-scrolled", scrolled, Some(dp::Hit::ClearList)));
    let done = queue_rows()[7..].to_vec();
    out.push((
        "downloads-done",
        dl_view(done, EngineCard::None, Field::Empty),
        Some(dp::Hit::Folder),
    ));
    out
}

/// (name, chosen format, hovered row)
pub fn menu_views() -> Vec<(&'static str, Format, Option<usize>)> {
    vec![
        (
            "downloads-menu",
            Format::Video {
                max_height: Some(720),
            },
            Some(2),
        ),
        (
            "downloads-menu-audio",
            Format::Audio {
                codec: app_downloads::AudioCodec::Mp3,
            },
            None,
        ),
    ]
}

pub fn scenes(_f: &Fonts) -> Vec<Scene> {
    let mut out = Vec::new();
    for (name, v, hover) in today_views() {
        let h = tp::height(&v);
        out.push(Scene::new(name, tp::W, h, move |cv, t, f| {
            tp::draw(cv, t, f, &v, hover)
        }));
    }
    for (name, v, hover) in notices_views() {
        let h = np::height(&v);
        out.push(Scene::new(name, np::W, h, move |cv, t, f| {
            np::draw(cv, t, f, &v, hover)
        }));
    }
    for (name, v, hover) in downloads_views() {
        let h = dp::height(&v);
        out.push(Scene::new(name, dp::W, h, move |cv, t, f| {
            dp::draw(cv, t, f, &v, hover)
        }));
    }
    for (name, chosen, hover) in menu_views() {
        out.push(Scene::new(name, dp::MENU_W, dp::MENU_H, move |cv, t, f| {
            dp::draw_menu(cv, t, f, chosen, hover)
        }));
    }
    out
}
