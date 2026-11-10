#![allow(clippy::too_many_arguments)] // drawing helpers take a canvas plus a full set of geometry
//! The Today panel (frame "v6.2 · 09 Calendar + Today"): the date, all-day chips, the next
//! prayer banner, then one timeline of events, tasks and prayer markers around a now line.
//! Prayer markers are dashed rules with a name and a time: never a row, never a checkbox.

use crate::canvas::{
    clipped_text, ellipse, line_to, path, pt, rrect, text_width, Canvas, Color, Stroke,
};
use crate::ui::{lip, num_text, paint_icon, with_alpha, Fonts, Icon, Theme};

pub const W: f32 = 372.0;
/// Past this the timeline scrolls inside the panel.
pub const MAX_H: f32 = 620.0;
const X0: f32 = 16.0;
const X1: f32 = W - 16.0;
const FOOT: f32 = 12.0 + 32.0 + 16.0;

#[derive(Clone, Debug, PartialEq)]
pub enum Line {
    /// Ended events and done tasks, folded. `label` counts tasks only.
    Earlier {
        label: String,
        open: bool,
    },
    Now(String),
    Event {
        title: String,
        sub: String,
        time: String,
        join: bool,
        past: bool,
    },
    Task {
        title: String,
        sub: String,
        time: String,
        done: bool,
        overdue: bool,
    },
    Marker {
        name: String,
        time: String,
    },
    /// A muted line, e.g. "No events or tasks left today".
    Quiet(String),
}

impl Line {
    fn h(&self) -> f32 {
        match self {
            Line::Now(_) => 22.0,
            Line::Marker { .. } => 30.0,
            _ => 36.0,
        }
    }

    /// Up and Down stop on events, tasks and the Earlier row; markers are not stops.
    pub fn is_stop(&self) -> bool {
        matches!(
            self,
            Line::Earlier { .. } | Line::Event { .. } | Line::Task { .. }
        )
    }
}

/// A state card (not connected, loading, offline, link broken). A button without a hit is
/// shown disabled ("Google · coming soon").
#[derive(Clone, Debug, PartialEq)]
pub struct Card {
    pub icon: Icon,
    pub title: String,
    pub body: Vec<String>,
    pub buttons: Vec<(Option<Hit>, String)>,
}

pub struct View {
    pub date: String,
    /// (title, calendar name)
    pub all_day: Vec<(String, String)>,
    /// (prayer name, "in 1 h 6 min", "11:26")
    pub banner: (String, String, String),
    pub card: Option<Card>,
    pub lines: Vec<Line>,
    pub footer: String,
    pub scroll: f32,
    pub focus: Option<usize>,
}

#[derive(Clone, Copy, PartialEq, Debug)]
pub enum Hit {
    Earlier,
    Row(usize),
    Check(usize),
    Join(usize),
    AddTask,
    Paste,
    Retry,
}

type Rect = (f32, f32, f32, f32);

fn inside(r: &Rect, x: f32, y: f32) -> bool {
    x >= r.0 && x <= r.2 && y >= r.1 && y <= r.3
}

struct Geo {
    all_day: f32,
    banner: f32,
    card: f32,
    list_top: f32,
    list_bottom: f32,
    h: f32,
}

fn card_h(c: &Card) -> f32 {
    12.0 + 18.0
        + 4.0
        + 16.0 * c.body.len() as f32
        + if c.buttons.is_empty() {
            0.0
        } else {
            10.0 + 28.0
        }
        + 12.0
}

fn content_h(v: &View) -> f32 {
    v.lines.iter().map(Line::h).sum()
}

fn geo(v: &View) -> Geo {
    let mut y = 34.0;
    let all_day = y + 10.0;
    if !v.all_day.is_empty() {
        y = all_day + 24.0;
    }
    let banner = y + 12.0;
    y = banner + 54.0;
    let card = y + 8.0;
    if let Some(c) = &v.card {
        y = card + card_h(c);
    }
    let list_top = y + 8.0;
    let h = (list_top + content_h(v) + FOOT).min(MAX_H).ceil();
    Geo {
        all_day,
        banner,
        card,
        list_top,
        list_bottom: h - FOOT,
        h,
    }
}

pub fn height(v: &View) -> f32 {
    geo(v).h
}

pub fn max_scroll(v: &View) -> f32 {
    let g = geo(v);
    (content_h(v) - (g.list_bottom - g.list_top)).max(0.0)
}

/// The scroll that keeps line `i` in view, moving as little as possible.
pub fn reveal(v: &View, i: usize) -> f32 {
    let g = geo(v);
    let top: f32 = v.lines[..i].iter().map(Line::h).sum();
    let h = v.lines[i].h();
    let view_h = g.list_bottom - g.list_top;
    v.scroll
        .min(top)
        .max(top + h - view_h)
        .clamp(0.0, max_scroll(v))
}

fn line_tops(v: &View) -> Vec<(f32, f32)> {
    let g = geo(v);
    let mut y = g.list_top - v.scroll;
    v.lines
        .iter()
        .map(|l| {
            let r = (y, l.h());
            y += l.h();
            r
        })
        .collect()
}

fn join_rect(f: &Fonts, time: &str, top: f32, h: f32) -> Rect {
    let tw = text_width(time, &f.small);
    let bw = 12.0 + 12.0 + 6.0 + text_width("Join", &f.small_medium) + 12.0;
    let right = X1 - 8.0 - tw - 8.0;
    let cy = top + h / 2.0;
    (right - bw, cy - 14.0, right, cy + 14.0)
}

fn add_rect(f: &Fonts, v: &View) -> Rect {
    let top = geo(v).list_bottom + 12.0;
    let w = 12.0 + 14.0 + 6.0 + text_width("Add task", &f.title) + 14.0;
    (X0, top, X0 + w, top + 32.0)
}

fn card_buttons(f: &Fonts, v: &View) -> Vec<(Option<Hit>, Rect)> {
    let Some(c) = &v.card else { return Vec::new() };
    let top = geo(v).card + card_h(c) - 12.0 - 28.0;
    let mut x = X0 + 14.0;
    c.buttons
        .iter()
        .map(|(hit, label)| {
            let w = text_width(label, &f.small_medium) + 24.0;
            let r = (x, top, x + w, top + 28.0);
            x += w + 8.0;
            (*hit, r)
        })
        .collect()
}

fn layout(f: &Fonts, v: &View) -> Vec<(Hit, Rect)> {
    let geo = geo(v);
    let mut out: Vec<(Hit, Rect)> = Vec::new();
    for (hit, r) in card_buttons(f, v) {
        if let Some(h) = hit {
            out.push((h, r));
        }
    }
    out.push((Hit::AddTask, add_rect(f, v)));
    for (i, ((top, h), line)) in line_tops(v).into_iter().zip(&v.lines).enumerate() {
        if top + h <= geo.list_top || top >= geo.list_bottom {
            continue;
        }
        let row = (
            X0,
            top.max(geo.list_top),
            X1,
            (top + h).min(geo.list_bottom),
        );
        match line {
            Line::Earlier { .. } => out.push((Hit::Earlier, row)),
            Line::Event { join, time, .. } => {
                if *join {
                    out.push((Hit::Join(i), join_rect(f, time, top, h)));
                }
                out.push((Hit::Row(i), row));
            }
            Line::Task { .. } => {
                out.push((Hit::Check(i), (X0, row.1, X0 + 40.0, row.3)));
                out.push((Hit::Row(i), row));
            }
            _ => {}
        }
    }
    out
}

pub fn hit(f: &Fonts, v: &View, x: f32, y: f32) -> Option<Hit> {
    layout(f, v)
        .into_iter()
        .find(|(_, r)| inside(r, x, y))
        .map(|(h, _)| h)
}

fn button(
    cv: &mut dyn Canvas,
    t: &Theme,
    f: &Fonts,
    r: Rect,
    label: &str,
    enabled: bool,
    hover: bool,
) {
    if enabled {
        cv.fill_rrect(
            &rrect(r.0, r.1, r.2, r.3, 14.0),
            if hover { t.button_hover } else { t.button },
        );
        lip(cv, t, r.0, r.1, r.2, r.3, 14.0, 1.0);
    } else {
        cv.stroke_rrect(
            &rrect(r.0 + 0.5, r.1 + 0.5, r.2 - 0.5, r.3 - 0.5, 13.5),
            t.hair,
            1.0,
            Stroke::Plain,
        );
    }
    let ink = if enabled { t.text } else { t.muted };
    cv.center_text(
        label,
        &f.small_medium,
        (r.0 + r.2) / 2.0,
        r.1,
        r.3 - r.1,
        ink,
    );
}

fn checkbox(cv: &mut dyn Canvas, t: &Theme, cx: f32, cy: f32, done: bool, hover: bool) {
    let (l, top) = (cx - 8.0, cy - 8.0);
    if done {
        cv.fill_rrect(&rrect(l, top, l + 16.0, top + 16.0, 5.0), t.muted);
        let tick = path(pt(l + 4.0, top + 8.5), false, |k| {
            line_to(k, pt(l + 7.0, top + 11.3));
            line_to(k, pt(l + 12.0, top + 5.6));
        });
        cv.stroke_path(&tick, t.panel, 1.6, Stroke::Round);
    } else {
        // 3:1 against the panel: .45 ink in white, .5 in dark.
        let ring = if hover {
            t.text
        } else {
            with_alpha(t.text, if t.dark { 0.5 } else { 0.45 })
        };
        cv.stroke_rrect(
            &rrect(l + 0.75, top + 0.75, l + 15.25, top + 15.25, 4.5),
            ring,
            1.5,
            Stroke::Plain,
        );
    }
}

fn draw_lines(cv: &mut dyn Canvas, t: &Theme, f: &Fonts, v: &View, hover: Option<Hit>) {
    let geo = geo(v);
    let ink = t.text;
    let muted = t.muted;
    cv.push_clip(0.0, geo.list_top, W, geo.list_bottom);
    let mut shown = 0;
    for (i, ((top, h), line)) in line_tops(v).into_iter().zip(&v.lines).enumerate() {
        if top + h < geo.list_top || top > geo.list_bottom {
            continue;
        }
        // Rows ripple in when the panel opens, top row first (motion.rs).
        let fx = crate::motion::ripple_row("today.rows", shown);
        shown += 1;
        crate::motion::row(cv, fx, |cv| {
            let cy = top + h / 2.0;
            let row_hover = match hover {
                Some(Hit::Row(j) | Hit::Check(j) | Hit::Join(j)) => j == i,
                Some(Hit::Earlier) => matches!(line, Line::Earlier { .. }),
                _ => false,
            };
            if row_hover && line.is_stop() {
                cv.fill_rrect(&rrect(X0, top, X1, top + h, 8.0), t.raised);
            }
            if v.focus == Some(i) {
                // A 2 px ink ring with a 2 px panel-coloured gap.
                cv.stroke_rrect(
                    &rrect(X0 - 3.0, top - 3.0, X1 + 3.0, top + h + 3.0, 11.0),
                    ink,
                    2.0,
                    Stroke::Plain,
                );
            }
            match line {
                Line::Earlier { label, open } => {
                    paint_icon(
                        cv,
                        if *open {
                            Icon::ChevronDown
                        } else {
                            Icon::ChevronRight
                        },
                        X0 + 20.0,
                        cy,
                        14.0,
                        muted,
                    );
                    let tw = text_width("Earlier today", &f.body);
                    cv.text("Earlier today", &f.body, X0 + 42.0, top, tw + 2.0, h, ink);
                    cv.text(
                        label,
                        &f.body,
                        X0 + 48.0 + tw,
                        top,
                        X1 - X0 - 60.0 - tw,
                        h,
                        muted,
                    );
                }
                Line::Now(label) => {
                    let tw = text_width(label, &f.small_medium);
                    cv.text(label, &f.small_medium, X0 + 8.0, top, tw + 2.0, h, ink);
                    let x = X0 + 16.0 + tw;
                    cv.line(pt(x, cy), pt(X1 - 8.0, cy), ink, 1.5, Stroke::Plain);
                    cv.fill_ellipse(&ellipse(x + 3.5, cy, 3.5), ink);
                }
                Line::Event {
                    title,
                    sub,
                    time,
                    join,
                    past,
                } => {
                    let chip = rrect(X0 + 8.0, cy - 12.0, X0 + 32.0, cy + 12.0, 7.0);
                    cv.fill_rrect(&chip, t.button);
                    cv.stroke_rrect(
                        &rrect(X0 + 8.5, cy - 11.5, X0 + 31.5, cy + 11.5, 6.5),
                        t.hair,
                        1.0,
                        Stroke::Plain,
                    );
                    paint_icon(cv, Icon::Today, X0 + 20.0, cy, 12.0, muted);
                    let tw = text_width(time, &f.small);
                    num_text(cv, time, &f.small, X1 - 8.0, top, h, muted);
                    let mut lane = X1 - 8.0 - tw - 10.0;
                    if *join {
                        let r = join_rect(f, time, top, h);
                        let fill = if hover == Some(Hit::Join(i)) {
                            with_alpha(t.text, 0.85)
                        } else {
                            t.text
                        };
                        cv.fill_rrect(&rrect(r.0, r.1, r.2, r.3, 14.0), fill);
                        let on = t.panel;
                        paint_icon(cv, Icon::Video, r.0 + 18.0, cy, 12.0, on);
                        cv.text(
                            "Join",
                            &f.small_medium,
                            r.0 + 30.0,
                            r.1,
                            r.2 - r.0 - 30.0,
                            28.0,
                            on,
                        );
                        lane = r.0 - 8.0;
                    }
                    draw_title_sub(
                        cv,
                        f,
                        title,
                        sub,
                        X0 + 42.0,
                        lane,
                        top,
                        h,
                        if *past { muted } else { ink },
                        muted,
                    );
                }
                Line::Task {
                    title,
                    sub,
                    time,
                    done,
                    overdue,
                } => {
                    checkbox(cv, t, X0 + 20.0, cy, *done, hover == Some(Hit::Check(i)));
                    let fmt = if *overdue { &f.small_medium } else { &f.small };
                    let tw = text_width(time, fmt);
                    cv.right_text(
                        time,
                        fmt,
                        X1 - 8.0,
                        top,
                        h,
                        if *overdue { ink } else { muted },
                    );
                    draw_title_sub(
                        cv,
                        f,
                        title,
                        sub,
                        X0 + 42.0,
                        X1 - 8.0 - tw - 10.0,
                        top,
                        h,
                        if *done { muted } else { ink },
                        muted,
                    );
                }
                Line::Marker { name, time } => {
                    let nw = text_width(name, &f.small_medium);
                    let tw = text_width(time, &f.small);
                    let total = nw + 6.0 + tw;
                    let mid = (X0 + X1) / 2.0;
                    let (l, r) = (mid - total / 2.0 - 10.0, mid + total / 2.0 + 10.0);
                    let rule = t.track;
                    cv.line(pt(X0 + 8.0, cy), pt(l, cy), rule, 1.0, Stroke::Dashed);
                    cv.line(pt(r, cy), pt(X1 - 8.0, cy), rule, 1.0, Stroke::Dashed);
                    cv.text(
                        name,
                        &f.small_medium,
                        mid - total / 2.0,
                        top,
                        nw + 2.0,
                        h,
                        ink,
                    );
                    cv.text(
                        time,
                        &f.small,
                        mid - total / 2.0 + nw + 6.0,
                        top,
                        tw + 2.0,
                        h,
                        muted,
                    );
                }
                Line::Quiet(s) => cv.center_text(s, &f.small, (X0 + X1) / 2.0, top, h, muted),
            }
        });
    }
    cv.pop_clip();
    let ms = max_scroll(v);
    if ms > 0.0 {
        // A thin thumb says the timeline scrolls.
        let track = geo.list_bottom - geo.list_top;
        let th = (track * track / content_h(v)).max(24.0);
        let ty = geo.list_top + (track - th) * (v.scroll / ms);
        cv.fill_rrect(&rrect(W - 7.0, ty, W - 4.0, ty + th, 1.5), t.track);
    }
}

/// The title, then the muted second part on the same line ("Standup  Work · Meet"), cut to fit.
fn draw_title_sub(
    cv: &mut dyn Canvas,
    f: &Fonts,
    title: &str,
    sub: &str,
    x: f32,
    right: f32,
    top: f32,
    h: f32,
    ink: Color,
    muted: Color,
) {
    let avail = (right - x).max(0.0);
    let tw = text_width(title, &f.body).min(avail);
    clipped_text(cv, title, &f.body, x, top, tw, h, ink);
    if !sub.is_empty() && avail - tw > 40.0 {
        clipped_text(
            cv,
            sub,
            &f.body,
            x + tw + 6.0,
            top,
            avail - tw - 6.0,
            h,
            muted,
        );
    }
}

pub fn draw(cv: &mut dyn Canvas, t: &Theme, f: &Fonts, v: &View, hover: Option<Hit>) {
    let geo = geo(v);
    let ink = t.text;
    let muted = t.muted;
    let hair = t.hair;
    cv.fill_rrect(&rrect(0.0, 0.0, W, geo.h, 20.0), t.panel);
    cv.stroke_rrect(
        &rrect(0.5, 0.5, W - 0.5, geo.h - 0.5, 19.5),
        hair,
        1.0,
        Stroke::Plain,
    );
    cv.text("Today", &f.panel_title, X0, 16.0, 120.0, 18.0, ink);
    cv.right_text(&v.date, &f.small, X1, 17.0, 16.0, muted);

    if !v.all_day.is_empty() {
        let y = geo.all_day;
        let lw = text_width("All day", &f.small);
        cv.text("All day", &f.small, X0, y, lw + 2.0, 24.0, muted);
        let mut x = X0 + lw + 8.0;
        cv.push_clip(X0, y - 1.0, X1, y + 25.0);
        for (title, cal) in &v.all_day {
            let a = text_width(title, &f.small_medium);
            let b = if cal.is_empty() {
                0.0
            } else {
                6.0 + text_width(cal, &f.small)
            };
            let w = 10.0 + a + b + 10.0;
            cv.stroke_rrect(
                &rrect(x + 0.5, y + 0.5, x + w - 0.5, y + 23.5, 11.5),
                with_alpha(t.text, 0.14),
                1.0,
                Stroke::Plain,
            );
            cv.text(title, &f.small_medium, x + 10.0, y, a + 2.0, 24.0, ink);
            if !cal.is_empty() {
                cv.text(cal, &f.small, x + 16.0 + a, y, b + 2.0, 24.0, muted);
            }
            x += w + 6.0;
        }
        cv.pop_clip();
    }

    // The next prayer: a quiet banner, no button, no checkbox.
    let y = geo.banner;
    cv.fill_rrect(&rrect(X0, y, X1, y + 54.0, 12.0), t.raised);
    cv.stroke_rrect(
        &rrect(X0 + 0.5, y + 0.5, X1 - 0.5, y + 53.5, 11.5),
        hair,
        1.0,
        Stroke::Plain,
    );
    lip(cv, t, X0, y, X1, y + 54.0, 12.0, 1.0);
    let (name, when, at) = &v.banner;
    cv.text(
        "Next prayer",
        &f.small,
        X0 + 14.0,
        y + 10.0,
        120.0,
        16.0,
        muted,
    );
    let nw = text_width(name, &f.panel_title);
    cv.text(
        name,
        &f.panel_title,
        X0 + 14.0,
        y + 26.0,
        nw + 2.0,
        18.0,
        ink,
    );
    cv.text(when, &f.body, X0 + 20.0 + nw, y + 26.0, 160.0, 18.0, muted);
    num_text(cv, at, &f.big, X1 - 14.0, y + 12.0, 30.0, ink);

    if let Some(c) = &v.card {
        let y = geo.card;
        let h = card_h(c);
        cv.fill_rrect(&rrect(X0, y, X1, y + h, 12.0), t.raised);
        cv.stroke_rrect(
            &rrect(X0 + 0.5, y + 0.5, X1 - 0.5, y + h - 0.5, 11.5),
            hair,
            1.0,
            Stroke::Plain,
        );
        paint_icon(cv, c.icon, X0 + 22.0, y + 21.0, 16.0, ink);
        cv.text(
            &c.title,
            &f.title,
            X0 + 38.0,
            y + 12.0,
            X1 - X0 - 52.0,
            18.0,
            ink,
        );
        for (k, line) in c.body.iter().enumerate() {
            clipped_text(
                cv,
                line,
                &f.small,
                X0 + 14.0,
                y + 34.0 + 16.0 * k as f32,
                X1 - X0 - 28.0,
                16.0,
                muted,
            );
        }
        for ((hit, r), (_, label)) in card_buttons(f, v).into_iter().zip(&c.buttons) {
            button(
                cv,
                t,
                f,
                r,
                label,
                hit.is_some(),
                hit.is_some() && hover == hit,
            );
        }
    }

    draw_lines(cv, t, f, v, hover);

    let r = add_rect(f, v);
    let fill = if hover == Some(Hit::AddTask) {
        with_alpha(t.text, 0.85)
    } else {
        t.text
    };
    cv.fill_rrect(&rrect(r.0, r.1, r.2, r.3, 16.0), fill);
    let on = t.panel;
    paint_icon(cv, Icon::Plus, r.0 + 19.0, r.1 + 16.0, 14.0, on);
    cv.text(
        "Add task",
        &f.title,
        r.0 + 32.0,
        r.1,
        r.2 - r.0 - 32.0,
        32.0,
        on,
    );
    if !v.footer.is_empty() {
        let w = (X1 - r.2 - 16.0).max(0.0);
        let fw = text_width(&v.footer, &f.small).min(w);
        clipped_text(cv, &v.footer, &f.small, X1 - fw, r.1, fw, 32.0, muted);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn view(lines: Vec<Line>, card: Option<Card>) -> View {
        View {
            date: "Friday, 6 Nov".into(),
            all_day: Vec::new(),
            banner: ("Dzuhur".into(), "in 1 h".into(), "11:26".into()),
            card,
            lines,
            footer: String::new(),
            scroll: 0.0,
            focus: None,
        }
    }

    fn marker() -> Line {
        Line::Marker {
            name: "Ashar".into(),
            time: "14:41".into(),
        }
    }

    #[test]
    fn the_panel_grows_with_its_timeline_up_to_a_limit_then_scrolls() {
        let short = view(vec![Line::Now("Now 10:20".into()), marker()], None);
        let h = height(&short);
        assert!(h < MAX_H);
        assert_eq!(max_scroll(&short), 0.0);
        let long = view((0..40).map(|_| marker()).collect(), None);
        assert_eq!(height(&long), MAX_H);
        assert!(max_scroll(&long) > 0.0);
    }

    #[test]
    fn a_state_card_takes_room_above_the_timeline() {
        let card = Card {
            icon: Icon::Today,
            title: "No calendar connected".into(),
            body: vec!["a".into(), "b".into()],
            buttons: vec![(Some(Hit::Paste), "Paste ICS link".into())],
        };
        let without = height(&view(vec![marker()], None));
        let with = height(&view(vec![marker()], Some(card)));
        assert_eq!(with - without, 8.0 + 12.0 + 18.0 + 4.0 + 32.0 + 38.0 + 12.0);
    }

    #[test]
    fn markers_and_the_now_line_are_not_keyboard_stops() {
        assert!(!marker().is_stop());
        assert!(!Line::Now("Now".into()).is_stop());
        assert!(Line::Task {
            title: "a".into(),
            sub: String::new(),
            time: "14:00".into(),
            done: false,
            overdue: false
        }
        .is_stop());
    }

    #[test]
    fn revealing_a_line_scrolls_only_as_far_as_needed() {
        let mut v = view((0..40).map(|_| marker()).collect(), None);
        assert_eq!(reveal(&v, 0), 0.0);
        let s = reveal(&v, 39);
        assert_eq!(s, max_scroll(&v));
        v.scroll = s;
        assert_eq!(reveal(&v, 38), s);
    }
}
