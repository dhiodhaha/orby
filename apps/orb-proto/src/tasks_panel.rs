#![allow(clippy::too_many_arguments)] // drawing helpers take a device context plus a full set of geometry
//! The Tasks panel (frame "v6.2 · 03"): sidebar (Inbox, Today, Upcoming, spaces with nested
//! folders and lists), list header, the quick add field with token chips and the "understood"
//! line, To do / Done tabs, rows, and the footer. Drawing and hit tests on a plain [`View`].

use std::ops::Range;

use app_quickadd::TokenKind;
use windows::Win32::Graphics::Direct2D::Common::D2D_RECT_F;
use windows::Win32::Graphics::Direct2D::*;

use crate::gfx::*;
use crate::tasks_model::{SideIcon, SideItem, Tab};
use crate::ui::{draw_icon, with_alpha, Fonts, Icon, Theme};

pub const W: f32 = 780.0;
pub const H: f32 = 560.0;
pub const SIDE: f32 = 228.0;
const X0: f32 = SIDE + 22.0;
const X1: f32 = W - 22.0;
const FIELD_T: f32 = 78.0;
const FIELD_B: f32 = 118.0;
const TEXT_X: f32 = X0 + 38.0;
const TABS_Y: f32 = 150.0;
pub const ROWS_T: f32 = 192.0;
pub const ROWS_B: f32 = H - 44.0;
const TREE_Y: f32 = 152.0;
/// The tree of spaces scrolls between TREE_Y and here; the sync line sits below.
const TREE_B: f32 = H - 48.0;
const SIDE_ROW: f32 = 32.0;
const CHIP_PAD: f32 = 7.0;
const CHIP_ICON: f32 = 16.0;

#[derive(Clone, Debug, PartialEq)]
pub struct FieldView {
    pub text: String,
    pub caret: usize,
    pub selection: Range<usize>,
    pub tokens: Vec<(Range<usize>, TokenKind)>,
    pub placeholder: String,
    pub focused: bool,
    /// The Add button's label ("Add", or "Create" for a new list).
    pub action: &'static str,
}

#[derive(Clone, Debug, PartialEq)]
pub enum RowView {
    Task {
        title: String,
        sub: String,
        due: String,
        overdue: bool,
        repeat: bool,
        assignee: Option<String>,
        done: bool,
    },
    /// A prayer time in Today: a dashed rule with the name and time. Never checkable.
    Prayer { name: String, time: String },
}

#[derive(Clone, Debug, PartialEq)]
pub struct View {
    pub side: Vec<SideItem>,
    pub crumb: String,
    pub title: String,
    pub field: FieldView,
    pub understood: String,
    pub tab: Tab,
    pub todo_n: usize,
    pub done_n: usize,
    pub rows: Vec<RowView>,
    pub scroll: f32,
    /// How far the tree of spaces is scrolled (Inbox, Today and Upcoming stay put).
    pub side_scroll: f32,
    /// (title, tip) when the tab has no rows.
    pub empty: Option<(String, String)>,
    pub footer: String,
    pub sync: String,
}

#[derive(Clone, Copy, PartialEq, Debug)]
pub enum Hit {
    Side(usize),
    Chevron(usize),
    NewList,
    Share,
    /// Opens the Tasks window.
    AllTasks,
    Field,
    Add,
    Tab(Tab),
    Row(usize),
    Check(usize),
    Delete(usize),
    Footer,
}

type Rect = (f32, f32, f32, f32);

fn inside(r: &Rect, x: f32, y: f32) -> bool {
    x >= r.0 && x <= r.2 && y >= r.1 && y <= r.3
}

fn row_h(r: &RowView) -> f32 {
    match r {
        RowView::Task { sub, .. } if !sub.is_empty() => 50.0,
        RowView::Task { .. } => 40.0,
        RowView::Prayer { .. } => 30.0,
    }
}

pub fn content_height(v: &View) -> f32 {
    v.rows.iter().map(row_h).sum()
}

/// Each row's top (before scrolling) and height.
fn row_tops(v: &View) -> Vec<(f32, f32)> {
    let mut y = ROWS_T + 4.0;
    v.rows
        .iter()
        .map(|r| {
            let h = row_h(r);
            let top = y;
            y += h;
            (top, h)
        })
        .collect()
}

fn side_rect(i: usize, scroll: f32) -> Rect {
    let y = if i < 3 {
        14.0 + SIDE_ROW * i as f32
    } else {
        TREE_Y + SIDE_ROW * (i - 3) as f32 - scroll
    };
    (10.0, y, SIDE - 10.0, y + SIDE_ROW)
}

/// How far the tree can scroll when the sidebar has `items` rows.
pub fn side_max_scroll(items: usize) -> f32 {
    (SIDE_ROW * items.saturating_sub(3) as f32 - (TREE_B - TREE_Y)).max(0.0)
}

/// The tree's scroll, clamped, and moved just enough to show the selected row in full.
pub fn reveal_selected(side: &[SideItem], scroll: f32) -> f32 {
    let scroll = scroll.clamp(0.0, side_max_scroll(side.len()));
    let Some(i) = side.iter().position(|s| s.selected).filter(|i| *i >= 3) else {
        return scroll;
    };
    let top = SIDE_ROW * (i - 3) as f32;
    let room = TREE_B - TREE_Y;
    if top < scroll {
        top
    } else if top + SIDE_ROW > scroll + room {
        top + SIDE_ROW - room
    } else {
        scroll
    }
}

/// The sidebar rows on screen, with their rects; tree rows only while inside the tree's band.
fn side_layout(v: &View) -> Vec<(usize, Rect)> {
    (0..v.side.len())
        .map(|i| (i, side_rect(i, v.side_scroll)))
        .filter(|(i, r)| *i < 3 || (r.3 > TREE_Y && r.1 < TREE_B))
        .collect()
}

/// A tree row's hit rect, cut to the tree's band so a half-hidden row never takes a click above it.
fn clamp_to_tree(i: usize, r: Rect) -> Rect {
    if i < 3 {
        r
    } else {
        (r.0, r.1.max(TREE_Y), r.2, r.3.min(TREE_B))
    }
}

fn tree_base(item: &SideItem) -> f32 {
    14.0 + item.depth as f32 * 20.0
}

/// "All tasks", left of Share in the header.
const ALL_RECT: Rect = (X1 - 84.0 - 8.0 - 100.0, 28.0, X1 - 84.0 - 8.0, 60.0);

fn add_button(g: &Shared, f: &Fonts, v: &View) -> Rect {
    let w = g.text_width(v.field.action, &f.title) + 28.0;
    (X1 - 4.0 - w, FIELD_T + 4.0, X1 - 4.0, FIELD_B - 4.0)
}

fn tab_rects(g: &Shared, f: &Fonts, v: &View) -> [(Tab, Rect); 2] {
    let w1 = g.text_width("To do", &f.title) + g.text_width(&v.todo_n.to_string(), &f.body) + 30.0;
    let w2 = g.text_width("Done", &f.title) + g.text_width(&v.done_n.to_string(), &f.body) + 30.0;
    [
        (Tab::Todo, (X0, TABS_Y, X0 + w1, TABS_Y + 28.0)),
        (
            Tab::Done,
            (X0 + w1 + 6.0, TABS_Y, X0 + w1 + 6.0 + w2, TABS_Y + 28.0),
        ),
    ]
}

/// The right lane of a task row: the delete button's rect (shown on hover).
fn delete_rect(g: &Shared, f: &Fonts, due: &str, repeat: bool, top: f32, h: f32) -> Rect {
    let due_w = g.text_width(due, &f.small) + if repeat { 16.0 } else { 0.0 };
    let right = X1 - 38.0 - due_w - 10.0;
    let cy = top + h / 2.0;
    (right - 28.0, cy - 14.0, right, cy + 14.0)
}

fn layout(g: &Shared, f: &Fonts, v: &View) -> Vec<(Hit, Rect)> {
    let mut out = Vec::new();
    for (i, r) in side_layout(v) {
        let item = &v.side[i];
        if item.chevron.is_some() {
            let b = tree_base(item);
            out.push((
                Hit::Chevron(i),
                clamp_to_tree(i, (b - 2.0, r.1, b + 20.0, r.3)),
            ));
        }
        out.push((Hit::Side(i), clamp_to_tree(i, r)));
    }
    out.push((Hit::NewList, (SIDE - 36.0, 124.0, SIDE - 12.0, 148.0)));
    out.push((Hit::Share, (X1 - 84.0, 28.0, X1, 60.0)));
    out.push((Hit::AllTasks, ALL_RECT));
    out.push((Hit::Add, add_button(g, f, v)));
    out.push((Hit::Field, (X0, FIELD_T, X1, FIELD_B)));
    for (t, r) in tab_rects(g, f, v) {
        out.push((Hit::Tab(t), r));
    }
    for (i, ((top, h), row)) in row_tops(v).into_iter().zip(&v.rows).enumerate() {
        let top = top - v.scroll;
        if top + h < ROWS_T || top > ROWS_B {
            continue;
        }
        if let RowView::Task { due, repeat, .. } = row {
            let cy = top + h / 2.0;
            out.push((Hit::Check(i), (X0 + 4.0, cy - 14.0, X0 + 36.0, cy + 14.0)));
            out.push((Hit::Delete(i), delete_rect(g, f, due, *repeat, top, h)));
            out.push((
                Hit::Row(i),
                (X0, top.max(ROWS_T), X1, (top + h).min(ROWS_B)),
            ));
        }
    }
    if !v.footer.is_empty() {
        out.push((Hit::Footer, (X0, ROWS_B + 6.0, X1, H - 12.0)));
    }
    out
}

pub fn hit(g: &Shared, f: &Fonts, v: &View, x: f32, y: f32) -> Option<Hit> {
    layout(g, f, v)
        .into_iter()
        .find(|(_, r)| inside(r, x, y))
        .map(|(h, _)| h)
}

// ---- the quick add field ----

/// The field's text split into plain runs and token chips, in order.
fn segments(fv: &FieldView) -> Vec<(Range<usize>, Option<TokenKind>)> {
    let mut out = Vec::new();
    let mut at = 0;
    let mut tokens = fv.tokens.clone();
    tokens.sort_by_key(|(r, _)| r.start);
    for (r, k) in tokens {
        if r.start < at || r.end > fv.text.len() {
            continue;
        }
        if r.start > at {
            out.push((at..r.start, None));
        }
        out.push((r.clone(), Some(k)));
        at = r.end;
    }
    if at < fv.text.len() {
        out.push((at..fv.text.len(), None));
    }
    out
}

fn chip_icon(k: TokenKind) -> Icon {
    match k {
        TokenKind::Date | TokenKind::Time => Icon::Today,
        TokenKind::Repeat => Icon::Repeat,
        TokenKind::List => Icon::List,
        TokenKind::Folder => Icon::Folder,
        TokenKind::Person => Icon::Person,
    }
}

fn seg_width(g: &Shared, f: &Fonts, s: &str, chip: bool) -> f32 {
    let w = g.text_width(s, &f.body);
    if chip {
        w + CHIP_PAD * 2.0 + CHIP_ICON
    } else {
        w
    }
}

/// The caret's x for byte `b`, before horizontal scrolling, from the text's left edge.
fn caret_x(g: &Shared, f: &Fonts, fv: &FieldView, b: usize) -> f32 {
    let mut x = 0.0;
    for (r, k) in segments(fv) {
        let chip = k.is_some();
        if b == r.start {
            return x;
        }
        if b < r.end || (b == r.end && !chip) {
            let lead = if chip { CHIP_PAD + CHIP_ICON } else { 0.0 };
            return x + lead + g.text_width(&fv.text[r.start..b], &f.body);
        }
        x += seg_width(g, f, &fv.text[r.clone()], chip);
    }
    x
}

fn visible_w(g: &Shared, f: &Fonts, v: &View) -> f32 {
    add_button(g, f, v).0 - 10.0 - TEXT_X
}

/// How far the text is scrolled left so the caret stays in view.
fn field_scroll(g: &Shared, f: &Fonts, v: &View) -> f32 {
    (caret_x(g, f, &v.field, v.field.caret) - visible_w(g, f, v) + 2.0).max(0.0)
}

/// The byte nearest to a click at panel x.
pub fn field_byte_at(g: &Shared, f: &Fonts, v: &View, x: f32) -> usize {
    let fv = &v.field;
    let local = x - TEXT_X + field_scroll(g, f, v);
    let mut best = (0usize, f32::MAX);
    let bounds = fv
        .text
        .char_indices()
        .map(|(i, _)| i)
        .chain(std::iter::once(fv.text.len()));
    for b in bounds {
        let d = (caret_x(g, f, fv, b) - local).abs();
        if d < best.1 {
            best = (b, d);
        }
    }
    best.0
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
    let ink = brush(dc, t.text);
    let muted = brush(dc, t.muted);
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
    draw_icon(g, dc, Icon::Plus, X0 + 20.0, cy, 15.0, &muted);

    let add = add_button(g, f, v);
    let can_add = !fv.text.trim().is_empty();
    let add_fill = match (can_add, hover == Some(Hit::Add)) {
        (true, true) => with_alpha(t.text, 0.86),
        (true, false) => t.text,
        (false, _) => t.button,
    };
    unsafe {
        dc.FillRoundedRectangle(
            &rrect(add.0, add.1, add.2, add.3, (add.3 - add.1) / 2.0),
            &brush(dc, add_fill),
        );
    }
    center_text(
        g,
        dc,
        fv.action,
        &f.title,
        (add.0 + add.2) / 2.0,
        add.1,
        add.3 - add.1,
        &brush(dc, if can_add { t.panel } else { t.muted }),
    );

    let clip = D2D_RECT_F {
        left: TEXT_X - 2.0,
        top: FIELD_T + 2.0,
        right: add.0 - 6.0,
        bottom: FIELD_B - 2.0,
    };
    unsafe { dc.PushAxisAlignedClip(&clip, D2D1_ANTIALIAS_MODE_PER_PRIMITIVE) };
    if fv.text.is_empty() {
        text(
            dc,
            &fv.placeholder,
            &f.body,
            TEXT_X,
            FIELD_T,
            clip.right - TEXT_X,
            FIELD_B - FIELD_T,
            &muted,
        );
    }
    let x0 = TEXT_X - field_scroll(g, f, v);
    let sel = &fv.selection;
    if fv.focused && !sel.is_empty() {
        let (a, b) = (caret_x(g, f, fv, sel.start), caret_x(g, f, fv, sel.end));
        unsafe {
            dc.FillRectangle(
                &D2D_RECT_F {
                    left: x0 + a,
                    top: cy - 10.0,
                    right: x0 + b,
                    bottom: cy + 10.0,
                },
                &brush(dc, with_alpha(t.text, 0.16)),
            );
        }
    }
    let mut x = x0;
    for (r, k) in segments(fv) {
        let s = &fv.text[r.clone()];
        let w = seg_width(g, f, s, k.is_some());
        match k {
            Some(kind) => {
                unsafe {
                    dc.FillRoundedRectangle(
                        &rrect(x, cy - 12.0, x + w, cy + 12.0, 12.0),
                        &brush(dc, t.chip),
                    );
                    dc.DrawRoundedRectangle(
                        &rrect(x + 0.5, cy - 11.5, x + w - 0.5, cy + 11.5, 11.5),
                        &brush(dc, t.hair),
                        1.0,
                        None,
                    );
                }
                draw_icon(g, dc, chip_icon(kind), x + CHIP_PAD + 6.0, cy, 12.0, &muted);
                text(
                    dc,
                    s,
                    &f.title,
                    x + CHIP_PAD + CHIP_ICON,
                    FIELD_T,
                    w,
                    FIELD_B - FIELD_T,
                    &ink,
                );
            }
            None => text(dc, s, &f.body, x, FIELD_T, w + 2.0, FIELD_B - FIELD_T, &ink),
        }
        x += w;
    }
    if fv.focused {
        let cx = (x0 + caret_x(g, f, fv, fv.caret)).round() + 0.5;
        unsafe {
            dc.DrawLine(pt(cx, cy - 9.0), pt(cx, cy + 9.0), &ink, 1.2, None);
        }
    }
    unsafe { dc.PopAxisAlignedClip() };
}

// ---- drawing ----

/// `s` when it fits `w`, else its longest start that fits with an ellipsis after it.
#[cfg(test)]
fn fit(s: &str, w: f32, measure: impl Fn(&str) -> f32) -> std::borrow::Cow<'_, str> {
    crate::canvas::ellipsize(s, w, measure)
}

/// One line of text, cut to `w` with an ellipsis (`canvas::clipped_text` on Direct2D).
pub(crate) fn clipped_text(
    g: &Shared,
    dc: &ID2D1DeviceContext,
    s: &str,
    fmt: &windows::Win32::Graphics::DirectWrite::IDWriteTextFormat,
    x: f32,
    y: f32,
    w: f32,
    h: f32,
    b: &ID2D1SolidColorBrush,
) {
    let c = unsafe { b.GetColor() };
    crate::canvas::clipped_text(&mut D2dCanvas::new(g, dc), s, fmt, x, y, w, h, c);
}

fn side_icon(i: SideIcon) -> Icon {
    match i {
        SideIcon::Inbox => Icon::Inbox,
        SideIcon::Today => Icon::Sun,
        SideIcon::Upcoming => Icon::Today,
        SideIcon::Private => Icon::Lock,
        SideIcon::Shared => Icon::People,
        SideIcon::Folder => Icon::Folder,
        SideIcon::List => Icon::List,
    }
}

fn draw_sidebar(
    g: &Shared,
    dc: &ID2D1DeviceContext,
    t: &Theme,
    f: &Fonts,
    v: &View,
    hover: Option<Hit>,
) {
    let ink = brush(dc, t.text);
    let muted = brush(dc, t.muted);
    text(dc, "Spaces", &f.small, 20.0, 126.0, 120.0, 20.0, &muted);
    let nr = (SIDE - 36.0, 124.0, SIDE - 12.0, 148.0);
    if hover == Some(Hit::NewList) {
        unsafe {
            dc.FillEllipse(
                &ellipse((nr.0 + nr.2) / 2.0, (nr.1 + nr.3) / 2.0, 12.0),
                &brush(dc, t.button),
            )
        };
    }
    draw_icon(
        g,
        dc,
        Icon::Plus,
        (nr.0 + nr.2) / 2.0,
        (nr.1 + nr.3) / 2.0,
        13.0,
        &muted,
    );

    let mut tree_clip = false;
    for (i, r) in side_layout(v) {
        let item = &v.side[i];
        if i >= 3 && !tree_clip {
            unsafe {
                dc.PushAxisAlignedClip(
                    &D2D_RECT_F {
                        left: 0.0,
                        top: TREE_Y,
                        right: SIDE,
                        bottom: TREE_B,
                    },
                    D2D1_ANTIALIAS_MODE_PER_PRIMITIVE,
                )
            };
            tree_clip = true;
        }
        let hov = matches!(hover, Some(Hit::Side(j)) | Some(Hit::Chevron(j)) if j == i);
        let fill = if item.selected {
            Some(t.chip)
        } else if hov {
            Some(t.raised)
        } else {
            None
        };
        if let Some(c) = fill {
            unsafe { dc.FillRoundedRectangle(&rrect(r.0, r.1, r.2, r.3, 8.0), &brush(dc, c)) };
        }
        let cy = (r.1 + r.3) / 2.0;
        let name_fmt = if item.selected { &f.title } else { &f.body };
        let (icon_x, name_x) = if i < 3 {
            (30.0, 48.0)
        } else {
            let b = tree_base(item);
            if let Some(open) = item.chevron {
                draw_icon(
                    g,
                    dc,
                    if open {
                        Icon::ChevronDown
                    } else {
                        Icon::ChevronRight
                    },
                    b + 8.0,
                    cy,
                    12.0,
                    &muted,
                );
            }
            (b + 30.0, b + 46.0)
        };
        if matches!(item.icon, SideIcon::Private | SideIcon::Shared) {
            unsafe {
                dc.FillRoundedRectangle(
                    &rrect(icon_x - 12.0, cy - 12.0, icon_x + 12.0, cy + 12.0, 7.0),
                    &brush(dc, t.chip),
                )
            };
            draw_icon(g, dc, side_icon(item.icon), icon_x, cy, 13.0, &ink);
        } else {
            draw_icon(g, dc, side_icon(item.icon), icon_x, cy, 15.0, &ink);
        }
        let right = r.2 - 10.0;
        let note = if item.count > 0 {
            item.count.to_string()
        } else {
            item.note.clone()
        };
        let note_w = if note.is_empty() {
            0.0
        } else {
            g.text_width(&note, &f.small)
        };
        clipped_text(
            g,
            dc,
            &item.name,
            name_fmt,
            name_x
                + if matches!(item.icon, SideIcon::Private | SideIcon::Shared) {
                    6.0
                } else {
                    0.0
                },
            r.1,
            right - note_w - 8.0 - name_x,
            SIDE_ROW,
            &ink,
        );
        if !note.is_empty() {
            right_text(g, dc, &note, &f.small, right, r.1, SIDE_ROW, &muted);
        }
    }
    if tree_clip {
        unsafe { dc.PopAxisAlignedClip() };
    }
    // A thin thumb says there is more of the tree to scroll to.
    let max = side_max_scroll(v.side.len());
    if max > 0.0 {
        let room = TREE_B - TREE_Y;
        let len = (room * room / (room + max)).max(24.0);
        let top = TREE_Y + (room - len) * (v.side_scroll / max).clamp(0.0, 1.0);
        unsafe {
            dc.FillRoundedRectangle(
                &rrect(SIDE - 6.0, top, SIDE - 3.0, top + len, 1.5),
                &brush(dc, t.track),
            )
        };
    }

    let sy = H - 40.0;
    draw_icon(g, dc, Icon::CloudOff, 26.0, sy + 12.0, 14.0, &muted);
    text(dc, &v.sync, &f.small, 40.0, sy, SIDE - 50.0, 24.0, &muted);
}

fn avatar(
    g: &Shared,
    dc: &ID2D1DeviceContext,
    t: &Theme,
    f: &Fonts,
    who: Option<&str>,
    cx: f32,
    cy: f32,
) {
    unsafe {
        match who {
            Some(initial) => {
                dc.FillEllipse(&ellipse(cx, cy, 10.0), &brush(dc, t.chip));
                dc.DrawEllipse(&ellipse(cx, cy, 9.5), &brush(dc, t.hair), 1.0, None);
                center_text(
                    g,
                    dc,
                    initial,
                    &f.small_medium,
                    cx,
                    cy - 10.0,
                    20.0,
                    &brush(dc, t.text),
                );
            }
            None => dc.DrawEllipse(
                &ellipse(cx, cy, 9.5),
                &brush(dc, t.passed),
                1.0,
                &dashed_stroke(g),
            ),
        }
    }
}

fn draw_rows(
    g: &Shared,
    dc: &ID2D1DeviceContext,
    t: &Theme,
    f: &Fonts,
    v: &View,
    hover: Option<Hit>,
) {
    let ink = brush(dc, t.text);
    let muted = brush(dc, t.muted);
    unsafe {
        dc.PushAxisAlignedClip(
            &D2D_RECT_F {
                left: X0 - 2.0,
                top: ROWS_T,
                right: X1 + 2.0,
                bottom: ROWS_B,
            },
            D2D1_ANTIALIAS_MODE_PER_PRIMITIVE,
        )
    };
    if let Some((title, tip)) = &v.empty {
        let cy = ROWS_T + 70.0;
        center_text(g, dc, title, &f.title, (X0 + X1) / 2.0, cy, 20.0, &ink);
        center_text(
            g,
            dc,
            tip,
            &f.small,
            (X0 + X1) / 2.0,
            cy + 22.0,
            18.0,
            &muted,
        );
    }
    for (i, ((top, h), row)) in row_tops(v).into_iter().zip(&v.rows).enumerate() {
        let top = top - v.scroll;
        if top + h < ROWS_T || top > ROWS_B {
            continue;
        }
        let cy = top + h / 2.0;
        match row {
            RowView::Prayer { name, time } => {
                let nw = g.text_width(name, &f.title);
                let tw = g.text_width(time, &f.small);
                let total = nw + 6.0 + tw;
                let mid = (X0 + X1) / 2.0;
                let (l, r) = (mid - total / 2.0 - 10.0, mid + total / 2.0 + 10.0);
                let rule = brush(dc, t.track);
                unsafe {
                    dc.DrawLine(pt(X0 + 8.0, cy), pt(l, cy), &rule, 1.0, &dashed_stroke(g));
                    dc.DrawLine(pt(r, cy), pt(X1 - 8.0, cy), &rule, 1.0, &dashed_stroke(g));
                }
                text(
                    dc,
                    name,
                    &f.title,
                    mid - total / 2.0,
                    top,
                    nw + 2.0,
                    h,
                    &ink,
                );
                text(
                    dc,
                    time,
                    &f.small,
                    mid - total / 2.0 + nw + 6.0,
                    top,
                    tw + 2.0,
                    h,
                    &muted,
                );
            }
            RowView::Task {
                title,
                sub,
                due,
                overdue,
                repeat,
                assignee,
                done,
            } => {
                let hov = matches!(hover, Some(Hit::Row(j)) | Some(Hit::Check(j)) | Some(Hit::Delete(j)) if j == i);
                if hov {
                    unsafe {
                        dc.FillRoundedRectangle(
                            &rrect(X0, top, X1, top + h, 8.0),
                            &brush(dc, t.raised),
                        )
                    };
                }
                // The 18 px rounded checkbox; a done task is filled with a tick.
                let (bx, by) = (X0 + 11.0, cy - 9.0);
                let check_hover = hover == Some(Hit::Check(i));
                unsafe {
                    if *done {
                        dc.FillRoundedRectangle(&rrect(bx, by, bx + 18.0, by + 18.0, 6.0), &ink);
                        let tick = path(g, pt(bx + 4.5, by + 9.5), false, |k| {
                            line_to(k, pt(bx + 7.8, by + 12.6));
                            line_to(k, pt(bx + 13.5, by + 6.2));
                        });
                        dc.DrawGeometry(&tick, &brush(dc, t.panel), 1.8, &round_stroke(g));
                    } else {
                        let edge = if check_hover { t.text } else { t.passed };
                        dc.DrawRoundedRectangle(
                            &rrect(bx + 0.75, by + 0.75, bx + 17.25, by + 17.25, 5.5),
                            &brush(dc, edge),
                            1.5,
                            None,
                        );
                    }
                }
                let due_fmt = if *overdue { &f.small_medium } else { &f.small };
                let due_w = g.text_width(due, due_fmt);
                let due_right = X1 - 38.0;
                let lane_left = due_right
                    - due_w
                    - if *repeat { 16.0 } else { 0.0 }
                    - if hov { 48.0 } else { 10.0 };
                let title_ink = if *done { &muted } else { &ink };
                let (ty, th) = if sub.is_empty() {
                    (top, h)
                } else {
                    (top + 6.0, 20.0)
                };
                clipped_text(
                    g,
                    dc,
                    title,
                    &f.body,
                    X0 + 42.0,
                    ty,
                    lane_left - X0 - 50.0,
                    th,
                    title_ink,
                );
                if !sub.is_empty() {
                    clipped_text(
                        g,
                        dc,
                        sub,
                        &f.small,
                        X0 + 42.0,
                        top + 26.0,
                        lane_left - X0 - 50.0,
                        18.0,
                        &muted,
                    );
                }
                right_text(
                    g,
                    dc,
                    due,
                    due_fmt,
                    due_right,
                    top,
                    h,
                    if *overdue { &ink } else { &muted },
                );
                if *repeat {
                    draw_icon(
                        g,
                        dc,
                        Icon::Repeat,
                        due_right - due_w - 9.0,
                        cy,
                        11.0,
                        &muted,
                    );
                }
                avatar(g, dc, t, f, assignee.as_deref(), X1 - 16.0, cy);
                if hov {
                    let d = delete_rect(g, f, due, *repeat, top, h);
                    let dcx = (d.0 + d.2) / 2.0;
                    let fill = if hover == Some(Hit::Delete(i)) {
                        t.button_hover
                    } else {
                        t.button
                    };
                    unsafe { dc.FillEllipse(&ellipse(dcx, cy, 14.0), &brush(dc, fill)) };
                    draw_icon(g, dc, Icon::Close, dcx, cy, 13.0, &muted);
                }
            }
        }
    }
    unsafe { dc.PopAxisAlignedClip() };
}

pub fn draw(
    g: &Shared,
    dc: &ID2D1DeviceContext,
    t: &Theme,
    f: &Fonts,
    v: &View,
    hover: Option<Hit>,
) {
    let ink = brush(dc, t.text);
    let muted = brush(dc, t.muted);
    let hair = brush(dc, t.hair);
    unsafe {
        dc.FillRoundedRectangle(&rrect(0.0, 0.0, W, H, 20.0), &brush(dc, t.panel));
        let side = brush(dc, with_alpha(t.raised, 0.6));
        dc.PushAxisAlignedClip(
            &D2D_RECT_F {
                left: 0.0,
                top: 0.0,
                right: SIDE,
                bottom: H,
            },
            D2D1_ANTIALIAS_MODE_PER_PRIMITIVE,
        );
        dc.FillRoundedRectangle(&rrect(0.0, 0.0, W, H, 20.0), &side);
        dc.PopAxisAlignedClip();
        dc.DrawLine(pt(SIDE + 0.5, 0.0), pt(SIDE + 0.5, H), &hair, 1.0, None);
        dc.DrawRoundedRectangle(&rrect(0.5, 0.5, W - 0.5, H - 0.5, 19.5), &hair, 1.0, None);
    }
    draw_sidebar(g, dc, t, f, v, hover);

    text(
        dc,
        &v.crumb,
        &f.small,
        X0,
        16.0,
        X1 - X0 - 100.0,
        18.0,
        &muted,
    );
    clipped_text(
        g,
        dc,
        &v.title,
        &f.heading,
        X0,
        34.0,
        ALL_RECT.0 - X0 - 12.0,
        30.0,
        &ink,
    );
    let sr = (X1 - 84.0, 28.0, X1, 60.0);
    for (r, h) in [(sr, Hit::Share), (ALL_RECT, Hit::AllTasks)] {
        unsafe {
            let fill = if hover == Some(h) {
                t.button_hover
            } else {
                t.surface
            };
            dc.FillRoundedRectangle(&rrect(r.0, r.1, r.2, r.3, 16.0), &brush(dc, fill));
            dc.DrawRoundedRectangle(
                &rrect(r.0 + 0.5, r.1 + 0.5, r.2 - 0.5, r.3 - 0.5, 15.5),
                &hair,
                1.0,
                None,
            );
        }
        crate::depth::lip(g, dc, t, r.0, r.1, r.2, r.3, 16.0, 1.0);
    }
    draw_icon(g, dc, Icon::Link, sr.0 + 22.0, 44.0, 14.0, &ink);
    text(dc, "Share", &f.title, sr.0 + 34.0, sr.1, 50.0, 32.0, &ink);
    let ar = ALL_RECT;
    draw_icon(g, dc, Icon::List, ar.0 + 22.0, 44.0, 14.0, &ink);
    text(
        dc,
        "All tasks",
        &f.title,
        ar.0 + 34.0,
        ar.1,
        66.0,
        32.0,
        &ink,
    );

    draw_field(g, dc, t, f, v, hover);
    clipped_text(
        g,
        dc,
        &v.understood,
        &f.small,
        X0 + 14.0,
        FIELD_B + 6.0,
        X1 - X0 - 28.0,
        18.0,
        &muted,
    );

    for (tab, r) in tab_rects(g, f, v) {
        let on = tab == v.tab;
        if on || hover == Some(Hit::Tab(tab)) {
            unsafe {
                dc.FillRoundedRectangle(
                    &rrect(r.0, r.1, r.2, r.3, 14.0),
                    &brush(dc, if on { t.chip } else { t.raised }),
                )
            };
        }
        let (label, n) = match tab {
            Tab::Todo => ("To do", v.todo_n),
            Tab::Done => ("Done", v.done_n),
        };
        let lw = g.text_width(label, &f.title);
        text(
            dc,
            label,
            &f.title,
            r.0 + 12.0,
            r.1,
            lw + 2.0,
            28.0,
            if on { &ink } else { &muted },
        );
        text(
            dc,
            &n.to_string(),
            &f.body,
            r.0 + 18.0 + lw,
            r.1,
            30.0,
            28.0,
            &muted,
        );
    }
    unsafe { dc.DrawLine(pt(X0, ROWS_T - 4.5), pt(X1, ROWS_T - 4.5), &hair, 1.0, None) };

    draw_rows(g, dc, t, f, v, hover);

    if !v.footer.is_empty() {
        unsafe { dc.DrawLine(pt(X0, ROWS_B + 0.5), pt(X1, ROWS_B + 0.5), &hair, 1.0, None) };
        draw_icon(
            g,
            dc,
            Icon::ChevronRight,
            X0 + 6.0,
            ROWS_B + 22.0,
            12.0,
            &muted,
        );
        let fmt = if hover == Some(Hit::Footer) {
            &f.small_medium
        } else {
            &f.small
        };
        clipped_text(
            g,
            dc,
            &v.footer,
            fmt,
            X0 + 20.0,
            ROWS_B + 12.0,
            X1 - X0 - 20.0,
            20.0,
            &muted,
        );
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn field(text: &str, tokens: Vec<(Range<usize>, TokenKind)>) -> FieldView {
        FieldView {
            text: text.into(),
            caret: 0,
            selection: 0..0,
            tokens,
            placeholder: String::new(),
            focused: true,
            action: "Add",
        }
    }

    #[test]
    fn the_field_splits_into_plain_runs_and_chips_in_order() {
        let fv = field(
            "beli galon besok 9am #belanja",
            vec![
                (11..16, TokenKind::Date),
                (17..20, TokenKind::Time),
                (21..29, TokenKind::List),
            ],
        );
        let s: Vec<(&str, bool)> = segments(&fv)
            .into_iter()
            .map(|(r, k)| (&fv.text[r], k.is_some()))
            .collect();
        assert_eq!(
            s,
            vec![
                ("beli galon ", false),
                ("besok", true),
                (" ", false),
                ("9am", true),
                (" ", false),
                ("#belanja", true)
            ]
        );
    }

    #[test]
    fn rows_with_a_second_line_are_taller_and_prayer_markers_shorter() {
        let task = |sub: &str| RowView::Task {
            title: "t".into(),
            sub: sub.into(),
            due: String::new(),
            overdue: false,
            repeat: false,
            assignee: None,
            done: false,
        };
        assert_eq!(row_h(&task("")), 40.0);
        assert_eq!(row_h(&task("Sample task")), 50.0);
        assert_eq!(
            row_h(&RowView::Prayer {
                name: "Ashar".into(),
                time: "14:40".into()
            }),
            30.0
        );
    }

    #[test]
    fn a_long_line_ends_in_an_ellipsis_that_fits_and_a_short_one_is_untouched() {
        // A stand-in for DirectWrite: every char is 7 px wide.
        let w7 = |t: &str| t.chars().count() as f32 * 7.0;
        assert_eq!(fit("Beras 5 kg", 200.0, w7), "Beras 5 kg");
        let long = "Beli buku tulis, pensil penghapus dan penggaris";
        let cut = fit(long, 140.0, w7);
        assert!(cut.ends_with('…') && w7(&cut) <= 140.0, "{cut}");
        assert_eq!(cut, "Beli buku tulis, pe…");
        // Cut on a character boundary, never inside one, and no space before the ellipsis.
        assert_eq!(fit("Telepon ibu ☕☕☕☕", 84.0, w7), "Telepon ibu…");
        assert_eq!(fit("ééééé", 21.0, w7), "éé…");
    }

    fn side(n: usize, selected: usize) -> View {
        let item = |i: usize| SideItem {
            key: crate::tasks_model::SideKey::Inbox,
            depth: 1,
            icon: SideIcon::List,
            name: format!("List {i}"),
            count: 0,
            chevron: None,
            note: String::new(),
            selected: i == selected,
        };
        View {
            side: (0..n).map(item).collect(),
            crumb: String::new(),
            title: String::new(),
            field: field("", Vec::new()),
            understood: String::new(),
            tab: Tab::Todo,
            todo_n: 0,
            done_n: 0,
            rows: Vec::new(),
            scroll: 0.0,
            side_scroll: 0.0,
            empty: None,
            footer: String::new(),
            sync: String::new(),
        }
    }

    #[test]
    fn a_tall_sidebar_scrolls_and_every_list_can_be_reached() {
        let short = side(10, 0);
        assert_eq!(
            side_max_scroll(short.side.len()),
            0.0,
            "seven tree rows fit"
        );
        let mut v = side(20, 0);
        let max = side_max_scroll(v.side.len());
        assert_eq!(max, 17.0 * SIDE_ROW - (TREE_B - TREE_Y));
        let shown: Vec<usize> = side_layout(&v).into_iter().map(|(i, _)| i).collect();
        assert!(!shown.contains(&19), "the last list starts below the fold");
        v.side_scroll = max;
        let (_, r) = side_layout(&v)
            .into_iter()
            .find(|(i, _)| *i == 19)
            .expect("scrolled into view");
        assert!(r.3 <= TREE_B + 0.01);
        // Inbox, Today and Upcoming never scroll; rows scrolled above the band take no clicks.
        assert_eq!(side_layout(&v)[0], (0, side_rect(0, 0.0)));
        assert!(side_layout(&v)
            .iter()
            .all(|(i, r)| *i < 3 || clamp_to_tree(*i, *r).1 >= TREE_Y));
    }

    #[test]
    fn the_selected_list_is_scrolled_into_view_only_as_far_as_needed() {
        let v = side(20, 19);
        let max = side_max_scroll(20);
        assert_eq!(
            reveal_selected(&v.side, 0.0),
            max,
            "the last row ends at the band's foot"
        );
        let v = side(20, 4);
        assert_eq!(
            reveal_selected(&v.side, max),
            SIDE_ROW,
            "a row above the band comes to its top"
        );
        assert_eq!(
            reveal_selected(&v.side, 20.0),
            20.0,
            "a row already in view leaves the scroll alone"
        );
        let v = side(20, 1);
        assert_eq!(
            reveal_selected(&v.side, 999.0),
            max,
            "Today does not scroll; the scroll is only clamped"
        );
    }
}
