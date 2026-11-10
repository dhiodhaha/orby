#![allow(clippy::too_many_arguments)] // drawing helpers take a canvas plus a full set of geometry
//! The Tasks window: the title band, the add field and the line under it, every task grouped
//! Today / Upcoming / No date / Done (folded), rows with a checkbox, an inline title edit, the due
//! menu, and the Undo bar after a delete. Drawing and hit tests on a plain [`View`] that
//! tasks_window.rs builds.

use std::ops::Range;

use crate::canvas::{
    clipped_text, ellipse, line_to, path, pt, rrect, text_width, Canvas, Font, Stroke,
};
use crate::motion::{self, Roll};
use crate::ui::{lip, num_text, paint_icon, paint_title_button, with_alpha, Fonts, Icon, Theme};

/// The window's logical width when it opens, and the least it can be resized to.
pub const W: f32 = 600.0;
/// Its height when the screen has room.
pub const H: f32 = 680.0;
/// The least height it shrinks to on a short screen.
pub const MIN_H: f32 = 420.0;
/// The title band.
pub const TOP: f32 = 44.0;
const X0: f32 = 20.0;
const FIELD_T: f32 = TOP + 16.0;
const FIELD_B: f32 = FIELD_T + 40.0;
const TEXT_X: f32 = X0 + 38.0;
const LIST_T: f32 = FIELD_B + 34.0;
const ROW: f32 = 40.0;
const HEAD: f32 = 34.0;
const UNDO_H: f32 = 48.0;
const MENU_W: f32 = 220.0;
const MENU_ROW: f32 = 32.0;
const PLACEHOLDER: &str = "Add a task, e.g. beli galon besok 9am #list";

type Rect = (f32, f32, f32, f32);

fn inside(r: &Rect, x: f32, y: f32) -> bool {
    x >= r.0 && x <= r.2 && y >= r.1 && y <= r.3
}

#[derive(Clone, Debug, PartialEq)]
pub struct FieldV {
    pub text: String,
    pub caret: usize,
    pub selection: Range<usize>,
    /// The window is active and the caret is here.
    pub focused: bool,
}

#[derive(Clone, Debug, PartialEq)]
pub struct RowV {
    pub title: String,
    /// The list's name, muted after the title (empty for the Inbox).
    pub list: String,
    pub due: String,
    /// "Overdue · Thu" in ink 500, never red.
    pub overdue: bool,
    pub repeat: bool,
    pub done: bool,
}

#[derive(Clone, Debug, PartialEq)]
pub enum ItemV {
    Header {
        name: &'static str,
        count: usize,
        /// The rolling number's key.
        key: &'static str,
        /// Some(open) for the group that folds (Done).
        fold: Option<bool>,
    },
    Task(RowV),
}

#[derive(Clone, Debug, PartialEq)]
pub struct MenuV {
    /// The row it opened from.
    pub item: usize,
    /// (label, the date it means, muted).
    pub rows: Vec<(String, String)>,
    pub sel: usize,
}

#[derive(Clone, Debug, PartialEq)]
pub struct View {
    /// The window's logical size: `W` wide when it opens, wider when resized or maximized.
    pub w: f32,
    pub h: f32,
    /// Maximized: square corners.
    pub max: bool,
    pub open_n: usize,
    pub field: FieldV,
    pub line: String,
    pub items: Vec<ItemV>,
    pub scroll: f32,
    /// The keyboard's ring, on an item.
    pub ring: Option<usize>,
    pub editing: Option<(usize, FieldV)>,
    pub menu: Option<MenuV>,
    /// The deleted task's title while Undo is offered.
    pub undo: Option<String>,
    pub empty: Option<(String, String)>,
}

#[derive(Clone, Copy, PartialEq, Debug)]
pub enum Hit {
    Close,
    /// The title band's maximize / restore button.
    Max,
    Field,
    Add,
    Header(usize),
    Check(usize),
    Title(usize),
    Edit(usize),
    Due(usize),
    Delete(usize),
    Menu(usize),
    Undo,
}

// ---- geometry ----

fn item_h(it: &ItemV) -> f32 {
    match it {
        ItemV::Header { .. } => HEAD,
        ItemV::Task(_) => ROW,
    }
}

/// Where the list ends: above the Undo bar while it shows.
pub fn list_bottom(h: f32, undo: bool) -> f32 {
    h - 12.0 - if undo { UNDO_H + 8.0 } else { 0.0 }
}

/// Each item's top (before scrolling) and height.
pub fn item_tops(v: &View) -> Vec<(f32, f32)> {
    let mut y = LIST_T + 4.0;
    v.items
        .iter()
        .map(|it| {
            let h = item_h(it);
            let top = y;
            y += h;
            (top, h)
        })
        .collect()
}

/// How far the list can scroll.
pub fn max_scroll(v: &View) -> f32 {
    let content: f32 = v.items.iter().map(item_h).sum();
    (content + 8.0 - (list_bottom(v.h, v.undo.is_some()) - LIST_T)).max(0.0)
}

/// The scroll that shows item `i` whole, moving as little as it can.
pub fn reveal(v: &View, i: usize) -> f32 {
    let Some((top, h)) = item_tops(v).get(i).copied() else {
        return v.scroll;
    };
    let room = list_bottom(v.h, v.undo.is_some()) - LIST_T - 8.0;
    let (top, bottom) = (top - LIST_T - 4.0, top - LIST_T - 4.0 + h);
    let s = if top < v.scroll {
        top
    } else if bottom > v.scroll + room {
        bottom - room
    } else {
        v.scroll
    };
    s.clamp(0.0, max_scroll(v))
}

/// The content column's widest. A wider window (maximized) centres the column instead of
/// stretching the rows, so a due date stays near its title.
const COL: f32 = 720.0;

/// The content column: its left offset in the window, and the view as the column sees it
/// (`w` is the column's width). The title band keeps the window's own width.
fn column(v: &View) -> (f32, View) {
    let cw = v.w.min(COL);
    let off = ((v.w - cw) / 2.0).floor();
    (off, View { w: cw, ..v.clone() })
}

/// The right edge of the content: 20 px in from the column's right edge.
fn x1(w: f32) -> f32 {
    w - X0
}

fn close_rect(w: f32) -> Rect {
    (w - 8.0 - 28.0, 8.0, w - 8.0, 36.0)
}

/// Maximize / restore, just left of close.
fn max_rect(w: f32) -> Rect {
    let c = close_rect(w);
    (c.0 - 4.0 - 28.0, c.1, c.0 - 4.0, c.3)
}

fn add_rect(f: &Fonts, w: f32) -> Rect {
    let bw = text_width("Add", &f.title) + 28.0;
    let r = x1(w) - 4.0;
    (r - bw, FIELD_T + 4.0, r, FIELD_B - 4.0)
}

fn undo_rect(v: &View) -> Rect {
    (X0, v.h - 12.0 - UNDO_H, x1(v.w), v.h - 12.0)
}

fn undo_button(v: &View) -> Rect {
    let r = undo_rect(v);
    let cy = (r.1 + r.3) / 2.0;
    (r.2 - 10.0 - 72.0, cy - 16.0, r.2 - 10.0, cy + 16.0)
}

/// The parts of a task row on screen at `top`: (due label's left edge, due button, delete
/// button). The buttons sit just left of the due label.
fn row_parts(f: &Fonts, r: &RowV, top: f32, w: f32) -> (f32, Rect, Rect) {
    let fmt = if r.overdue { &f.small_medium } else { &f.small };
    let due_w = text_width(&r.due, fmt) + if r.repeat { 16.0 } else { 0.0 };
    let lane = x1(w) - 12.0 - due_w;
    let cy = top + ROW / 2.0;
    let del = (lane - 10.0 - 28.0, cy - 14.0, lane - 10.0, cy + 14.0);
    let due = (del.0 - 4.0 - 28.0, cy - 14.0, del.0 - 4.0, cy + 14.0);
    (lane, due, del)
}

fn edit_rect(f: &Fonts, r: &RowV, top: f32, w: f32) -> Rect {
    let (lane, _, _) = row_parts(f, r, top, w);
    let cy = top + ROW / 2.0;
    (X0 + 34.0, cy - 15.0, lane - 10.0, cy + 15.0)
}

fn menu_rect(v: &View, m: &MenuV, f: &Fonts) -> Option<Rect> {
    let (top, _) = *item_tops(v).get(m.item)?;
    let ItemV::Task(r) = v.items.get(m.item)? else {
        return None;
    };
    let top = top - v.scroll;
    let (_, due, _) = row_parts(f, r, top, v.w);
    let mh = m.rows.len() as f32 * MENU_ROW + 12.0;
    let l = ((due.0 + due.2) / 2.0 - MENU_W / 2.0).clamp(X0, x1(v.w) - MENU_W);
    let below = top + ROW + 4.0;
    let t = if below + mh > list_bottom(v.h, v.undo.is_some()) {
        top - 4.0 - mh
    } else {
        below
    };
    Some((l, t, l + MENU_W, t + mh))
}

fn menu_row(r: Rect, k: usize) -> Rect {
    let y = r.1 + 6.0 + k as f32 * MENU_ROW;
    (r.0 + 6.0, y, r.2 - 6.0, y + MENU_ROW)
}

/// Items on screen: (index, top after scrolling, height).
fn visible(v: &View) -> Vec<(usize, f32, f32)> {
    let bottom = list_bottom(v.h, v.undo.is_some());
    item_tops(v)
        .into_iter()
        .enumerate()
        .map(|(i, (t, h))| (i, t - v.scroll, h))
        .filter(|(_, t, h)| t + h > LIST_T && *t < bottom)
        .collect()
}

pub fn hit(f: &Fonts, v: &View, x: f32, y: f32) -> Option<Hit> {
    if inside(&close_rect(v.w), x, y) {
        return Some(Hit::Close);
    }
    if inside(&max_rect(v.w), x, y) {
        return Some(Hit::Max);
    }
    if y < TOP {
        return None;
    }
    let (off, col) = column(v);
    let (v, x) = (&col, x - off);
    let x1 = x1(v.w);
    if let Some(m) = &v.menu {
        if let Some(r) = menu_rect(v, m, f) {
            if inside(&r, x, y) {
                return (0..m.rows.len())
                    .find(|k| inside(&menu_row(r, *k), x, y))
                    .map(Hit::Menu);
            }
        }
    }
    if v.undo.is_some() {
        if inside(&undo_button(v), x, y) {
            return Some(Hit::Undo);
        }
        if inside(&undo_rect(v), x, y) {
            return None;
        }
    }
    if inside(&add_rect(f, v.w), x, y) {
        return Some(Hit::Add);
    }
    if inside(&(X0, FIELD_T, x1, FIELD_B), x, y) {
        return Some(Hit::Field);
    }
    if y < LIST_T || y > list_bottom(v.h, v.undo.is_some()) {
        return None;
    }
    for (i, top, h) in visible(v) {
        if !(top..top + h).contains(&y) || !(X0..=x1).contains(&x) {
            continue;
        }
        return match &v.items[i] {
            ItemV::Header { fold: Some(_), .. } => Some(Hit::Header(i)),
            ItemV::Header { .. } => None,
            ItemV::Task(r) => {
                if v.editing.as_ref().is_some_and(|(e, _)| *e == i) {
                    return inside(&edit_rect(f, r, top, v.w), x, y).then_some(Hit::Edit(i));
                }
                let (_, due, del) = row_parts(f, r, top, v.w);
                if x < X0 + 36.0 {
                    Some(Hit::Check(i))
                } else if inside(&del, x, y) {
                    Some(Hit::Delete(i))
                } else if !r.done && inside(&due, x, y) {
                    Some(Hit::Due(i))
                } else {
                    Some(Hit::Title(i))
                }
            }
        };
    }
    None
}

// ---- text fields ----

fn caret_x(fmt: &Font, s: &str, b: usize) -> f32 {
    text_width(&s[..b.min(s.len())], fmt)
}

/// How far a field's text is scrolled left so the caret stays in view.
fn text_scroll(fmt: &Font, fv: &FieldV, room: f32) -> f32 {
    (caret_x(fmt, &fv.text, fv.caret) - room + 2.0).max(0.0)
}

fn byte_at(fmt: &Font, fv: &FieldV, local: f32) -> usize {
    let bounds = fv
        .text
        .char_indices()
        .map(|(i, _)| i)
        .chain(std::iter::once(fv.text.len()));
    let mut best = (0usize, f32::MAX);
    for b in bounds {
        let d = (caret_x(fmt, &fv.text, b) - local).abs();
        if d < best.1 {
            best = (b, d);
        }
    }
    best.0
}

/// The add field's byte nearest a click at window x.
pub fn field_byte_at(f: &Fonts, v: &View, x: f32) -> usize {
    let (off, col) = column(v);
    let (v, x) = (&col, x - off);
    let room = add_rect(f, v.w).0 - 10.0 - TEXT_X;
    let local = x - TEXT_X + text_scroll(&f.body, &v.field, room);
    byte_at(&f.body, &v.field, local)
}

/// The inline edit's byte nearest a click at window x.
pub fn edit_byte_at(f: &Fonts, v: &View, x: f32) -> usize {
    let (off, col) = column(v);
    let (v, x) = (&col, x - off);
    let Some((i, fv)) = &v.editing else { return 0 };
    let (Some((top, _)), Some(ItemV::Task(r))) = (item_tops(v).get(*i).copied(), v.items.get(*i))
    else {
        return 0;
    };
    let er = edit_rect(f, r, top - v.scroll, v.w);
    let room = er.2 - er.0 - 20.0;
    let local = x - er.0 - 10.0 + text_scroll(&f.body, fv, room);
    byte_at(&f.body, fv, local)
}

/// One line of editable text in `clip`, starting at `x0`, with its selection and caret.
fn draw_text_line(
    cv: &mut dyn Canvas,
    t: &Theme,
    f: &Fonts,
    fv: &FieldV,
    placeholder: &str,
    x0: f32,
    clip: Rect,
) {
    let (top, bottom) = (clip.1, clip.3);
    let cy = (top + bottom) / 2.0;
    let ink = t.text;
    cv.push_clip(clip.0, top, clip.2, bottom);
    if fv.text.is_empty() && !placeholder.is_empty() {
        cv.text(
            placeholder,
            &f.body,
            x0,
            top,
            clip.2 - x0,
            bottom - top,
            t.muted,
        );
    }
    let x = x0 - text_scroll(&f.body, fv, clip.2 - x0 - 2.0);
    let sel = &fv.selection;
    if fv.focused && !sel.is_empty() {
        let (a, b) = (
            caret_x(&f.body, &fv.text, sel.start),
            caret_x(&f.body, &fv.text, sel.end),
        );
        cv.fill_rect(x + a, cy - 10.0, x + b, cy + 10.0, with_alpha(t.text, 0.16));
    }
    let w = text_width(&fv.text, &f.body);
    cv.text(&fv.text, &f.body, x, top, w + 4.0, bottom - top, ink);
    if fv.focused {
        let cx = (x + caret_x(&f.body, &fv.text, fv.caret)).round() + 0.5;
        cv.line(pt(cx, cy - 9.0), pt(cx, cy + 9.0), ink, 1.2, Stroke::Plain);
    }
    cv.pop_clip();
}

fn draw_field(cv: &mut dyn Canvas, t: &Theme, f: &Fonts, v: &View, hover: Option<Hit>) {
    let fv = &v.field;
    let x1 = x1(v.w);
    let cy = (FIELD_T + FIELD_B) / 2.0;
    if fv.focused {
        cv.stroke_rrect(
            &rrect(X0 - 2.5, FIELD_T - 2.5, x1 + 2.5, FIELD_B + 2.5, 22.5),
            with_alpha(t.text, 0.10),
            4.0,
            Stroke::Plain,
        );
    }
    cv.fill_rrect(&rrect(X0, FIELD_T, x1, FIELD_B, 20.0), t.surface);
    let edge = if fv.focused {
        with_alpha(t.text, 0.35)
    } else {
        t.hair
    };
    cv.stroke_rrect(
        &rrect(X0 + 0.5, FIELD_T + 0.5, x1 - 0.5, FIELD_B - 0.5, 19.5),
        edge,
        1.0,
        Stroke::Plain,
    );
    lip(cv, t, X0, FIELD_T, x1, FIELD_B, 20.0, 1.0);
    paint_icon(cv, Icon::Plus, X0 + 20.0, cy, 15.0, t.muted);
    let add = add_rect(f, v.w);
    let can = !fv.text.trim().is_empty();
    let fill = match (can, hover == Some(Hit::Add)) {
        (true, true) => with_alpha(t.text, 0.86),
        (true, false) => t.text,
        (false, _) => t.button,
    };
    cv.fill_rrect(
        &rrect(add.0, add.1, add.2, add.3, (add.3 - add.1) / 2.0),
        fill,
    );
    cv.center_text(
        "Add",
        &f.title,
        (add.0 + add.2) / 2.0,
        add.1,
        add.3 - add.1,
        if can { t.panel } else { t.muted },
    );
    draw_text_line(
        cv,
        t,
        f,
        fv,
        PLACEHOLDER,
        TEXT_X,
        (TEXT_X - 2.0, FIELD_T + 2.0, add.0 - 6.0, FIELD_B - 2.0),
    );
}

// ---- the list ----

fn draw_header(
    cv: &mut dyn Canvas,
    t: &Theme,
    f: &Fonts,
    it: &ItemV,
    top: f32,
    first: bool,
    hovered: bool,
    ring: bool,
    x1: f32,
) {
    let ItemV::Header {
        name,
        count,
        key,
        fold,
    } = it
    else {
        return;
    };
    let muted = t.muted;
    if !first {
        cv.line(
            pt(X0, top + 6.5),
            pt(x1, top + 6.5),
            t.hair,
            1.0,
            Stroke::Plain,
        );
    }
    let r = (X0, top + 8.0, x1, top + HEAD);
    if hovered {
        cv.fill_rrect(&rrect(r.0, r.1, r.2, r.3, 8.0), t.raised);
    }
    if ring {
        focus_ring(cv, t, r, 8.0);
    }
    cv.text(
        name,
        &f.small_medium,
        X0 + 12.0,
        r.1,
        200.0,
        r.3 - r.1,
        t.text,
    );
    let right = if let Some(open) = fold {
        paint_icon(
            cv,
            if *open {
                Icon::ChevronDown
            } else {
                Icon::ChevronRight
            },
            x1 - 18.0,
            (r.1 + r.3) / 2.0,
            12.0,
            muted,
        );
        x1 - 32.0
    } else {
        x1 - 12.0
    };
    motion::roll_text_right(
        cv,
        key,
        &count.to_string(),
        Roll::Slow,
        &f.small,
        right,
        r.1,
        r.3 - r.1,
        muted,
    );
}

fn focus_ring(cv: &mut dyn Canvas, t: &Theme, r: Rect, radius: f32) {
    cv.stroke_rrect(
        &rrect(r.0 - 1.0, r.1 - 1.0, r.2 + 1.0, r.3 + 1.0, radius + 1.0),
        t.text,
        2.0,
        Stroke::Plain,
    );
}

fn round_button(cv: &mut dyn Canvas, t: &Theme, r: Rect, icon: Icon, hovered: bool) {
    let (cx, cy) = ((r.0 + r.2) / 2.0, (r.1 + r.3) / 2.0);
    let fill = if hovered { t.button_hover } else { t.button };
    cv.fill_ellipse(&ellipse(cx, cy, 14.0), fill);
    paint_icon(cv, icon, cx, cy, 13.0, t.muted);
}

fn draw_task(
    cv: &mut dyn Canvas,
    t: &Theme,
    f: &Fonts,
    v: &View,
    i: usize,
    r: &RowV,
    top: f32,
    hover: Option<Hit>,
) {
    let ink = t.text;
    let muted = t.muted;
    let x1 = x1(v.w);
    let cy = top + ROW / 2.0;
    let editing = v
        .editing
        .as_ref()
        .filter(|(e, _)| *e == i)
        .map(|(_, fv)| fv);
    let menu_here = v.menu.as_ref().is_some_and(|m| m.item == i);
    let hov = matches!(hover, Some(Hit::Title(j) | Hit::Check(j) | Hit::Due(j) | Hit::Delete(j)) if j == i);
    let ring = v.ring == Some(i);
    let lit = (hov || ring || menu_here) && editing.is_none();
    if hov || menu_here {
        cv.fill_rrect(&rrect(X0, top, x1, top + ROW, 8.0), t.raised);
    }
    if ring {
        focus_ring(cv, t, (X0, top, x1, top + ROW), 8.0);
    }
    // The 18 px rounded checkbox; a done task is filled with a tick.
    let (bx, by) = (X0 + 11.0, cy - 9.0);
    if r.done {
        cv.fill_rrect(&rrect(bx, by, bx + 18.0, by + 18.0, 6.0), ink);
        let tick = path(pt(bx + 4.5, by + 9.5), false, |k| {
            line_to(k, pt(bx + 7.8, by + 12.6));
            line_to(k, pt(bx + 13.5, by + 6.2));
        });
        cv.stroke_path(&tick, t.panel, 1.8, Stroke::Round);
    } else {
        let edge = if hover == Some(Hit::Check(i)) {
            t.text
        } else {
            t.passed
        };
        cv.stroke_rrect(
            &rrect(bx + 0.75, by + 0.75, bx + 17.25, by + 17.25, 5.5),
            edge,
            1.5,
            Stroke::Plain,
        );
    }
    let (lane, due_b, del_b) = row_parts(f, r, top, v.w);
    let due_fmt = if r.overdue { &f.small_medium } else { &f.small };
    cv.right_text(
        &r.due,
        due_fmt,
        x1 - 12.0,
        top,
        ROW,
        if r.overdue { ink } else { muted },
    );
    if r.repeat {
        paint_icon(cv, Icon::Repeat, lane + 7.0, cy, 11.0, muted);
    }
    if let Some(fv) = editing {
        let er = edit_rect(f, r, top, v.w);
        cv.fill_rrect(&rrect(er.0, er.1, er.2, er.3, 15.0), t.surface);
        cv.stroke_rrect(
            &rrect(er.0 + 0.5, er.1 + 0.5, er.2 - 0.5, er.3 - 0.5, 14.5),
            with_alpha(t.text, 0.35),
            1.0,
            Stroke::Plain,
        );
        draw_text_line(
            cv,
            t,
            f,
            fv,
            "",
            er.0 + 10.0,
            (er.0 + 8.0, er.1 + 1.0, er.2 - 8.0, er.3 - 1.0),
        );
        return;
    }
    let right = if lit { due_b.0 - 8.0 } else { lane - 10.0 };
    let room = right - (X0 + 42.0);
    let list_w = if r.list.is_empty() {
        0.0
    } else {
        (text_width(&r.list, &f.small) + 2.0).min(room * 0.4)
    };
    let title_room = room - if list_w > 0.0 { list_w + 10.0 } else { 0.0 };
    let title_w = text_width(&r.title, &f.body).min(title_room);
    clipped_text(
        cv,
        &r.title,
        &f.body,
        X0 + 42.0,
        top,
        title_room,
        ROW,
        if r.done { muted } else { ink },
    );
    if list_w > 0.0 {
        clipped_text(
            cv,
            &r.list,
            &f.small,
            X0 + 42.0 + title_w + 10.0,
            top,
            list_w,
            ROW,
            muted,
        );
    }
    if lit {
        if !r.done {
            round_button(cv, t, due_b, Icon::Today, hover == Some(Hit::Due(i)));
        }
        round_button(cv, t, del_b, Icon::Trash, hover == Some(Hit::Delete(i)));
    }
}

fn draw_menu(cv: &mut dyn Canvas, t: &Theme, f: &Fonts, v: &View, hover: Option<Hit>) {
    let Some(m) = &v.menu else { return };
    let Some(r) = menu_rect(v, m, f) else {
        return;
    };
    cv.fill_rrect(&rrect(r.0, r.1, r.2, r.3, 12.0), t.panel);
    cv.stroke_rrect(
        &rrect(r.0 + 0.5, r.1 + 0.5, r.2 - 0.5, r.3 - 0.5, 11.5),
        t.hair,
        1.0,
        Stroke::Plain,
    );
    lip(cv, t, r.0, r.1, r.2, r.3, 12.0, 1.0);
    let (ink, muted) = (t.text, t.muted);
    for (k, (label, date)) in m.rows.iter().enumerate() {
        let rr = menu_row(r, k);
        let lit = match hover {
            Some(Hit::Menu(j)) => j == k,
            _ => m.sel == k,
        };
        if lit {
            cv.fill_rrect(&rrect(rr.0, rr.1, rr.2, rr.3, 8.0), t.raised);
        }
        cv.text(label, &f.body, rr.0 + 10.0, rr.1, 140.0, MENU_ROW, ink);
        if !date.is_empty() {
            num_text(cv, date, &f.small, rr.2 - 10.0, rr.1, MENU_ROW, muted);
        }
    }
}

fn draw_undo(cv: &mut dyn Canvas, t: &Theme, f: &Fonts, v: &View, hover: Option<Hit>) {
    let Some(title) = &v.undo else { return };
    let r = undo_rect(v);
    let cy = (r.1 + r.3) / 2.0;
    cv.fill_rrect(&rrect(r.0, r.1, r.2, r.3, 12.0), t.surface);
    cv.stroke_rrect(
        &rrect(r.0 + 0.5, r.1 + 0.5, r.2 - 0.5, r.3 - 0.5, 11.5),
        t.hair,
        1.0,
        Stroke::Plain,
    );
    cv.fill_rrect(
        &rrect(r.0 + 12.0, cy - 12.0, r.0 + 36.0, cy + 12.0, 7.0),
        t.chip,
    );
    lip(cv, t, r.0, r.1, r.2, r.3, 12.0, 1.0);
    let ink = t.text;
    paint_icon(cv, Icon::Trash, r.0 + 24.0, cy, 13.0, ink);
    let label = "Deleted";
    let lw = text_width(label, &f.title);
    cv.text(label, &f.title, r.0 + 46.0, r.1, lw + 2.0, UNDO_H, ink);
    let b = undo_button(v);
    clipped_text(
        cv,
        title,
        &f.body,
        r.0 + 46.0 + lw + 8.0,
        r.1,
        b.0 - 12.0 - (r.0 + 46.0 + lw + 8.0),
        UNDO_H,
        t.muted,
    );
    let fill = if hover == Some(Hit::Undo) {
        t.button_hover
    } else {
        t.button
    };
    cv.fill_rrect(&rrect(b.0, b.1, b.2, b.3, 16.0), fill);
    lip(cv, t, b.0, b.1, b.2, b.3, 16.0, 1.0);
    cv.center_text("Undo", &f.title, (b.0 + b.2) / 2.0, b.1, 32.0, ink);
}

/// The whole window: its card and lip, the title band, the field, the list, the menu and the
/// Undo bar.
pub fn draw(cv: &mut dyn Canvas, t: &Theme, f: &Fonts, v: &View, hover: Option<Hit>) {
    let (ink, muted, hair) = (t.text, t.muted, t.hair);
    let w = v.w;
    // A maximized window has square corners (it has no rounded region then either).
    let rad = if v.max { 0.0 } else { 16.0 };
    cv.fill_rrect(&rrect(0.0, 0.0, w, v.h, rad), t.panel);
    if !v.max {
        cv.stroke_rrect(
            &rrect(0.5, 0.5, w - 0.5, v.h - 0.5, 15.5),
            hair,
            1.0,
            Stroke::Plain,
        );
    }
    // The title band: "Tasks", how many are open, the close button.
    let tw = text_width("Tasks", &f.title);
    cv.text("Tasks", &f.title, 16.0, 0.0, tw + 2.0, TOP, ink);
    motion::roll_text(
        cv,
        "tasks.win.open",
        &format!("{} open", v.open_n),
        Roll::Slow,
        &f.small,
        16.0 + tw + 8.0,
        0.0,
        TOP,
        muted,
    );
    let max_icon = if v.max { Icon::Restore } else { Icon::Maximize };
    paint_title_button(cv, t, max_rect(w), max_icon, hover == Some(Hit::Max));
    paint_title_button(cv, t, close_rect(w), Icon::Close, hover == Some(Hit::Close));
    let inset = if v.max { 0.0 } else { 1.0 };
    cv.line(
        pt(inset, TOP - 0.5),
        pt(w - inset, TOP - 0.5),
        hair,
        1.0,
        Stroke::Plain,
    );

    // Everything below the title band draws in the content column.
    let win_w = w;
    let (off, col) = column(v);
    let v = &col;
    let (w, x1) = (v.w, self::x1(v.w));
    cv.push_translate(off, 0.0);

    draw_field(cv, t, f, v, hover);
    clipped_text(
        cv,
        &v.line,
        &f.small,
        X0 + 14.0,
        FIELD_B + 6.0,
        x1 - X0 - 28.0,
        18.0,
        muted,
    );

    let bottom = list_bottom(v.h, v.undo.is_some());
    cv.push_clip(X0 - 4.0, LIST_T, x1 + 4.0, bottom);
    if let Some((title, tip)) = &v.empty {
        let cy = LIST_T + 80.0;
        cv.center_text(title, &f.panel_title, w / 2.0, cy, 20.0, ink);
        cv.center_text(tip, &f.small, w / 2.0, cy + 22.0, 18.0, muted);
    }
    // Rows ripple in from the top of what shows, on open.
    for (k, (i, top, _)) in visible(v).into_iter().enumerate() {
        motion::row(cv, motion::ripple_row("tasks.win", k), |cv| {
            match &v.items[i] {
                it @ ItemV::Header { .. } => draw_header(
                    cv,
                    t,
                    f,
                    it,
                    top,
                    i == 0,
                    hover == Some(Hit::Header(i)),
                    v.ring == Some(i),
                    x1,
                ),
                ItemV::Task(r) => draw_task(cv, t, f, v, i, r, top, hover),
            }
        });
    }
    cv.pop_clip();
    let max = max_scroll(v);
    if max > 0.0 {
        let room = bottom - LIST_T;
        let len = (room * room / (room + max)).max(24.0);
        let top = LIST_T + (room - len) * (v.scroll / max).clamp(0.0, 1.0);
        cv.fill_rrect(&rrect(w - 9.0, top, w - 6.0, top + len, 1.5), t.track);
    }
    draw_menu(cv, t, f, v, hover);
    draw_undo(cv, t, f, v, hover);
    cv.pop_translate();
    if !v.max {
        lip(cv, t, 0.0, 0.0, win_w, v.h, 16.0, 1.0);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn field() -> FieldV {
        FieldV {
            text: String::new(),
            caret: 0,
            selection: 0..0,
            focused: false,
        }
    }

    fn view(n: usize, undo: bool) -> View {
        let row = || {
            ItemV::Task(RowV {
                title: "t".into(),
                list: String::new(),
                due: "No date".into(),
                overdue: false,
                repeat: false,
                done: false,
            })
        };
        let mut items = vec![ItemV::Header {
            name: "No date",
            count: n,
            key: "test",
            fold: None,
        }];
        items.extend((0..n).map(|_| row()));
        View {
            w: W,
            h: H,
            max: false,
            open_n: n,
            field: field(),
            line: String::new(),
            items,
            scroll: 0.0,
            ring: None,
            editing: None,
            menu: None,
            undo: undo.then(|| "t".into()),
            empty: None,
        }
    }

    #[test]
    fn a_long_list_scrolls_and_the_undo_bar_takes_its_room_from_the_list() {
        assert_eq!(max_scroll(&view(3, false)), 0.0, "three rows fit");
        let v = view(40, false);
        let max = max_scroll(&v);
        assert!(max > 0.0);
        let with_bar = max_scroll(&view(40, true));
        assert_eq!(with_bar - max, UNDO_H + 8.0);
        // Revealing the last row scrolls to the end; the first comes back to the top.
        assert_eq!(reveal(&v, 40), max);
        let mut v = v;
        v.scroll = max;
        assert_eq!(reveal(&v, 0), 0.0);
        assert_eq!(
            reveal(&v, 39),
            max,
            "a row already in view leaves the scroll alone"
        );
    }

    #[test]
    fn rows_stay_inside_the_list_band() {
        let mut v = view(40, true);
        v.scroll = 37.0;
        let b = list_bottom(v.h, true);
        for (_, top, h) in visible(&v) {
            assert!(top + h > LIST_T && top < b);
        }
        assert!(b < undo_rect(&v).1);
    }

    #[test]
    fn a_wider_window_widens_the_rows_and_keeps_the_buttons_right() {
        let mut v = view(3, true);
        v.w = 1400.0;
        assert_eq!(
            close_rect(v.w).2,
            1400.0 - 8.0,
            "close stays at the right edge"
        );
        assert!(
            max_rect(v.w).2 < close_rect(v.w).0,
            "maximize sits left of close"
        );
        assert!(max_rect(v.w).0 > 1300.0);
        let (off, col) = column(&v);
        assert_eq!(col.w, COL, "the content column stops widening");
        assert_eq!(off, (1400.0 - COL) / 2.0, "and sits in the middle");
        assert_eq!(undo_rect(&col).2, COL - X0, "the Undo bar spans the column");
        assert_eq!(undo_button(&col).2, COL - X0 - 10.0, "Undo stays right");
        let at_default = View { w: W, ..v.clone() };
        assert_eq!(
            column(&at_default),
            (0.0, at_default.clone()),
            "no change at 600"
        );
    }
}
