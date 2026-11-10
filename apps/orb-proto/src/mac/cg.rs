#![allow(clippy::too_many_arguments)]
//! The `Canvas` on Core Graphics: a premultiplied BGRA bitmap in device pixels, with a y-down
//! transform in logical pixels, so the shared drawing code runs unchanged. The bitmap becomes a
//! layer's contents (a window) or a PNG (the snapshot mode).
//!
//! Colours are given in sRGB (as the Windows tokens are); the bitmap itself is in the window's
//! colour space and the screen's native byte order, so Core Animation shows it without
//! converting every pixel each frame.

use std::ffi::c_void;

use objc2_core_foundation::{CFRetained, CGAffineTransform, CGFloat, CGPoint, CGRect, CGSize};
use objc2_core_graphics::{
    kCGColorSpaceSRGB, CGBitmapContextCreate, CGBitmapContextCreateImage, CGColor, CGColorSpace,
    CGContext, CGGradient, CGGradientDrawingOptions, CGImage, CGImageAlphaInfo,
    CGImageByteOrderInfo, CGLineCap, CGLineJoin,
};

use crate::canvas::{arc_center, Canvas, Color, Ellipse, Font, Path, RRect, Seg, Stroke, Vector2};

/// An offscreen RGBA bitmap, `w` x `h` logical pixels at `scale`.
pub struct Bitmap {
    ctx: CFRetained<CGContext>,
    data: Vec<u8>,
    pub pw: usize,
    pub ph: usize,
}

impl Bitmap {
    /// In sRGB (snapshots).
    pub fn new(w: f32, h: f32, scale: f32) -> Bitmap {
        let space = CGColorSpace::with_name(Some(unsafe { kCGColorSpaceSRGB })).expect("sRGB");
        Bitmap::with_space(w, h, scale, &space)
    }

    /// In a window's colour space.
    pub fn with_space(w: f32, h: f32, scale: f32, space: &CGColorSpace) -> Bitmap {
        let (pw, ph) = (
            ((w * scale).ceil() as usize).max(1),
            ((h * scale).ceil() as usize).max(1),
        );
        let mut data = vec![0u8; pw * ph * 4];
        // SAFETY: `data` is pw x ph x 4 bytes and lives as long as the context (both in Bitmap).
        let ctx = unsafe { context(data.as_mut_ptr(), pw, ph, pw * 4, scale, space) }
            .expect("a bitmap context");
        Bitmap { ctx, data, pw, ph }
    }

    pub fn canvas(&self) -> CgCanvas<'_> {
        CgCanvas::over(&self.ctx)
    }

    pub fn image(&self) -> Option<CFRetained<CGImage>> {
        CGBitmapContextCreateImage(Some(&self.ctx))
    }

    /// The pixels, premultiplied RGBA rows.
    pub fn rgba(&self) -> Vec<u8> {
        let mut v = self.data.clone();
        for p in v.chunks_mut(4) {
            p.swap(0, 2);
        }
        v
    }
}

/// A context drawing into `pw` x `ph` premultiplied BGRA pixels at `data` (`row` bytes a row),
/// set up as every Orby surface is: y down in logical pixels at `scale`, glyphs upright, no
/// font smoothing.
///
/// # Safety
/// `data` must hold `row * ph` writable bytes for as long as the context draws.
pub(crate) unsafe fn context(
    data: *mut u8,
    pw: usize,
    ph: usize,
    row: usize,
    scale: f32,
    space: &CGColorSpace,
) -> Option<CFRetained<CGContext>> {
    let ctx = unsafe {
        CGBitmapContextCreate(
            data as *mut c_void,
            pw,
            ph,
            8,
            row,
            Some(space),
            CGImageAlphaInfo::PremultipliedFirst.0 | CGImageByteOrderInfo::Order32Little.0,
        )
    }?;
    let c = Some(&*ctx);
    CGContext::translate_ctm(c, 0.0, ph as CGFloat);
    CGContext::scale_ctm(c, scale as CGFloat, -scale as CGFloat);
    // Glyphs upright in the flipped space.
    CGContext::set_text_matrix(
        c,
        CGAffineTransform {
            a: 1.0,
            b: 0.0,
            c: 0.0,
            d: -1.0,
            tx: 0.0,
            ty: 0.0,
        },
    );
    CGContext::set_should_smooth_fonts(c, false);
    Some(ctx)
}

/// Rounded rects covering at least this many device pixels are drawn in parts (see
/// `fill_big_rrect`): a Disk window at 2x is 1.5 million.
const BIG: f64 = 160_000.0;

pub struct CgCanvas<'a> {
    ctx: &'a CGContext,
    saves: usize,
}

fn rect(l: f32, t: f32, r: f32, b: f32) -> CGRect {
    CGRect {
        origin: CGPoint {
            x: l as CGFloat,
            y: t as CGFloat,
        },
        size: CGSize {
            width: (r - l) as CGFloat,
            height: (b - t) as CGFloat,
        },
    }
}

fn srgb(c: Color) -> CFRetained<CGColor> {
    CGColor::new_srgb(
        c.r as CGFloat,
        c.g as CGFloat,
        c.b as CGFloat,
        c.a as CGFloat,
    )
}

impl<'a> CgCanvas<'a> {
    /// The Canvas over a context made by `context`.
    pub(crate) fn over(ctx: &'a CGContext) -> CgCanvas<'a> {
        CgCanvas { ctx, saves: 0 }
    }
}

impl CgCanvas<'_> {
    fn c(&self) -> Option<&CGContext> {
        Some(self.ctx)
    }

    fn fill(&self, col: Color) {
        CGContext::set_fill_color_with_color(self.c(), Some(&srgb(col)));
    }

    fn stroke_style(&self, col: Color, w: f32, s: Stroke) {
        let c = self.c();
        CGContext::set_stroke_color_with_color(c, Some(&srgb(col)));
        CGContext::set_line_width(c, w as CGFloat);
        CGContext::set_miter_limit(c, 10.0);
        let round = matches!(s, Stroke::Round | Stroke::Dotted);
        CGContext::set_line_cap(
            c,
            if round {
                CGLineCap::Round
            } else {
                CGLineCap::Butt
            },
        );
        CGContext::set_line_join(
            c,
            if round {
                CGLineJoin::Round
            } else {
                CGLineJoin::Miter
            },
        );
        match s.dashes() {
            Some((d, _)) => {
                let lens: Vec<CGFloat> = d.iter().map(|x| (*x * w) as CGFloat).collect();
                unsafe { CGContext::set_line_dash(c, 0.0, lens.as_ptr(), lens.len()) };
            }
            None => unsafe { CGContext::set_line_dash(c, 0.0, std::ptr::null(), 0) },
        }
    }

    fn add_rrect(&self, r: &RRect) {
        let c = self.c();
        let rad = r.rad.min((r.r - r.l) / 2.0).min((r.b - r.t) / 2.0).max(0.0) as CGFloat;
        let (l, t, rr, b) = (
            r.l as CGFloat,
            r.t as CGFloat,
            r.r as CGFloat,
            r.b as CGFloat,
        );
        CGContext::move_to_point(c, l + rad, t);
        CGContext::add_arc_to_point(c, rr, t, rr, b, rad);
        CGContext::add_arc_to_point(c, rr, b, l, b, rad);
        CGContext::add_arc_to_point(c, l, b, l, t, rad);
        CGContext::add_arc_to_point(c, l, t, rr, t, rad);
        CGContext::close_path(c);
    }

    fn add_ellipse(&self, e: &Ellipse) {
        CGContext::add_ellipse_in_rect(
            self.c(),
            rect(
                e.point.X - e.rx,
                e.point.Y - e.ry,
                e.point.X + e.rx,
                e.point.Y + e.ry,
            ),
        );
    }

    fn add_path(&self, p: &Path) {
        let c = self.c();
        CGContext::move_to_point(c, p.start.X as CGFloat, p.start.Y as CGFloat);
        let mut at = p.start;
        for seg in &p.segs {
            match *seg {
                Seg::Line(to) => {
                    CGContext::add_line_to_point(c, to.X as CGFloat, to.Y as CGFloat);
                    at = to;
                }
                Seg::Quad(k, to) => {
                    CGContext::add_quad_curve_to_point(
                        c,
                        k.X as CGFloat,
                        k.Y as CGFloat,
                        to.X as CGFloat,
                        to.Y as CGFloat,
                    );
                    at = to;
                }
                Seg::Arc { to, r, large, cw } => {
                    let (cx, cy, a0, a1) = arc_center(at, to, r, large, cw);
                    let rad = r.max(((to.X - at.X).powi(2) + (to.Y - at.Y).powi(2)).sqrt() / 2.0);
                    // In this y-down space a clockwise turn on screen is the increasing angle,
                    // which Core Graphics calls counter-clockwise (0).
                    CGContext::add_arc(
                        c,
                        cx as CGFloat,
                        cy as CGFloat,
                        rad as CGFloat,
                        a0 as CGFloat,
                        a1 as CGFloat,
                        if cw { 0 } else { 1 },
                    );
                    at = to;
                }
            }
        }
        if p.filled {
            CGContext::close_path(c);
        }
    }

    /// The device transform as (scale, x scale sign, y scale sign, x move, y move) when it only
    /// scales evenly and moves, as on every Orby surface.
    fn plain_ctm(&self) -> Option<(f64, f64, f64, f64, f64)> {
        let m = CGContext::ctm(self.c());
        (m.b == 0.0 && m.c == 0.0 && m.a.abs() == m.d.abs() && m.a != 0.0).then_some((
            m.a.abs(),
            m.a,
            m.d,
            m.tx,
            m.ty,
        ))
    }

    /// The rounded rect in device pixels (left, top, right, bottom, whichever way y runs) and
    /// the scale, when it is big enough to be worth drawing in parts.
    fn big_device_rect(&self, r: &RRect, grow: f64) -> Option<([f64; 4], f64)> {
        let (k, a, d, tx, ty) = self.plain_ctm()?;
        let (xa, xb) = (a * r.l as f64 + tx, a * r.r as f64 + tx);
        let (ya, yb) = (d * r.t as f64 + ty, d * r.b as f64 + ty);
        let dr = [
            xa.min(xb) - grow,
            ya.min(yb) - grow,
            xa.max(xb) + grow,
            ya.max(yb) + grow,
        ];
        ((dr[2] - dr[0]) * (dr[3] - dr[1]) >= BIG).then_some((dr, k))
    }

    /// A device-pixel rect back in the context's own (logical) space.
    fn user_rect(&self, l: f64, t: f64, r: f64, b: f64) -> CGRect {
        let (_, a, d, tx, ty) = self.plain_ctm().unwrap_or((1.0, 1.0, 1.0, 0.0, 0.0));
        let (x0, x1) = ((l - tx) / a, (r - tx) / a);
        let (y0, y1) = ((t - ty) / d, (b - ty) / d);
        CGRect {
            origin: CGPoint {
                x: x0.min(x1),
                y: y0.min(y1),
            },
            size: CGSize {
                width: (x1 - x0).abs(),
                height: (y1 - y0).abs(),
            },
        }
    }

    /// A big rounded rect on whole device pixels (a window's card, a scrim) filled in parts:
    /// what lies past its corners is fully covered, so it is three plain rectangles; only the
    /// four corner squares go through the anti-aliased path fill (clipped to each). The pixels
    /// are the ones a single path fill gives (the Mac snapshots are byte-identical), but the
    /// rasterizer no longer walks the whole window twice a frame.
    fn fill_big_rrect(&mut self, r: &RRect) -> bool {
        let Some(([x0, y0, x1, y1], s)) = self.big_device_rect(r, 0.0) else {
            return false;
        };
        let whole = |v: f64| (v - v.round()).abs() < 1e-6;
        if !(whole(x0) && whole(y0) && whole(x1) && whole(y1)) {
            return false;
        }
        let rad = r.rad.min((r.r - r.l) / 2.0).min((r.b - r.t) / 2.0).max(0.0) as f64;
        let k = (rad * s).ceil();
        if 2.0 * k > x1 - x0 || 2.0 * k > y1 - y0 {
            return false;
        }
        let c = self.c();
        CGContext::fill_rect(c, self.user_rect(x0 + k, y0, x1 - k, y1));
        if k > 0.0 {
            CGContext::fill_rect(c, self.user_rect(x0, y0 + k, x0 + k, y1 - k));
            CGContext::fill_rect(c, self.user_rect(x1 - k, y0 + k, x1, y1 - k));
            for (l, t) in [(x0, y0), (x1 - k, y0), (x0, y1 - k), (x1 - k, y1 - k)] {
                self.save();
                CGContext::clip_to_rect(self.c(), self.user_rect(l, t, l + k, t + k));
                self.add_rrect(r);
                CGContext::fill_path(self.c());
                self.restore();
            }
        }
        true
    }

    /// A big rounded rect's plain stroke drawn as the same stroke clipped to its four corner
    /// squares and four edge bands (whole device pixels, not overlapping, covering every pixel
    /// the stroke touches): the same pixels, without rasterizing the whole window's area.
    fn stroke_big_rrect(&mut self, r: &RRect, w: f32) -> bool {
        let Some(([ox0, oy0, ox1, oy1], s)) = self.big_device_rect(r, 0.0) else {
            return false;
        };
        let sw = w as f64 * s;
        let (x0, y0) = (
            (ox0 - sw / 2.0).floor() - 1.0,
            (oy0 - sw / 2.0).floor() - 1.0,
        );
        let (x1, y1) = ((ox1 + sw / 2.0).ceil() + 1.0, (oy1 + sw / 2.0).ceil() + 1.0);
        let rad = r.rad.min((r.r - r.l) / 2.0).min((r.b - r.t) / 2.0).max(0.0) as f64;
        let k = (rad * s + sw).ceil() + 2.0;
        let band = sw.ceil() + 2.0;
        if 2.0 * k > x1 - x0 || 2.0 * k > y1 - y0 {
            return false;
        }
        let parts = [
            (x0, y0, x0 + k, y0 + k),
            (x1 - k, y0, x1, y0 + k),
            (x0, y1 - k, x0 + k, y1),
            (x1 - k, y1 - k, x1, y1),
            (x0 + k, y0, x1 - k, y0 + band),
            (x0 + k, y1 - band, x1 - k, y1),
            (x0, y0 + k, x0 + band, y1 - k),
            (x1 - band, y0 + k, x1, y1 - k),
        ];
        for (l, t, rr, b) in parts {
            self.save();
            CGContext::clip_to_rect(self.c(), self.user_rect(l, t, rr, b));
            self.add_rrect(r);
            CGContext::stroke_path(self.c());
            self.restore();
        }
        true
    }

    fn save(&mut self) {
        CGContext::save_g_state(self.c());
        self.saves += 1;
    }

    fn restore(&mut self) {
        if self.saves > 0 {
            CGContext::restore_g_state(self.c());
            self.saves -= 1;
        }
    }
}

impl Canvas for CgCanvas<'_> {
    fn fill_rrect(&mut self, r: &RRect, c: Color) {
        self.fill(c);
        if self.fill_big_rrect(r) {
            return;
        }
        self.add_rrect(r);
        CGContext::fill_path(self.c());
    }

    fn stroke_rrect(&mut self, r: &RRect, c: Color, w: f32, s: Stroke) {
        self.stroke_style(c, w, s);
        if matches!(s, Stroke::Plain) && self.stroke_big_rrect(r, w) {
            return;
        }
        self.add_rrect(r);
        CGContext::stroke_path(self.c());
    }

    fn fill_ellipse(&mut self, e: &Ellipse, c: Color) {
        self.fill(c);
        self.add_ellipse(e);
        CGContext::fill_path(self.c());
    }

    fn stroke_ellipse(&mut self, e: &Ellipse, c: Color, w: f32, s: Stroke) {
        self.stroke_style(c, w, s);
        self.add_ellipse(e);
        CGContext::stroke_path(self.c());
    }

    fn line(&mut self, a: Vector2, b: Vector2, c: Color, w: f32, s: Stroke) {
        self.stroke_style(c, w, s);
        CGContext::move_to_point(self.c(), a.X as CGFloat, a.Y as CGFloat);
        CGContext::add_line_to_point(self.c(), b.X as CGFloat, b.Y as CGFloat);
        CGContext::stroke_path(self.c());
    }

    fn fill_path(&mut self, p: &Path, c: Color) {
        self.fill(c);
        self.add_path(p);
        CGContext::eo_fill_path(self.c());
    }

    fn stroke_path(&mut self, p: &Path, c: Color, w: f32, s: Stroke) {
        self.stroke_style(c, w, s);
        self.add_path(p);
        CGContext::stroke_path(self.c());
    }

    fn text(&mut self, s: &str, f: &Font, x: f32, y: f32, w: f32, h: f32, c: Color) {
        self.fill(c);
        super::text::draw(self.ctx, s, f, x, y, w, h);
    }

    fn text_tracked(
        &mut self,
        s: &str,
        f: &Font,
        x: f32,
        y: f32,
        h: f32,
        c: Color,
        spacing: f32,
    ) -> f32 {
        self.fill(c);
        super::text::draw_tracked(self.ctx, s, f, x, y, h, spacing)
    }

    fn fill_rect(&mut self, l: f32, t: f32, r: f32, b: f32, c: Color) {
        self.fill(c);
        CGContext::fill_rect(self.c(), rect(l, t, r, b));
    }

    fn fill_radial(&mut self, e: &Ellipse, origin: Vector2, stops: &[(f32, Color)]) {
        if stops.is_empty() {
            return;
        }
        // Direct2D blends the stops in gamma 2.2; Core Graphics blends linearly in the colour
        // space's encoded values, which in sRGB is close to the same curve.
        let mut comps: Vec<CGFloat> = Vec::with_capacity(stops.len() * 4);
        let mut locs: Vec<CGFloat> = Vec::with_capacity(stops.len());
        for (p, c) in stops {
            comps.extend([c.r, c.g, c.b, c.a].map(|v| v as CGFloat));
            locs.push(*p as CGFloat);
        }
        let Some(space) = CGColorSpace::with_name(Some(unsafe { kCGColorSpaceSRGB })) else {
            return;
        };
        let Some(grad) = (unsafe {
            CGGradient::with_color_components(
                Some(&space),
                comps.as_ptr(),
                locs.as_ptr(),
                locs.len(),
            )
        }) else {
            return;
        };
        self.save();
        self.add_ellipse(e);
        CGContext::clip(self.c());
        let focus = CGPoint {
            x: (e.point.X + origin.X) as CGFloat,
            y: (e.point.Y + origin.Y) as CGFloat,
        };
        let centre = CGPoint {
            x: e.point.X as CGFloat,
            y: e.point.Y as CGFloat,
        };
        CGContext::draw_radial_gradient(
            self.c(),
            Some(&grad),
            focus,
            0.0,
            centre,
            e.rx.max(e.ry) as CGFloat,
            CGGradientDrawingOptions::DrawsAfterEndLocation,
        );
        self.restore();
    }

    fn lip(&mut self, c: Color, dy: f32, l: f32, t: f32, r: f32, b: f32, radius: f32) {
        self.save();
        self.add_rrect(&RRect {
            l,
            t,
            r,
            b,
            rad: radius,
        });
        CGContext::clip(self.c());
        self.fill(c);
        self.add_rrect(&RRect {
            l,
            t,
            r,
            b,
            rad: radius,
        });
        self.add_rrect(&RRect {
            l,
            t: t + dy,
            r,
            b: b + dy,
            rad: radius,
        });
        CGContext::eo_fill_path(self.c());
        self.restore();
    }

    fn push_clip(&mut self, l: f32, t: f32, r: f32, b: f32) {
        self.save();
        CGContext::clip_to_rect(self.c(), rect(l, t, r, b));
    }

    fn pop_clip(&mut self) {
        self.restore();
    }

    fn push_layer(&mut self, opacity: f32) {
        self.save();
        CGContext::set_alpha(self.c(), opacity as CGFloat);
        unsafe { CGContext::begin_transparency_layer(self.c(), None) };
    }

    fn pop_layer(&mut self) {
        CGContext::end_transparency_layer(self.c());
        self.restore();
    }

    fn push_translate(&mut self, dx: f32, dy: f32) {
        self.save();
        CGContext::translate_ctm(self.c(), dx as CGFloat, dy as CGFloat);
    }

    fn pop_translate(&mut self) {
        self.restore();
    }
}
