//! Inter on macOS: the bundled variable font through Core Text, with the same axes as Windows
//! (`wght` = the weight, `opsz` = the size clamped to 14..32) and the same features (tabular
//! figures, the single-storey a: tnum and cv11). Nothing is installed on the system.

use std::ptr;

use objc2_core_foundation::{
    CFArray, CFAttributedString, CFData, CFDictionary, CFNumber, CFRange, CFRetained, CFString,
    CFType, CGFloat,
};
use objc2_core_graphics::CGContext;
use objc2_core_text::{
    kCTFontAttributeName, kCTFontFeatureSettingsAttribute, kCTFontOpenTypeFeatureTag,
    kCTFontOpenTypeFeatureValue, kCTForegroundColorFromContextAttributeName,
    CTFontManagerCreateFontDescriptorFromData, CTLine, CTTypesetter,
};

static INTER_TTF: &[u8] = include_bytes!("../../assets/fonts/InterVariable.ttf");

const WGHT: i32 = 0x7767_6874; // 'wght'
const OPSZ: i32 = 0x6f70_737a; // 'opsz'

/// One of the app's text styles: a Core Text font and its line metrics.
pub struct Font {
    ct: CFRetained<objc2_core_text::CTFont>,
    pub size: f32,
    /// Wraps at the box width, top aligned (the Windows "wrap" formats).
    pub wrap: bool,
    ascent: f32,
    descent: f32,
    leading: f32,
}

impl Font {
    /// The height of one line: ascent, descent and leading.
    pub fn line(&self) -> f32 {
        self.ascent + self.descent + self.leading
    }
}

fn feature(tag: &str) -> CFRetained<CFDictionary<CFString, CFType>> {
    let tag = CFString::from_str(tag);
    let one = CFNumber::new_i32(1);
    unsafe {
        CFDictionary::from_slices(
            &[kCTFontOpenTypeFeatureTag, kCTFontOpenTypeFeatureValue],
            &[tag.as_ref(), one.as_ref()],
        )
    }
}

/// Inter at `size` px and `weight` (400 regular, 500 medium).
pub fn font(size: f32, weight: f32, wrap: bool) -> Font {
    unsafe {
        let data = CFData::from_static_bytes(INTER_TTF);
        let base = CTFontManagerCreateFontDescriptorFromData(&data)
            .expect("the bundled Inter loads through Core Text");
        let d = base.copy_with_variation(&CFNumber::new_i32(WGHT), weight as CGFloat);
        let d = d.copy_with_variation(&CFNumber::new_i32(OPSZ), size.clamp(14.0, 32.0) as CGFloat);
        let feats = CFArray::from_retained_objects(&[feature("tnum"), feature("cv11")]);
        let attrs: CFRetained<CFDictionary<CFString, CFType>> =
            CFDictionary::from_slices(&[kCTFontFeatureSettingsAttribute], &[feats.as_ref()]);
        let d = d.copy_with_attributes(attrs.as_ref());
        let ct = objc2_core_text::CTFont::with_font_descriptor(&d, size as CGFloat, ptr::null());
        Font {
            ascent: ct.ascent() as f32,
            descent: ct.descent() as f32,
            leading: ct.leading() as f32,
            ct,
            size,
            wrap,
        }
    }
}

fn attributed(s: &str, f: &Font) -> CFRetained<CFAttributedString> {
    let text = CFString::from_str(s);
    unsafe {
        let attrs: CFRetained<CFDictionary<CFString, CFType>> = CFDictionary::from_slices(
            &[
                kCTFontAttributeName,
                kCTForegroundColorFromContextAttributeName,
            ],
            &[
                f.ct.as_ref(),
                objc2_core_foundation::CFBoolean::new(true).as_ref(),
            ],
        );
        CFAttributedString::new(None, Some(&text), Some(attrs.as_ref()))
            .expect("an attributed string")
    }
}

fn line_of(s: &str, f: &Font) -> CFRetained<CTLine> {
    unsafe { CTLine::with_attributed_string(&attributed(s, f)) }
}

/// The width of `s` in `f`, trailing spaces included (as DirectWrite measures it).
pub fn text_width(s: &str, f: &Font) -> f32 {
    if s.is_empty() {
        return 0.0;
    }
    unsafe {
        line_of(s, f).typographic_bounds(ptr::null_mut(), ptr::null_mut(), ptr::null_mut()) as f32
    }
}

pub fn line_height(f: &Font) -> f32 {
    f.line()
}

/// Where the baseline falls in a box `h` tall: the line is centred, as `draw` places it.
pub fn baseline_in(_s: &str, f: &Font, h: f32) -> f32 {
    (h - f.line()) / 2.0 + f.ascent
}

/// Names a font for caches: the Core Text font object and its size.
pub fn font_key(f: &Font) -> (usize, u32) {
    (
        &*f.ct as *const objc2_core_text::CTFont as usize,
        f.size.to_bits(),
    )
}

/// How many lines `s` takes wrapped at `w`, breaking where `draw` breaks.
fn wrapped_lines(s: &str, f: &Font, w: f32) -> usize {
    if s.is_empty() {
        return 0;
    }
    unsafe {
        let a = attributed(s, f);
        let ts = CTTypesetter::with_attributed_string(&a);
        let len = s.encode_utf16().count() as isize;
        let (mut start, mut n) = (0isize, 0usize);
        while start < len {
            start += ts.suggest_line_break(start, w.max(1.0) as f64).max(1);
            n += 1;
        }
        n
    }
}

/// The height of `s` wrapped at `w`, rounded up to whole pixels.
pub fn wrap_height(s: &str, f: &Font, w: f32) -> f32 {
    (wrapped_lines(s, f, w).max(1) as f32 * f.line()).ceil()
}

/// One line with `spacing` px of tracking after each character, its baseline as `draw` puts
/// it in a box `h` tall. Returns its width.
pub fn draw_tracked(
    ctx: &CGContext,
    s: &str,
    f: &Font,
    x: f32,
    y: f32,
    h: f32,
    spacing: f32,
) -> f32 {
    if s.is_empty() {
        return 0.0;
    }
    unsafe {
        let text = CFString::from_str(s);
        let kern = CFNumber::new_f64(spacing as f64);
        let attrs: CFRetained<CFDictionary<CFString, CFType>> = CFDictionary::from_slices(
            &[
                kCTFontAttributeName,
                kCTForegroundColorFromContextAttributeName,
                objc2_core_text::kCTKernAttributeName,
            ],
            &[
                f.ct.as_ref(),
                objc2_core_foundation::CFBoolean::new(true).as_ref(),
                kern.as_ref(),
            ],
        );
        let a = CFAttributedString::new(None, Some(&text), Some(attrs.as_ref()))
            .expect("an attributed string");
        let line = CTLine::with_attributed_string(&a);
        let base = y + (h - f.line()) / 2.0 + f.ascent;
        CGContext::set_text_position(Some(ctx), x as CGFloat, base as CGFloat);
        line.draw(ctx);
        line.typographic_bounds(ptr::null_mut(), ptr::null_mut(), ptr::null_mut()) as f32
    }
}

/// Draws `s` into the box. The context must already be y-down with a flipped text matrix and the
/// fill colour set: one line vertically centred, or (a wrapping font) lines from the top.
pub fn draw(ctx: &CGContext, s: &str, f: &Font, x: f32, y: f32, w: f32, h: f32) {
    if s.is_empty() {
        return;
    }
    unsafe {
        if !f.wrap {
            let base = y + (h - f.line()) / 2.0 + f.ascent;
            CGContext::set_text_position(Some(ctx), x as CGFloat, base as CGFloat);
            line_of(s, f).draw(ctx);
            return;
        }
        let a = attributed(s, f);
        let ts = CTTypesetter::with_attributed_string(&a);
        let len = s.encode_utf16().count() as isize;
        let (mut start, mut top) = (0isize, y);
        while start < len && top + f.line() <= y + h + 0.5 {
            let n = ts.suggest_line_break(start, w as f64).max(1);
            let line = ts.line(CFRange {
                location: start,
                length: n,
            });
            CGContext::set_text_position(Some(ctx), x as CGFloat, (top + f.ascent) as CGFloat);
            line.draw(ctx);
            start += n;
            top += f.line();
        }
    }
}
