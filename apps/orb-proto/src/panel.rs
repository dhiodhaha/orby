#![allow(clippy::too_many_arguments)] // drawing helpers take a device context plus a full set of geometry
//! The Prayer panel (frames "v6.2 · 05" and "06"): the arc with the Isya moon, the times,
//! the live countdown, reminder chips and buttons. Nothing here counts or checks a prayer.

use crate::canvas::*;
use crate::ui::{lip, with_alpha, Fonts, Theme};
use app_core::prayer::{hhmm, Prayer, Times};

pub const W: f32 = 380.0;
pub const H: f32 = 340.0;

pub struct View<'a> {
    pub times: &'a Times,
    pub now_min: f64,
    pub next: Prayer,
    pub mins_left: f64,
    pub place: &'a str,
    /// "Kemenag" or "Muhammadiyah", after the place in the header.
    pub method: &'a str,
    pub qibla: String,
    pub reminders: [bool; 3],
    pub hover: Option<Hit>,
}

#[derive(Clone, Copy, PartialEq, Debug)]
pub enum Hit {
    Chip(usize),
    Button(usize),
}

const CHIPS: [&str; 3] = ["30 min", "10 min", "At time"];
const BUTTONS: [&str; 3] = ["Reminders", "Place", "Qibla"];

// Arc positions along the curve for Subuh, Dzuhur, Ashar, Maghrib, Isya (the frame's anchors).
const S: [f32; 5] = [0.049, 0.299, 0.517, 0.736, 0.922];
const BOX_X: f32 = 16.0;
const BOX_Y: f32 = 58.0;
const K: f32 = 348.0 / 380.0;

fn arc_pt(s: f32) -> Vector2 {
    // The frame's curve: M16 106 Q190 -12 364 106, in a 380 x 116 box.
    let x = 16.0 + 348.0 * s;
    let y = 106.0 - 236.0 * s + 236.0 * s * s;
    pt(BOX_X + x * K, BOX_Y + y * K)
}

fn now_s(t: &Times, now: f64) -> f32 {
    let at: Vec<f64> = Prayer::ALL.iter().map(|p| t.of(*p) as f64).collect();
    if now <= at[0] {
        return (S[0] as f64 * (now / at[0]).max(0.0)) as f32;
    }
    for i in 0..4 {
        if now <= at[i + 1] {
            let f = (now - at[i]) / (at[i + 1] - at[i]);
            return S[i] + (S[i + 1] - S[i]) * f as f32;
        }
    }
    let f = ((now - at[4]) / (1440.0 - at[4])).clamp(0.0, 1.0);
    S[4] + (1.0 - S[4]) * f as f32
}

fn chip_rects(f: &Fonts) -> Vec<(f32, f32)> {
    // Right-aligned inside the card, 28 px tall, 6 px apart.
    let widths: Vec<f32> = CHIPS
        .iter()
        .map(|c| text_width(c, &f.small_medium) + 20.0)
        .collect();
    let total: f32 = widths.iter().sum::<f32>() + 6.0 * 2.0;
    let mut x = W - 16.0 - 14.0 - total;
    widths
        .iter()
        .map(|w| {
            let r = (x, *w);
            x += w + 6.0;
            r
        })
        .collect()
}

fn button_rects(f: &Fonts) -> Vec<(f32, f32)> {
    let mut x = 16.0;
    BUTTONS
        .iter()
        .map(|b| {
            let w = text_width(b, &f.title) + 24.0;
            let r = (x, w);
            x += w + 8.0;
            r
        })
        .collect()
}

pub fn hit(f: &Fonts, x: f32, y: f32) -> Option<Hit> {
    if (246.0..=274.0).contains(&y) {
        for (i, (cx, cw)) in chip_rects(f).into_iter().enumerate() {
            if x >= cx && x <= cx + cw {
                return Some(Hit::Chip(i));
            }
        }
    }
    if (292.0..=324.0).contains(&y) {
        for (i, (bx, bw)) in button_rects(f).into_iter().enumerate() {
            if x >= bx && x <= bx + bw {
                return Some(Hit::Button(i));
            }
        }
    }
    None
}

/// The Isya moon: a disc with a bite taken out in the panel colour.
fn moon(cv: &mut dyn Canvas, c: Vector2, r: f32, ink: Color, bg: Color) {
    {
        cv.fill_ellipse(&ellipse(c.X, c.Y, r), ink);
        cv.fill_ellipse(&ellipse(c.X + r * 0.55, c.Y - r * 0.45, r * 0.85), bg);
    }
}

pub fn draw(cv: &mut dyn Canvas, t: &Theme, f: &Fonts, v: &View) {
    let text_b = t.text;
    let muted = t.muted;
    let panel_b = t.panel;
    {
        cv.fill_rrect(&rrect(0.0, 0.0, W, H, 20.0), panel_b);
        cv.stroke_rrect(
            &rrect(0.5, 0.5, W - 0.5, H - 0.5, 19.5),
            t.hair,
            1.0,
            Stroke::Plain,
        );
    }

    // Header and meta line.
    cv.text("Prayer", &f.panel_title, 16.0, 14.0, 100.0, 20.0, text_b);
    cv.right_text(
        &format!("{} · {}", v.place, v.method),
        &f.small,
        W - 16.0,
        15.0,
        18.0,
        muted,
    );
    let meta = format!(
        "Imsak {} · Terbit {} · Qibla {}",
        hhmm(v.times.imsak),
        hhmm(v.times.terbit),
        v.qibla
    );
    cv.text(&meta, &f.small, 16.0, 34.0, W - 32.0, 16.0, muted);

    // The arc: dotted for what is still ahead, solid for the day so far.
    let st = Stroke::Round;
    {
        let start = arc_pt(0.0);
        let full = path(start, false, |k| {
            quad_to(k, pt(BOX_X + 190.0 * K, BOX_Y - 12.0 * K), arc_pt(1.0))
        });
        cv.stroke_path(&full, t.track, 1.5, Stroke::Dotted);
        let s = now_s(v.times, v.now_min);
        let ctrl = pt(
            BOX_X + (16.0 + 174.0 * s) * K,
            BOX_Y + (106.0 - 118.0 * s) * K,
        );
        let past = path(start, false, |k| quad_to(k, ctrl, arc_pt(s)));
        cv.stroke_path(&past, t.passed, 1.5, st);
    }

    let passed_b = t.passed;
    let track_b = t.track;
    for (i, p) in Prayer::ALL.iter().enumerate() {
        let c = arc_pt(S[i]);
        let is_next = *p == v.next;
        let is_past = (v.times.of(*p) as f64) <= v.now_min && !is_next;
        {
            if is_next {
                let r = if *p == Prayer::Isya { 10.0 } else { 8.5 };
                cv.fill_ellipse(&ellipse(c.X, c.Y, r), panel_b);
                cv.stroke_ellipse(&ellipse(c.X, c.Y, r), text_b, 1.5, Stroke::Plain);
                if *p == Prayer::Isya {
                    moon(cv, c, 4.6, text_b, panel_b);
                } else {
                    cv.fill_ellipse(&ellipse(c.X, c.Y, 4.0), text_b);
                }
            } else {
                let b = if is_past { passed_b } else { track_b };
                if *p == Prayer::Isya {
                    moon(cv, c, 5.0, b, panel_b);
                } else {
                    cv.fill_ellipse(&ellipse(c.X, c.Y, 3.5), b);
                }
            }
        }
    }

    // Now: a solid drop on the arc with a soft halo, apart from the prayer markers, so the
    // ring stays "next" and this stays "you are here". Neutral, never a colour.
    let now = arc_pt(now_s(v.times, v.now_min));
    {
        cv.fill_ellipse(&ellipse(now.X, now.Y, 9.0), with_alpha(t.text, 0.08));
        cv.fill_ellipse(&ellipse(now.X, now.Y, 5.5), panel_b);
        cv.fill_ellipse(&ellipse(now.X, now.Y, 4.5), text_b);
    }

    // Times row.
    let col = (W - 32.0) / 5.0;
    for (i, p) in Prayer::ALL.iter().enumerate() {
        let cx = 16.0 + col * (i as f32 + 0.5);
        let on = *p == v.next;
        cv.center_text(
            p.name(),
            if on { &f.small_medium } else { &f.small },
            cx,
            166.0,
            16.0,
            if on { text_b } else { muted },
        );
        cv.center_text(
            &hhmm(v.times.of(*p)),
            if on { &f.title } else { &f.body },
            cx,
            182.0,
            18.0,
            if on { text_b } else { muted },
        );
    }

    // Countdown card.
    {
        cv.fill_rrect(&rrect(16.0, 214.0, W - 16.0, 278.0, 12.0), t.raised);
        cv.stroke_rrect(
            &rrect(16.5, 214.5, W - 16.5, 277.5, 11.5),
            t.hair,
            1.0,
            Stroke::Plain,
        );
    }
    lip(cv, t, 16.0, 214.0, W - 16.0, 278.0, 12.0, 1.0);
    cv.text(
        &format!("{} in", v.next.name()),
        &f.small,
        30.0,
        224.0,
        160.0,
        16.0,
        muted,
    );
    // The clock reads whole seconds and prayer times are whole minutes, so the gap is a whole
    // number of seconds: round away the float error that would show a second twice or skip it.
    let secs = (v.mins_left * 60.0).round().max(0.0) as u64;
    let countdown = if secs >= 3600 {
        format!("{}:{:02}:{:02}", secs / 3600, (secs / 60) % 60, secs % 60)
    } else {
        format!("{:02}:{:02}", secs / 60, secs % 60)
    };
    crate::motion::roll_text(
        cv,
        "prayer.countdown",
        &countdown,
        crate::motion::Roll::Quick,
        &f.big,
        30.0,
        240.0,
        32.0,
        text_b,
    );
    cv.right_text("Reminders", &f.small, W - 30.0, 224.0, 16.0, muted);
    for (i, (x, w)) in chip_rects(f).into_iter().enumerate() {
        let on = v.reminders[i];
        let hover = v.hover == Some(Hit::Chip(i));
        {
            let r = rrect(x, 246.0, x + w, 274.0, 14.0);
            if on {
                cv.fill_rrect(&r, if hover { t.button_hover } else { t.button });
            } else {
                cv.stroke_rrect(
                    &rrect(x + 0.5, 246.5, x + w - 0.5, 273.5, 13.5),
                    with_alpha(t.muted, 0.5),
                    1.0,
                    Stroke::Plain,
                );
            }
        }
        cv.center_text(
            CHIPS[i],
            if on { &f.small_medium } else { &f.small },
            x + w / 2.0,
            246.0,
            28.0,
            if on { text_b } else { muted },
        );
    }

    // Buttons.
    for (i, (x, w)) in button_rects(f).into_iter().enumerate() {
        let hover = v.hover == Some(Hit::Button(i));
        {
            cv.fill_rrect(
                &rrect(x, 292.0, x + w, 324.0, 16.0),
                if hover { t.button_hover } else { t.button },
            );
        }
        lip(cv, t, x, 292.0, x + w, 324.0, 16.0, 1.0);
        cv.text(BUTTONS[i], &f.title, x + 12.0, 292.0, w, 32.0, text_b);
    }
}
