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
    CGContext, CGImage, CGImageAlphaInfo, CGImageByteOrderInfo, CGLineCap, CGLineJoin,
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
        let ctx = unsafe {
            CGBitmapContextCreate(
                data.as_mut_ptr() as *mut c_void,
                pw,
                ph,
                8,
                pw * 4,
                Some(space),
                CGImageAlphaInfo::PremultipliedFirst.0 | CGImageByteOrderInfo::Order32Little.0,
            )
            .expect("a bitmap context")
        };
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
        Bitmap { ctx, data, pw, ph }
    }

    pub fn canvas(&self) -> CgCanvas<'_> {
        CgCanvas {
            ctx: &self.ctx,
            saves: 0,
        }
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
        self.add_rrect(r);
        CGContext::fill_path(self.c());
    }

    fn stroke_rrect(&mut self, r: &RRect, c: Color, w: f32, s: Stroke) {
        self.stroke_style(c, w, s);
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
