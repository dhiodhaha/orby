//! Snapshot scenes for the Tasks panel and the Tasks window: fixed views (no clock, no store)
//! that cover the empty list, rows of every kind, hover states, the quick add field with chips,
//! its caret and a selection, the inline edit, the due menu, the Undo bar, and the window at its
//! least size, wide (the 720 px column centred) and maximized.

use app_quickadd::TokenKind;
use app_store::{FolderId, ListId, SpaceId};

use crate::snapshot::Scene;
use crate::tasks_model::{SideIcon, SideItem, SideKey, Tab};
use crate::tasks_panel::{self as tp, FieldView, RowView};
use crate::tasks_window_panel::{self as twp, FieldV, ItemV, MenuV, RowV};
use crate::ui::Fonts;

const GRAMMAR: &str = "# list · / folder · @ person";

// ---- the Tasks panel ----

fn side_item(
    key: SideKey,
    depth: u32,
    icon: SideIcon,
    name: &str,
    count: usize,
    chevron: Option<bool>,
    note: &str,
) -> SideItem {
    SideItem {
        key,
        depth,
        icon,
        name: name.into(),
        count,
        chevron,
        note: note.into(),
        selected: false,
    }
}

fn list(id: &str) -> SideKey {
    SideKey::List(ListId(id.into()))
}

/// Inbox, Today, Upcoming, a private space with a folder and lists, a shared space.
fn sidebar(selected: usize) -> Vec<SideItem> {
    let mut side = vec![
        side_item(SideKey::Inbox, 0, SideIcon::Inbox, "Inbox", 2, None, ""),
        side_item(SideKey::Today, 0, SideIcon::Today, "Today", 5, None, ""),
        side_item(
            SideKey::Upcoming,
            0,
            SideIcon::Upcoming,
            "Upcoming",
            3,
            None,
            "",
        ),
        side_item(
            SideKey::Space(SpaceId("me".into())),
            0,
            SideIcon::Private,
            "Personal",
            0,
            Some(true),
            "private",
        ),
        side_item(
            SideKey::Folder(FolderId("home".into())),
            1,
            SideIcon::Folder,
            "Home",
            4,
            Some(true),
            "",
        ),
        side_item(
            list("groceries"),
            2,
            SideIcon::List,
            "Groceries",
            3,
            None,
            "",
        ),
        side_item(
            list("repairs"),
            2,
            SideIcon::List,
            "Repairs and the long list of things to fix",
            1,
            None,
            "",
        ),
        side_item(list("reading"), 1, SideIcon::List, "Reading", 0, None, ""),
        side_item(
            SideKey::Space(SpaceId("family".into())),
            0,
            SideIcon::Shared,
            "Family",
            0,
            Some(false),
            "",
        ),
    ];
    if let Some(s) = side.get_mut(selected) {
        s.selected = true;
    }
    side
}

/// A sidebar taller than its band: the tree scrolls.
fn tall_sidebar(selected: usize) -> Vec<SideItem> {
    let mut side = sidebar(usize::MAX);
    for i in 0..10 {
        side.push(side_item(
            list(&format!("l{i}")),
            1,
            SideIcon::List,
            &format!("Project {}", i + 1),
            i % 3,
            None,
            "",
        ));
    }
    if let Some(s) = side.get_mut(selected) {
        s.selected = true;
    }
    side
}

fn field(text: &str, caret: usize, focused: bool) -> FieldView {
    FieldView {
        text: text.into(),
        caret,
        selection: caret..caret,
        tokens: Vec::new(),
        placeholder: "Add a task, e.g. beli galon besok 9am #list".into(),
        focused,
        action: "Add",
    }
}

fn task(title: &str, sub: &str, due: &str) -> RowView {
    RowView::Task {
        title: title.into(),
        sub: sub.into(),
        due: due.into(),
        overdue: false,
        repeat: false,
        assignee: None,
        done: false,
    }
}

fn prayer(name: &str, time: &str) -> RowView {
    RowView::Prayer {
        name: name.into(),
        time: time.into(),
    }
}

/// Today: an overdue task, prayer markers between tasks, a repeat, an assignee, a long title.
fn today_rows() -> Vec<RowView> {
    let mut overdue = task("Bayar listrik", "Home", "Overdue · Mon");
    if let RowView::Task { overdue: o, .. } = &mut overdue {
        *o = true;
    }
    let mut repeat = task("Water the plants", "", "17:00");
    if let RowView::Task { repeat: r, .. } = &mut repeat {
        *r = true;
    }
    let mut shared = task("Pick up the kids", "Family · Shared", "15:30");
    if let RowView::Task { assignee, .. } = &mut shared {
        *assignee = Some("R".into());
    }
    vec![
        overdue,
        prayer("Dzuhur", "11:41"),
        task("Standup notes", "", "13:00"),
        prayer("Ashar", "14:47"),
        shared,
        task(
            "Beli buku tulis, pensil, penghapus, penggaris dan map plastik untuk sekolah minggu depan",
            "Groceries · a second line that is also long enough to be cut with an ellipsis at the end",
            "Today",
        ),
        repeat,
        prayer("Maghrib", "17:52"),
        task("Call ibu", "", "19:00"),
        prayer("Isya", "19:03"),
    ]
}

fn done_rows() -> Vec<RowView> {
    let done = |title: &str, due: &str, by: Option<&str>| RowView::Task {
        title: title.into(),
        sub: String::new(),
        due: due.into(),
        overdue: false,
        repeat: false,
        assignee: by.map(Into::into),
        done: true,
    };
    vec![
        done("Beli galon", "Done 09:12", None),
        done("Send the invoice", "Done 10:40 by Rani", Some("R")),
        done(
            "Return the library books before they are overdue and the fine grows",
            "Done yesterday",
            None,
        ),
    ]
}

fn base_view() -> tp::View {
    tp::View {
        side: sidebar(1),
        crumb: "Smart list".into(),
        title: "Today".into(),
        field: field("", 0, false),
        understood: format!(
            "Type a task, dates in Indonesian or English (besok 9am, jumat).  {GRAMMAR}"
        ),
        tab: Tab::Todo,
        todo_n: 7,
        done_n: 3,
        rows: today_rows(),
        scroll: 0.0,
        side_scroll: 0.0,
        empty: None,
        footer: "3 done today · last: Rani finished Send the invoice at 10:40".into(),
        sync: "On this PC only · sync is off".into(),
    }
}

/// The panel's scenes: name, view and hover.
pub fn panel_views() -> Vec<(&'static str, tp::View, Option<tp::Hit>)> {
    use tp::Hit;
    let mut out = Vec::new();

    let mut empty = base_view();
    empty.side = sidebar(0);
    empty.crumb = "Personal".into();
    empty.title = "Inbox".into();
    empty.rows = Vec::new();
    empty.todo_n = 0;
    empty.done_n = 0;
    empty.empty = Some((
        "Inbox is empty".into(),
        "A task with no #list lands here.".into(),
    ));
    empty.footer = String::new();
    empty.sync = "Couldn't open app.db, changes aren't saved".into();
    out.push(("tasks-empty", empty, None));

    out.push(("tasks-today", base_view(), None));
    out.push(("tasks-hover-row", base_view(), Some(Hit::Row(2))));
    out.push(("tasks-hover-delete", base_view(), Some(Hit::Delete(5))));
    out.push(("tasks-hover-check", base_view(), Some(Hit::Check(4))));
    out.push(("tasks-hover-share", base_view(), Some(Hit::Share)));
    out.push(("tasks-hover-side", base_view(), Some(Hit::Side(6))));
    out.push(("tasks-hover-footer", base_view(), Some(Hit::Footer)));

    // The quick add field with chips, focused, the caret at the end; Add hovered.
    let text = "beli galon besok 9am #belanja";
    let mut typing = base_view();
    typing.field = FieldView {
        tokens: vec![
            (11..16, TokenKind::Date),
            (17..20, TokenKind::Time),
            (21..29, TokenKind::List),
        ],
        ..field(text, text.len(), true)
    };
    typing.understood = format!("Beli galon · tomorrow 09:00 · in Belanja (new list)  {GRAMMAR}");
    out.push(("tasks-field-chips", typing.clone(), Some(Hit::Add)));

    // A selection across a chip, the caret inside the plain run.
    let mut sel = typing.clone();
    sel.field.caret = 5;
    sel.field.selection = 5..16;
    out.push(("tasks-field-selection", sel, None));

    // Text longer than the field: it scrolls so the caret stays in view.
    let long = "Telepon tukang ledeng soal pipa bocor di kamar mandi atas besok pagi @rani #home";
    let mut scrolled = base_view();
    scrolled.field = FieldView {
        tokens: vec![
            (58..63, TokenKind::Date),
            (69..74, TokenKind::Person),
            (75..80, TokenKind::List),
        ],
        ..field(long, long.len(), true)
    };
    out.push(("tasks-field-long", scrolled, None));

    // A new list's name, "Create"; the tree scrolled to show a list below the fold; Done tab.
    let mut new_list = base_view();
    new_list.side = tall_sidebar(17);
    new_list.side_scroll = tp::reveal_selected(&new_list.side, 0.0);
    new_list.crumb = "Personal".into();
    new_list.title = "Project 8".into();
    new_list.field = FieldView {
        placeholder: "New list name; Home/Groceries puts it in a folder".into(),
        action: "Create",
        ..field("Home/Ga", 7, true)
    };
    new_list.understood = "Enter creates the list in Personal. Esc cancels.".into();
    new_list.tab = Tab::Done;
    new_list.rows = done_rows();
    new_list.todo_n = 0;
    out.push(("tasks-done-tree", new_list, Some(Hit::Tab(Tab::Todo))));

    // The list scrolled part way, the sidebar's New list hovered.
    let mut rows_scrolled = base_view();
    rows_scrolled.scroll = 57.0;
    out.push(("tasks-scrolled", rows_scrolled, Some(Hit::NewList)));
    out
}

// ---- the Tasks window ----

fn win_field(text: &str, caret: usize, focused: bool) -> FieldV {
    FieldV {
        text: text.into(),
        caret,
        selection: caret..caret,
        focused,
    }
}

fn row(title: &str, list: &str, due: &str) -> ItemV {
    ItemV::Task(RowV {
        title: title.into(),
        list: list.into(),
        due: due.into(),
        overdue: false,
        repeat: false,
        done: false,
    })
}

fn header(name: &'static str, count: usize, key: &'static str, fold: Option<bool>) -> ItemV {
    ItemV::Header {
        name,
        count,
        key,
        fold,
    }
}

/// Today (with an overdue row), Upcoming, No date and an open Done group.
fn groups() -> Vec<ItemV> {
    let mut overdue = row("Bayar listrik", "Home", "Overdue · Mon");
    if let ItemV::Task(r) = &mut overdue {
        r.overdue = true;
    }
    let mut repeat = row("Water the plants", "", "Today 17:00");
    if let ItemV::Task(r) = &mut repeat {
        r.repeat = true;
    }
    let mut done = row("Beli galon", "Groceries", "Done 09:12");
    if let ItemV::Task(r) = &mut done {
        r.done = true;
    }
    vec![
        header("Today", 4, "tasks.win.today", None),
        overdue,
        row("Standup notes", "Work", "Today 13:00"),
        repeat,
        row(
            "Beli buku tulis, pensil, penghapus, penggaris dan map plastik untuk sekolah",
            "A list with a long name too",
            "Today",
        ),
        header("Upcoming", 3, "tasks.win.upcoming", None),
        row("Dentist", "", "Thu 10:00"),
        row("Send the quarterly report to the accountant", "Work", "Fri"),
        row("Renew passport", "Errands", "12 Dec"),
        header("No date", 2, "tasks.win.nodate", None),
        row("Read Laskar Pelangi", "Reading", "No date"),
        row("Fix the gate hinge", "Repairs", "No date"),
        header("Done", 1, "tasks.win.done", Some(true)),
        done,
    ]
}

fn win_view(w: f32, h: f32) -> twp::View {
    twp::View {
        w,
        h,
        max: false,
        open_n: 9,
        field: win_field("", 0, false),
        line: "Enter adds it. Dates work in Indonesian or English, and # picks a list.".into(),
        items: groups(),
        scroll: 0.0,
        ring: None,
        editing: None,
        menu: None,
        undo: None,
        empty: None,
    }
}

fn menu(item: usize, sel: usize) -> MenuV {
    MenuV {
        item,
        rows: vec![
            ("Today".into(), String::new()),
            ("Tomorrow".into(), "Wed".into()),
            ("Next week".into(), "Mon 16 Nov".into()),
            ("No date".into(), String::new()),
        ],
        sel,
    }
}

/// The window's scenes: name, view and hover. The view's `w`/`h` is the scene's size.
pub fn window_views() -> Vec<(&'static str, twp::View, Option<twp::Hit>)> {
    use twp::Hit;
    let mut out = Vec::new();

    let mut empty = win_view(twp::W, twp::H);
    empty.open_n = 0;
    empty.items = Vec::new();
    empty.field = win_field("", 0, true);
    empty.empty = Some((
        "No tasks yet".into(),
        "Add one above. Dates work too, like besok 9am.".into(),
    ));
    out.push(("tasks-win-empty", empty, None));

    out.push(("tasks-win", win_view(twp::W, twp::H), Some(Hit::Title(2))));

    // The least size: the list scrolls (a thumb shows), the Done group folded and hovered.
    let mut min = win_view(twp::W, twp::MIN_H);
    min.items.pop();
    if let Some(ItemV::Header { fold, .. }) = min.items.last_mut() {
        *fold = Some(false);
    }
    min.scroll = 200.0;
    let last = min.items.len() - 1;
    out.push(("tasks-win-min", min, Some(Hit::Header(last))));

    // Wide: the 720 px column centred; the keyboard's ring on a row, its delete hovered.
    let mut wide = win_view(1200.0, twp::H);
    wide.ring = Some(7);
    out.push(("tasks-win-wide", wide, Some(Hit::Delete(6))));

    // Maximized: square corners, Restore; close hovered; the ring on a header.
    let mut max = win_view(1280.0, 760.0);
    max.max = true;
    max.ring = Some(5);
    out.push(("tasks-win-max", max, Some(Hit::Close)));

    // The add field with text and a selection; Add hovered.
    let mut typed = win_view(twp::W, twp::H);
    let text = "beli galon besok 9am #belanja";
    typed.field = FieldV {
        selection: 11..20,
        ..win_field(text, 20, true)
    };
    typed.line = "Beli galon · tomorrow 09:00 · in Belanja (new list)".into();
    out.push(("tasks-win-field", typed, Some(Hit::Add)));

    // An inline title edit with a selection.
    let mut edit = win_view(twp::W, twp::H);
    let title = "Send the quarterly report to the accountant";
    edit.editing = Some((
        7,
        FieldV {
            selection: 9..18,
            ..win_field(title, 18, true)
        },
    ));
    out.push(("tasks-win-edit", edit, None));

    // The due menu below its row, the second pick hovered; the due button hovered elsewhere.
    let mut menu_below = win_view(twp::W, twp::H);
    menu_below.menu = Some(menu(2, 0));
    out.push(("tasks-win-menu", menu_below, Some(Hit::Menu(1))));

    // The due menu above a row near the bottom, the keyboard's pick lit; the Undo bar shows.
    let mut menu_above = win_view(twp::W, twp::H);
    menu_above.menu = Some(menu(10, 2));
    menu_above.undo =
        Some("Beli buku tulis, pensil, penghapus, penggaris dan map plastik untuk sekolah".into());
    out.push(("tasks-win-menu-above", menu_above, None));

    // The Undo bar, Undo hovered; the due button of a row hovered.
    let mut undo = win_view(twp::W, twp::H);
    undo.undo = Some("Renew passport".into());
    out.push(("tasks-win-undo", undo.clone(), Some(Hit::Undo)));
    out.push(("tasks-win-hover-due", undo, Some(Hit::Due(1))));
    out
}

/// Every Tasks scene, both the panel and the window.
pub fn scenes(_f: &Fonts) -> Vec<Scene> {
    let mut out = Vec::new();
    for (name, v, hover) in panel_views() {
        out.push(Scene::new(name, tp::W, tp::H, move |cv, t, f| {
            tp::draw(cv, t, f, &v, hover)
        }));
    }
    for (name, v, hover) in window_views() {
        let (w, h) = (v.w, v.h);
        out.push(Scene::new(name, w, h, move |cv, t, f| {
            twp::draw(cv, t, f, &v, hover)
        }));
    }
    out
}
