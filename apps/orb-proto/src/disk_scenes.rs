//! Fixed Disk screens for the snapshot mode: the drive list, a scan in progress, Results
//! (folders, files, types), Safe to clean, a Review, Duplicates (the offer and the groups) and the
//! confirm sheet. Hand-made data, no file system, so Windows and macOS draw the same views and a
//! change to the drawing can be checked pixel for pixel against the last run.

use crate::disk_model::{self as m, Rect};
use crate::disk_panel::{self as dp, *};
use crate::ui::Icon;

const GB: u64 = 1 << 30;
const MB: u64 = 1 << 20;

fn row(
    name: &str,
    bytes: u64,
    top: u64,
    total: u64,
    dir: bool,
    tag: Option<&'static str>,
) -> ListRow {
    let pct = bytes as f64 * 100.0 / total as f64;
    ListRow {
        name: name.into(),
        bytes,
        size: size_label(bytes),
        pct: pct_label(pct),
        pct_v: pct,
        bar: bytes as f32 / top as f32,
        dir,
        tag,
        checked: false,
        counting: false,
    }
}

fn folder_rows() -> Vec<ListRow> {
    let sizes: [(&str, u64, bool, Option<&'static str>); 12] = [
        ("Library", 61 * GB + 300 * MB, true, None),
        ("Movies", 24 * GB + 700 * MB, true, None),
        ("Developer", 18 * GB + 200 * MB, true, None),
        ("Downloads", 9 * GB + 400 * MB, true, None),
        ("Pictures", 6 * GB + 100 * MB, true, None),
        ("Documents", 3 * GB + 800 * MB, true, None),
        (".cache", 2 * GB + 200 * MB, true, Some("hidden")),
        ("Music", GB + 300 * MB, true, None),
        ("Desktop", 640 * MB, true, None),
        ("Public", 12 * MB, true, None),
        ("recording-2026-11-03.mov", 410 * MB, false, None),
        (".zsh_history", 2 * MB, false, Some("hidden")),
    ];
    let total: u64 = sizes.iter().map(|s| s.1).sum();
    let top = sizes[0].1;
    sizes
        .iter()
        .map(|(n, b, d, t)| row(n, *b, top, total, *d, *t))
        .collect()
}

fn segs(rows: &[ListRow]) -> Vec<Seg> {
    let mut out: Vec<Seg> = rows
        .iter()
        .take(5)
        .enumerate()
        .map(|(i, r)| Seg {
            label: r.name.clone(),
            bytes: r.bytes,
            kind: SegKind::Grey(i),
            row: Some(i),
        })
        .collect();
    out.push(Seg {
        label: "Other".into(),
        bytes: rows[5..].iter().map(|r| r.bytes).sum(),
        kind: SegKind::Grey(5),
        row: None,
    });
    out.push(Seg {
        label: "Free".into(),
        bytes: 14 * GB,
        kind: SegKind::Free,
        row: None,
    });
    out
}

/// The treemap's tiles for these rows in `map`, the way disk.rs lays them out.
fn tiles(rows: &[ListRow], map: Rect, skeleton: u64) -> Vec<MapTile> {
    let shown = 6;
    let rest: u64 = rows[shown..].iter().map(|r| r.bytes).sum();
    let mut vals: Vec<f64> = rows[..shown].iter().map(|r| r.bytes as f64).collect();
    vals.push(rest as f64);
    if skeleton > 0 {
        vals.push(skeleton as f64);
    }
    let rects = m::squarify(&vals, map);
    let mut out: Vec<MapTile> = (0..shown)
        .map(|i| MapTile {
            row: Some(i),
            rect: rects[i],
            inner: if i == 0 {
                let r = rects[0];
                let area = (r.0 + 6.0, r.1 + 42.0, r.2 - 6.0, r.3 - 6.0);
                let kids = [
                    ("Caches", 22 * GB),
                    ("Containers", 17 * GB),
                    ("Application Support", 12 * GB),
                    ("Developer", 6 * GB),
                ];
                let ih = (area.3 - area.1) * (57.0 / 61.3);
                m::squarify(
                    &kids.iter().map(|k| k.1 as f64).collect::<Vec<_>>(),
                    (area.0, area.1, area.2, area.1 + ih),
                )
                .into_iter()
                .zip(kids)
                .map(|(r, (n, s))| (r, n.to_string(), s))
                .collect()
            } else {
                Vec::new()
            },
            more: 0,
            more_bytes: 0,
            skeleton: false,
        })
        .collect();
    out.push(MapTile {
        row: None,
        rect: rects[shown],
        inner: Vec::new(),
        more: rows.len() - shown,
        more_bytes: rest,
        skeleton: false,
    });
    if skeleton > 0 {
        out.push(MapTile {
            row: None,
            rect: rects[shown + 1],
            inner: Vec::new(),
            more: 0,
            more_bytes: 0,
            skeleton: true,
        });
    }
    out
}

fn file_rows() -> Vec<FileRow> {
    [
        (
            "recording-2026-11-03.mov",
            "Movies/Screen",
            "Nov 2026",
            4 * GB + 200 * MB,
            Icon::Film,
            true,
        ),
        (
            "Xcode_26.1.xip",
            "Downloads",
            "Oct 2026",
            3 * GB + 100 * MB,
            Icon::Archive,
            true,
        ),
        (
            "ubuntu-26.04-desktop-arm64.iso",
            "Downloads",
            "Sep 2026",
            2 * GB + 800 * MB,
            Icon::Archive,
            false,
        ),
        (
            "Family trip.mp4",
            "Movies",
            "Aug 2025",
            GB + 900 * MB,
            Icon::Film,
            false,
        ),
        (
            "model-weights-q4.gguf",
            "Developer/llm/models/very/deep/folder/that/needs/a/cut",
            "Jul 2026",
            GB + 400 * MB,
            Icon::File,
            false,
        ),
        (
            "Photos Library.photoslibrary",
            "Pictures",
            "Nov 2026",
            GB + 100 * MB,
            Icon::File,
            false,
        ),
        (
            "node_modules.tar.gz",
            "Developer/archive",
            "Mar 2026",
            820 * MB,
            Icon::Archive,
            false,
        ),
        (
            "Keynote talk final final.key",
            "Documents/Talks",
            "Feb 2026",
            610 * MB,
            Icon::File,
            false,
        ),
        (
            "simulator-runtime.dmg",
            "Library/Developer",
            "Jan 2026",
            540 * MB,
            Icon::Archive,
            false,
        ),
        (
            "lecture-07.mp4",
            "Movies/Lectures",
            "Dec 2025",
            480 * MB,
            Icon::Film,
            false,
        ),
        (
            "backup.zip",
            "Desktop",
            "Nov 2025",
            300 * MB,
            Icon::Archive,
            false,
        ),
    ]
    .into_iter()
    .map(|(n, sub, month, b, icon, checked)| FileRow {
        name: n.into(),
        sub: sub.into(),
        month: month.into(),
        size: size_label(b),
        icon,
        checked,
    })
    .collect()
}

fn types() -> Vec<TypeRow> {
    [
        ("Video", 31 * GB, "212 files"),
        ("Archives", 14 * GB, "96 files"),
        ("Apps", 11 * GB, "41 files"),
        ("Images", 7 * GB, "18,204 files"),
        ("Documents", 3 * GB, "2,310 files"),
        ("Other", 62 * GB, "1,212,044 files"),
    ]
    .into_iter()
    .enumerate()
    .map(|(i, (l, b, files))| TypeRow {
        label: l,
        bytes: b,
        size: size_label(b),
        files: files.into(),
        grey: i,
    })
    .collect()
}

fn results(tab: Tab, map: Rect, scanning: bool) -> ResultsView {
    let mut rows = folder_rows();
    if scanning {
        rows.truncate(6);
        rows[2].counting = true;
    }
    let tiles = if scanning {
        tiles(&folder_rows(), map, 40 * GB)
    } else {
        tiles(&rows, map, 0)
    };
    let files = file_rows();
    ResultsView {
        folder: true,
        chip: "dhafin".into(),
        chip_sub: if scanning {
            "· scanning".into()
        } else {
            "· scanned 14:37".into()
        },
        big: "128 GB".into(),
        of: "used of 228 GB".into(),
        right: "14 GB free".into(),
        segs: segs(&folder_rows()),
        crumbs: vec!["dhafin".into(), "Library".into()],
        tab,
        rows,
        first: 0,
        sel: if tab == Tab::Folders && !scanning {
            Some(1)
        } else {
            None
        },
        tiles,
        types: types(),
        files_count: "200 biggest files under dhafin".into(),
        files,
        files_first: 0,
        selected: match tab {
            Tab::Files => Some(("2 selected".into(), "7.3 GB".into())),
            _ => None,
        },
        can_recycle: true,
        detail: (!scanning).then(|| Detail {
            name: "Movies".into(),
            path: "/Users/dhafin/Movies/Screen Recordings/2026/November/week 45".into(),
            pct: "19.3%".into(),
            size: "24.7 GB".into(),
        }),
        scanning,
        live: scanning.then(|| LiveView {
            label: "Scanning dhafin".into(),
            path: "/Users/dhafin/Library/Containers/com.apple.Safari/Data/Library/Caches".into(),
            frac: Some(0.64),
            foot: "1,051,388 files so far · 0:09".into(),
        }),
    }
}

fn clean() -> CleanView {
    let rows = |base: usize, items: &[(&str, &str, bool, u64, Icon)]| -> Vec<CleanRow> {
        items
            .iter()
            .enumerate()
            .map(|(k, (title, body, cf, b, icon))| CleanRow {
                index: base + k,
                icon: *icon,
                title: (*title).into(),
                body: (*body).into(),
                check_first: *cf,
                size: size_label(*b),
                checked: base + k == 1,
                can_review: true,
            })
            .collect()
    };
    CleanView {
        place: "dhafin".into(),
        total: "23.4 GB".into(),
        filter: Filter::All,
        counts: [5, 3, 2],
        groups: vec![
            CleanGroup {
                title: "Suggested".into(),
                sub: "3 · 17.1 GB".into(),
                rows: rows(
                    0,
                    &[
                        (
                            "Xcode DerivedData",
                            "Build products Xcode makes again",
                            false,
                            9 * GB,
                            Icon::Layers,
                        ),
                        (
                            "Browser caches",
                            "Chrome, Edge · never cookies or logins",
                            false,
                            3 * GB + 200 * MB,
                            Icon::Globe,
                        ),
                        (
                            "Old downloads",
                            "Not opened for 90 days · 41 items",
                            true,
                            4 * GB + 900 * MB,
                            Icon::Download,
                        ),
                    ],
                ),
            },
            CleanGroup {
                title: "Developer".into(),
                sub: "2 · 6.3 GB".into(),
                rows: rows(
                    3,
                    &[
                        (
                            "iOS Simulator runtimes",
                            "Unused simulators · download again when needed",
                            true,
                            5 * GB,
                            Icon::Layers,
                        ),
                        (
                            "npm cache",
                            "Packages npm keeps to install faster",
                            false,
                            GB + 300 * MB,
                            Icon::Archive,
                        ),
                    ],
                ),
            },
        ],
        first: 0,
        bin: "1.2 GB".into(),
        nothing: None,
        selected: Some(("1 selected".into(), "3.2 GB".into())),
    }
}

fn review() -> ReviewView {
    let rows = file_rows();
    ReviewView {
        title: "Old downloads".into(),
        sub: "41 items · 4.9 GB".into(),
        len: rows.len(),
        rows_from: 0,
        rows,
        first: 0,
        selected: Some(("2 selected".into(), "7.3 GB".into())),
        can_recycle: true,
        all: false,
    }
}

fn dupe_group(
    name: &str,
    paths: &[(&str, &str)],
    keep: usize,
    checked: usize,
    sub: &str,
) -> DupeGroupView {
    DupeGroupView {
        len: paths.len(),
        name: name.into(),
        sub: sub.into(),
        rows: paths
            .iter()
            .enumerate()
            .map(|(i, (p, d))| DupeRowView {
                path: (*p).into(),
                date: (*d).into(),
                keep: i == keep,
                checked: i == checked && i != keep,
            })
            .collect(),
    }
}

fn dupes(groups: bool) -> DupesView {
    let body = if groups {
        DupesBody::Groups(vec![
            dupe_group(
                "Family trip.mp4",
                &[
                    ("/Users/dhafin/Movies/Family trip.mp4", "6 Aug 2025"),
                    ("/Users/dhafin/Downloads/Family trip.mp4", "2 Nov 2026"),
                    (
                        "/Users/dhafin/Desktop/old stuff/to sort/videos/Family trip.mp4",
                        "3 Nov 2026",
                    ),
                ],
                0,
                1,
                "3 copies · 3.8 GB to reclaim",
            ),
            dupe_group(
                "Xcode_26.1.xip",
                &[
                    ("/Users/dhafin/Downloads/Xcode_26.1.xip", "14 Oct 2026"),
                    (
                        "/Users/dhafin/Developer/installers/Xcode_26.1.xip",
                        "15 Oct 2026",
                    ),
                ],
                1,
                9,
                "2 copies · 3.1 GB to reclaim · shares space (APFS clone)",
            ),
            dupe_group(
                "IMG_2041.HEIC",
                &[
                    ("/Users/dhafin/Pictures/2026/IMG_2041.HEIC", "1 Jan 2026"),
                    ("/Users/dhafin/Downloads/IMG_2041.HEIC", "4 Jan 2026"),
                ],
                0,
                9,
                "2 copies · 3.2 MB to reclaim",
            ),
        ])
    } else {
        DupesBody::Offer {
            title: "Possible duplicates: up to 4.1 GB".into(),
            body: "23 groups of files share a size. Only a check of their contents can say which are real copies; nothing is moved until you pick."
                .into(),
            cost: m::dupe_cost_words(None),
            check: true,
        }
    };
    DupesView {
        place: "dhafin, from the 14:37 scan".into(),
        total: if groups {
            "6.9 GB".into()
        } else {
            "up to 4.1 GB".into()
        },
        total_label: if groups { " to reclaim" } else { " possible" },
        sub: if groups {
            "3 groups · 1 skipped".into()
        } else {
            "23 same-size groups".into()
        },
        body,
        first: 0,
        selected: groups.then(|| ("1 selected".into(), "1.9 GB".into())),
        all: false,
        can_recycle: true,
    }
}

fn confirm() -> ConfirmView {
    ConfirmView {
        title: m::confirm_title(3),
        groups: vec![
            ConfirmGroup {
                check_first: true,
                count: "1 · 4.9 GB".into(),
                items: vec![(
                    "Old downloads".into(),
                    "/Users/dhafin/Downloads".into(),
                    "4.9 GB".into(),
                )],
                more: None,
            },
            ConfirmGroup {
                check_first: false,
                count: "2 · 12.2 GB".into(),
                items: vec![
                    (
                        "Xcode DerivedData".into(),
                        "/Users/dhafin/Library/Developer/Xcode/DerivedData".into(),
                        "9 GB".into(),
                    ),
                    ("Browser caches".into(), String::new(), "3.2 GB".into()),
                ],
                more: None,
            },
        ],
        note: m::put_back_words(3),
        footer: "Moving 17.1 GB to the Trash".into(),
        action: "Move 3 to Trash".into(),
        on_move: false,
    }
}

fn view(screen: Screen) -> View {
    View {
        screen,
        cards: Vec::new(),
        confirm: None,
        cursor: None,
        kb: false,
        w: dp::W,
        room: dp::H,
        max: false,
        motion: Motion::default(),
    }
}

/// Every scene by name, with the pointer's hit. `map` is the treemap's box for a bare Results
/// window (`dp::map_rect` at `W` x `H`, no cards).
pub fn scenes(map: Rect) -> Vec<(&'static str, View, Option<Hit>)> {
    let mut out = Vec::new();
    let mut start = view(Screen::Start(
        vec![
            DriveRow {
                name: "dhafin (home folder)".into(),
                free: "128 GB used · 14 GB free".into(),
                used: 0.94,
            },
            DriveRow {
                name: "Macintosh HD".into(),
                free: "14 GB free of 228 GB".into(),
                used: 0.94,
            },
        ],
        false,
    ));
    start.cards.push(Card {
        icon: Icon::Lock,
        ember: false,
        title: "Some folders need Full Disk Access".into(),
        body: "Mail, Messages and Safari keep their folders private, so 286 folders were skipped. Grant access in System Settings to count them too."
            .into(),
        button: Some("Open Settings".into()),
    });
    out.push(("disk-start", start, Some(Hit::ScanDrive(0))));

    let mut scanning = view(Screen::Results(results(Tab::Folders, map, true)));
    scanning.motion.veil = 1.0;
    out.push(("disk-scanning", scanning, None));

    let mut res = view(Screen::Results(results(Tab::Folders, map, false)));
    res.motion.focus = Some(Focus::Row(1));
    res.motion.dim = 1.0;
    out.push(("disk-results", res, Some(Hit::Row(1))));

    let mut files = view(Screen::Results(results(Tab::Files, map, false)));
    files.kb = true;
    files.cursor = Some(2);
    files.motion.suffix = 1.0;
    files.motion.suffix_alpha = 1.0;
    files.motion.suffix_text = " (7.3 GB)".into();
    out.push(("disk-files", files, Some(Hit::File(3))));

    out.push((
        "disk-types",
        view(Screen::Results(results(Tab::Types, map, false))),
        None,
    ));
    out.push((
        "disk-clean",
        view(Screen::Clean(clean())),
        Some(Hit::Clean(0)),
    ));
    out.push(("disk-review", view(Screen::Review(review())), None));
    out.push(("disk-dupes-offer", view(Screen::Dupes(dupes(false))), None));
    out.push((
        "disk-dupes",
        view(Screen::Dupes(dupes(true))),
        Some(Hit::DupeRow(0, 2)),
    ));
    let mut sheet = view(Screen::Clean(clean()));
    sheet.confirm = Some(confirm());
    out.push(("disk-confirm", sheet, Some(Hit::Cancel)));
    out
}
