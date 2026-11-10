#![allow(clippy::too_many_arguments)] // drawing helpers take a canvas plus a full set of geometry
//! The Notices panel, "Earlier today" (frame `Y_alKzOsBY`, column 3): the header with Clear all,
//! Waiting and Earlier rows, the row menu (mark as seen, mute), muted sources with Unmute, and
//! the mute confirmation with Undo. Drawing and hit tests.

use crate::canvas::{ellipse, pt, rrect, text_width, Canvas, Font, Stroke};
use crate::notices_model::Source;
use crate::ui::{lip, num_text, paint_icon, with_alpha, Fonts, Icon, Theme};

pub const W: f32 = 300.0;
const PAD: f32 = 12.0;
const ROW_H: f32 = 40.0;
const LABEL_H: f32 = 26.0;
/// Rows shown at most; older ones are counted in one line.
pub const MAX_ROWS: usize = 8;
const MENU_W: f32 = 236.0;
const ITEM_H: f32 = 32.0;

#[derive(Clone, Debug, PartialEq)]
pub struct Row {
    pub id: u64,
    pub icon: Icon,
    pub title: String,
    pub value: String,
    pub time: String,
    pub unseen: bool,
    pub source: Source,
}

#[derive(Clone, Debug, PartialEq)]
pub struct View {
    /// Waiting rows first (unseen), then Earlier, each newest first.
    pub rows: Vec<Row>,
    pub n_waiting: usize,
    /// Sources muted now, with their words ("for good").
    pub muted: Vec<(Source, String)>,
    pub undo_clear: bool,
    pub focus: Option<usize>,
    /// The row whose menu is open, and the menu's keyboard item.
    pub menu: Option<usize>,
    pub menu_item: usize,
    /// "Downloads muted" "for good", with Undo.
    pub note: Option<(String, String)>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum MenuItem {
    Seen,
    Hour,
    Tomorrow,
    Always,
}

pub const MENU: [MenuItem; 4] = [
    MenuItem::Seen,
    MenuItem::Hour,
    MenuItem::Tomorrow,
    MenuItem::Always,
];

#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Hit {
    ClearAll,
    UndoClear,
    Row(usize),
    /// The row's "···" button.
    More(usize),
    Menu(MenuItem),
    Unmute(Source),
    UndoMute,
}

type Rect = (f32, f32, f32, f32);

fn inside(r: Rect, x: f32, y: f32) -> bool {
    x >= r.0 && x <= r.2 && y >= r.1 && y <= r.3
}

/// Where everything sits, top to bottom.
struct Layout {
    rows: Vec<(usize, Rect)>,
    waiting_label: Option<f32>,
    earlier_label: Option<f32>,
    older: Option<(usize, f32)>,
    empty: Option<f32>,
    muted: Vec<(Source, Rect)>,
    note: Option<Rect>,
    menu: Option<Rect>,
    h: f32,
}

fn layout(v: &View) -> Layout {
    let mut y = 52.0;
    let mut rows = Vec::new();
    let (mut waiting_label, mut earlier_label, mut older, mut empty) = (None, None, None, None);
    if v.rows.is_empty() {
        empty = Some(y);
        y += 76.0;
    }
    for (i, _) in v.rows.iter().enumerate().take(MAX_ROWS) {
        if i == 0 && v.n_waiting > 0 {
            waiting_label = Some(y);
            y += LABEL_H;
        }
        if i == v.n_waiting {
            earlier_label = Some(y);
            y += LABEL_H;
        }
        rows.push((i, (PAD, y, W - PAD, y + ROW_H)));
        y += ROW_H;
    }
    if v.rows.len() > MAX_ROWS {
        older = Some((v.rows.len() - MAX_ROWS, y));
        y += 28.0;
    }
    let mut muted = Vec::new();
    let mut note = None;
    let noted = v.note.as_ref().map(|(t, _)| t.clone());
    let shown: Vec<&(Source, String)> = v
        .muted
        .iter()
        .filter(|(s, _)| noted.as_deref() != Some(format!("{} muted", s.name()).as_str()))
        .collect();
    if v.note.is_some() || !shown.is_empty() {
        y += 9.0;
    }
    if v.note.is_some() {
        note = Some((PAD, y, W - PAD, y + ROW_H));
        y += ROW_H;
    }
    for (s, _) in shown {
        muted.push((*s, (PAD, y, W - PAD, y + ROW_H)));
        y += ROW_H;
    }
    let mut h = y + PAD;
    let menu = v
        .menu
        .and_then(|i| rows.iter().find(|(j, _)| *j == i))
        .map(|(_, r)| {
            let mh = 6.0 + ITEM_H + 9.0 + 24.0 + ITEM_H * 3.0 + 6.0;
            let below = r.3 + 4.0;
            // Above the row when it would run past the panel's foot; the panel grows otherwise.
            let top = if below + mh > h && r.1 - 4.0 - mh >= 44.0 {
                r.1 - 4.0 - mh
            } else {
                below
            };
            (W - PAD - MENU_W, top, W - PAD, top + mh)
        });
    if let Some(m) = menu {
        h = h.max(m.3 + PAD);
    }
    Layout {
        rows,
        waiting_label,
        earlier_label,
        older,
        empty,
        muted,
        note,
        menu,
        h,
    }
}

pub fn height(v: &View) -> f32 {
    layout(v).h
}

fn header_button(f: &Fonts, v: &View) -> (Hit, &'static str, Rect) {
    let (hit, label) = if v.undo_clear {
        (Hit::UndoClear, "Undo")
    } else {
        (Hit::ClearAll, "Clear all")
    };
    let w = text_width(label, &f.title) + 24.0;
    (hit, label, (W - PAD - w, 14.0, W - PAD, 42.0))
}

fn small_button(f: &Fonts, label: &str, r: Rect) -> Rect {
    let w = text_width(label, &f.title) + 24.0;
    (r.2 - 6.0 - w, r.1 + 6.0, r.2 - 6.0, r.3 - 6.0)
}

fn more_rect(r: Rect) -> Rect {
    (r.2 - 34.0, r.1 + 6.0, r.2 - 6.0, r.3 - 6.0)
}

/// The menu's items: (item, rect). The heading sits between Seen and the mutes.
fn menu_items(m: Rect) -> Vec<(MenuItem, Rect)> {
    let x = (m.0 + 4.0, m.2 - 4.0);
    let mut y = m.1 + 6.0;
    let mut out = vec![(MenuItem::Seen, (x.0, y, x.1, y + ITEM_H))];
    y += ITEM_H + 9.0 + 24.0;
    for it in [MenuItem::Hour, MenuItem::Tomorrow, MenuItem::Always] {
        out.push((it, (x.0, y, x.1, y + ITEM_H)));
        y += ITEM_H;
    }
    out
}

pub fn hit(f: &Fonts, v: &View, x: f32, y: f32) -> Option<Hit> {
    let l = layout(v);
    if let Some(m) = l.menu {
        if inside(m, x, y) {
            return menu_items(m)
                .into_iter()
                .find(|(_, r)| inside(*r, x, y))
                .map(|(it, _)| Hit::Menu(it));
        }
    }
    let (h, _, r) = header_button(f, v);
    if (!v.rows.is_empty() || v.undo_clear) && inside(r, x, y) {
        return Some(h);
    }
    for (i, r) in &l.rows {
        if inside(*r, x, y) {
            return Some(if inside(more_rect(*r), x, y) {
                Hit::More(*i)
            } else {
                Hit::Row(*i)
            });
        }
    }
    if let Some(r) = l.note {
        if inside(small_button(f, "Undo", r), x, y) {
            return Some(Hit::UndoMute);
        }
    }
    for (s, r) in &l.muted {
        if inside(small_button(f, "Unmute", *r), x, y) {
            return Some(Hit::Unmute(*s));
        }
    }
    None
}

fn button(cv: &mut dyn Canvas, t: &Theme, f: &Fonts, label: &str, r: Rect, hover: bool) {
    cv.fill_rrect(
        &rrect(r.0, r.1, r.2, r.3, (r.3 - r.1) / 2.0),
        if hover { t.button_hover } else { t.button },
    );
    lip(cv, t, r.0, r.1, r.2, r.3, (r.3 - r.1) / 2.0, 1.0);
    cv.center_text(label, &f.title, (r.0 + r.2) / 2.0, r.1, r.3 - r.1, t.text);
}

fn chip(cv: &mut dyn Canvas, t: &Theme, icon: Icon, x: f32, cy: f32) {
    cv.fill_rrect(&rrect(x, cy - 12.0, x + 24.0, cy + 12.0, 7.0), t.chip);
    let ink = if icon.is_ember() { t.ember } else { t.text };
    paint_icon(cv, icon, x + 12.0, cy, 14.0, ink);
}

/// The noun in ink, the value muted, on one line; an ellipsis when it runs long.
fn two_tone(
    cv: &mut dyn Canvas,
    t: &Theme,
    f: &Fonts,
    title: &str,
    value: &str,
    x: f32,
    y: f32,
    w: f32,
    h: f32,
) {
    let title = ellipsize(title, &f.title, w);
    let tw = text_width(&title, &f.title);
    cv.text(&title, &f.title, x, y, tw + 2.0, h, t.text);
    let room = w - tw - 4.0;
    if !value.is_empty() && room > 24.0 {
        let value = ellipsize(value, &f.body, room);
        cv.text(&value, &f.body, x + tw + 4.0, y, room + 2.0, h, t.muted);
    }
}

fn ellipsize(s: &str, fmt: &Font, max: f32) -> String {
    if text_width(s, fmt) <= max {
        return s.to_string();
    }
    let chars: Vec<char> = s.chars().collect();
    (1..chars.len())
        .rev()
        .map(|n| chars[..n].iter().collect::<String>().trim_end().to_string() + "…")
        .find(|c| text_width(c, fmt) <= max)
        .unwrap_or_default()
}

pub fn draw(cv: &mut dyn Canvas, t: &Theme, f: &Fonts, v: &View, hover: Option<Hit>) {
    let l = layout(v);
    let ink = t.text;
    let muted = t.muted;
    let hair = t.hair;
    cv.fill_rrect(&rrect(0.0, 0.0, W, l.h, 20.0), t.panel);
    cv.stroke_rrect(
        &rrect(0.5, 0.5, W - 0.5, l.h - 0.5, 19.5),
        hair,
        1.0,
        Stroke::Plain,
    );
    // Header: "Notices" with a muted "Today", and Clear all (Undo for 6 s after it).
    cv.text("Notices", &f.panel_title, 20.0, 18.0, 80.0, 20.0, ink);
    let nw = text_width("Notices", &f.panel_title);
    cv.text("Today", &f.small, 20.0 + nw + 8.0, 19.0, 60.0, 18.0, muted);
    if !v.rows.is_empty() || v.undo_clear {
        let (h, label, r) = header_button(f, v);
        button(cv, t, f, label, r, hover == Some(h));
    }

    if let Some(y) = l.empty {
        cv.fill_rrect(&rrect(PAD, y, W - PAD, y + 68.0, 12.0), t.raised);
        cv.fill_ellipse(&ellipse(PAD + 26.0, y + 34.0, 16.0), t.chip);
        paint_icon(cv, Icon::Inbox, PAD + 26.0, y + 34.0, 16.0, ink);
        cv.text(
            "Nothing today",
            &f.title,
            PAD + 52.0,
            y + 10.0,
            W - PAD * 2.0 - 60.0,
            18.0,
            ink,
        );
        cv.text(
            "Notices you missed or dismissed show here until midnight.",
            &f.small_wrap,
            PAD + 52.0,
            y + 29.0,
            W - PAD * 2.0 - 62.0,
            34.0,
            muted,
        );
    }
    if let Some(y) = l.waiting_label {
        cv.text(
            &format!("Waiting · {}", v.n_waiting),
            &f.small,
            20.0,
            y + 6.0,
            160.0,
            16.0,
            muted,
        );
    }
    if let Some(y) = l.earlier_label {
        cv.text("Earlier", &f.small, 20.0, y + 6.0, 160.0, 16.0, muted);
    }
    for (i, r) in &l.rows {
        let row = &v.rows[*i];
        let hov =
            matches!(hover, Some(Hit::Row(j) | Hit::More(j)) if j == *i) || v.menu == Some(*i);
        let focused = v.focus == Some(*i);
        let cy = (r.1 + r.3) / 2.0;
        if hov || focused {
            cv.fill_rrect(&rrect(r.0, r.1, r.2, r.3, 8.0), t.raised);
        }
        if focused {
            cv.stroke_rrect(
                &rrect(r.0 + 1.0, r.1 + 1.0, r.2 - 1.0, r.3 - 1.0, 7.0),
                ink,
                2.0,
                Stroke::Plain,
            );
        }
        if row.unseen {
            cv.fill_ellipse(&ellipse(r.0 + 8.0, cy, 3.0), ink);
        }
        chip(cv, t, row.icon, r.0 + 18.0, cy);
        let right = if hov || focused {
            r.2 - 40.0
        } else {
            r.2 - 8.0 - text_width(&row.time, &f.small) - 10.0
        };
        two_tone(
            cv,
            t,
            f,
            &row.title,
            &row.value,
            r.0 + 50.0,
            r.1,
            right - r.0 - 50.0,
            ROW_H,
        );
        if hov || focused {
            let m = more_rect(*r);
            let (cx, my) = ((m.0 + m.2) / 2.0, (m.1 + m.3) / 2.0);
            let fill = if hover == Some(Hit::More(*i)) || v.menu == Some(*i) {
                t.button_hover
            } else {
                t.button
            };
            cv.fill_ellipse(&ellipse(cx, my, 14.0), fill);
            for dx in [-5.0, 0.0, 5.0] {
                cv.fill_ellipse(&ellipse(cx + dx, my, 1.4), muted);
            }
        } else {
            num_text(cv, &row.time, &f.small, r.2 - 8.0, r.1, ROW_H, muted);
        }
    }
    if let Some((n, y)) = l.older {
        cv.text(
            &format!("and {n} earlier"),
            &f.small,
            20.0,
            y + 6.0,
            200.0,
            16.0,
            muted,
        );
    }
    if l.note.is_some() || !l.muted.is_empty() {
        let y = l
            .note
            .map(|r| r.1)
            .or(l.muted.first().map(|(_, r)| r.1))
            .unwrap_or(0.0)
            - 5.0;
        cv.line(pt(20.0, y), pt(W - 20.0, y), hair, 1.0, Stroke::Plain);
    }
    if let (Some(r), Some((title, words))) = (l.note, &v.note) {
        let b = small_button(f, "Undo", r);
        two_tone(
            cv,
            t,
            f,
            title,
            words,
            r.0 + 8.0,
            r.1,
            b.0 - r.0 - 16.0,
            ROW_H,
        );
        button(cv, t, f, "Undo", b, hover == Some(Hit::UndoMute));
    }
    for (s, r) in &l.muted {
        let words = v
            .muted
            .iter()
            .find(|(m, _)| m == s)
            .map(|(_, w)| w.as_str())
            .unwrap_or("");
        let b = small_button(f, "Unmute", *r);
        two_tone(
            cv,
            t,
            f,
            s.name(),
            &format!("muted {words}"),
            r.0 + 8.0,
            r.1,
            b.0 - r.0 - 16.0,
            ROW_H,
        );
        button(cv, t, f, "Unmute", b, hover == Some(Hit::Unmute(*s)));
    }

    // The row menu: Mark as seen, then the source's mutes under its name.
    if let (Some(m), Some(i)) = (l.menu, v.menu) {
        let source = v.rows[i].source;
        cv.fill_rrect(&rrect(m.0, m.1, m.2, m.3, 12.0), t.surface);
        cv.stroke_rrect(
            &rrect(m.0 + 0.5, m.1 + 0.5, m.2 - 0.5, m.3 - 0.5, 11.5),
            hair,
            1.0,
            Stroke::Plain,
        );
        let items = menu_items(m);
        for (k, (it, r)) in items.iter().enumerate() {
            let on = hover == Some(Hit::Menu(*it)) || v.menu_item == k;
            if on {
                cv.fill_rrect(
                    &rrect(r.0, r.1, r.2, r.3, 8.0),
                    if v.menu_item == k && hover != Some(Hit::Menu(*it)) {
                        t.raised
                    } else {
                        t.button
                    },
                );
            }
            let (icon, label) = match it {
                MenuItem::Seen => (Icon::Tick, "Mark as seen".to_string()),
                MenuItem::Hour => (Icon::Clock, "Mute for 1 hour".to_string()),
                MenuItem::Tomorrow => (Icon::Clock, "Mute until tomorrow".to_string()),
                MenuItem::Always => (Icon::Close, format!("Don't show {} again", source.noun())),
            };
            paint_icon(cv, icon, r.0 + 16.0, (r.1 + r.3) / 2.0, 14.0, muted);
            cv.text(
                &label,
                &f.body,
                r.0 + 32.0,
                r.1,
                r.2 - r.0 - 36.0,
                ITEM_H,
                ink,
            );
            if *it == MenuItem::Seen {
                let y = r.3 + 4.5;
                cv.line(
                    pt(m.0 + 10.0, y),
                    pt(m.2 - 10.0, y),
                    with_alpha(t.hair, 1.0),
                    1.0,
                    Stroke::Plain,
                );
                cv.text(
                    source.name(),
                    &f.small,
                    m.0 + 12.0,
                    y + 6.0,
                    MENU_W - 24.0,
                    16.0,
                    muted,
                );
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn row(id: u64, unseen: bool) -> Row {
        Row {
            id,
            icon: Icon::Bell,
            title: "Kirim invoice".into(),
            value: "due 14:00".into(),
            time: "14:00".into(),
            unseen,
            source: Source::Tasks,
        }
    }

    fn view(waiting: usize, earlier: usize) -> View {
        let rows = (0..waiting)
            .map(|i| row(i as u64, true))
            .chain((0..earlier).map(|i| row(100 + i as u64, false)))
            .collect();
        View {
            rows,
            n_waiting: waiting,
            muted: Vec::new(),
            undo_clear: false,
            focus: None,
            menu: None,
            menu_item: 0,
            note: None,
        }
    }

    #[test]
    fn groups_rows_and_the_older_line() {
        let l = layout(&view(2, 3));
        assert_eq!(l.rows.len(), 5);
        assert!(
            l.waiting_label.unwrap() < l.rows[0].1 .1 && l.earlier_label.unwrap() > l.rows[1].1 .1
        );
        let l = layout(&view(0, 12));
        assert!(l.waiting_label.is_none());
        assert_eq!(l.rows.len(), MAX_ROWS);
        assert_eq!(l.older.map(|(n, _)| n), Some(4));
        assert!(layout(&view(0, 0)).empty.is_some());
    }

    #[test]
    fn the_menu_fits_inside_the_panel() {
        let mut v = view(1, 0);
        v.menu = Some(0);
        let l = layout(&v);
        let m = l.menu.unwrap();
        assert!(m.3 + PAD <= l.h && m.1 >= l.rows[0].1 .3);
        v.muted = vec![(Source::Downloads, "for good".into())];
        v.note = Some(("Downloads muted".into(), "for good".into()));
        assert!(
            layout(&v).muted.is_empty(),
            "the note stands in for its own muted row"
        );
    }
}
