#![allow(clippy::too_many_arguments)] // drawing calls take a full set of geometry
//! The portable drawing layer: what the orb's pill, the dial and the Prayer panel draw with.
//!
//! `Canvas` is a small immediate-mode API in logical pixels, y down: rounded rects, circles,
//! lines and paths (lines, quadratic curves, circular arcs), filled or stroked with a solid
//! colour; Inter text in one of the app's fonts (left, right or centred in a box, vertically
//! centred); axis-aligned clips, translation and opacity layers.
//!
//! - Windows: `gfx::D2dCanvas` replays every call onto the Direct2D device context exactly as the
//!   code did before (same brushes, geometries and stroke styles), so output is pixel-identical.
//! - macOS: `mac::cg::CgCanvas` draws with Core Graphics and Core Text into a bitmap that becomes
//!   a layer's contents (or a snapshot PNG).
//!
//! `Color`, `Vector2` and `Font` are the platform's own types on Windows (D2D1_COLOR_F,
//! Vector2, IDWriteTextFormat), so code that still draws with Direct2D shares them unchanged.

#[cfg(windows)]
pub use windows::Win32::Graphics::Direct2D::Common::D2D1_COLOR_F as Color;
#[cfg(windows)]
pub use windows::Win32::Graphics::DirectWrite::IDWriteTextFormat as Font;
#[cfg(windows)]
pub use windows_numerics::Vector2;

#[cfg(not(windows))]
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct Color {
    pub r: f32,
    pub g: f32,
    pub b: f32,
    pub a: f32,
}

#[cfg(not(windows))]
#[allow(non_snake_case)] // the same fields as windows_numerics::Vector2
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct Vector2 {
    pub X: f32,
    pub Y: f32,
}

#[cfg(target_os = "macos")]
pub use crate::mac::text::Font;

pub fn rgba(hex: u32, a: f32) -> Color {
    Color {
        r: ((hex >> 16) & 0xff) as f32 / 255.0,
        g: ((hex >> 8) & 0xff) as f32 / 255.0,
        b: (hex & 0xff) as f32 / 255.0,
        a,
    }
}

pub fn pt(x: f32, y: f32) -> Vector2 {
    Vector2 { X: x, Y: y }
}

/// A point on a circle, `deg` clockwise from 12 o'clock.
pub fn on_circle(cx: f32, cy: f32, r: f32, deg: f32) -> Vector2 {
    let a = deg.to_radians();
    pt(cx + r * a.sin(), cy - r * a.cos())
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct RRect {
    pub l: f32,
    pub t: f32,
    pub r: f32,
    pub b: f32,
    pub rad: f32,
}

pub fn rrect(l: f32, t: f32, r: f32, b: f32, rad: f32) -> RRect {
    RRect { l, t, r, b, rad }
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Ellipse {
    pub point: Vector2,
    pub rx: f32,
    pub ry: f32,
}

/// A circle.
pub fn ellipse(cx: f32, cy: f32, r: f32) -> Ellipse {
    Ellipse {
        point: pt(cx, cy),
        rx: r,
        ry: r,
    }
}

/// How a line ends and whether it is dashed. `Plain` is flat ends, mitred joins.
#[cfg_attr(windows, allow(dead_code))] // Dashed: the drink goal line, still Direct2D on Windows
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Stroke {
    Plain,
    /// Round ends and joins.
    Round,
    /// The dotted time track: at 1.5 px, round dots every 6 px.
    Dotted,
    /// A flat dashed rule: 3 px on, 3 px off at 1 px.
    Dashed,
}

#[cfg_attr(windows, allow(dead_code))] // the macOS canvas reads the patterns
impl Stroke {
    /// Dash pattern in stroke widths, and whether the caps are round.
    pub fn dashes(self) -> Option<(&'static [f32], bool)> {
        match self {
            Stroke::Dotted => Some((&[0.0, 4.0], true)),
            Stroke::Dashed => Some((&[3.0, 3.0], false)),
            _ => None,
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Seg {
    Line(Vector2),
    Quad(Vector2, Vector2),
    /// A circular arc to a point (SVG style: radius, large arc, clockwise on screen).
    Arc {
        to: Vector2,
        r: f32,
        large: bool,
        cw: bool,
    },
}

/// A figure: a start point and segments, open (stroked) or closed (filled).
#[derive(Clone, Debug, PartialEq)]
pub struct Path {
    pub start: Vector2,
    pub filled: bool,
    pub segs: Vec<Seg>,
}

/// A path made of line segments, quadratic curves and circular arcs, built with a small closure.
pub fn path(start: Vector2, filled: bool, build: impl FnOnce(&mut Path)) -> Path {
    let mut p = Path {
        start,
        filled,
        segs: Vec::new(),
    };
    build(&mut p);
    p
}

pub fn line_to(k: &mut Path, to: Vector2) {
    k.segs.push(Seg::Line(to));
}

pub fn quad_to(k: &mut Path, ctrl: Vector2, to: Vector2) {
    k.segs.push(Seg::Quad(ctrl, to));
}

pub fn arc_to(k: &mut Path, to: Vector2, r: f32, large: bool, cw: bool) {
    k.segs.push(Seg::Arc { to, r, large, cw });
}

/// An SVG endpoint arc as centre form: (cx, cy, start angle, end angle) in radians, y down,
/// angles measured from +x toward +y. For circular arcs only (one radius).
#[cfg_attr(windows, allow(dead_code))] // Core Graphics draws arcs in centre form
pub fn arc_center(
    from: Vector2,
    to: Vector2,
    r: f32,
    large: bool,
    cw: bool,
) -> (f32, f32, f32, f32) {
    let (dx, dy) = (to.X - from.X, to.Y - from.Y);
    let d = (dx * dx + dy * dy).sqrt().max(1e-6);
    let r = r.max(d / 2.0);
    let (mx, my) = ((from.X + to.X) / 2.0, (from.Y + to.Y) / 2.0);
    let h = (r * r - d * d / 4.0).max(0.0).sqrt();
    // Unit normal to the chord; which side the centre sits on depends on the flags.
    let (nx, ny) = (-dy / d, dx / d);
    let sign = if large == cw { -1.0 } else { 1.0 };
    let (cx, cy) = (mx + sign * h * nx, my + sign * h * ny);
    let a0 = (from.Y - cy).atan2(from.X - cx);
    let a1 = (to.Y - cy).atan2(to.X - cx);
    (cx, cy, a0, a1)
}

#[cfg_attr(windows, allow(dead_code))] // some calls are only used by the macOS app so far
pub trait Canvas {
    fn fill_rrect(&mut self, r: &RRect, c: Color);
    fn stroke_rrect(&mut self, r: &RRect, c: Color, w: f32, s: Stroke);
    fn fill_ellipse(&mut self, e: &Ellipse, c: Color);
    fn stroke_ellipse(&mut self, e: &Ellipse, c: Color, w: f32, s: Stroke);
    fn line(&mut self, a: Vector2, b: Vector2, c: Color, w: f32, s: Stroke);
    fn fill_path(&mut self, p: &Path, c: Color);
    fn stroke_path(&mut self, p: &Path, c: Color, w: f32, s: Stroke);
    /// One line of text in the box (`x`, `y`, `w`, `h`): left aligned, vertically centred (a
    /// wrapping font wraps instead, top aligned).
    fn text(&mut self, s: &str, f: &Font, x: f32, y: f32, w: f32, h: f32, c: Color);
    /// The T3 lip: a one-pixel crescent inside the rounded rect, along the bottom edge (`dy`
    /// -1, light) or the top edge (`dy` 1, dark).
    fn lip(&mut self, c: Color, dy: f32, l: f32, t: f32, r: f32, b: f32, radius: f32);
    fn push_clip(&mut self, l: f32, t: f32, r: f32, b: f32);
    fn pop_clip(&mut self);
    /// Everything until `pop_layer` is composited at `opacity`.
    fn push_layer(&mut self, opacity: f32);
    fn pop_layer(&mut self);
    /// Moves the origin by (dx, dy) until `pop_translate`.
    fn push_translate(&mut self, dx: f32, dy: f32);
    fn pop_translate(&mut self);

    /// One line of text at (`x`, `y`) in a box `h` tall, with `spacing` px added after each
    /// character (negative tightens): the big numbers. Returns its width.
    fn text_tracked(
        &mut self,
        s: &str,
        f: &Font,
        x: f32,
        y: f32,
        h: f32,
        c: Color,
        spacing: f32,
    ) -> f32;

    /// A plain axis-aligned rectangle (a text selection).
    fn fill_rect(&mut self, l: f32, t: f32, r: f32, b: f32, c: Color);

    /// An ellipse filled with a radial gradient centred on it, its focus moved by `origin`
    /// (the metal sphere in Settings). `stops` are (position 0..1, colour), interpolated with
    /// gamma 2.2 as Direct2D's default.
    fn fill_radial(&mut self, e: &Ellipse, origin: Vector2, stops: &[(f32, Color)]);

    fn text_width(&self, s: &str, f: &Font) -> f32 {
        text_width(s, f)
    }

    fn center_text(&mut self, s: &str, f: &Font, cx: f32, y: f32, h: f32, c: Color) {
        let w = text_width(s, f);
        self.text(s, f, cx - w / 2.0, y, w + 2.0, h, c);
    }

    fn right_text(&mut self, s: &str, f: &Font, right: f32, y: f32, h: f32, c: Color) {
        let w = text_width(s, f);
        self.text(s, f, right - w, y, w + 2.0, h, c);
    }

    /// A circular arc clockwise from `from` to `to` degrees (from 12 o'clock); a full turn is a
    /// circle.
    fn arc(&mut self, cx: f32, cy: f32, r: f32, from: f32, to: f32, c: Color, w: f32, s: Stroke) {
        let sweep = to - from;
        if sweep <= 0.01 {
            return;
        }
        if sweep >= 359.9 {
            self.stroke_ellipse(&ellipse(cx, cy, r), c, w, s);
            return;
        }
        let geo = path(on_circle(cx, cy, r, from), false, |k| {
            arc_to(k, on_circle(cx, cy, r, to), r, sweep > 180.0, true)
        });
        self.stroke_path(&geo, c, w, s);
    }
}

/// The width of `s` in `f`, the same as drawing measures it.
#[cfg(windows)]
pub fn text_width(s: &str, f: &Font) -> f32 {
    crate::gfx::measure(s, f).0
}

/// The height of one line in `f`.
#[cfg(windows)]
pub fn line_height(f: &Font) -> f32 {
    crate::gfx::measure("0", f).1
}

/// The height of `s` wrapped at `w` in a wrapping font, rounded up to whole pixels.
#[cfg(windows)]
pub fn wrap_height(s: &str, f: &Font, w: f32) -> f32 {
    crate::gfx::wrap_height(s, f, w)
}

/// Where the baseline of `s` falls in a box `h` tall (one line, vertically centred).
#[cfg(windows)]
pub fn baseline_in(s: &str, f: &Font, h: f32) -> f32 {
    crate::gfx::baseline_in(s, f, h)
}

/// Names a font for caches: which font object, and its size.
#[cfg(windows)]
pub fn font_key(f: &Font) -> (usize, u32) {
    use windows::core::Interface;
    (f.as_raw() as usize, unsafe { f.GetFontSize() }.to_bits())
}

#[cfg(target_os = "macos")]
pub use crate::mac::text::{baseline_in, font_key, line_height, text_width, wrap_height};

/// `s` when it fits `w`, else its longest start that fits with an ellipsis after it.
pub fn ellipsize(s: &str, w: f32, measure: impl Fn(&str) -> f32) -> std::borrow::Cow<'_, str> {
    use std::borrow::Cow;
    if measure(s) <= w {
        return Cow::Borrowed(s);
    }
    let cut = |end: usize| format!("{}…", s[..end].trim_end());
    let ends: Vec<usize> = s.char_indices().map(|(i, _)| i).collect();
    // Binary search for the widest start that fits: widths only grow with length.
    let (mut lo, mut hi) = (0, ends.len());
    while lo + 1 < hi {
        let mid = (lo + hi) / 2;
        if measure(&cut(ends[mid])) <= w {
            lo = mid;
        } else {
            hi = mid;
        }
    }
    Cow::Owned(cut(ends.get(lo).copied().unwrap_or(0)))
}

/// One line of text, cut to `w` with an ellipsis.
pub fn clipped_text(
    cv: &mut dyn Canvas,
    s: &str,
    f: &Font,
    x: f32,
    y: f32,
    w: f32,
    h: f32,
    c: Color,
) {
    let w = w.max(0.0);
    let shown = ellipsize(s, w, |t| text_width(t, f));
    cv.push_clip(x, y, x + w + 2.0, y + h);
    cv.text(&shown, f, x, y, w + 400.0, h, c);
    cv.pop_clip();
}

#[cfg(test)]
mod tests {
    use super::*;

    fn close(a: f32, b: f32) -> bool {
        (a - b).abs() < 1e-3
    }

    #[test]
    fn a_half_circle_arc_has_its_centre_mid_chord() {
        let (cx, cy, _, _) = arc_center(pt(0.0, 0.0), pt(20.0, 0.0), 10.0, false, true);
        assert!(close(cx, 10.0) && close(cy, 0.0));
    }

    #[test]
    fn a_small_clockwise_arc_bulges_up_on_a_y_down_screen() {
        // From the left to the right point of a circle of r 10 around (10, 6): clockwise on
        // screen goes over the top, so the centre is below the chord.
        let (cx, cy, a0, a1) = arc_center(pt(2.0, 0.0), pt(18.0, 0.0), 10.0, false, true);
        assert!(close(cx, 10.0) && close(cy, 6.0), "{cx} {cy}");
        assert!(a0 < 0.0 && a1 < 0.0);
    }
}
