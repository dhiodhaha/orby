#![allow(clippy::too_many_arguments)] // drawing helpers take a device context plus a full set of geometry
//! The Focus panel (frame "v6.2 · 07"): running (the ring, session dots, held notices and the
//! buttons) and setup (the knob, presets, task, breaks, start by itself). Drawing and hit tests.

use windows::Win32::Graphics::Direct2D::*;
use windows::Win32::Graphics::DirectWrite::IDWriteTextFormat;

use crate::gfx::*;
use crate::ui::{draw_icon, with_alpha, Fonts, Icon, Theme};

pub const W: f32 = 360.0;
pub const KNOB_MIN: u32 = 5;
pub const KNOB_MAX: u32 = 90;
pub const KNOB_STEP: u32 = 5;
pub const PRESETS: [u32; 3] = [15, 25, 50];
/// The knob's 270 degree sweep, clockwise from 12 o'clock: 5 min at 225 degrees, 90 at 135.
const SWEEP_FROM: f32 = 225.0;
const SWEEP: f32 = 270.0;
const KNOB_C: (f32, f32) = (W / 2.0, 128.0);
const RING_R: f32 = 84.0;

#[derive(Clone, Debug, PartialEq)]
pub enum Run {
    Focus,
    Paused { prayer: Option<String> },
    Break { long: bool },
}

#[derive(Clone, Debug, PartialEq)]
pub enum View {
    Setup {
        minutes: u32,
        short_break: u32,
        long_break: u32,
        every: u32,
        auto_start: bool,
        task: Option<String>,
    },
    Running {
        run: Run,
        left_secs: u64,
        length_secs: u64,
        progress: f32,
        session: u32,
        of: u32,
        done: u32,
        held: usize,
        task: Option<String>,
    },
}

#[derive(Clone, Copy, PartialEq, Debug)]
pub enum Hit {
    Knob,
    Preset(u32),
    Task,
    Breaks,
    AutoStart,
    Start,
    Primary,
    Skip,
    Stop,
}

type Rect = (f32, f32, f32, f32);

fn inside(r: Rect, x: f32, y: f32) -> bool {
    x >= r.0 && x <= r.2 && y >= r.1 && y <= r.3
}

/// Knob value from a point in panel coordinates: the angle around the knob, snapped to 5 min.
pub fn knob_value(x: f32, y: f32) -> u32 {
    let (dx, dy) = (x - KNOB_C.0, y - KNOB_C.1);
    let mut deg = dx.atan2(-dy).to_degrees();
    if deg < 0.0 {
        deg += 360.0;
    }
    // Degrees past the 5 min end, clockwise; the dead zone at the bottom goes to the nearer end.
    let past = (deg - SWEEP_FROM).rem_euclid(360.0);
    let past = if past > SWEEP {
        if past > SWEEP + 45.0 {
            0.0
        } else {
            SWEEP
        }
    } else {
        past
    };
    let steps = ((KNOB_MAX - KNOB_MIN) / KNOB_STEP) as f32;
    KNOB_MIN + (past / SWEEP * steps).round() as u32 * KNOB_STEP
}

pub fn knob_step(v: u32, up: bool) -> u32 {
    if up {
        (v + KNOB_STEP).min(KNOB_MAX)
    } else {
        v.saturating_sub(KNOB_STEP).max(KNOB_MIN)
    }
}

fn knob_deg(v: u32) -> f32 {
    SWEEP_FROM
        + (v.clamp(KNOB_MIN, KNOB_MAX) - KNOB_MIN) as f32 / (KNOB_MAX - KNOB_MIN) as f32 * SWEEP
}

const SETUP_H: f32 = 466.0;

fn running_top(task: &Option<String>) -> f32 {
    if task.is_some() {
        80.0
    } else {
        48.0
    }
}

fn holding(run: &Run) -> bool {
    // A prayer pause holds nothing (FocusTimer::is_holding is false then), so no banner promises it.
    !matches!(
        run,
        Run::Break { .. }
            | Run::Paused {
                prayer: Some(_),
                ..
            }
    )
}

pub fn height(v: &View) -> f32 {
    match v {
        View::Setup { .. } => SETUP_H,
        View::Running { run, task, .. } => {
            let dots = running_top(task) + RING_R * 2.0 + 18.0;
            let card = if holding(run) { 16.0 + 56.0 } else { 0.0 };
            dots + card + 16.0 + 32.0 + 16.0
        }
    }
}

fn buttons(run: &Run) -> Vec<(&'static str, Hit)> {
    match run {
        Run::Focus => vec![
            ("Pause", Hit::Primary),
            ("Skip to break", Hit::Skip),
            ("Stop", Hit::Stop),
        ],
        Run::Paused { .. } => vec![
            ("Resume", Hit::Primary),
            ("Skip to break", Hit::Skip),
            ("Stop", Hit::Stop),
        ],
        Run::Break { .. } => vec![("Start focus", Hit::Primary), ("Stop", Hit::Stop)],
    }
}

fn row_of(
    g: &Shared,
    fmt: &IDWriteTextFormat,
    labels: &[String],
    y: f32,
    h: f32,
    gap: f32,
    pad: f32,
) -> Vec<Rect> {
    let widths: Vec<f32> = labels.iter().map(|l| g.text_width(l, fmt) + pad).collect();
    let total = widths.iter().sum::<f32>() + gap * (widths.len().saturating_sub(1)) as f32;
    let mut x = (W - total) / 2.0;
    widths
        .iter()
        .map(|w| {
            let r = (x, y, x + w, y + h);
            x += w + gap;
            r
        })
        .collect()
}

fn preset_labels() -> Vec<String> {
    PRESETS
        .iter()
        .map(|p| p.to_string())
        .chain(std::iter::once("Custom".to_string()))
        .collect()
}

fn task_label(task: &Option<String>) -> String {
    task.clone().unwrap_or_else(|| "No task".into())
}

/// Every tappable rect, shared by drawing and hit testing so they can't drift apart.
fn rects(g: &Shared, f: &Fonts, v: &View) -> Vec<(Hit, Rect)> {
    let mut out = Vec::new();
    match v {
        View::Setup { task, .. } => {
            out.push((
                Hit::Knob,
                (
                    KNOB_C.0 - 80.0,
                    KNOB_C.1 - 80.0,
                    KNOB_C.0 + 80.0,
                    KNOB_C.1 + 80.0,
                ),
            ));
            for (i, r) in row_of(g, &f.title, &preset_labels(), 220.0, 28.0, 6.0, 28.0)
                .into_iter()
                .enumerate()
            {
                if let Some(p) = PRESETS.get(i) {
                    out.push((Hit::Preset(*p), r));
                }
            }
            let tw = g.text_width(&task_label(task), &f.title) + 28.0;
            out.push((Hit::Task, (W - 20.0 - tw, 274.0, W - 20.0, 302.0)));
            let cw = g.text_width("Change", &f.title) + 28.0;
            out.push((Hit::Breaks, (W - 20.0 - cw, 326.0, W - 20.0, 354.0)));
            out.push((Hit::AutoStart, (W - 20.0 - 40.0, 374.0, W - 20.0, 398.0)));
            out.push((Hit::Start, (16.0, 418.0, W - 16.0, 450.0)));
        }
        View::Running { run, .. } => {
            let y = height(v) - 16.0 - 32.0;
            let labels: Vec<String> = buttons(run).iter().map(|b| b.0.to_string()).collect();
            for (r, (_, hit)) in row_of(g, &f.title, &labels, y, 32.0, 8.0, 28.0)
                .into_iter()
                .zip(buttons(run))
            {
                out.push((hit, r));
            }
        }
    }
    out
}

pub fn hit(g: &Shared, f: &Fonts, v: &View, x: f32, y: f32) -> Option<Hit> {
    rects(g, f, v)
        .into_iter()
        .find(|(h, r)| {
            if *h == Hit::Knob {
                ((x - KNOB_C.0).powi(2) + (y - KNOB_C.1).powi(2)).sqrt() <= 80.0
            } else {
                inside(*r, x, y)
            }
        })
        .map(|(h, _)| h)
}

fn button(
    g: &Shared,
    dc: &ID2D1DeviceContext,
    t: &Theme,
    f: &Fonts,
    label: &str,
    r: Rect,
    primary: bool,
    hover: bool,
) {
    let h = r.3 - r.1;
    let fill = if primary {
        t.text
    } else if hover {
        t.button_hover
    } else {
        t.button
    };
    let fill = if primary && hover {
        with_alpha(fill, 0.86)
    } else {
        fill
    };
    unsafe {
        dc.FillRoundedRectangle(&rrect(r.0, r.1, r.2, r.3, h / 2.0), &brush(dc, fill));
    }
    let ink = brush(dc, if primary { t.panel } else { t.text });
    center_text(g, dc, label, &f.title, (r.0 + r.2) / 2.0, r.1, h, &ink);
}

fn mmss(secs: u64) -> String {
    format!("{}:{:02}", secs / 60, secs % 60)
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
    let hgt = height(v);
    unsafe {
        dc.FillRoundedRectangle(&rrect(0.0, 0.0, W, hgt, 20.0), &brush(dc, t.panel));
        dc.DrawRoundedRectangle(
            &rrect(0.5, 0.5, W - 0.5, hgt - 0.5, 19.5),
            &brush(dc, t.hair),
            1.0,
            None,
        );
    }
    let rs = rects(g, f, v);
    let rect_of = |h: Hit| rs.iter().find(|(k, _)| *k == h).map(|(_, r)| *r);
    let st = round_stroke(g);
    match v {
        View::Setup {
            minutes,
            short_break,
            long_break,
            every,
            auto_start,
            task,
        } => {
            text(
                dc,
                "New focus",
                &f.panel_title,
                16.0,
                14.0,
                160.0,
                20.0,
                &text_b,
            );
            right_text(
                g,
                dc,
                "turn, scroll or use arrows",
                &f.small,
                W - 16.0,
                15.0,
                18.0,
                &muted,
            );

            // The knob: ticks up to the value are ink 2 px, the rest muted 1.5 px.
            let (cx, cy) = KNOB_C;
            let n = (KNOB_MAX - KNOB_MIN) / KNOB_STEP;
            for i in 0..=n {
                let val = KNOB_MIN + i * KNOB_STEP;
                let a = knob_deg(val);
                let on = val <= *minutes;
                let (r0, w, b) = if on {
                    (60.0, 2.0, &text_b)
                } else {
                    (64.0, 1.5, &muted)
                };
                let r0 = if val == *minutes { 54.0 } else { r0 };
                unsafe {
                    dc.DrawLine(
                        on_circle(cx, cy, r0, a),
                        on_circle(cx, cy, 72.0, a),
                        b,
                        w,
                        &st,
                    );
                }
            }
            unsafe {
                dc.FillEllipse(&ellipse(cx, cy, 46.0), &brush(dc, t.raised));
                dc.DrawEllipse(&ellipse(cx, cy, 46.0), &brush(dc, t.hair), 1.0, None);
            }
            center_text(
                g,
                dc,
                &minutes.to_string(),
                &f.big,
                cx,
                cy - 24.0,
                36.0,
                &text_b,
            );
            center_text(g, dc, "min", &f.small, cx, cy + 10.0, 16.0, &muted);
            let p5 = on_circle(cx, cy, 82.0, SWEEP_FROM - 8.0);
            let p90 = on_circle(cx, cy, 82.0, SWEEP_FROM + SWEEP + 8.0);
            center_text(g, dc, "5", &f.small, p5.X, p5.Y - 2.0, 16.0, &muted);
            center_text(g, dc, "90", &f.small, p90.X, p90.Y - 2.0, 16.0, &muted);

            // Presets; Custom is lit when the knob sits between them.
            let labels = preset_labels();
            for (i, r) in row_of(g, &f.title, &labels, 220.0, 28.0, 6.0, 28.0)
                .into_iter()
                .enumerate()
            {
                let selected = match PRESETS.get(i) {
                    Some(p) => p == minutes,
                    None => !PRESETS.contains(minutes),
                };
                let hov = PRESETS
                    .get(i)
                    .is_some_and(|p| hover == Some(Hit::Preset(*p)));
                if selected || hov {
                    unsafe {
                        dc.FillRoundedRectangle(
                            &rrect(r.0, r.1, r.2, r.3, 14.0),
                            &brush(
                                dc,
                                if hov && !selected {
                                    t.button
                                } else {
                                    t.button_hover
                                },
                            ),
                        );
                    }
                }
                center_text(
                    g,
                    dc,
                    &labels[i],
                    if selected { &f.title } else { &f.body },
                    (r.0 + r.2) / 2.0,
                    r.1,
                    28.0,
                    if selected { &text_b } else { &muted },
                );
            }

            // Rows.
            let hair = brush(dc, t.hair);
            for y in [262.0, 314.0, 366.0] {
                unsafe {
                    dc.DrawLine(pt(20.0, y), pt(W - 20.0, y), &hair, 1.0, None);
                }
            }
            text(
                dc,
                "Focus on a task",
                &f.title,
                20.0,
                270.0,
                200.0,
                18.0,
                &text_b,
            );
            text(
                dc,
                "Optional. Sessions are logged against it.",
                &f.small,
                20.0,
                288.0,
                240.0,
                16.0,
                &muted,
            );
            if let Some(r) = rect_of(Hit::Task) {
                button(
                    g,
                    dc,
                    t,
                    f,
                    &task_label(task),
                    r,
                    false,
                    hover == Some(Hit::Task),
                );
            }
            text(dc, "Breaks", &f.title, 20.0, 322.0, 200.0, 18.0, &text_b);
            text(
                dc,
                &format!("{short_break} min, then {long_break} min after every {every} sessions"),
                &f.small,
                20.0,
                340.0,
                260.0,
                16.0,
                &muted,
            );
            if let Some(r) = rect_of(Hit::Breaks) {
                button(g, dc, t, f, "Change", r, false, hover == Some(Hit::Breaks));
            }
            text(
                dc,
                "Start the next session by itself",
                &f.title,
                20.0,
                377.0,
                240.0,
                18.0,
                &text_b,
            );
            if let Some(r) = rect_of(Hit::AutoStart) {
                unsafe {
                    let track = if *auto_start {
                        t.text
                    } else if hover == Some(Hit::AutoStart) {
                        t.button_hover
                    } else {
                        t.button
                    };
                    dc.FillRoundedRectangle(&rrect(r.0, r.1, r.2, r.3, 12.0), &brush(dc, track));
                    if !auto_start {
                        dc.DrawRoundedRectangle(
                            &rrect(r.0 + 0.5, r.1 + 0.5, r.2 - 0.5, r.3 - 0.5, 11.5),
                            &hair,
                            1.0,
                            None,
                        );
                    }
                    let kx = if *auto_start { r.2 - 12.0 } else { r.0 + 12.0 };
                    dc.FillEllipse(
                        &ellipse(kx, (r.1 + r.3) / 2.0, 9.0),
                        &brush(dc, if *auto_start { t.panel } else { t.muted }),
                    );
                }
            }
            if let Some(r) = rect_of(Hit::Start) {
                button(
                    g,
                    dc,
                    t,
                    f,
                    &format!("Start {minutes} min"),
                    r,
                    true,
                    hover == Some(Hit::Start),
                );
            }
        }
        View::Running {
            run,
            left_secs,
            length_secs,
            progress,
            session,
            of,
            done,
            held,
            task,
        } => {
            text(
                dc,
                "Focus",
                &f.panel_title,
                16.0,
                14.0,
                160.0,
                20.0,
                &text_b,
            );
            let right = match run {
                Run::Break { long: true } => "Long break".to_string(),
                Run::Break { long: false } => "Short break".to_string(),
                _ => format!("Session {session} of {of}"),
            };
            right_text(g, dc, &right, &f.small, W - 16.0, 15.0, 18.0, &muted);
            let top = running_top(task);
            if let Some(name) = task {
                text(dc, "Focus on", &f.small, 16.0, 44.0, 60.0, 28.0, &muted);
                let w = g.text_width(name, &f.title) + 24.0;
                unsafe {
                    dc.FillRoundedRectangle(
                        &rrect(76.0, 44.0, 76.0 + w, 72.0, 14.0),
                        &brush(dc, t.button),
                    );
                }
                text(dc, name, &f.title, 88.0, 44.0, w, 28.0, &text_b);
            }

            // The ring: dotted is time left, solid is time spent (the orb's own motif).
            let (cx, cy) = (W / 2.0, top + RING_R);
            unsafe {
                dc.DrawEllipse(
                    &ellipse(cx, cy, RING_R),
                    &brush(dc, t.track),
                    1.5,
                    &dotted_stroke(g),
                );
            }
            draw_arc(
                g,
                dc,
                cx,
                cy,
                RING_R,
                0.0,
                360.0 * progress,
                &text_b,
                2.5,
                &st,
            );
            center_text(
                g,
                dc,
                &mmss(*left_secs),
                &f.timer,
                cx,
                cy - 34.0,
                52.0,
                &text_b,
            );
            let sub = match run {
                Run::Focus => format!("left of {}", mmss(*length_secs)),
                Run::Paused { prayer: Some(p) } => format!("paused for {p}"),
                Run::Paused { prayer: None } => "paused".to_string(),
                Run::Break { .. } => format!("break, left of {}", mmss(*length_secs)),
            };
            center_text(g, dc, &sub, &f.small, cx, cy + 18.0, 16.0, &muted);

            // Session dots: done ones filled, the current one half filled.
            let dy = top + RING_R * 2.0 + 18.0;
            let x0 = cx - (*of as f32 - 1.0) * 6.0;
            for i in 0..*of {
                let x = x0 + i as f32 * 12.0;
                unsafe {
                    if i < *done {
                        dc.FillEllipse(&ellipse(x, dy, 3.0), &text_b);
                    } else if i == *done && !matches!(run, Run::Break { .. }) {
                        dc.DrawEllipse(&ellipse(x, dy, 2.6), &text_b, 0.8, None);
                        dc.PushAxisAlignedClip(
                            &windows::Win32::Graphics::Direct2D::Common::D2D_RECT_F {
                                left: x - 4.0,
                                top: dy - 4.0,
                                right: x,
                                bottom: dy + 4.0,
                            },
                            D2D1_ANTIALIAS_MODE_ALIASED,
                        );
                        dc.FillEllipse(&ellipse(x, dy, 3.0), &text_b);
                        dc.PopAxisAlignedClip();
                    } else {
                        dc.FillEllipse(&ellipse(x, dy, 3.0), &brush(dc, t.track));
                    }
                }
            }

            if holding(run) {
                let (l, tp, r, b) = (16.0, dy + 16.0, W - 16.0, dy + 16.0 + 56.0);
                unsafe {
                    dc.FillRoundedRectangle(&rrect(l, tp, r, b, 12.0), &brush(dc, t.raised));
                    dc.DrawRoundedRectangle(
                        &rrect(l + 0.5, tp + 0.5, r - 0.5, b - 0.5, 11.5),
                        &brush(dc, t.hair),
                        1.0,
                        None,
                    );
                }
                draw_icon(g, dc, Icon::Bell, l + 20.0, tp + 28.0, 15.0, &muted);
                let line1 = match held {
                    0 => "Notices wait for your break.".to_string(),
                    1 => "1 notice waits for your break.".to_string(),
                    n => format!("{n} notices wait for your break."),
                };
                text(
                    dc,
                    &line1,
                    &f.small_medium,
                    l + 38.0,
                    tp + 10.0,
                    r - l - 48.0,
                    18.0,
                    &text_b,
                );
                text(
                    dc,
                    "Prayer time still comes through.",
                    &f.small,
                    l + 38.0,
                    tp + 28.0,
                    r - l - 48.0,
                    18.0,
                    &muted,
                );
            }

            for (label, h) in buttons(run) {
                if let Some(r) = rect_of(h) {
                    button(g, dc, t, f, label, r, h == Hit::Primary, hover == Some(h));
                }
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_knob_maps_angles_to_five_minute_detents() {
        let (cx, cy) = KNOB_C;
        let at = |deg: f32| {
            let p = on_circle(cx, cy, 60.0, deg);
            knob_value(p.X, p.Y)
        };
        assert_eq!(at(SWEEP_FROM), 5);
        assert_eq!(at(SWEEP_FROM + SWEEP), 90);
        assert_eq!(at(0.0), 50); // straight up is the middle: 5 + 135/270 * 85, snapped
                                 // The dead zone at the bottom snaps to the nearer end.
        assert_eq!(at(170.0), 90);
        assert_eq!(at(215.0), 5);
    }

    #[test]
    fn arrows_step_the_knob_within_its_range() {
        assert_eq!(knob_step(25, true), 30);
        assert_eq!(knob_step(5, false), 5);
        assert_eq!(knob_step(90, true), 90);
    }

    #[test]
    fn running_buttons_follow_the_phase() {
        assert_eq!(buttons(&Run::Focus)[0].0, "Pause");
        assert_eq!(
            buttons(&Run::Paused {
                prayer: Some("Isya".into())
            })[0]
                .0,
            "Resume"
        );
        assert_eq!(buttons(&Run::Break { long: false }).len(), 2);
    }
}
