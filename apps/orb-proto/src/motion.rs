#![allow(clippy::too_many_arguments)] // drawing helpers take a device context plus a full set of geometry
//! Shared motion (frame "v6.3 · 01 Foundation"): the rolling number and the row ripple.
//!
//! Both are keyed by a short static name and keep their own clock, so a panel's draw code can use
//! them with only `&App`: the first draw that sees a new value (or a reset list) starts the
//! motion. While any of them moves, `active()` is true; `panels.rs` keeps drawing frames until it
//! turns false and then the loop sleeps again. Nothing here loops: every motion ends at a fixed
//! time, and Windows "Animation effects" off makes them all static.
//!
//! - **Rolling number:** each digit is its own column that rolls to the new value on a critically
//!   damped spring (no overshoot, so a digit never shows a value it doesn't reach). `Roll::Slow`
//!   is the frame's "roll", ω 7.5, about 0.9 s; `Roll::Quick` is ω 22, about 0.3 s, for values that
//!   change every second. The width snaps first, so neighbours never slide.
//! - **Ripple:** list rows enter `y 6 → 0` and fade in on `settle`, 200 ms each, 30 ms apart,
//!   at most 8 rows staggered (240 ms in all).

use std::cell::{Cell, RefCell};
use std::collections::HashMap;
use std::time::{Duration, Instant};

use crate::canvas::{line_height, text_width, Canvas, Color, Font};

thread_local! {
    /// The last moment any motion still moves.
    static UNTIL: Cell<Option<Instant>> = const { Cell::new(None) };
    static ROLLS: RefCell<HashMap<&'static str, RollState>> = RefCell::new(HashMap::new());
    static RIPPLES: RefCell<HashMap<&'static str, RippleState>> = RefCell::new(HashMap::new());
}

fn hold(until: Instant) {
    UNTIL.with(|u| {
        if u.get().is_none_or(|t| t < until) {
            u.set(Some(until));
        }
    });
}

/// True while a rolling number or a ripple still moves. Turns false (and stays false) once the
/// last one has settled, so the frame loop can stop.
pub fn active() -> bool {
    UNTIL.with(|u| match u.get() {
        Some(t) if Instant::now() < t => true,
        Some(_) => {
            u.set(None);
            false
        }
        None => false,
    })
}

thread_local! {
    /// Snapshot mode: every number shows its value and every list rests, so scenes are exact.
    static STILL: Cell<bool> = const { Cell::new(false) };
    static TRACK: Cell<bool> = const { Cell::new(false) };
    static RECTS: RefCell<Vec<(f32, f32, f32, f32)>> = const { RefCell::new(Vec::new()) };
}

#[cfg_attr(windows, allow(dead_code))] // the macOS panel host
/// Remembers where rolls draw while they move, for a host that redraws only that part
/// (`track_rolls(true)` turns it on; macOS). Rects are in the canvas's untranslated space.
pub fn track_rolls(on: bool) {
    TRACK.with(|t| t.set(on));
}

#[cfg_attr(windows, allow(dead_code))]
/// The rects (l, t, r, b) of the rolls drawn since the last call, cleared.
pub fn take_roll_rects() -> Vec<(f32, f32, f32, f32)> {
    RECTS.with(|r| std::mem::take(&mut *r.borrow_mut()))
}

fn note_roll(l: f32, t: f32, r: f32, b: f32) {
    if TRACK.with(|t| t.get()) {
        RECTS.with(|v| v.borrow_mut().push((l - 2.0, t, r + 2.0, b)));
    }
}

/// Makes all motion still (snapshot mode).
pub fn set_still(on: bool) {
    STILL.with(|s| s.set(on));
}

#[cfg(windows)]
fn reduced() -> bool {
    STILL.with(|s| s.get()) || crate::downloads::reduced_motion()
}

#[cfg(target_os = "macos")]
fn reduced() -> bool {
    STILL.with(|s| s.get()) || crate::mac::reduced_motion()
}

// ---- the critically damped spring ----

/// Which spring a rolling number uses.
#[derive(Clone, Copy, PartialEq, Debug)]
pub enum Roll {
    /// ω 7.5, settled in 0.9 s: totals and sizes.
    Slow,
    /// ω 22, settled in 0.3 s: values that change every second (countdowns, percents).
    Quick,
}

impl Roll {
    fn omega(self) -> f32 {
        match self {
            Roll::Slow => 7.5,
            Roll::Quick => 22.0,
        }
    }
    fn secs(self) -> f32 {
        match self {
            Roll::Slow => 0.9,
            Roll::Quick => 0.3,
        }
    }
}

/// Progress of a critically damped spring from 0 to 1 after `t` seconds: 1 − (1 + ωt)·e^(−ωt).
/// Snaps to exactly 1 at `settle` so it always ends.
pub fn critical(omega: f32, t: f32, settle: f32) -> f32 {
    if t >= settle {
        return 1.0;
    }
    let x = omega * t.max(0.0);
    (1.0 - (1.0 + x) * (-x).exp()).clamp(0.0, 1.0)
}

// ---- rolling number ----

struct RollState {
    from: String,
    to: String,
    start: Instant,
    kind: Roll,
    still: bool,
}

impl RollState {
    fn progress(&self) -> f32 {
        if self.still {
            return 1.0;
        }
        critical(
            self.kind.omega(),
            self.start.elapsed().as_secs_f32(),
            self.kind.secs(),
        )
    }
}

/// How each column of `to` moves from `from`, right-aligned: `Some((old, steps))` for a digit
/// that rolls `steps` places (positive counts up, negative down), `None` for a static character.
fn columns(from: &str, to: &str, up: bool) -> Vec<Option<(u32, i32)>> {
    let f: Vec<char> = from.chars().collect();
    let t: Vec<char> = to.chars().collect();
    let off = t.len() as isize - f.len() as isize;
    t.iter()
        .enumerate()
        .map(|(i, c)| {
            let new = c.to_digit(10)?;
            let j = i as isize - off;
            let old = if j >= 0 {
                f.get(j as usize).and_then(|c| c.to_digit(10))
            } else {
                None
            }?;
            if old == new {
                return None;
            }
            let steps = if up {
                (new as i32 - old as i32).rem_euclid(10)
            } else {
                -((old as i32 - new as i32).rem_euclid(10))
            };
            Some((old, steps))
        })
        .collect()
}

/// The numeric value of a label's digits, to tell whether it went up or down.
fn digits_value(s: &str) -> u64 {
    s.chars()
        .filter_map(|c| c.to_digit(10))
        .fold(0u64, |v, d| v.saturating_mul(10).saturating_add(d as u64))
}

pub fn roll_text(
    cv: &mut dyn Canvas,
    key: &'static str,
    value: &str,
    kind: Roll,
    fmt: &Font,
    x: f32,
    y: f32,
    h: f32,
    c: Color,
) {
    let (from, p) = ROLLS.with(|r| {
        let mut r = r.borrow_mut();
        let st = r.entry(key).or_insert_with(|| RollState {
            from: value.to_string(),
            to: value.to_string(),
            start: Instant::now(),
            kind,
            still: true,
        });
        if st.to != value {
            // A new value mid-roll rolls on from the last one it was heading to.
            st.from = std::mem::replace(&mut st.to, value.to_string());
            st.start = Instant::now();
            st.kind = kind;
            st.still = reduced();
            if !st.still {
                hold(st.start + Duration::from_secs_f32(kind.secs()));
            }
        }
        (st.from.clone(), st.progress())
    });
    if p >= 1.0 {
        cv.text(value, fmt, x, y, text_width(value, fmt) + 2.0, h, c);
        return;
    }
    note_roll(
        x,
        y,
        x + text_width(value, fmt).max(text_width(&from, fmt)),
        y + h,
    );
    let up = digits_value(value) >= digits_value(&from);
    let cols = columns(&from, value, up);
    // One line's height: digits travel a full line per step.
    let line = line_height(fmt);
    let chars: Vec<char> = value.chars().collect();
    let mut cx = x;
    for (i, ch) in chars.iter().enumerate() {
        let s = ch.to_string();
        let cw = text_width(&s, fmt);
        match cols[i] {
            None => cv.text(&s, fmt, cx, y, cw + 2.0, h, c),
            Some((old, steps)) => {
                // The column's position in digits, and the two digits in view.
                let pos = steps as f32 * p;
                let base = pos.floor();
                let frac = pos - base;
                cv.push_clip(
                    cx - 1.0,
                    y + (h - line) / 2.0,
                    cx + cw + 1.0,
                    y + (h + line) / 2.0,
                );
                for k in 0..2 {
                    let d = (old as i32 + base as i32 + k).rem_euclid(10);
                    // Counting up, the next digit comes from below; counting down, from above.
                    let dy = (k as f32 - frac) * line * if steps >= 0 { 1.0 } else { -1.0 };
                    let a = (1.0 - (k as f32 - frac).abs()).clamp(0.0, 1.0);
                    if a <= 0.01 {
                        continue;
                    }
                    let faded = Color { a: c.a * a, ..c };
                    cv.text(&d.to_string(), fmt, cx, y + dy, cw + 2.0, h, faded);
                }
                cv.pop_clip();
            }
        }
        cx += cw;
    }
}

/// `roll_text` with its right edge at `right` (the portable `rolling_text_right`).
pub fn roll_text_right(
    cv: &mut dyn Canvas,
    key: &'static str,
    value: &str,
    kind: Roll,
    fmt: &Font,
    right: f32,
    y: f32,
    h: f32,
    c: Color,
) {
    let w = text_width(value, fmt);
    roll_text(cv, key, value, kind, fmt, right - w, y, h, c);
}

// ---- ripple ----

struct RippleState {
    start: Instant,
    still: bool,
}

/// One row's entry: draw it `dy` px lower at `alpha` opacity.
#[derive(Clone, Copy, PartialEq, Debug)]
pub struct RowFx {
    pub dy: f32,
    pub alpha: f32,
}

impl RowFx {
    pub const REST: RowFx = RowFx {
        dy: 0.0,
        alpha: 1.0,
    };
}

pub const RIPPLE_MS: f32 = 200.0;
pub const RIPPLE_STAGGER_MS: f32 = 30.0;
pub const RIPPLE_ROWS: usize = 8;
pub const RIPPLE_RISE: f32 = 6.0;

/// The `settle` curve, cubic-bezier(.2, .85, .25, 1.05).
pub fn settle(t: f32) -> f32 {
    cubic_bezier(0.2, 0.85, 0.25, 1.05, t)
}

/// CSS cubic-bezier(x1, y1, x2, y2) at time `t` (0..1), solved for x by Newton then bisection.
pub fn cubic_bezier(x1: f32, y1: f32, x2: f32, y2: f32, t: f32) -> f32 {
    let t = t.clamp(0.0, 1.0);
    let bez = |a: f32, b: f32, s: f32| {
        let u = 1.0 - s;
        3.0 * u * u * s * a + 3.0 * u * s * s * b + s * s * s
    };
    let (mut lo, mut hi, mut s) = (0.0f32, 1.0f32, t);
    for _ in 0..24 {
        let x = bez(x1, x2, s) - t;
        if x.abs() < 1e-5 {
            break;
        }
        if x > 0.0 {
            hi = s;
        } else {
            lo = s;
        }
        s = (lo + hi) / 2.0;
    }
    bez(y1, y2, s)
}

/// Where row `index` of a ripple is `ms` after it started.
pub fn ripple_at(ms: f32, index: usize) -> RowFx {
    let delay = index.min(RIPPLE_ROWS - 1) as f32 * RIPPLE_STAGGER_MS;
    let t = ((ms - delay) / RIPPLE_MS).clamp(0.0, 1.0);
    if t >= 1.0 {
        return RowFx::REST;
    }
    let e = settle(t);
    RowFx {
        dy: RIPPLE_RISE * (1.0 - e),
        alpha: t.min(1.0),
    }
}

/// The entry of row `index` in the list named `key`. The first draw after `ripple_reset` (or
/// ever) starts the ripple; rows past the eighth come in with the eighth.
pub fn ripple_row(key: &'static str, index: usize) -> RowFx {
    RIPPLES.with(|r| {
        let mut r = r.borrow_mut();
        let st = r.entry(key).or_insert_with(|| {
            let still = reduced();
            let start = Instant::now();
            if !still {
                hold(
                    start
                        + Duration::from_millis(
                            (RIPPLE_MS + RIPPLE_STAGGER_MS * (RIPPLE_ROWS - 1) as f32) as u64,
                        ),
                );
            }
            RippleState { start, still }
        });
        if st.still {
            RowFx::REST
        } else {
            ripple_at(st.start.elapsed().as_secs_f32() * 1000.0, index)
        }
    })
}

/// Lets the list named `key` ripple in again on its next draw (a panel opened, a list replaced).
#[allow(dead_code)] // for lists that are replaced while their window stays open (Disk)
pub fn ripple_reset(key: &'static str) {
    RIPPLES.with(|r| {
        r.borrow_mut().remove(key);
    });
}

/// Every list ripples in again on its next draw: called when a panel opens.
pub fn ripple_reset_all() {
    RIPPLES.with(|r| r.borrow_mut().clear());
}

/// `with_row` on the portable `Canvas`: the same translation and layer, so a row drawn through
/// `D2dCanvas` gives the pixels `with_row` gave.
///
/// `motion::row(cv, motion::ripple_row("drink.rows", i), |cv| draw_row(cv, ..))`
pub fn row(cv: &mut dyn Canvas, fx: RowFx, draw: impl FnOnce(&mut dyn Canvas)) {
    if fx == RowFx::REST {
        draw(cv);
        return;
    }
    if fx.alpha <= 0.003 {
        return;
    }
    cv.push_translate(0.0, fx.dy);
    cv.push_layer(fx.alpha);
    draw(cv);
    cv.pop_layer();
    cv.pop_translate();
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_spring_never_overshoots_and_ends_on_time() {
        let mut last = 0.0;
        for i in 0..=100 {
            let t = i as f32 * 0.01;
            let p = critical(7.5, t, 0.9);
            assert!((0.0..=1.0).contains(&p));
            assert!(p >= last, "monotonic");
            last = p;
        }
        assert_eq!(critical(7.5, 0.9, 0.9), 1.0);
        assert_eq!(critical(22.0, 0.3, 0.3), 1.0);
        // Within about 1% of a line when it snaps (no visible jump).
        assert!(critical(7.5, 0.899, 0.9) > 0.99);
        assert!(critical(22.0, 0.299, 0.3) > 0.985);
    }

    #[test]
    fn digits_roll_forward_through_the_wrap() {
        // 38 to 41: the tens roll 3 to 4, the ones 8 through 9, 0 to 1 (as the frame shows).
        assert_eq!(columns("38", "41", true), vec![Some((3, 1)), Some((8, 3))]);
        // Down: 41 to 38 rolls the other way.
        assert_eq!(
            columns("41", "38", false),
            vec![Some((4, -1)), Some((1, -3))]
        );
        // Units and unchanged digits stay still; a new leading digit just appears.
        assert_eq!(
            columns("750 ml", "1000 ml", true),
            vec![None, Some((7, 3)), Some((5, 5)), None, None, None, None]
        );
    }

    #[test]
    fn ripple_staggers_30_ms_and_rests_after_240() {
        assert_eq!(
            ripple_at(0.0, 0),
            RowFx {
                dy: 6.0,
                alpha: 0.0
            }
        );
        let a = ripple_at(100.0, 0);
        let b = ripple_at(100.0, 1);
        assert!(a.dy < b.dy && a.alpha > b.alpha);
        // Row 12 enters with row 8 (index 7).
        assert_eq!(ripple_at(150.0, 12), ripple_at(150.0, 7));
        assert_eq!(ripple_at(410.0, 7), RowFx::REST);
        assert_eq!(ripple_at(200.0, 0), RowFx::REST);
    }

    #[test]
    fn settle_curve_ends_where_it_should() {
        assert!(settle(0.0).abs() < 1e-4);
        assert!((settle(1.0) - 1.0).abs() < 1e-4);
        // Slight overshoot near the end, like the CSS curve.
        assert!((0..100).any(|i| settle(i as f32 / 100.0) > 1.0));
    }
}
