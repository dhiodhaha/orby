//! How a window's drawing reaches the screen: a small ring of IOSurfaces per view.
//!
//! Each frame draws through the shared Canvas straight into an IOSurface the compositor is not
//! reading (premultiplied BGRA, the window's colour space, the screen's backing scale), which
//! then becomes the layer's contents. Nothing is allocated, zeroed by the allocator or copied
//! per frame: before, every frame made a new bitmap (4 to 8 MB, zeroed), a CGImage of it, and
//! Core Animation copied that image again for the window server. A surface is reused once the
//! window server is done with it (`IOSurfaceIsInUse`), so three are enough at 60 Hz; they are
//! made again only when the size or the colour space changes, and let go when the window
//! closes (`release`). `ORB_SURFACE=0` goes back to the bitmap and CGImage path (for
//! comparing).

use std::cell::RefCell;
use std::collections::HashMap;
use std::ffi::c_void;
use std::sync::OnceLock;

use objc2::runtime::AnyObject;
use objc2_app_kit::NSView;
use objc2_core_foundation::{CFDictionary, CFNumber, CFRetained, CFString, CFType, Type};
use objc2_core_graphics::CGColorSpace;
use objc2_quartz_core::CATransaction;

use super::cg::{self, Bitmap, CgCanvas};
use super::frames::Stopwatch;
use crate::canvas::Canvas;

#[link(name = "IOSurface", kind = "framework")]
extern "C" {
    static kIOSurfaceWidth: &'static CFString;
    static kIOSurfaceHeight: &'static CFString;
    static kIOSurfaceBytesPerElement: &'static CFString;
    static kIOSurfaceBytesPerRow: &'static CFString;
    static kIOSurfacePixelFormat: &'static CFString;
    static kIOSurfaceColorSpace: &'static CFString;
    fn IOSurfaceCreate(props: *const c_void) -> *mut c_void;
    fn IOSurfaceLock(s: *mut c_void, options: u32, seed: *mut u32) -> i32;
    fn IOSurfaceUnlock(s: *mut c_void, options: u32, seed: *mut u32) -> i32;
    fn IOSurfaceGetBaseAddress(s: *mut c_void) -> *mut c_void;
    fn IOSurfaceGetBytesPerRow(s: *mut c_void) -> usize;
    fn IOSurfaceIsInUse(s: *mut c_void) -> u8;
    fn IOSurfaceSetValue(s: *mut c_void, key: *const c_void, value: *const c_void);
    fn IOSurfaceAlignProperty(key: *const c_void, value: usize) -> usize;
}

#[link(name = "CoreGraphics", kind = "framework")]
extern "C" {
    fn CGColorSpaceCopyPropertyList(space: *const c_void) -> *const c_void;
}

#[link(name = "CoreFoundation", kind = "framework")]
extern "C" {
    fn CFRelease(p: *const c_void);
    fn CFEqual(a: *const c_void, b: *const c_void) -> u8;
}

/// 'BGRA': premultiplied first, little endian, as the Canvas draws.
const BGRA: i32 = 0x4247_5241;
/// More than this many surfaces in use at once means the window server is far behind: the
/// frame goes through a bitmap instead of a fifth surface.
const MOST: usize = 4;

fn enabled() -> bool {
    static ON: OnceLock<bool> = OnceLock::new();
    *ON.get_or_init(|| std::env::var_os("ORB_SURFACE").is_none_or(|v| v != "0"))
}

/// One retained IOSurface.
struct Surf(*mut c_void);

impl Drop for Surf {
    fn drop(&mut self) {
        // SAFETY: made by IOSurfaceCreate, released once.
        unsafe { CFRelease(self.0) };
    }
}

impl Surf {
    fn new(pw: usize, ph: usize, space: &CGColorSpace) -> Option<Surf> {
        unsafe {
            let row = IOSurfaceAlignProperty(
                kIOSurfaceBytesPerRow as *const CFString as *const c_void,
                pw * 4,
            );
            let nums = [
                CFNumber::new_isize(pw as isize),
                CFNumber::new_isize(ph as isize),
                CFNumber::new_i32(4),
                CFNumber::new_isize(row as isize),
                CFNumber::new_i32(BGRA),
            ];
            let keys = [
                kIOSurfaceWidth,
                kIOSurfaceHeight,
                kIOSurfaceBytesPerElement,
                kIOSurfaceBytesPerRow,
                kIOSurfacePixelFormat,
            ];
            let vals: Vec<&CFType> = nums.iter().map(|n| n.as_ref()).collect();
            let props: CFRetained<CFDictionary<CFString, CFType>> =
                CFDictionary::from_slices(&keys, &vals);
            let s =
                IOSurfaceCreate(&*props as *const CFDictionary<CFString, CFType> as *const c_void);
            if s.is_null() {
                return None;
            }
            // Tagged with the colour space the pixels are in, so the compositor matches them
            // as it matched the CGImage before.
            let plist = CGColorSpaceCopyPropertyList(space as *const CGColorSpace as *const c_void);
            if !plist.is_null() {
                IOSurfaceSetValue(
                    s,
                    kIOSurfaceColorSpace as *const CFString as *const c_void,
                    plist,
                );
                CFRelease(plist);
            }
            Some(Surf(s))
        }
    }

    fn in_use(&self) -> bool {
        // SAFETY: a live surface.
        unsafe { IOSurfaceIsInUse(self.0) != 0 }
    }
}

/// The surfaces of one view.
struct Ring {
    pw: usize,
    ph: usize,
    space: CFRetained<CGColorSpace>,
    bufs: Vec<Surf>,
    /// The one the layer shows now.
    shown: Option<usize>,
}

thread_local! {
    static RINGS: RefCell<HashMap<usize, Ring>> = RefCell::new(HashMap::new());
}

fn key(view: &NSView) -> usize {
    view as *const NSView as usize
}

fn same_space(a: &CGColorSpace, b: &CGColorSpace) -> bool {
    // SAFETY: two live CF objects.
    unsafe {
        CFEqual(
            a as *const CGColorSpace as *const c_void,
            b as *const CGColorSpace as *const c_void,
        ) != 0
    }
}

fn srgb() -> CFRetained<CGColorSpace> {
    CGColorSpace::with_name(Some(unsafe { objc2_core_graphics::kCGColorSpaceSRGB })).expect("sRGB")
}

/// Sets the layer's contents without an implicit animation.
fn show(view: &NSView, obj: &AnyObject, scale: f32) {
    let Some(layer) = view.layer() else { return };
    CATransaction::begin();
    CATransaction::setDisableActions(true);
    unsafe { layer.setContents(Some(obj)) };
    layer.setContentsScale(scale as f64);
    CATransaction::commit();
}

/// Draws `w` x `h` logical pixels at `scale` into `view`'s layer: `draw` paints on a cleared
/// surface in `space` (sRGB when None). `sw` gets the stages "buffer", "draw" and "commit".
pub fn paint(
    view: &NSView,
    w: f32,
    h: f32,
    scale: f32,
    space: Option<&CGColorSpace>,
    sw: Option<&mut Stopwatch>,
    draw: impl FnOnce(&mut dyn Canvas),
) {
    let (pw, ph) = (
        ((w * scale).ceil() as usize).max(1),
        ((h * scale).ceil() as usize).max(1),
    );
    let fallback = srgb();
    let space = space.unwrap_or(&fallback);
    let mut sw = sw;
    let lap = |sw: &mut Option<&mut Stopwatch>, n: &'static str| {
        if let Some(s) = sw.as_deref_mut() {
            s.lap(n);
        }
    };
    if !enabled() {
        bitmap(view, w, h, scale, space, &mut sw, draw);
        return;
    }
    let k = key(view);
    // A surface the window server is not reading: the ring's own, or a new one.
    let pick = RINGS.with(|r| {
        let mut r = r.borrow_mut();
        let ring = r.entry(k).or_insert_with(|| Ring {
            pw,
            ph,
            space: space.retain(),
            bufs: Vec::new(),
            shown: None,
        });
        if ring.pw != pw || ring.ph != ph || !same_space(&ring.space, space) {
            // The layer keeps its own reference to what it shows.
            ring.bufs.clear();
            ring.shown = None;
            ring.pw = pw;
            ring.ph = ph;
            ring.space = space.retain();
        }
        let free = (0..ring.bufs.len()).find(|&i| Some(i) != ring.shown && !ring.bufs[i].in_use());
        match free {
            Some(i) => Some((i, ring.bufs[i].0)),
            None if ring.bufs.len() < MOST => {
                let s = Surf::new(pw, ph, space)?;
                let p = s.0;
                ring.bufs.push(s);
                Some((ring.bufs.len() - 1, p))
            }
            None => None,
        }
    });
    let Some((i, s)) = pick else {
        bitmap(view, w, h, scale, space, &mut sw, draw);
        return;
    };
    unsafe {
        if IOSurfaceLock(s, 0, std::ptr::null_mut()) != 0 {
            bitmap(view, w, h, scale, space, &mut sw, draw);
            return;
        }
        let base = IOSurfaceGetBaseAddress(s) as *mut u8;
        let row = IOSurfaceGetBytesPerRow(s);
        // What a new bitmap starts as: transparent black.
        std::ptr::write_bytes(base, 0, row * ph);
        // SAFETY: the surface is locked and holds `row * ph` bytes until the unlock below.
        let ctx = cg::context(base, pw, ph, row, scale, space);
        lap(&mut sw, "buffer");
        if let Some(ctx) = &ctx {
            draw(&mut CgCanvas::over(ctx));
        }
        drop(ctx);
        IOSurfaceUnlock(s, 0, std::ptr::null_mut());
        lap(&mut sw, "draw");
        // An IOSurfaceRef is toll-free bridged to an IOSurface object.
        show(view, &*(s as *const AnyObject), scale);
    }
    RINGS.with(|r| {
        if let Some(ring) = r.borrow_mut().get_mut(&k) {
            ring.shown = Some(i);
        }
    });
    lap(&mut sw, "commit");
}

/// Draws only `part` (logical px, rounded out to whole device pixels) over a copy of what the
/// layer shows now: the frame a list scroll makes, when nothing outside the list moved. Every
/// pixel in `part` is drawn as a whole frame would draw it (the clip is pixel aligned), the
/// rest is the last frame's. False (nothing drawn) when there is no last frame of this size
/// and colour space to start from.
#[allow(clippy::too_many_arguments)]
pub fn paint_part(
    view: &NSView,
    w: f32,
    h: f32,
    scale: f32,
    space: Option<&CGColorSpace>,
    part: (f32, f32, f32, f32),
    sw: Option<&mut Stopwatch>,
    draw: impl FnOnce(&mut dyn Canvas),
) -> bool {
    if !enabled() {
        return false;
    }
    let (pw, ph) = (
        ((w * scale).ceil() as usize).max(1),
        ((h * scale).ceil() as usize).max(1),
    );
    let fallback = srgb();
    let space = space.unwrap_or(&fallback);
    let k = key(view);
    let pick = RINGS.with(|r| {
        let mut r = r.borrow_mut();
        let ring = r.get_mut(&k)?;
        let shown = ring.shown?;
        if ring.pw != pw || ring.ph != ph || !same_space(&ring.space, space) {
            return None;
        }
        let free = (0..ring.bufs.len()).find(|&i| i != shown && !ring.bufs[i].in_use());
        let i = match free {
            Some(i) => i,
            None if ring.bufs.len() < MOST => {
                ring.bufs.push(Surf::new(pw, ph, space)?);
                ring.bufs.len() - 1
            }
            None => return None,
        };
        Some((i, ring.bufs[i].0, ring.bufs[shown].0))
    });
    let Some((i, dst, src)) = pick else {
        return false;
    };
    const READ_ONLY: u32 = 1;
    let mut sw = sw;
    unsafe {
        if IOSurfaceLock(src, READ_ONLY, std::ptr::null_mut()) != 0 {
            return false;
        }
        if IOSurfaceLock(dst, 0, std::ptr::null_mut()) != 0 {
            IOSurfaceUnlock(src, READ_ONLY, std::ptr::null_mut());
            return false;
        }
        let row = IOSurfaceGetBytesPerRow(dst);
        let base = IOSurfaceGetBaseAddress(dst) as *mut u8;
        if IOSurfaceGetBytesPerRow(src) == row {
            std::ptr::copy_nonoverlapping(
                IOSurfaceGetBaseAddress(src) as *const u8,
                base,
                row * ph,
            );
        }
        IOSurfaceUnlock(src, READ_ONLY, std::ptr::null_mut());
        // SAFETY: the surface is locked and holds `row * ph` bytes until the unlock below.
        let ctx = cg::context(base, pw, ph, row, scale, space);
        if let Some(s) = sw.as_deref_mut() {
            s.lap("buffer");
        }
        if let Some(ctx) = &ctx {
            let s = scale;
            let (l, t) = ((part.0 * s).floor() / s, (part.1 * s).floor() / s);
            let (r, b) = ((part.2 * s).ceil() / s, (part.3 * s).ceil() / s);
            let rect = objc2_core_foundation::CGRect {
                origin: objc2_core_foundation::CGPoint {
                    x: l as f64,
                    y: t as f64,
                },
                size: objc2_core_foundation::CGSize {
                    width: (r - l) as f64,
                    height: (b - t) as f64,
                },
            };
            objc2_core_graphics::CGContext::clip_to_rect(Some(ctx), rect);
            // As a new frame starts there: transparent black.
            objc2_core_graphics::CGContext::clear_rect(Some(ctx), rect);
            draw(&mut CgCanvas::over(ctx));
        }
        drop(ctx);
        IOSurfaceUnlock(dst, 0, std::ptr::null_mut());
        if let Some(s) = sw.as_deref_mut() {
            s.lap("draw");
        }
        show(view, &*(dst as *const AnyObject), scale);
    }
    RINGS.with(|r| {
        if let Some(ring) = r.borrow_mut().get_mut(&k) {
            ring.shown = Some(i);
        }
    });
    if let Some(s) = sw {
        s.lap("commit");
    }
    true
}

/// The old way, one bitmap and a CGImage a frame.
fn bitmap(
    view: &NSView,
    w: f32,
    h: f32,
    scale: f32,
    space: &CGColorSpace,
    sw: &mut Option<&mut Stopwatch>,
    draw: impl FnOnce(&mut dyn Canvas),
) {
    let b = Bitmap::with_space(w, h, scale, space);
    if let Some(s) = sw.as_deref_mut() {
        s.lap("buffer");
    }
    draw(&mut b.canvas());
    if let Some(s) = sw.as_deref_mut() {
        s.lap("draw");
    }
    if let Some(img) = b.image() {
        // A CGImage is a CFType, toll-free bridged to an object.
        let obj: &AnyObject =
            unsafe { &*(&*img as *const objc2_core_graphics::CGImage as *const AnyObject) };
        show(view, obj, scale);
    }
    drop(b);
    if let Some(s) = sw.as_deref_mut() {
        s.lap("commit");
    }
}

/// The view's window closed: its surfaces go (the layer's contents are cleared too).
pub fn release(view: &NSView) {
    if let Some(layer) = view.layer() {
        unsafe { layer.setContents(None) };
    }
    RINGS.with(|r| r.borrow_mut().remove(&key(view)));
}

/// Bytes the rings hold (for `disk-resources`).
pub fn held_bytes() -> usize {
    RINGS.with(|r| {
        r.borrow()
            .values()
            .map(|g| g.bufs.len() * g.pw * g.ph * 4)
            .sum()
    })
}
