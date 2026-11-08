#![allow(clippy::too_many_arguments)] // drawing helpers take a device context plus a full set of geometry
//! The Tasks window: the title band, the add field and the line under it, every task grouped
//! Today / Upcoming / No date / Done (folded), rows with a checkbox, an inline title edit, the due
//! menu, and the Undo bar after a delete. Drawing and hit tests on a plain [`View`] that
//! tasks_window.rs builds.

use std::ops::Range;

use windows::Win32::Graphics::Direct2D::Common::D2D_RECT_F;
use windows::Win32::Graphics::Direct2D::*;
use windows::Win32::Graphics::DirectWrite::IDWriteTextFormat;

use crate::gfx::*;
use crate::motion::{self, Roll};
use crate::tasks_panel::clipped_text;
use crate::ui::{draw_icon, text_num, with_alpha, Fonts, Icon, Theme};

/// The window's logical width.
pub const W: f32 = 600.0;
/// Its height when the screen has room.
pub const H: f32 = 680.0;
/// The least height it shrinks to on a short screen.
pub const MIN_H: f32 = 420.0;
/// The title band.
pub const TOP: f32 = 44.0;
const X0: f32 = 20.0;
const X1: f32 = W - X0;
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
    pub h: f32,
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

fn close_rect() -> Rect {
    (W - 8.0 - 28.0, 8.0, W - 8.0, 36.0)
}

fn add_rect(g: &Shared, f: &Fonts) -> Rect {
    let w = g.text_width("Add", &f.title) + 28.0;
    (X1 - 4.0 - w, FIELD_T + 4.0, X1 - 4.0, FIELD_B - 4.0)
}

fn undo_rect(h: f32) -> Rect {
    (X0, h - 12.0 - UNDO_H, X1, h - 12.0)
}

fn undo_button(h: f32) -> Rect {
    let r = undo_rect(h);
    let cy = (r.1 + r.3) / 2.0;
    (r.2 - 10.0 - 72.0, cy - 16.0, r.2 - 10.0, cy + 16.0)
}

/// The parts of a task row on screen at `top`: (due label's left edge, due button, delete
/// button). The buttons sit just left of the due label.
fn row_parts(g: &Shared, f: &Fonts, r: &RowV, top: f32) -> (f32, Rect, Rect) {
    let fmt = if r.overdue { &f.small_medium } else { &f.small };
    let due_w = g.text_width(&r.due, fmt) + if r.repeat { 16.0 } else { 0.0 };
    let lane = X1 - 12.0 - due_w;
    let cy = top + ROW / 2.0;
    let del = (lane - 10.0 - 28.0, cy - 14.0, lane - 10.0, cy + 14.0);
    let due = (del.0 - 4.0 - 28.0, cy - 14.0, del.0 - 4.0, cy + 14.0);
    (lane, due, del)
}

fn edit_rect(g: &Shared, f: &Fonts, r: &RowV, top: f32) -> Rect {
    let (lane, _, _) = row_parts(g, f, r, top);
    let cy = top + ROW / 2.0;
    (X0 + 34.0, cy - 15.0, lane - 10.0, cy + 15.0)
}

fn menu_rect(v: &View, m: &MenuV, g: &Shared, f: &Fonts) -> Option<Rect> {
    let (top, _) = *item_tops(v).get(m.item)?;
    let ItemV::Task(r) = v.items.get(m.item)? else {
        return None;
    };
    let top = top - v.scroll;
    let (_, due, _) = row_parts(g, f, r, top);
    let mh = m.rows.len() as f32 * MENU_ROW + 12.0;
    let l = ((due.0 + due.2) / 2.0 - MENU_W / 2.0).clamp(X0, X1 - MENU_W);
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

pub fn hit(g: &Shared, f: &Fonts, v: &View, x: f32, y: f32) -> Option<Hit> {
    if inside(&close_rect(), x, y) {
        return Some(Hit::Close);
    }
    if y < TOP {
        return None;
    }
    if let Some(m) = &v.menu {
        if let Some(r) = menu_rect(v, m, g, f) {
            if inside(&r, x, y) {
                return (0..m.rows.len())
                    .find(|k| inside(&menu_row(r, *k), x, y))
                    .map(Hit::Menu);
            }
        }
    }
    if v.undo.is_some() {
        if inside(&undo_button(v.h), x, y) {
            return Some(Hit::Undo);
        }
        if inside(&undo_rect(v.h), x, y) {
            return None;
        }
    }
    if inside(&add_rect(g, f), x, y) {
        return Some(Hit::Add);
    }
    if inside(&(X0, FIELD_T, X1, FIELD_B), x, y) {
        return Some(Hit::Field);
    }
    if y < LIST_T || y > list_bottom(v.h, v.undo.is_some()) {
        return None;
    }
    for (i, top, h) in visible(v) {
        if !(top..top + h).contains(&y) || !(X0..=X1).contains(&x) {
            continue;
        }
        return match &v.items[i] {
            ItemV::Header { fold: Some(_), .. } => Some(Hit::Header(i)),
            ItemV::Header { .. } => None,
            ItemV::Task(r) => {
                if v.editing.as_ref().is_some_and(|(e, _)| *e == i) {
                    return inside(&edit_rect(g, f, r, top), x, y).then_some(Hit::Edit(i));
                }
                let (_, due, del) = row_parts(g, f, r, top);
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

fn caret_x(g: &Shared, fmt: &IDWriteTextFormat, s: &str, b: usize) -> f32 {
    g.text_width(&s[..b.min(s.len())], fmt)
}

/// How far a field's text is scrolled left so the caret stays in view.
fn text_scroll(g: &Shared, fmt: &IDWriteTextFormat, fv: &FieldV, room: f32) -> f32 {
    (caret_x(g, fmt, &fv.text, fv.caret) - room + 2.0).max(0.0)
}

fn byte_at(g: &Shared, fmt: &IDWriteTextFormat, fv: &FieldV, local: f32) -> usize {
    let bounds = fv
        .text
        .char_indices()
        .map(|(i, _)| i)
        .chain(std::iter::once(fv.text.len()));
    let mut best = (0usize, f32::MAX);
    for b in bounds {
        let d = (caret_x(g, fmt, &fv.text, b) - local).abs();
        if d < best.1 {
            best = (b, d);
        }
    }
    best.0
}

/// The add field's byte nearest a click at window x.
pub fn field_byte_at(g: &Shared, f: &Fonts, v: &View, x: f32) -> usize {
    let room = add_rect(g, f).0 - 10.0 - TEXT_X;
    let local = x - TEXT_X + text_scroll(g, &f.body, &v.field, room);
    byte_at(g, &f.body, &v.field, local)
}

/// The inline edit's byte nearest a click at window x.
pub fn edit_byte_at(g: &Shared, f: &Fonts, v: &View, x: f32) -> usize {
    let Some((i, fv)) = &v.editing else { return 0 };
    let (Some((top, _)), Some(ItemV::Task(r))) = (item_tops(v).get(*i).copied(), v.items.get(*i))
    else {
        return 0;
    };
    let er = edit_rect(g, f, r, top - v.scroll);
    let room = er.2 - er.0 - 20.0;
    let local = x - er.0 - 10.0 + text_scroll(g, &f.body, fv, room);
    byte_at(g, &f.body, fv, local)
}

/// One line of editable text in `clip`, starting at `x0`, with its selection and caret.
fn draw_text_line(
    g: &Shared,
    dc: &ID2D1DeviceContext,
    t: &Theme,
    f: &Fonts,
    fv: &FieldV,
    placeholder: &str,
    x0: f32,
    clip: Rect,
) {
    let (top, bottom) = (clip.1, clip.3);
    let cy = (top + bottom) / 2.0;
    let ink = brush(dc, t.text);
    unsafe {
        dc.PushAxisAlignedClip(
            &D2D_RECT_F {
                left: clip.0,
                top,
                right: clip.2,
                bottom,
            },
            D2D1_ANTIALIAS_MODE_PER_PRIMITIVE,
        )
    };
    if fv.text.is_empty() && !placeholder.is_empty() {
        text(
            dc,
            placeholder,
            &f.body,
            x0,
            top,
            clip.2 - x0,
            bottom - top,
            &brush(dc, t.muted),
        );
    }
    let x = x0 - text_scroll(g, &f.body, fv, clip.2 - x0 - 2.0);
    let sel = &fv.selection;
    if fv.focused && !sel.is_empty() {
        let (a, b) = (
            caret_x(g, &f.body, &fv.text, sel.start),
            caret_x(g, &f.body, &fv.text, sel.end),
        );
        unsafe {
            dc.FillRectangle(
                &D2D_RECT_F {
                    left: x + a,
                    top: cy - 10.0,
                    right: x + b,
                    bottom: cy + 10.0,
                },
                &brush(dc, with_alpha(t.text, 0.16)),
            );
        }
    }
    let w = g.text_width(&fv.text, &f.body);
    text(dc, &fv.text, &f.body, x, top, w + 4.0, bottom - top, &ink);
    if fv.focused {
        let cx = (x + caret_x(g, &f.body, &fv.text, fv.caret)).round() + 0.5;
        unsafe { dc.DrawLine(pt(cx, cy - 9.0), pt(cx, cy + 9.0), &ink, 1.2, None) };
    }
    unsafe { dc.PopAxisAlignedClip() };
}

fn draw_field(
    g: &Shared,
    dc: &ID2D1DeviceContext,
    t: &Theme,
    f: &Fonts,
    v: &View,
    hover: Option<Hit>,
) {
    let fv = &v.field;
    let cy = (FIELD_T + FIELD_B) / 2.0;
    unsafe {
        if fv.focused {
            dc.DrawRoundedRectangle(
                &rrect(X0 - 2.5, FIELD_T - 2.5, X1 + 2.5, FIELD_B + 2.5, 22.5),
                &brush(dc, with_alpha(t.text, 0.10)),
                4.0,
                None,
            );
        }
        dc.FillRoundedRectangle(
            &rrect(X0, FIELD_T, X1, FIELD_B, 20.0),
            &brush(dc, t.surface),
        );
        let edge = if fv.focused {
            with_alpha(t.text, 0.35)
        } else {
            t.hair
        };
        dc.DrawRoundedRectangle(
            &rrect(X0 + 0.5, FIELD_T + 0.5, X1 - 0.5, FIELD_B - 0.5, 19.5),
            &brush(dc, edge),
            1.0,
            None,
        );
    }
    crate::depth::lip(g, dc, t, X0, FIELD_T, X1, FIELD_B, 20.0, 1.0);
    draw_icon(g, dc, Icon::Plus, X0 + 20.0, cy, 15.0, &brush(dc, t.muted));
    let add = add_rect(g, f);
    let can = !fv.text.trim().is_empty();
    let fill = match (can, hover == Some(Hit::Add)) {
        (true, true) => with_alpha(t.text, 0.86),
        (true, false) => t.text,
        (false, _) => t.button,
    };
    unsafe {
        dc.FillRoundedRectangle(
            &rrect(add.0, add.1, add.2, add.3, (add.3 - add.1) / 2.0),
            &brush(dc, fill),
        )
    };
    center_text(
        g,
        dc,
        "Add",
        &f.title,
        (add.0 + add.2) / 2.0,
        add.1,
        add.3 - add.1,
        &brush(dc, if can { t.panel } else { t.muted }),
    );
    draw_text_line(
        g,
        dc,
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
    g: &Shared,
    dc: &ID2D1DeviceContext,
    t: &Theme,
    f: &Fonts,
    it: &ItemV,
    top: f32,
    first: bool,
    hovered: bool,
    ring: bool,
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
    let muted = brush(dc, t.muted);
    if !first {
        unsafe {
            dc.DrawLine(
                pt(X0, top + 6.5),
                pt(X1, top + 6.5),
                &brush(dc, t.hair),
                1.0,
                None,
            )
        };
    }
    let r = (X0, top + 8.0, X1, top + HEAD);
    if hovered {
        unsafe { dc.FillRoundedRectangle(&rrect(r.0, r.1, r.2, r.3, 8.0), &brush(dc, t.raised)) };
    }
    if ring {
        focus_ring(dc, t, r, 8.0);
    }
    text(
        dc,
        name,
        &f.small_medium,
        X0 + 12.0,
        r.1,
        200.0,
        r.3 - r.1,
        &brush(dc, t.text),
    );
    let right = if let Some(open) = fold {
        draw_icon(
            g,
            dc,
            if *open {
                Icon::ChevronDown
            } else {
                Icon::ChevronRight
            },
            X1 - 18.0,
            (r.1 + r.3) / 2.0,
            12.0,
            &muted,
        );
        X1 - 32.0
    } else {
        X1 - 12.0
    };
    motion::rolling_text_right(
        g,
        dc,
        key,
        &count.to_string(),
        Roll::Slow,
        &f.small,
        right,
        r.1,
        r.3 - r.1,
        &muted,
    );
}

fn focus_ring(dc: &ID2D1DeviceContext, t: &Theme, r: Rect, radius: f32) {
    unsafe {
        dc.DrawRoundedRectangle(
            &rrect(r.0 - 1.0, r.1 - 1.0, r.2 + 1.0, r.3 + 1.0, radius + 1.0),
            &brush(dc, t.text),
            2.0,
            None,
        )
    };
}

fn round_button(
    g: &Shared,
    dc: &ID2D1DeviceContext,
    t: &Theme,
    r: Rect,
    icon: Icon,
    hovered: bool,
) {
    let (cx, cy) = ((r.0 + r.2) / 2.0, (r.1 + r.3) / 2.0);
    let fill = if hovered { t.button_hover } else { t.button };
    unsafe { dc.FillEllipse(&ellipse(cx, cy, 14.0), &brush(dc, fill)) };
    draw_icon(g, dc, icon, cx, cy, 13.0, &brush(dc, t.muted));
}

fn draw_task(
    g: &Shared,
    dc: &ID2D1DeviceContext,
    t: &Theme,
    f: &Fonts,
    v: &View,
    i: usize,
    r: &RowV,
    top: f32,
    hover: Option<Hit>,
) {
    let ink = brush(dc, t.text);
    let muted = brush(dc, t.muted);
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
        unsafe {
            dc.FillRoundedRectangle(&rrect(X0, top, X1, top + ROW, 8.0), &brush(dc, t.raised))
        };
    }
    if ring {
        focus_ring(dc, t, (X0, top, X1, top + ROW), 8.0);
    }
    // The 18 px rounded checkbox; a done task is filled with a tick.
    let (bx, by) = (X0 + 11.0, cy - 9.0);
    unsafe {
        if r.done {
            dc.FillRoundedRectangle(&rrect(bx, by, bx + 18.0, by + 18.0, 6.0), &ink);
            let tick = path(g, pt(bx + 4.5, by + 9.5), false, |k| {
                line_to(k, pt(bx + 7.8, by + 12.6));
                line_to(k, pt(bx + 13.5, by + 6.2));
            });
            dc.DrawGeometry(&tick, &brush(dc, t.panel), 1.8, &round_stroke(g));
        } else {
            let edge = if hover == Some(Hit::Check(i)) {
                t.text
            } else {
                t.passed
            };
            dc.DrawRoundedRectangle(
                &rrect(bx + 0.75, by + 0.75, bx + 17.25, by + 17.25, 5.5),
                &brush(dc, edge),
                1.5,
                None,
            );
        }
    }
    let (lane, due_b, del_b) = row_parts(g, f, r, top);
    let due_fmt = if r.overdue { &f.small_medium } else { &f.small };
    right_text(
        g,
        dc,
        &r.due,
        due_fmt,
        X1 - 12.0,
        top,
        ROW,
        if r.overdue { &ink } else { &muted },
    );
    if r.repeat {
        draw_icon(g, dc, Icon::Repeat, lane + 7.0, cy, 11.0, &muted);
    }
    if let Some(fv) = editing {
        let er = edit_rect(g, f, r, top);
        unsafe {
            dc.FillRoundedRectangle(&rrect(er.0, er.1, er.2, er.3, 15.0), &brush(dc, t.surface));
            dc.DrawRoundedRectangle(
                &rrect(er.0 + 0.5, er.1 + 0.5, er.2 - 0.5, er.3 - 0.5, 14.5),
                &brush(dc, with_alpha(t.text, 0.35)),
                1.0,
                None,
            );
        }
        draw_text_line(
            g,
            dc,
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
        (g.text_width(&r.list, &f.small) + 2.0).min(room * 0.4)
    };
    let title_room = room - if list_w > 0.0 { list_w + 10.0 } else { 0.0 };
    let title_w = g.text_width(&r.title, &f.body).min(title_room);
    clipped_text(
        g,
        dc,
        &r.title,
        &f.body,
        X0 + 42.0,
        top,
        title_room,
        ROW,
        if r.done { &muted } else { &ink },
    );
    if list_w > 0.0 {
        clipped_text(
            g,
            dc,
            &r.list,
            &f.small,
            X0 + 42.0 + title_w + 10.0,
            top,
            list_w,
            ROW,
            &muted,
        );
    }
    if lit {
        if !r.done {
            round_button(g, dc, t, due_b, Icon::Today, hover == Some(Hit::Due(i)));
        }
        round_button(g, dc, t, del_b, Icon::Trash, hover == Some(Hit::Delete(i)));
    }
}

fn draw_menu(
    g: &Shared,
    dc: &ID2D1DeviceContext,
    t: &Theme,
    f: &Fonts,
    v: &View,
    hover: Option<Hit>,
) {
    let Some(m) = &v.menu else { return };
    let Some(r) = menu_rect(v, m, g, f) else {
        return;
    };
    unsafe {
        dc.FillRoundedRectangle(&rrect(r.0, r.1, r.2, r.3, 12.0), &brush(dc, t.panel));
        dc.DrawRoundedRectangle(
            &rrect(r.0 + 0.5, r.1 + 0.5, r.2 - 0.5, r.3 - 0.5, 11.5),
            &brush(dc, t.hair),
            1.0,
            None,
        );
    }
    crate::depth::lip(g, dc, t, r.0, r.1, r.2, r.3, 12.0, 1.0);
    let (ink, muted) = (brush(dc, t.text), brush(dc, t.muted));
    for (k, (label, date)) in m.rows.iter().enumerate() {
        let rr = menu_row(r, k);
        let lit = match hover {
            Some(Hit::Menu(j)) => j == k,
            _ => m.sel == k,
        };
        if lit {
            unsafe {
                dc.FillRoundedRectangle(&rrect(rr.0, rr.1, rr.2, rr.3, 8.0), &brush(dc, t.raised))
            };
        }
        text(dc, label, &f.body, rr.0 + 10.0, rr.1, 140.0, MENU_ROW, &ink);
        if !date.is_empty() {
            text_num(g, dc, date, &f.small, rr.2 - 10.0, rr.1, MENU_ROW, &muted);
        }
    }
}

fn draw_undo(
    g: &Shared,
    dc: &ID2D1DeviceContext,
    t: &Theme,
    f: &Fonts,
    v: &View,
    hover: Option<Hit>,
) {
    let Some(title) = &v.undo else { return };
    let r = undo_rect(v.h);
    let cy = (r.1 + r.3) / 2.0;
    unsafe {
        dc.FillRoundedRectangle(&rrect(r.0, r.1, r.2, r.3, 12.0), &brush(dc, t.surface));
        dc.DrawRoundedRectangle(
            &rrect(r.0 + 0.5, r.1 + 0.5, r.2 - 0.5, r.3 - 0.5, 11.5),
            &brush(dc, t.hair),
            1.0,
            None,
        );
        dc.FillRoundedRectangle(
            &rrect(r.0 + 12.0, cy - 12.0, r.0 + 36.0, cy + 12.0, 7.0),
            &brush(dc, t.chip),
        );
    }
    crate::depth::lip(g, dc, t, r.0, r.1, r.2, r.3, 12.0, 1.0);
    let ink = brush(dc, t.text);
    draw_icon(g, dc, Icon::Trash, r.0 + 24.0, cy, 13.0, &ink);
    let label = "Deleted";
    let lw = g.text_width(label, &f.title);
    text(dc, label, &f.title, r.0 + 46.0, r.1, lw + 2.0, UNDO_H, &ink);
    let b = undo_button(v.h);
    clipped_text(
        g,
        dc,
        title,
        &f.body,
        r.0 + 46.0 + lw + 8.0,
        r.1,
        b.0 - 12.0 - (r.0 + 46.0 + lw + 8.0),
        UNDO_H,
        &brush(dc, t.muted),
    );
    let fill = if hover == Some(Hit::Undo) {
        t.button_hover
    } else {
        t.button
    };
    unsafe { dc.FillRoundedRectangle(&rrect(b.0, b.1, b.2, b.3, 16.0), &brush(dc, fill)) };
    crate::depth::lip(g, dc, t, b.0, b.1, b.2, b.3, 16.0, 1.0);
    center_text(g, dc, "Undo", &f.title, (b.0 + b.2) / 2.0, b.1, 32.0, &ink);
}

/// The whole window: its card and lip, the title band, the field, the list, the menu and the
/// Undo bar.
pub fn draw(
    g: &Shared,
    dc: &ID2D1DeviceContext,
    t: &Theme,
    f: &Fonts,
    v: &View,
    hover: Option<Hit>,
) {
    let (ink, muted, hair) = (brush(dc, t.text), brush(dc, t.muted), brush(dc, t.hair));
    unsafe {
        dc.FillRoundedRectangle(&rrect(0.0, 0.0, W, v.h, 16.0), &brush(dc, t.panel));
        dc.DrawRoundedRectangle(&rrect(0.5, 0.5, W - 0.5, v.h - 0.5, 15.5), &hair, 1.0, None);
    }
    // The title band: "Tasks", how many are open, the close button.
    let tw = g.text_width("Tasks", &f.title);
    text(dc, "Tasks", &f.title, 16.0, 0.0, tw + 2.0, TOP, &ink);
    motion::rolling_text(
        g,
        dc,
        "tasks.win.open",
        &format!("{} open", v.open_n),
        Roll::Slow,
        &f.small,
        16.0 + tw + 8.0,
        0.0,
        TOP,
        &muted,
    );
    let cr = close_rect();
    if hover == Some(Hit::Close) {
        unsafe {
            dc.FillRoundedRectangle(
                &rrect(cr.0, cr.1, cr.2, cr.3, 14.0),
                &brush(dc, t.button_hover),
            )
        };
    }
    draw_icon(
        g,
        dc,
        Icon::Close,
        (cr.0 + cr.2) / 2.0,
        (cr.1 + cr.3) / 2.0,
        14.0,
        &muted,
    );
    unsafe { dc.DrawLine(pt(1.0, TOP - 0.5), pt(W - 1.0, TOP - 0.5), &hair, 1.0, None) };

    draw_field(g, dc, t, f, v, hover);
    clipped_text(
        g,
        dc,
        &v.line,
        &f.small,
        X0 + 14.0,
        FIELD_B + 6.0,
        X1 - X0 - 28.0,
        18.0,
        &muted,
    );

    let bottom = list_bottom(v.h, v.undo.is_some());
    unsafe {
        dc.PushAxisAlignedClip(
            &D2D_RECT_F {
                left: X0 - 4.0,
                top: LIST_T,
                right: X1 + 4.0,
                bottom,
            },
            D2D1_ANTIALIAS_MODE_PER_PRIMITIVE,
        )
    };
    if let Some((title, tip)) = &v.empty {
        let cy = LIST_T + 80.0;
        center_text(g, dc, title, &f.panel_title, W / 2.0, cy, 20.0, &ink);
        center_text(g, dc, tip, &f.small, W / 2.0, cy + 22.0, 18.0, &muted);
    }
    // Rows ripple in from the top of what shows, on open.
    for (k, (i, top, _)) in visible(v).into_iter().enumerate() {
        motion::with_row(dc, motion::ripple_row("tasks.win", k), || {
            match &v.items[i] {
                it @ ItemV::Header { .. } => draw_header(
                    g,
                    dc,
                    t,
                    f,
                    it,
                    top,
                    i == 0,
                    hover == Some(Hit::Header(i)),
                    v.ring == Some(i),
                ),
                ItemV::Task(r) => draw_task(g, dc, t, f, v, i, r, top, hover),
            }
        });
    }
    unsafe { dc.PopAxisAlignedClip() };
    let max = max_scroll(v);
    if max > 0.0 {
        let room = bottom - LIST_T;
        let len = (room * room / (room + max)).max(24.0);
        let top = LIST_T + (room - len) * (v.scroll / max).clamp(0.0, 1.0);
        unsafe {
            dc.FillRoundedRectangle(
                &rrect(W - 9.0, top, W - 6.0, top + len, 1.5),
                &brush(dc, t.track),
            )
        };
    }
    draw_menu(g, dc, t, f, v, hover);
    draw_undo(g, dc, t, f, v, hover);
    crate::depth::lip(g, dc, t, 0.0, 0.0, W, v.h, 16.0, 1.0);
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
            h: H,
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
        assert!(b < undo_rect(v.h).1);
    }
}
