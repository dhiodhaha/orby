//! Inter on macOS: the bundled variable font through Core Text, with the same axes as Windows
//! (`wght` = the weight, `opsz` = the size clamped to 14..32) and the same features (tabular
//! figures, the single-storey a: tnum and cv11). Nothing is installed on the system.
//!
//! Shaping is the costly part of text (Inter's kerning and features, per string): a list that
//! scrolls draws the same names, paths and sizes frame after frame. So the typeset lines are
//! kept per font and string (`LINES`, the strings drawn; `WRAPS`, wrapped text per width) and
//! widths per font and string (`WIDTHS`, also the many trial cuts an ellipsis measures). A line
//! takes its colour from the context when drawn, so one line serves every colour. Each cache
//! is bounded and simply starts over when full.

use std::cell::RefCell;
use std::collections::HashMap;
use std::ptr;
use std::sync::atomic::{AtomicU64, Ordering};

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
    /// Unique for the life of the process (cache keys: an address can be used again).
    id: u64,
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
        static NEXT: AtomicU64 = AtomicU64::new(1);
        Font {
            id: NEXT.fetch_add(1, Ordering::Relaxed),
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

fn width_of(line: &CTLine) -> f32 {
    unsafe { line.typographic_bounds(ptr::null_mut(), ptr::null_mut(), ptr::null_mut()) as f32 }
}

/// Strings by font (and tracking or wrap width), each map bounded.
struct Cache<V> {
    maps: HashMap<(u64, u32), HashMap<String, V>>,
    len: usize,
    most: usize,
}

impl<V: Clone> Cache<V> {
    fn new(most: usize) -> Self {
        Cache {
            maps: HashMap::new(),
            len: 0,
            most,
        }
    }

    fn get(&self, k: (u64, u32), s: &str) -> Option<V> {
        self.maps.get(&k).and_then(|m| m.get(s)).cloned()
    }

    fn put(&mut self, k: (u64, u32), s: &str, v: V) {
        if self.len >= self.most {
            self.maps.clear();
            self.len = 0;
        }
        if self
            .maps
            .entry(k)
            .or_default()
            .insert(s.to_owned(), v)
            .is_none()
        {
            self.len += 1;
        }
    }
}

thread_local! {
    /// Lines that were drawn: one line, or one with tracking (the key's second half).
    static LINES: RefCell<Cache<CFRetained<CTLine>>> = RefCell::new(Cache::new(2048));
    /// Wrapped text: its lines at a width (the key's second half).
    static WRAPS: RefCell<Cache<std::rc::Rc<Vec<CFRetained<CTLine>>>>> = RefCell::new(Cache::new(256));
    /// Widths, drawn or only measured.
    static WIDTHS: RefCell<Cache<f32>> = RefCell::new(Cache::new(16384));
}

/// The line `s` in `f` (with `spacing` px tracking when not 0), typeset once.
fn cached_line(s: &str, f: &Font, spacing: f32) -> CFRetained<CTLine> {
    let k = (f.id, spacing.to_bits());
    if let Some(l) = LINES.with(|c| c.borrow().get(k, s)) {
        return l;
    }
    let l = if spacing == 0.0 {
        line_of(s, f)
    } else {
        tracked_line(s, f, spacing)
    };
    LINES.with(|c| c.borrow_mut().put(k, s, l.clone()));
    l
}

/// The width of `s` in `f`, trailing spaces included (as DirectWrite measures it).
pub fn text_width(s: &str, f: &Font) -> f32 {
    if s.is_empty() {
        return 0.0;
    }
    let k = (f.id, 0);
    if let Some(w) = WIDTHS.with(|c| c.borrow().get(k, s)) {
        return w;
    }
    // A line drawn before is measured from its cache; a trial cut is typeset and let go.
    let w = match LINES.with(|c| c.borrow().get(k, s)) {
        Some(l) => width_of(&l),
        None => width_of(&line_of(s, f)),
    };
    WIDTHS.with(|c| c.borrow_mut().put(k, s, w));
    w
}

/// `s` broken into lines at `w` as `draw` breaks it, typeset once per width.
fn wrapped(s: &str, f: &Font, w: f32) -> std::rc::Rc<Vec<CFRetained<CTLine>>> {
    let k = (f.id, w.to_bits());
    if let Some(v) = WRAPS.with(|c| c.borrow().get(k, s)) {
        return v;
    }
    let mut lines = Vec::new();
    unsafe {
        let a = attributed(s, f);
        let ts = CTTypesetter::with_attributed_string(&a);
        let len = s.encode_utf16().count() as isize;
        let mut start = 0isize;
        while start < len {
            let n = ts.suggest_line_break(start, w as f64).max(1);
            lines.push(ts.line(CFRange {
                location: start,
                length: n,
            }));
            start += n;
        }
    }
    let v = std::rc::Rc::new(lines);
    WRAPS.with(|c| c.borrow_mut().put(k, s, v.clone()));
    v
}

pub fn line_height(f: &Font) -> f32 {
    f.line()
}

/// Where the baseline falls in a box `h` tall: the line is centred, as `draw` places it.
pub fn baseline_in(_s: &str, f: &Font, h: f32) -> f32 {
    (h - f.line()) / 2.0 + f.ascent
}

/// Names a font for caches: the font (unique for the life of the process) and its size.
pub fn font_key(f: &Font) -> (usize, u32) {
    (f.id as usize, f.size.to_bits())
}

/// How many lines `s` takes wrapped at `w`, breaking where `draw` breaks.
fn wrapped_lines(s: &str, f: &Font, w: f32) -> usize {
    if s.is_empty() {
        return 0;
    }
    wrapped(s, f, w.max(1.0)).len()
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
    let line = cached_line(s, f, spacing);
    let base = y + (h - f.line()) / 2.0 + f.ascent;
    CGContext::set_text_position(Some(ctx), x as CGFloat, base as CGFloat);
    unsafe { line.draw(ctx) };
    width_of(&line)
}

/// One line with `spacing` px of tracking after each character.
fn tracked_line(s: &str, f: &Font, spacing: f32) -> CFRetained<CTLine> {
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
        CTLine::with_attributed_string(&a)
    }
}

/// Draws `s` into the box. The context must already be y-down with a flipped text matrix and the
/// fill colour set: one line vertically centred, or (a wrapping font) lines from the top.
pub fn draw(ctx: &CGContext, s: &str, f: &Font, x: f32, y: f32, w: f32, h: f32) {
    if s.is_empty() {
        return;
    }
    if !f.wrap {
        let base = y + (h - f.line()) / 2.0 + f.ascent;
        CGContext::set_text_position(Some(ctx), x as CGFloat, base as CGFloat);
        unsafe { cached_line(s, f, 0.0).draw(ctx) };
        return;
    }
    let mut top = y;
    for line in wrapped(s, f, w).iter() {
        if top + f.line() > y + h + 0.5 {
            break;
        }
        CGContext::set_text_position(Some(ctx), x as CGFloat, (top + f.ascent) as CGFloat);
        unsafe { line.draw(ctx) };
        top += f.line();
    }
}
