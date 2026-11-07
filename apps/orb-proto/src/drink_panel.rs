#![allow(clippy::too_many_arguments)] // drawing helpers take a device context plus a full set of geometry
//! The Drink panel (frame "v6.2 · 08"): the glass, quick buttons, today's log with Undo on the
//! latest entry, the last 7 days and the next reminder. Drawing and hit tests.

use windows::Win32::Graphics::Direct2D::*;

use crate::gfx::*;
use crate::ui::{draw_icon, with_alpha, Fonts, Icon, Theme};

pub const W: f32 = 340.0;
pub const MAX_ROWS: usize = 4;

#[derive(Clone, Debug, PartialEq)]
pub struct LogRow {
    pub time: String,
    pub ml: u32,
    pub source: String,
}

#[derive(Clone, Debug, PartialEq)]
pub struct Bar {
    pub label: String,
    pub ml: u32,
    pub today: bool,
}

#[derive(Clone, Debug, PartialEq)]
pub struct View {
    pub total: u32,
    pub goal: u32,
    pub to_go: String,
    pub pace: String,
    pub glasses: Vec<u32>,
    pub default_glass: u32,
    /// Today's entries, latest first, at most `MAX_ROWS`.
    pub rows: Vec<LogRow>,
    pub week: Vec<Bar>,
    pub next: String,
}

#[derive(Clone, Copy, PartialEq, Debug)]
pub enum Hit {
    Glass(u32),
    Undo,
    /// The last 7 days: opens History on Drink.
    Week,
}

type Rect = (f32, f32, f32, f32);

const ROWS_Y: f32 = 264.0;
const ROW_H: f32 = 36.0;
const BARS_H: f32 = 52.0;

fn rows_shown(v: &View) -> usize {
    v.rows.len().clamp(1, MAX_ROWS)
}

fn week_y(v: &View) -> f32 {
    ROWS_Y + ROW_H * rows_shown(v) as f32 + 14.0
}

pub fn height(v: &View) -> f32 {
    week_y(v) + 24.0 + BARS_H + 20.0 + 14.0 + 18.0 + 16.0
}

fn glass_label(ml: u32) -> String {
    format!("+{ml}")
}

fn rects(g: &Shared, f: &Fonts, v: &View) -> Vec<(Hit, Rect)> {
    let mut out = Vec::new();
    let mut x = 16.0;
    for ml in &v.glasses {
        let label = if *ml == v.default_glass {
            format!("+{ml} ml")
        } else {
            glass_label(*ml)
        };
        let w = g.text_width(&label, &f.title) + 28.0;
        out.push((Hit::Glass(*ml), (x, 196.0, x + w, 228.0)));
        x += w + 8.0;
    }
    if !v.rows.is_empty() {
        let w = g.text_width("Undo", &f.title) + 24.0;
        out.push((
            Hit::Undo,
            (W - 20.0 - w, ROWS_Y + 4.0, W - 20.0, ROWS_Y + 32.0),
        ));
    }
    let wy = week_y(v);
    out.push((Hit::Week, (16.0, wy, W - 16.0, wy + 24.0 + BARS_H + 20.0)));
    out
}

pub fn hit(g: &Shared, f: &Fonts, v: &View, x: f32, y: f32) -> Option<Hit> {
    rects(g, f, v)
        .into_iter()
        .find(|(_, r)| x >= r.0 && x <= r.2 && y >= r.1 && y <= r.3)
        .map(|(h, _)| h)
}

/// The glass: tapered, the empty part a dotted outline (still to come), the water solid.
fn glass(g: &Shared, dc: &ID2D1DeviceContext, t: &Theme, fill: f32) {
    let (top, bottom) = (52.0, 176.0);
    let (tl, tr, bl, br) = (26.0, 94.0, 32.0, 88.0);
    let x_at = |y: f32, left: bool| {
        let k = (y - top) / (bottom - top);
        if left {
            tl + (bl - tl) * k
        } else {
            tr + (br - tr) * k
        }
    };
    let outline = path(g, pt(tl, top), false, |k| {
        line_to(k, pt(tr, top));
        line_to(k, pt(br, bottom - 6.0));
        arc_to(k, pt(br - 6.0, bottom), 6.0, false, true);
        line_to(k, pt(bl + 6.0, bottom));
        arc_to(k, pt(bl, bottom - 6.0), 6.0, false, true);
        line_to(k, pt(tl, top));
    });
    unsafe {
        dc.DrawGeometry(&outline, &brush(dc, t.passed), 1.5, &dotted_stroke(g));
    }
    let fill = fill.clamp(0.0, 1.0);
    if fill <= 0.0 {
        return;
    }
    let wy = bottom - (bottom - top) * fill;
    let water = path(g, pt(x_at(wy, true), wy), true, |k| {
        line_to(k, pt(x_at(wy, false), wy));
        line_to(k, pt(br, bottom - 6.0));
        arc_to(k, pt(br - 6.0, bottom), 6.0, false, true);
        line_to(k, pt(bl + 6.0, bottom));
        arc_to(k, pt(bl, bottom - 6.0), 6.0, false, true);
    });
    unsafe {
        dc.FillGeometry(&water, &brush(dc, t.water), None);
        dc.DrawLine(
            pt(x_at(wy, true) + 1.0, wy),
            pt(x_at(wy, false) - 1.0, wy),
            &brush(dc, t.meniscus),
            1.5,
            &round_stroke(g),
        );
    }
}

pub fn draw(
    g: &Shared,
    dc: &ID2D1DeviceContext,
    t: &Theme,
    f: &Fonts,
    v: &View,
    hover: Option<Hit>,
) {
    let text_b = brush(dc, t.text);
    let muted = brush(dc, t.muted);
    let hair = brush(dc, t.hair);
    let hgt = height(v);
    unsafe {
        dc.FillRoundedRectangle(&rrect(0.0, 0.0, W, hgt, 20.0), &brush(dc, t.panel));
        dc.DrawRoundedRectangle(&rrect(0.5, 0.5, W - 0.5, hgt - 0.5, 19.5), &hair, 1.0, None);
    }
    text(
        dc,
        "Drink",
        &f.panel_title,
        16.0,
        14.0,
        120.0,
        20.0,
        &text_b,
    );
    right_text(
        g,
        dc,
        &format!("Goal {} ml", v.goal),
        &f.small,
        W - 16.0,
        15.0,
        18.0,
        &muted,
    );

    glass(g, dc, t, v.total as f32 / v.goal.max(1) as f32);
    let total = v.total.to_string();
    let tw = g.text_width(&total, &f.big);
    text(dc, &total, &f.big, 118.0, 70.0, tw + 2.0, 36.0, &text_b);
    text(
        dc,
        &format!("/ {} ml", v.goal),
        &f.body,
        118.0 + tw + 6.0,
        78.0,
        140.0,
        30.0,
        &muted,
    );
    text(
        dc,
        &v.to_go,
        &f.small,
        118.0,
        114.0,
        W - 134.0,
        16.0,
        &muted,
    );
    text(dc, &v.pace, &f.small, 118.0, 130.0, W - 134.0, 16.0, &muted);

    let rs = rects(g, f, v);
    for (h, r) in &rs {
        let hov = hover == Some(*h);
        match h {
            Hit::Glass(ml) => {
                let primary = *ml == v.default_glass;
                let label = if primary {
                    format!("+{ml} ml")
                } else {
                    glass_label(*ml)
                };
                let fill = if primary {
                    if hov {
                        with_alpha(t.text, 0.86)
                    } else {
                        t.text
                    }
                } else if hov {
                    t.button_hover
                } else {
                    t.button
                };
                unsafe {
                    dc.FillRoundedRectangle(&rrect(r.0, r.1, r.2, r.3, 16.0), &brush(dc, fill));
                }
                center_text(
                    g,
                    dc,
                    &label,
                    &f.title,
                    (r.0 + r.2) / 2.0,
                    r.1,
                    32.0,
                    &brush(dc, if primary { t.panel } else { t.text }),
                );
            }
            Hit::Undo => {
                unsafe {
                    dc.FillRoundedRectangle(
                        &rrect(r.0, r.1, r.2, r.3, 14.0),
                        &brush(dc, if hov { t.button_hover } else { t.button }),
                    );
                }
                center_text(
                    g,
                    dc,
                    "Undo",
                    &f.title,
                    (r.0 + r.2) / 2.0,
                    r.1,
                    28.0,
                    &text_b,
                );
            }
            Hit::Week => {}
        }
    }

    // Today's log, latest first.
    text(dc, "Today", &f.small, 16.0, 244.0, 100.0, 16.0, &muted);
    if v.rows.is_empty() {
        text(
            dc,
            "Nothing logged yet today.",
            &f.body,
            20.0,
            ROWS_Y,
            W - 40.0,
            ROW_H,
            &muted,
        );
    }
    for (i, row) in v.rows.iter().take(MAX_ROWS).enumerate() {
        let y = ROWS_Y + ROW_H * i as f32;
        if i > 0 {
            unsafe {
                dc.DrawLine(pt(16.0, y), pt(W - 16.0, y), &hair, 1.0, None);
            }
        }
        text(dc, &row.time, &f.small, 20.0, y, 48.0, ROW_H, &muted);
        let amount = format!("{} ml", row.ml);
        let aw = g.text_width(&amount, &f.title);
        text(dc, &amount, &f.title, 70.0, y, aw + 2.0, ROW_H, &text_b);
        text(
            dc,
            &row.source,
            &f.body,
            70.0 + aw + 5.0,
            y,
            140.0,
            ROW_H,
            &muted,
        );
    }

    // The last 7 days: met days grey, short days track, today ink; the goal is a dashed line.
    let wy = week_y(v);
    let week_hover = hover == Some(Hit::Week);
    text(
        dc,
        "Last 7 days",
        &f.small,
        16.0,
        wy,
        120.0,
        16.0,
        if week_hover { &text_b } else { &muted },
    );
    right_text(
        g,
        dc,
        if week_hover {
            "Open History"
        } else {
            "dashed line is the goal"
        },
        &f.small,
        W - 16.0,
        wy,
        16.0,
        if week_hover { &text_b } else { &muted },
    );
    let bottom = wy + 24.0 + BARS_H;
    let n = v.week.len().max(1) as f32;
    let gap = 8.0;
    let bw = (W - 32.0 - gap * (n - 1.0)) / n;
    let scale = v
        .week
        .iter()
        .map(|b| b.ml)
        .max()
        .unwrap_or(0)
        .max(v.goal * 6 / 5)
        .max(1) as f32;
    for (i, b) in v.week.iter().enumerate() {
        let x = 16.0 + i as f32 * (bw + gap);
        let h = (b.ml as f32 / scale * BARS_H).max(3.0);
        let c = if b.today {
            t.text
        } else if b.ml >= v.goal {
            t.muted
        } else {
            t.track
        };
        unsafe {
            dc.FillRoundedRectangle(&rrect(x, bottom - h, x + bw, bottom, 3.0), &brush(dc, c));
        }
        let (fmt, ink) = if b.today {
            (&f.small_medium, &text_b)
        } else {
            (&f.small, &muted)
        };
        center_text(g, dc, &b.label, fmt, x + bw / 2.0, bottom + 4.0, 16.0, ink);
    }
    let gy = bottom - v.goal as f32 / scale * BARS_H;
    unsafe {
        dc.DrawLine(
            pt(16.0, gy),
            pt(W - 16.0, gy),
            &brush(dc, with_alpha(t.muted, 0.8)),
            1.0,
            &dashed_stroke(g),
        );
    }

    let fy = bottom + 20.0 + 14.0;
    draw_icon(g, dc, Icon::Clock, 24.0, fy + 9.0, 14.0, &muted);
    text(dc, &v.next, &f.small, 38.0, fy, W - 54.0, 18.0, &muted);
}

#[cfg(test)]
mod tests {
    use super::*;

    fn view(rows: usize) -> View {
        View {
            total: 750,
            goal: 2000,
            to_go: String::new(),
            pace: String::new(),
            glasses: vec![150, 250, 500],
            default_glass: 250,
            rows: (0..rows)
                .map(|_| LogRow {
                    time: "10:42".into(),
                    ml: 250,
                    source: "glass".into(),
                })
                .collect(),
            week: Vec::new(),
            next: String::new(),
        }
    }

    #[test]
    fn the_panel_grows_with_the_log_up_to_four_rows() {
        assert_eq!(height(&view(0)), height(&view(1)));
        assert_eq!(height(&view(3)) - height(&view(1)), 2.0 * ROW_H);
        assert_eq!(height(&view(4)), height(&view(9)));
    }
}
