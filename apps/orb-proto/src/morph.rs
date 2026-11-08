//! The panel window changing from one panel to another while it is open (Today's "Add task"
//! opening Tasks, Esc going back). The window covers both sizes for the moment of the change; a
//! rounded shape grows from the old size to the new one, the old content fades out quickly and
//! the new content fades in, rising 6 px. Everything is a function of one spring, so a second
//! switch mid-way starts from wherever the shape is.

use crate::orb::Spring;

/// Near-critical (c = 2√k ≈ 52.9): no overshoot, so text never bounces. Reads as an ease-out:
/// 50 % at ~65 ms, 90 % at ~150 ms, done to the eye by ~250 ms.
pub const K: f32 = 700.0;
pub const C: f32 = 53.0;
/// The new content rises this far (logical px) as it fades in.
pub const RISE: f32 = 6.0;

/// A rectangle in screen pixels.
#[derive(Clone, Copy, PartialEq, Debug, Default)]
pub struct Rect {
    pub l: f32,
    pub t: f32,
    pub r: f32,
    pub b: f32,
}

impl Rect {
    pub fn w(&self) -> f32 {
        self.r - self.l
    }
    pub fn h(&self) -> f32 {
        self.b - self.t
    }
    pub fn lerp(a: Rect, b: Rect, e: f32) -> Rect {
        let m = |x: f32, y: f32| x + (y - x) * e;
        Rect {
            l: m(a.l, b.l),
            t: m(a.t, b.t),
            r: m(a.r, b.r),
            b: m(a.b, b.b),
        }
    }
    pub fn union(a: Rect, b: Rect) -> Rect {
        Rect {
            l: a.l.min(b.l),
            t: a.t.min(b.t),
            r: a.r.max(b.r),
            b: a.b.max(b.b),
        }
    }
}

/// The switch in progress, if any. `p` runs 0 → 1; at rest it sits at 1 with no `from`.
pub struct Morph<P> {
    pub p: Spring,
    /// The panel being left and the shape it had when the switch began.
    pub from: Option<(P, Rect)>,
    /// Where Esc returns to: set when a panel opened another from inside it.
    pub back: Option<P>,
    /// The last window region set (window-relative pixels), so it is only set when it changes.
    pub rgn: Option<(i32, i32, i32, i32)>,
}

impl<P> Default for Morph<P> {
    fn default() -> Self {
        Self {
            p: Spring::new(1.0, K, C),
            from: None,
            back: None,
            rgn: None,
        }
    }
}

impl<P> Morph<P> {
    /// Starts a switch away from `kind`, whose shape is `shape` right now.
    pub fn start(&mut self, kind: P, shape: Rect) {
        self.from = Some((kind, shape));
        self.p.snap(0.0);
        self.p.target = 1.0;
        self.p.k = K;
        self.p.c = C;
    }

    pub fn finish(&mut self) {
        self.from = None;
        self.p.snap(1.0);
    }

    /// Progress, 0 → 1, clamped (a near-critical spring may creep a hair past 1).
    pub fn e(&self) -> f32 {
        if self.from.is_some() {
            self.p.pos.clamp(0.0, 1.0)
        } else {
            1.0
        }
    }

    pub fn active(&self) -> bool {
        self.from.is_some()
    }
}

fn smoothstep(a: f32, b: f32, x: f32) -> f32 {
    let t = ((x - a) / (b - a)).clamp(0.0, 1.0);
    t * t * (3.0 - 2.0 * t)
}

/// Opacity of the old and the new content at progress `e`. The old one is gone by 35 % (exits are
/// faster than entrances); the new one starts at 10 % and is whole by 70 %. They overlap briefly
/// over the opaque panel fill, so the panel dips at most to about a third and is never empty.
pub fn fades(e: f32) -> (f32, f32) {
    (1.0 - smoothstep(0.0, 0.35, e), smoothstep(0.1, 0.7, e))
}

/// Where a panel's content sits inside the moving shape: against the orb side (the panel grows
/// away from the drop), at the top. `w` is the content width in pixels.
pub fn anchor(shape: Rect, w: f32, orb_on_right: bool) -> (f32, f32) {
    let x = if orb_on_right { shape.r - w } else { shape.l };
    (x, shape.t)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn lerp_and_union() {
        let a = Rect {
            l: 0.0,
            t: 10.0,
            r: 100.0,
            b: 60.0,
        };
        let b = Rect {
            l: -200.0,
            t: 0.0,
            r: 100.0,
            b: 200.0,
        };
        assert_eq!(Rect::lerp(a, b, 0.0), a);
        assert_eq!(Rect::lerp(a, b, 1.0), b);
        let mid = Rect::lerp(a, b, 0.5);
        assert_eq!(
            mid,
            Rect {
                l: -100.0,
                t: 5.0,
                r: 100.0,
                b: 130.0
            }
        );
        assert_eq!(
            Rect::union(a, b),
            Rect {
                l: -200.0,
                t: 0.0,
                r: 100.0,
                b: 200.0
            }
        );
    }

    #[test]
    fn fades_hand_over_without_a_gap() {
        assert_eq!(fades(0.0), (1.0, 0.0));
        assert_eq!(fades(1.0), (0.0, 1.0));
        for i in 0..=100 {
            let (o, n) = fades(i as f32 / 100.0);
            assert!((0.0..=1.0).contains(&o) && (0.0..=1.0).contains(&n));
            // Content never drops out entirely.
            assert!(o + n > 0.3, "gap at {i}%: {o} + {n}");
        }
        assert_eq!(fades(0.35).0, 0.0);
        assert_eq!(fades(0.7).1, 1.0);
    }

    #[test]
    fn content_hugs_the_orb_side() {
        let s = Rect {
            l: 100.0,
            t: 20.0,
            r: 500.0,
            b: 300.0,
        };
        assert_eq!(anchor(s, 300.0, true), (200.0, 20.0));
        assert_eq!(anchor(s, 300.0, false), (100.0, 20.0));
    }

    #[test]
    fn spring_is_quick_and_never_overshoots() {
        let mut m: Morph<u8> = Morph::default();
        m.start(0, Rect::default());
        let dt = 1.0 / 60.0;
        let (mut t, mut max, mut at90) = (0.0, 0.0f32, None);
        while !m.p.settled(0.002) {
            m.p.step(dt);
            t += dt;
            max = max.max(m.p.pos);
            if at90.is_none() && m.p.pos >= 0.9 {
                at90 = Some(t);
            }
            assert!(t < 1.0, "never settles");
        }
        assert!(max <= 1.0 + 1e-3, "overshoot {max}");
        let at90 = at90.unwrap();
        assert!((0.1..=0.2).contains(&at90), "90 % at {at90}s");
        assert!(t <= 0.45, "settles at {t}s");
    }
}
