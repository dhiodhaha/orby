#![allow(clippy::too_many_arguments)] // drawing helpers take a device context plus a full set of geometry
//! The History panel (frames "v6.2 · 23" and "· 24"): the Drink and Focus tabs, the month and
//! year grids, the summary numbers, the day detail, and the empty, first-week and loading
//! layouts. Drawing and hit tests; `history_model.rs` decides every word and level.

use windows::Win32::Graphics::Direct2D::Common::D2D1_COLOR_F;
use windows::Win32::Graphics::Direct2D::*;
use windows::Win32::Graphics::DirectWrite::{IDWriteTextFormat, DWRITE_TEXT_METRICS};

use crate::gfx::*;
use crate::history_model::{Body, Cell, CellView, Detail, Parts, Span, Tab, View};
use crate::ui::{draw_icon, with_alpha, Fonts, Icon, Theme};

const MONTH_W: f32 = 360.0;
const YEAR_W: f32 = 700.0;
const COMPACT_W: f32 = 456.0;
const COMPACT_H: f32 = 236.0;
const PAD: f32 = 16.0;
/// The stats band, between two hairlines.
const STATS_Y: f32 = 100.0;
const STATS_B: f32 = 190.0;
const GRID_Y: f32 = 226.0;
const GAP: f32 = 6.0;
const YEAR_CELL: f32 = 10.0;
const YEAR_STEP: f32 = 12.0;
const YEAR_X: f32 = PAD + 28.0;
const YEAR_Y: f32 = STATS_B + 14.0 + 18.0;
const ROW_H: f32 = 36.0;
/// The small grid beside the text in the compact layouts.
const SMALL_W: f32 = 192.0;
const SMALL_GAP: f32 = 4.0;

#[derive(Clone, Copy, PartialEq, Debug)]
pub enum Hit {
    Tab(Tab),
    Span(Span),
    Prev,
    Next,
    Day(usize),
    /// The empty state's one button.
    Action,
}

type Rect = (f32, f32, f32, f32);

fn compact_text_w() -> f32 {
    COMPACT_W - 2.0 * PAD - 20.0 - SMALL_W
}

/// The height wrapped text takes at width `w`.
fn wrapped_height(g: &Shared, s: &str, fmt: &IDWriteTextFormat, w: f32) -> f32 {
    let t: Vec<u16> = s.encode_utf16().collect();
    unsafe {
        g.dw.CreateTextLayout(&t, fmt, w, 400.0)
            .map(|l| {
                let mut m = DWRITE_TEXT_METRICS::default();
                let _ = l.GetMetrics(&mut m);
                m.height
            })
            .unwrap_or(32.0)
    }
}

fn inside(r: &Rect, x: f32, y: f32) -> bool {
    x >= r.0 && x <= r.2 && y >= r.1 && y <= r.3
}

fn month_cell() -> f32 {
    (MONTH_W - 2.0 * PAD - 6.0 * GAP) / 7.0
}

fn month_grid_bottom() -> f32 {
    GRID_Y + 5.0 * month_cell() + 4.0 * GAP
}

fn by_task_rows(v: &View) -> usize {
    match &v.body {
        Body::Full {
            by_task: Some(b), ..
        } => b.rows.len(),
        _ => 0,
    }
}

pub fn size(v: &View) -> (f32, f32) {
    if v.body.compact() {
        return (COMPACT_W, COMPACT_H);
    }
    match v.span {
        Span::Month => {
            let legend = month_grid_bottom() + 12.0 + 16.0;
            let n = by_task_rows(v);
            let tasks = if n > 0 {
                16.0 + 16.0 + 2.0 + ROW_H * n as f32
            } else {
                0.0
            };
            (MONTH_W, (legend + tasks + PAD).ceil())
        }
        Span::Year => (
            YEAR_W,
            YEAR_Y + 7.0 * YEAR_STEP - 2.0 + 12.0 + 16.0 + 10.0 + 20.0 + PAD,
        ),
    }
}

fn tabs_rect(w: f32) -> Rect {
    (w - PAD - 132.0, PAD, w - PAD, PAD + 32.0)
}

fn tab_rect(w: f32, t: Tab) -> Rect {
    let r = tabs_rect(w);
    let i = if t == Tab::Drink { 0.0 } else { 1.0 };
    (
        r.0 + 2.0 + 66.0 * i,
        r.1 + 2.0,
        r.0 + 66.0 + 66.0 * i,
        r.3 - 2.0,
    )
}

/// Every control and cell, in logical px.
fn rects(g: &Shared, f: &Fonts, v: &View) -> Vec<(Hit, Rect)> {
    let (w, _) = size(v);
    let mut out = vec![
        (Hit::Tab(Tab::Drink), tab_rect(w, Tab::Drink)),
        (Hit::Tab(Tab::Focus), tab_rect(w, Tab::Focus)),
    ];
    if v.body.compact() {
        if let Body::Empty {
            action, text: body, ..
        } = &v.body
        {
            let bw = g.text_width(action, &f.title) + 24.0;
            let y = 62.0 + 22.0 + wrapped_height(g, body, &f.small_wrap, compact_text_w()) + 14.0;
            out.push((Hit::Action, (PAD, y, PAD + bw, y + 32.0)));
        }
        out.extend(
            cell_rects(v)
                .into_iter()
                .enumerate()
                .map(|(i, r)| (Hit::Day(i), r)),
        );
        return out;
    }
    let y = 60.0;
    out.push((Hit::Prev, (PAD, y, PAD + 28.0, y + 28.0)));
    let lw = g.text_width(&v.range, &f.title);
    let nx = PAD + 28.0 + 4.0 + 6.0 + lw + 6.0 + 4.0;
    out.push((Hit::Next, (nx, y, nx + 28.0, y + 28.0)));
    let (yw, mw) = (
        g.text_width("Year", &f.small_medium) + 20.0,
        g.text_width("Month", &f.small_medium) + 20.0,
    );
    out.push((Hit::Span(Span::Year), (w - PAD - yw, y, w - PAD, y + 28.0)));
    out.push((
        Hit::Span(Span::Month),
        (w - PAD - yw - 2.0 - mw, y, w - PAD - yw - 2.0, y + 28.0),
    ));
    out.extend(
        cell_rects(v)
            .into_iter()
            .enumerate()
            .map(|(i, r)| (Hit::Day(i), r)),
    );
    out
}

/// One rect per cell, in the layout the view uses.
fn cell_rects(v: &View) -> Vec<Rect> {
    if v.body.compact() {
        let c = (SMALL_W - 6.0 * SMALL_GAP) / 7.0;
        let x0 = COMPACT_W - PAD - SMALL_W;
        let y0 = 62.0 + 16.0 + 6.0;
        return (0..v.cells.len())
            .map(|i| {
                let (col, row) = ((i % 7) as f32, (i / 7) as f32);
                let (x, y) = (x0 + col * (c + SMALL_GAP), y0 + row * (c + SMALL_GAP));
                (x, y, x + c, y + c)
            })
            .collect();
    }
    match v.span {
        Span::Month => {
            let c = month_cell();
            (0..v.cells.len())
                .map(|i| {
                    let (col, row) = ((i % 7) as f32, (i / 7) as f32);
                    let (x, y) = (PAD + col * (c + GAP), GRID_Y + row * (c + GAP));
                    (x, y, x + c, y + c)
                })
                .collect()
        }
        Span::Year => (0..v.cells.len())
            .map(|i| {
                let (col, row) = ((i / 7) as f32, (i % 7) as f32);
                let (x, y) = (YEAR_X + col * YEAR_STEP, YEAR_Y + row * YEAR_STEP);
                (x, y, x + YEAR_CELL, y + YEAR_CELL)
            })
            .collect(),
    }
}

pub fn hit(g: &Shared, f: &Fonts, v: &View, x: f32, y: f32) -> Option<Hit> {
    let rs = rects(g, f, v);
    // The year grid's 2 px gaps belong to the nearest day, so a sweep never flickers the detail.
    let slack = if !v.body.compact() && v.span == Span::Year {
        1.0
    } else {
        0.0
    };
    rs.into_iter()
        .find(|(h, r)| match h {
            Hit::Day(_) => inside(&(r.0 - slack, r.1 - slack, r.2 + slack, r.3 + slack), x, y),
            _ => inside(r, x, y),
        })
        .map(|(h, _)| h)
}

fn level_ink(t: &Theme, l: u8) -> D2D1_COLOR_F {
    if l >= 3 {
        t.panel
    } else {
        t.text
    }
}

/// Small units sit on the digits' baseline: 26 px digits, 13 px muted units.
fn draw_parts(
    g: &Shared,
    dc: &ID2D1DeviceContext,
    f: &Fonts,
    t: &Theme,
    parts: &Parts,
    x: f32,
    y: f32,
) {
    let ink = brush(dc, t.text);
    let muted = brush(dc, t.muted);
    let mut x = x;
    for (s, unit) in parts {
        if *unit {
            let w = g.text_width(s, &f.body);
            text(dc, s, &f.body, x + 2.0, y + 11.5, w + 2.0, 16.0, &muted);
            x += w + 5.0;
        } else {
            let w = g.text_width(s, &f.big);
            text(dc, s, &f.big, x, y, w + 2.0, 30.0, &ink);
            x += w;
        }
    }
}

fn panel_bg(dc: &ID2D1DeviceContext, t: &Theme, w: f32, h: f32) {
    unsafe {
        dc.FillRoundedRectangle(&rrect(0.0, 0.0, w, h, 20.0), &brush(dc, t.panel));
        dc.DrawRoundedRectangle(
            &rrect(0.5, 0.5, w - 0.5, h - 0.5, 19.5),
            &brush(dc, t.hair),
            1.0,
            None,
        );
    }
}

fn header(
    g: &Shared,
    dc: &ID2D1DeviceContext,
    t: &Theme,
    f: &Fonts,
    v: &View,
    w: f32,
    hover: Option<Hit>,
) {
    text(
        dc,
        "History",
        &f.panel_title,
        PAD,
        PAD,
        120.0,
        32.0,
        &brush(dc, t.text),
    );
    let r = tabs_rect(w);
    unsafe {
        dc.FillRoundedRectangle(&rrect(r.0, r.1, r.2, r.3, 16.0), &brush(dc, t.raised));
        dc.DrawRoundedRectangle(
            &rrect(r.0 + 0.5, r.1 + 0.5, r.2 - 0.5, r.3 - 0.5, 15.5),
            &brush(dc, t.hair),
            1.0,
            None,
        );
    }
    for tab in [Tab::Drink, Tab::Focus] {
        let tr = tab_rect(w, tab);
        let on = v.tab == tab;
        unsafe {
            if on {
                dc.FillRoundedRectangle(
                    &rrect(tr.0, tr.1, tr.2, tr.3, 14.0),
                    &brush(dc, if t.dark { rgba(0x2A2A2A, 1.0) } else { t.panel }),
                );
                dc.DrawRoundedRectangle(
                    &rrect(tr.0 + 0.5, tr.1 + 0.5, tr.2 - 0.5, tr.3 - 0.5, 13.5),
                    &brush(dc, t.hair),
                    1.0,
                    None,
                );
            } else if hover == Some(Hit::Tab(tab)) {
                dc.FillRoundedRectangle(&rrect(tr.0, tr.1, tr.2, tr.3, 14.0), &brush(dc, t.button));
            }
        }
        let name = if tab == Tab::Drink { "Drink" } else { "Focus" };
        center_text(
            g,
            dc,
            name,
            &f.title,
            (tr.0 + tr.2) / 2.0,
            tr.1,
            28.0,
            &brush(dc, if on { t.text } else { t.muted }),
        );
    }
}

fn round_button(
    g: &Shared,
    dc: &ID2D1DeviceContext,
    t: &Theme,
    r: &Rect,
    icon: Icon,
    on: bool,
    hov: bool,
) {
    let (cx, cy) = ((r.0 + r.2) / 2.0, (r.1 + r.3) / 2.0);
    unsafe {
        if on {
            dc.FillEllipse(
                &ellipse(cx, cy, 14.0),
                &brush(dc, if hov { t.button_hover } else { t.button }),
            );
        }
    }
    draw_icon(
        g,
        dc,
        icon,
        cx,
        cy,
        14.0,
        &brush(dc, if on { t.text } else { t.track }),
    );
}

pub fn draw(
    g: &Shared,
    dc: &ID2D1DeviceContext,
    t: &Theme,
    f: &Fonts,
    v: &View,
    detail: Option<(usize, Detail)>,
    hover: Option<Hit>,
) {
    let (w, h) = size(v);
    panel_bg(dc, t, w, h);
    header(g, dc, t, f, v, w, hover);
    let rs = rects(g, f, v);
    let rect_of = |hit: Hit| rs.iter().find(|(x, _)| *x == hit).map(|(_, r)| *r);
    if v.body.compact() {
        compact(
            g,
            dc,
            t,
            f,
            v,
            rect_of(Hit::Action),
            hover == Some(Hit::Action),
        );
    } else {
        controls(g, dc, t, f, v, &rs, hover);
        if let Body::Full { stats, .. } = &v.body {
            stats_band(g, dc, t, f, w, stats);
        }
        match v.span {
            Span::Month => {
                month_grid(g, dc, t, f, v);
                legend(g, dc, t, f, v, w, month_grid_bottom() + 12.0);
                by_task(g, dc, t, f, v, month_grid_bottom() + 12.0 + 16.0 + 16.0);
            }
            Span::Year => {
                year_grid(g, dc, t, f, v);
                let ly = YEAR_Y + 7.0 * YEAR_STEP - 2.0 + 12.0;
                legend(g, dc, t, f, v, w, ly);
                key_hint(g, dc, t, f, ly + 16.0 + 10.0);
            }
        }
    }
    if let Some((i, d)) = detail {
        let cr = cell_rects(v);
        if let Some(r) = cr.get(i) {
            let side = !v.body.compact() && v.span == Span::Year;
            tooltip(g, dc, t, f, &d, r, side, w, h);
        }
    }
}

fn controls(
    g: &Shared,
    dc: &ID2D1DeviceContext,
    t: &Theme,
    f: &Fonts,
    v: &View,
    rs: &[(Hit, Rect)],
    hover: Option<Hit>,
) {
    for (hit, r) in rs {
        match hit {
            Hit::Prev => round_button(
                g,
                dc,
                t,
                r,
                Icon::ChevronLeft,
                v.can_prev,
                hover == Some(*hit),
            ),
            Hit::Next => round_button(
                g,
                dc,
                t,
                r,
                Icon::ChevronRight,
                v.can_next,
                hover == Some(*hit),
            ),
            Hit::Span(s) => {
                let on = v.span == *s;
                let fill = if on {
                    Some(t.button)
                } else if hover == Some(*hit) {
                    Some(t.raised)
                } else {
                    None
                };
                if let Some(c) = fill {
                    unsafe {
                        dc.FillRoundedRectangle(&rrect(r.0, r.1, r.2, r.3, 14.0), &brush(dc, c));
                    }
                }
                let name = if *s == Span::Month { "Month" } else { "Year" };
                center_text(
                    g,
                    dc,
                    name,
                    &f.small_medium,
                    (r.0 + r.2) / 2.0,
                    r.1,
                    28.0,
                    &brush(dc, if on { t.text } else { t.muted }),
                );
            }
            _ => {}
        }
    }
    text(
        dc,
        &v.range,
        &f.title,
        PAD + 28.0 + 4.0 + 6.0,
        60.0,
        300.0,
        28.0,
        &brush(dc, t.text),
    );
}

fn stats_band(
    g: &Shared,
    dc: &ID2D1DeviceContext,
    t: &Theme,
    f: &Fonts,
    w: f32,
    stats: &[crate::history_model::Stat],
) {
    let hair = brush(dc, t.hair);
    unsafe {
        dc.DrawLine(
            pt(PAD, STATS_Y + 0.5),
            pt(w - PAD, STATS_Y + 0.5),
            &hair,
            1.0,
            None,
        );
        dc.DrawLine(
            pt(PAD, STATS_B - 0.5),
            pt(w - PAD, STATS_B - 0.5),
            &hair,
            1.0,
            None,
        );
    }
    let cw = (w - 2.0 * PAD) / stats.len().max(1) as f32;
    for (i, s) in stats.iter().enumerate() {
        let x0 = PAD + cw * i as f32;
        let x = if i > 0 {
            unsafe {
                dc.DrawLine(
                    pt(x0 + 0.5, STATS_Y + 12.0),
                    pt(x0 + 0.5, STATS_B - 12.0),
                    &hair,
                    1.0,
                    None,
                );
            }
            x0 + 12.0
        } else {
            x0
        };
        draw_parts(g, dc, f, t, &s.value, x, STATS_Y + 12.0);
        text(
            dc,
            &s.key,
            &f.small,
            x,
            STATS_Y + 46.0,
            cw - 14.0,
            16.0,
            &brush(dc, t.text),
        );
        text(
            dc,
            &s.sub,
            &f.small,
            x,
            STATS_Y + 62.0,
            cw - 14.0,
            16.0,
            &brush(dc, t.muted),
        );
    }
}

/// One day square. `small` is the year's 10 px square (no number, tighter rings).
fn cell(
    g: &Shared,
    dc: &ID2D1DeviceContext,
    t: &Theme,
    f: &Fonts,
    c: &CellView,
    r: &Rect,
    rad: f32,
    number: bool,
    focused: bool,
) {
    let (cx, cy) = ((r.0 + r.2) / 2.0, (r.1 + r.3) / 2.0);
    let small = r.2 - r.0 < 16.0;
    let (ink, show) = unsafe {
        match c.kind {
            Cell::Level(l) => {
                dc.FillRoundedRectangle(
                    &rrect(r.0, r.1, r.2, r.3, rad),
                    &brush(dc, t.heat[l as usize]),
                );
                (level_ink(t, l), true)
            }
            Cell::Before => {
                dc.DrawRoundedRectangle(
                    &rrect(r.0 + 0.5, r.1 + 0.5, r.2 - 0.5, r.3 - 0.5, rad - 0.5),
                    &brush(
                        dc,
                        if t.dark {
                            rgba(0xFFFFFF, 0.09)
                        } else {
                            rgba(0x000000, 0.09)
                        },
                    ),
                    1.0,
                    None,
                );
                (t.muted, true)
            }
            Cell::Ahead if small => return,
            Cell::Ahead => {
                dc.DrawRoundedRectangle(
                    &rrect(r.0 + 0.75, r.1 + 0.75, r.2 - 0.75, r.3 - 0.75, rad - 0.75),
                    &brush(dc, t.passed),
                    1.5,
                    &dotted_stroke(g),
                );
                (t.muted, true)
            }
            Cell::Loading(alt) => {
                dc.FillRoundedRectangle(
                    &rrect(r.0, r.1, r.2, r.3, rad),
                    &brush(dc, if alt { t.button } else { t.raised }),
                );
                (t.muted, false)
            }
        }
    };
    if number && show {
        let fmt = if c.today { &f.small_medium } else { &f.small };
        center_text(
            g,
            dc,
            &c.date.day().to_string(),
            fmt,
            cx,
            cy - 8.0,
            16.0,
            &brush(dc, ink),
        );
    }
    let ring = |width: f32, off: f32| unsafe {
        let o = off + width / 2.0;
        dc.DrawRoundedRectangle(
            &rrect(r.0 - o, r.1 - o, r.2 + o, r.3 + o, rad + o),
            &brush(dc, t.text),
            width,
            None,
        );
    };
    let off = if small { 1.0 } else { 2.0 };
    if focused {
        ring(2.0, off + if small { 0.5 } else { 0.0 });
    } else if c.today {
        ring(1.5, off);
    }
}

fn weekday_row(
    g: &Shared,
    dc: &ID2D1DeviceContext,
    t: &Theme,
    f: &Fonts,
    x0: f32,
    cw: f32,
    gap: f32,
    y: f32,
) {
    let muted = brush(dc, t.muted);
    for (i, d) in ["M", "T", "W", "T", "F", "S", "S"].iter().enumerate() {
        center_text(
            g,
            dc,
            d,
            &f.small,
            x0 + i as f32 * (cw + gap) + cw / 2.0,
            y,
            16.0,
            &muted,
        );
    }
}

fn month_grid(g: &Shared, dc: &ID2D1DeviceContext, t: &Theme, f: &Fonts, v: &View) {
    weekday_row(g, dc, t, f, PAD, month_cell(), GAP, STATS_B + 14.0);
    for (i, (c, r)) in v.cells.iter().zip(cell_rects(v)).enumerate() {
        cell(g, dc, t, f, c, &r, 8.0, true, v.focus == Some(i));
    }
}

fn year_grid(g: &Shared, dc: &ID2D1DeviceContext, t: &Theme, f: &Fonts, v: &View) {
    let muted = brush(dc, t.muted);
    for (col, name) in &v.months {
        text(
            dc,
            name,
            &f.small,
            YEAR_X + *col as f32 * YEAR_STEP,
            STATS_B + 14.0,
            40.0,
            16.0,
            &muted,
        );
    }
    for (row, name) in [(0, "Mon"), (2, "Wed"), (4, "Fri")] {
        text(
            dc,
            name,
            &f.small,
            PAD,
            YEAR_Y + row as f32 * YEAR_STEP + YEAR_CELL / 2.0 - 8.0,
            28.0,
            16.0,
            &muted,
        );
    }
    for (i, (c, r)) in v.cells.iter().zip(cell_rects(v)).enumerate() {
        cell(g, dc, t, f, c, &r, 3.0, false, v.focus == Some(i));
    }
}

fn legend(g: &Shared, dc: &ID2D1DeviceContext, t: &Theme, f: &Fonts, v: &View, w: f32, y: f32) {
    let muted = brush(dc, t.muted);
    text(dc, &v.legend, &f.small, PAD, y, w / 2.0, 16.0, &muted);
    let top_w = g.text_width(v.legend_top, &f.small);
    let mut x = w - PAD - top_w;
    text(dc, v.legend_top, &f.small, x, y, top_w + 2.0, 16.0, &muted);
    x -= 4.0 + 5.0 * 13.0 - 3.0;
    for (i, c) in t.heat.iter().enumerate() {
        let sx = x + i as f32 * 13.0;
        unsafe {
            dc.FillRoundedRectangle(
                &rrect(sx, y + 3.0, sx + 10.0, y + 13.0, 3.0),
                &brush(dc, *c),
            );
        }
    }
    let less_w = g.text_width("Less", &f.small);
    text(
        dc,
        "Less",
        &f.small,
        x - 4.0 - less_w,
        y,
        less_w + 2.0,
        16.0,
        &muted,
    );
}

fn by_task(g: &Shared, dc: &ID2D1DeviceContext, t: &Theme, f: &Fonts, v: &View, y: f32) {
    let Body::Full {
        by_task: Some(b), ..
    } = &v.body
    else {
        return;
    };
    let (ink, muted, hair) = (brush(dc, t.text), brush(dc, t.muted), brush(dc, t.hair));
    text(dc, &b.label, &f.small, PAD, y, 220.0, 16.0, &muted);
    crate::ui::text_num(g, dc, &b.total, &f.small, MONTH_W - PAD, y, 16.0, &muted);
    let top = y + 18.0;
    for (i, row) in b.rows.iter().enumerate() {
        let ry = top + i as f32 * ROW_H;
        if i + 1 < b.rows.len() {
            unsafe {
                dc.DrawLine(
                    pt(PAD, ry + ROW_H - 0.5),
                    pt(MONTH_W - PAD, ry + ROW_H - 0.5),
                    &hair,
                    1.0,
                    None,
                );
            }
        }
        let (cx, cy) = (PAD + 4.0 + 12.0, ry + ROW_H / 2.0);
        unsafe {
            dc.FillRoundedRectangle(
                &rrect(cx - 12.0, cy - 12.0, cx + 12.0, cy + 12.0, 7.0),
                &brush(dc, t.button),
            );
            dc.DrawRoundedRectangle(
                &rrect(cx - 11.5, cy - 11.5, cx + 11.5, cy + 11.5, 6.5),
                &hair,
                1.0,
                None,
            );
        }
        let named = row.name.is_some();
        draw_icon(
            g,
            dc,
            if named { Icon::Check } else { Icon::Focus },
            cx,
            cy,
            14.0,
            &ink,
        );
        let value = crate::history_model::minutes_text(row.minutes);
        let vx = MONTH_W - PAD - 4.0;
        crate::ui::text_num(g, dc, &value, &f.body, vx, ry, ROW_H, &ink);
        let bar_r = vx - 76.0 - crate::ui::NUM_COL_GAP;
        let bar_l = bar_r - 64.0;
        unsafe {
            dc.FillRoundedRectangle(
                &rrect(bar_l, cy - 2.0, bar_r, cy + 2.0, 2.0),
                &brush(dc, t.button),
            );
            dc.FillRoundedRectangle(
                &rrect(
                    bar_l,
                    cy - 2.0,
                    bar_l + (64.0 * row.share).max(4.0),
                    cy + 2.0,
                    2.0,
                ),
                &ink,
            );
        }
        let name = row.name.clone().unwrap_or_else(|| "No task".into());
        let nx = cx + 12.0 + 10.0;
        text(
            dc,
            &ellipsize(g, &name, &f.body, bar_l - 10.0 - nx),
            &f.body,
            nx,
            ry,
            bar_l - 10.0 - nx,
            ROW_H,
            if named { &ink } else { &muted },
        );
    }
}

fn ellipsize(g: &Shared, s: &str, fmt: &IDWriteTextFormat, max: f32) -> String {
    if g.text_width(s, fmt) <= max {
        return s.to_string();
    }
    let chars: Vec<char> = s.chars().collect();
    (1..chars.len())
        .rev()
        .map(|n| chars[..n].iter().collect::<String>().trim_end().to_string() + "…")
        .find(|c| g.text_width(c, fmt) <= max)
        .unwrap_or_default()
}

fn kbd(
    g: &Shared,
    dc: &ID2D1DeviceContext,
    t: &Theme,
    f: &Fonts,
    label: &str,
    x: f32,
    y: f32,
) -> f32 {
    let w = g.text_width(label, &f.small) + 12.0;
    unsafe {
        dc.FillRoundedRectangle(&rrect(x, y, x + w, y + 20.0, 6.0), &brush(dc, t.button));
        dc.DrawRoundedRectangle(
            &rrect(x + 0.5, y + 0.5, x + w - 0.5, y + 19.5, 5.5),
            &brush(dc, t.hair),
            1.0,
            None,
        );
    }
    center_text(
        g,
        dc,
        label,
        &f.small,
        x + w / 2.0,
        y + 2.0,
        16.0,
        &brush(dc, t.text),
    );
    w
}

fn key_hint(g: &Shared, dc: &ID2D1DeviceContext, t: &Theme, f: &Fonts, y: f32) {
    let muted = brush(dc, t.muted);
    let mut x = PAD;
    for (i, (keys, what)) in [
        (&["←", "→"][..], "a day"),
        (&["↑", "↓"][..], "a week"),
        (&["Page Up"][..], "a month"),
        (&["Home"][..], "today"),
    ]
    .iter()
    .enumerate()
    {
        if i > 0 {
            x += 6.0;
        }
        for k in *keys {
            x += kbd(g, dc, t, f, k, x, y) + 6.0;
        }
        let ww = g.text_width(what, &f.small);
        text(dc, what, &f.small, x, y + 2.0, ww + 2.0, 16.0, &muted);
        x += ww + 6.0;
    }
}

fn compact(
    g: &Shared,
    dc: &ID2D1DeviceContext,
    t: &Theme,
    f: &Fonts,
    v: &View,
    action: Option<Rect>,
    action_hover: bool,
) {
    let (ink, muted) = (brush(dc, t.text), brush(dc, t.muted));
    let lw = compact_text_w();
    let y = 62.0;
    match &v.body {
        Body::Empty {
            title,
            text: body,
            action: label,
        } => {
            text(dc, title, &f.title, PAD, y, lw, 18.0, &ink);
            text(dc, body, &f.small_wrap, PAD, y + 22.0, lw, 60.0, &muted);
            if let Some(r) = action {
                unsafe {
                    dc.FillRoundedRectangle(
                        &rrect(r.0, r.1, r.2, r.3, 16.0),
                        &brush(
                            dc,
                            if action_hover {
                                with_alpha(t.text, 0.86)
                            } else {
                                t.text
                            },
                        ),
                    );
                }
                center_text(
                    g,
                    dc,
                    label,
                    &f.title,
                    (r.0 + r.2) / 2.0,
                    r.1,
                    32.0,
                    &brush(dc, t.panel),
                );
            }
        }
        Body::FirstWeek { value, since, note } => {
            draw_parts(g, dc, f, t, value, PAD, y);
            text(dc, since, &f.small, PAD, y + 34.0, lw, 16.0, &ink);
            text(dc, note, &f.small_wrap, PAD, y + 60.0, lw, 80.0, &muted);
        }
        Body::Loading { done, total } => {
            draw_icon(g, dc, Icon::Refresh, PAD + 7.0, y + 9.0, 14.0, &ink);
            text(
                dc,
                "Syncing with your phone",
                &f.title,
                PAD + 22.0,
                y,
                lw - 22.0,
                18.0,
                &ink,
            );
            text(
                dc,
                "This PC's log is shown now. Days from your phone fill in as they arrive.",
                &f.small_wrap,
                PAD,
                y + 22.0,
                lw,
                48.0,
                &muted,
            );
            let by = y + 22.0 + 48.0 + 12.0;
            let share = *done as f32 / (*total).max(1) as f32;
            unsafe {
                dc.FillRoundedRectangle(
                    &rrect(PAD, by, PAD + lw, by + 4.0, 2.0),
                    &brush(dc, t.button),
                );
                dc.FillRoundedRectangle(
                    &rrect(PAD, by, PAD + (lw * share).max(4.0), by + 4.0, 2.0),
                    &ink,
                );
            }
            text(
                dc,
                &format!("{done} of {total} days"),
                &f.small,
                PAD,
                by + 10.0,
                lw,
                16.0,
                &muted,
            );
            unsafe {
                let sk = brush(dc, t.button);
                dc.FillRoundedRectangle(&rrect(PAD, by + 40.0, PAD + 72.0, by + 52.0, 6.0), &sk);
                dc.FillRoundedRectangle(
                    &rrect(PAD + 82.0, by + 40.0, PAD + 138.0, by + 52.0, 6.0),
                    &sk,
                );
            }
        }
        Body::Full { .. } => {}
    }
    let c = (SMALL_W - 6.0 * SMALL_GAP) / 7.0;
    weekday_row(g, dc, t, f, COMPACT_W - PAD - SMALL_W, c, SMALL_GAP, y);
    for (i, (cv, r)) in v.cells.iter().zip(cell_rects(v)).enumerate() {
        cell(g, dc, t, f, cv, &r, 6.0, true, v.focus == Some(i));
    }
}

/// The day detail: above a month square (its caret pointing down), or beside a year square.
fn tooltip(
    g: &Shared,
    dc: &ID2D1DeviceContext,
    t: &Theme,
    f: &Fonts,
    d: &Detail,
    cell: &Rect,
    side: bool,
    pw: f32,
    ph: f32,
) {
    let value_w = g.text_width(&d.value, &f.body)
        + if d.note.is_empty() {
            0.0
        } else {
            4.0 + g.text_width(&d.note, &f.body)
        };
    let mut w = g.text_width(&d.title, &f.title).max(value_w);
    if !d.line.is_empty() {
        w = w.max(g.text_width(&d.line, &f.small));
    }
    for (a, b) in &d.rows {
        w = w.max(g.text_width(a, &f.small) + 16.0 + g.text_width(b, &f.small));
    }
    let w = (w + 26.0).ceil();
    let mut h = 10.0 + 16.0 + 4.0 + 18.0 + 10.0;
    if !d.line.is_empty() {
        h += 2.0 + 16.0;
    }
    if !d.rows.is_empty() {
        h += 16.0 + d.rows.len() as f32 * 16.0 + (d.rows.len() as f32 - 1.0) * 4.0;
    }
    let (cx, cy) = ((cell.0 + cell.2) / 2.0, (cell.1 + cell.3) / 2.0);
    // Where the box goes and where its caret points.
    let (x, y, caret) = if side {
        let y = (cy - h / 2.0).clamp(6.0, ph - h - 6.0);
        if cell.0 - 8.0 - w >= 6.0 {
            (
                cell.0 - 8.0 - w,
                y,
                [
                    pt(cell.0 - 8.0, cy - 5.0),
                    pt(cell.0 - 3.0, cy),
                    pt(cell.0 - 8.0, cy + 5.0),
                ],
            )
        } else {
            (
                cell.2 + 8.0,
                y,
                [
                    pt(cell.2 + 8.0, cy - 5.0),
                    pt(cell.2 + 3.0, cy),
                    pt(cell.2 + 8.0, cy + 5.0),
                ],
            )
        }
    } else {
        let x = (cx - w / 2.0).clamp(6.0, pw - w - 6.0);
        if cell.1 - 8.0 - h >= 6.0 {
            (
                x,
                cell.1 - 8.0 - h,
                [
                    pt(cx - 5.0, cell.1 - 8.0),
                    pt(cx, cell.1 - 3.0),
                    pt(cx + 5.0, cell.1 - 8.0),
                ],
            )
        } else {
            (
                x,
                cell.3 + 8.0,
                [
                    pt(cx - 5.0, cell.3 + 8.0),
                    pt(cx, cell.3 + 3.0),
                    pt(cx + 5.0, cell.3 + 8.0),
                ],
            )
        }
    };
    let bg = brush(dc, t.surface);
    let hair = brush(dc, t.hair);
    unsafe {
        // A soft shadow: a few faint rings under the box.
        let shadow = brush(dc, rgba(0x000000, if t.dark { 0.10 } else { 0.025 }));
        for i in 1..=4 {
            let s = i as f32 * 2.0;
            dc.FillRoundedRectangle(
                &rrect(
                    x - s * 0.5,
                    y - s * 0.25 + 2.0,
                    x + w + s * 0.5,
                    y + h + s,
                    12.0 + s,
                ),
                &shadow,
            );
        }
        dc.FillRoundedRectangle(&rrect(x, y, x + w, y + h, 12.0), &bg);
        dc.DrawRoundedRectangle(
            &rrect(x + 0.5, y + 0.5, x + w - 0.5, y + h - 0.5, 11.5),
            &hair,
            1.0,
            None,
        );
        let tri = path(g, caret[0], true, |k| {
            line_to(k, caret[1]);
            line_to(k, caret[2]);
        });
        dc.FillGeometry(&tri, &bg, None);
        let edge = path(g, caret[0], false, |k| {
            line_to(k, caret[1]);
            line_to(k, caret[2]);
        });
        dc.DrawGeometry(&edge, &hair, 1.0, None);
    }
    let (ink, muted) = (brush(dc, t.text), brush(dc, t.muted));
    let (tx, mut ty) = (x + 12.0, y + 10.0);
    text(dc, &d.title, &f.title, tx, ty, w - 24.0, 16.0, &ink);
    ty += 20.0;
    let vw = g.text_width(&d.value, &f.body);
    text(
        dc,
        &d.value,
        &f.body,
        tx,
        ty,
        vw + 2.0,
        18.0,
        if d.quiet { &muted } else { &ink },
    );
    if !d.note.is_empty() {
        text(dc, &d.note, &f.body, tx + vw + 4.0, ty, w, 18.0, &muted);
    }
    ty += 18.0;
    if !d.line.is_empty() {
        text(dc, &d.line, &f.small, tx, ty + 2.0, w - 24.0, 16.0, &muted);
        ty += 18.0;
    }
    if !d.rows.is_empty() {
        unsafe {
            dc.DrawLine(
                pt(tx, ty + 8.5),
                pt(x + w - 12.0, ty + 8.5),
                &hair,
                1.0,
                None,
            );
        }
        ty += 16.0;
        for (a, b) in &d.rows {
            text(dc, a, &f.small, tx, ty, w - 24.0, 16.0, &ink);
            crate::ui::text_num(g, dc, b, &f.small, x + w - 12.0, ty, 16.0, &muted);
            ty += 20.0;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::history_model::{add_days, this_anchor, view, Ask, DrinkIn, Log};
    use app_habits::Date;

    fn log_and_ask(span: Span, tab: Tab, days: i64) -> (Log, Ask) {
        let today = Date::new(2026, 11, 6).unwrap();
        let drinks: Vec<DrinkIn> = (0..days)
            .map(|i| DrinkIn {
                at: add_days(today, -i).at(9, 0),
                ml: 2000,
                device: None,
            })
            .collect();
        (
            Log::new(&drinks, &[], 2000),
            Ask {
                tab,
                span,
                anchor: this_anchor(today),
                today,
                focus: None,
                loading: false,
                glass_ml: 250,
                focus_min: 25,
            },
        )
    }

    #[test]
    fn each_layout_has_its_width_and_every_square_its_place() {
        let (log, ask) = log_and_ask(Span::Month, Tab::Drink, 60);
        let v = view(&log, &ask);
        assert_eq!(size(&v).0, MONTH_W);
        let r = cell_rects(&v);
        assert_eq!(r.len(), 35);
        assert!(
            (r[6].2 - (MONTH_W - PAD)).abs() < 0.01,
            "the last column ends at the padding"
        );
        let (log, ask) = log_and_ask(Span::Year, Tab::Drink, 60);
        let v = view(&log, &ask);
        assert_eq!(size(&v).0, YEAR_W);
        let r = cell_rects(&v);
        assert!(r.last().unwrap().2 <= YEAR_W - PAD, "53 weeks fit");
        let (log, ask) = log_and_ask(Span::Year, Tab::Drink, 0);
        let v = view(&log, &ask);
        assert_eq!(size(&v), (COMPACT_W, COMPACT_H));
        assert!(cell_rects(&v).last().unwrap().3 <= COMPACT_H - PAD);
    }

    #[test]
    fn the_focus_month_grows_with_its_task_rows() {
        let today = Date::new(2026, 11, 6).unwrap();
        let sessions: Vec<_> = (0..10)
            .map(|i| crate::history_model::SessionIn {
                ended: add_days(today, -i).at(10, 0),
                minutes: 30,
                task: Some(format!("Task {}", i % 3)),
            })
            .collect();
        let log = Log::new(&[], &sessions, 2000);
        let ask = Ask {
            tab: Tab::Focus,
            span: Span::Month,
            anchor: this_anchor(today),
            today,
            focus: None,
            loading: false,
            glass_ml: 250,
            focus_min: 25,
        };
        let (w, h) = size(&view(&log, &ask));
        assert_eq!(
            (w, h),
            (
                MONTH_W,
                (month_grid_bottom() + 12.0 + 16.0 + 34.0 + 3.0 * ROW_H + PAD).ceil()
            )
        );
    }
}
