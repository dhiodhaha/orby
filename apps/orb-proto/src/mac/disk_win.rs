//! The Disk window on macOS: an ordinary titled, resizable NSWindow (not a floating panel), so
//! choosing what to move to the Trash has room and the keyboard. Like Windows it draws its own
//! title band ("Disk", maximize, close) through the shared `disk_panel::draw`; the system's
//! traffic lights are hidden, its edges resize, the band's empty part drags it and a double
//! click on it zooms. Esc (after the confirm sheet), Cmd+W and the close button close it;
//! Cmd+Q quits Orby. disk.rs owns the state; this file is the window and its input.

use std::cell::Cell;
use std::time::Instant;

use objc2::rc::Retained;
use objc2::runtime::Bool;
use objc2::runtime::{AnyObject, NSObjectProtocol, ProtocolObject};
use objc2::{define_class, msg_send, AllocAnyThread, MainThreadMarker, MainThreadOnly};
use objc2_app_kit::{
    NSApplication, NSBackingStoreType, NSColor, NSEvent, NSEventModifierFlags, NSTrackingArea,
    NSTrackingAreaOptions, NSView, NSWindow, NSWindowButton, NSWindowStyleMask,
    NSWindowTitleVisibility,
};
use objc2_foundation::{
    NSNotification, NSNotificationCenter, NSOperationQueue, NSPoint, NSRect, NSSize, NSString,
};
use objc2_quartz_core::CATransaction;

use super::app::with_app;
use super::cg::Bitmap;
use crate::disk;
use crate::disk_panel::{self as dp, Hit};
use crate::App;

/// The least height the window opens at on a short screen, and can be dragged to.
const MIN_ROOM: f32 = 480.0;
/// Trackpad pixels per row the list scrolls.
const ROW_PX: f32 = 36.0;

pub struct Win {
    pub open: bool,
    /// The window's logical width: `dp::W` when it opens, then whatever it is resized to.
    pub w: f32,
    /// The height there is room for: from the screen when it opens, then the window's own.
    pub room: f32,
    /// Zoomed to the screen: square corners.
    pub max: bool,
    pub hover: Option<Hit>,
    /// The pointer in the window (logical px, y down).
    pub mouse: Option<(f32, f32)>,
    /// Something moves: the run loop draws a frame per display refresh.
    pub ticking: bool,
    /// A redraw is due at the next turn of the run loop (input coalesces into one frame).
    pub dirty: bool,
    window: Option<Retained<NSWindow>>,
    view: Option<Retained<DiskView>>,
    observers: Vec<Retained<ProtocolObject<dyn NSObjectProtocol>>>,
    /// Trackpad scroll not yet a whole row.
    wheel: f32,
    /// Our own setFrame is under way: its resize notification is not a user's resize.
    sizing: bool,
    pub frames: u64,
}

impl Default for Win {
    fn default() -> Self {
        Win {
            open: false,
            w: dp::W,
            room: dp::H,
            max: false,
            hover: None,
            mouse: None,
            ticking: false,
            dirty: false,
            window: None,
            view: None,
            observers: Vec::new(),
            wheel: 0.0,
            sizing: false,
            frames: 0,
        }
    }
}

pub struct DiskViewIvars {
    _n: Cell<u8>,
}

define_class!(
    #[unsafe(super(NSView))]
    #[thread_kind = MainThreadOnly]
    #[name = "OrbyDiskView"]
    #[ivars = DiskViewIvars]
    struct DiskView;

    impl DiskView {
        #[unsafe(method(isFlipped))]
        fn is_flipped(&self) -> bool {
            true
        }

        #[unsafe(method(acceptsFirstResponder))]
        fn accepts_first_responder(&self) -> bool {
            true
        }

        #[unsafe(method(acceptsFirstMouse:))]
        fn accepts_first_mouse(&self, _e: Option<&NSEvent>) -> bool {
            true
        }

        #[unsafe(method(mouseDown:))]
        fn mouse_down(&self, e: &NSEvent) {
            let (x, y) = self.point(e);
            // The band's empty part drags the window (a double click zooms it), as on Windows.
            let band = y < dp::TOP;
            let mut empty = false;
            with_app(|a| empty = band && disk::hit(a, x, y).is_none());
            if empty {
                if e.clickCount() == 2 {
                    with_app(toggle_zoom);
                } else if let Some(w) = self.window() {
                    w.performWindowDragWithEvent(e);
                }
            }
        }

        #[unsafe(method(mouseUp:))]
        fn mouse_up(&self, e: &NSEvent) {
            let (x, y) = self.point(e);
            with_app(|a| {
                if let Some(h) = disk::hit(a, x, y) {
                    disk::click(a, h);
                }
            });
        }

        #[unsafe(method(mouseMoved:))]
        fn mouse_moved(&self, e: &NSEvent) {
            let p = self.point(e);
            with_app(|a| on_move(a, p));
        }

        #[unsafe(method(mouseDragged:))]
        fn mouse_dragged(&self, e: &NSEvent) {
            let p = self.point(e);
            with_app(|a| on_move(a, p));
        }

        #[unsafe(method(mouseExited:))]
        fn mouse_exited(&self, _e: &NSEvent) {
            with_app(on_leave);
        }

        #[unsafe(method(scrollWheel:))]
        fn scroll_wheel(&self, e: &NSEvent) {
            let (x, y) = self.point(e);
            let precise = e.hasPreciseScrollingDeltas();
            let dy = e.scrollingDeltaY() as f32;
            with_app(|a| on_scroll(a, x, y, dy, precise));
        }

        #[unsafe(method(keyDown:))]
        fn key_down(&self, e: &NSEvent) {
            let code = e.keyCode();
            let cmd = e.modifierFlags().contains(NSEventModifierFlags::Command);
            with_app(|a| on_key(a, code, cmd));
        }

        #[unsafe(method(performKeyEquivalent:))]
        fn perform_key_equivalent(&self, e: &NSEvent) -> Bool {
            let cmd = e.modifierFlags().contains(NSEventModifierFlags::Command);
            if !cmd {
                return Bool::NO;
            }
            let chars = e
                .charactersIgnoringModifiers()
                .map(|s| s.to_string().to_lowercase())
                .unwrap_or_default();
            match chars.as_str() {
                "w" => {
                    with_app(close);
                    Bool::YES
                }
                "q" => {
                    with_app(|a| a.log("quit (Cmd+Q in the Disk window)"));
                    std::process::exit(0);
                }
                _ => Bool::NO,
            }
        }
    }
);

impl DiskView {
    fn new(mtm: MainThreadMarker, w: f32, h: f32) -> Retained<DiskView> {
        let this = Self::alloc(mtm).set_ivars(DiskViewIvars { _n: Cell::new(0) });
        let frame = NSRect::new(NSPoint::new(0.0, 0.0), NSSize::new(w as f64, h as f64));
        let view: Retained<DiskView> = unsafe { msg_send![super(this), initWithFrame: frame] };
        let opts = NSTrackingAreaOptions::MouseMoved
            | NSTrackingAreaOptions::MouseEnteredAndExited
            | NSTrackingAreaOptions::ActiveAlways
            | NSTrackingAreaOptions::InVisibleRect;
        let area = unsafe {
            NSTrackingArea::initWithRect_options_owner_userInfo(
                NSTrackingArea::alloc(),
                frame,
                opts,
                Some(&view),
                None,
            )
        };
        view.addTrackingArea(&area);
        view.setWantsLayer(true);
        view
    }

    /// The event's point in the view: logical px, y down.
    fn point(&self, e: &NSEvent) -> (f32, f32) {
        let p = self.convertPoint_fromView(e.locationInWindow(), None);
        (p.x as f32, p.y as f32)
    }
}

fn make(app: &mut App, mtm: MainThreadMarker) {
    let rect = NSRect::new(
        NSPoint::new(0.0, 0.0),
        NSSize::new(dp::W as f64, dp::H as f64),
    );
    let style = NSWindowStyleMask::Titled
        | NSWindowStyleMask::Closable
        | NSWindowStyleMask::Miniaturizable
        | NSWindowStyleMask::Resizable
        | NSWindowStyleMask::FullSizeContentView;
    let w: Retained<NSWindow> = unsafe {
        msg_send![NSWindow::alloc(mtm), initWithContentRect: rect, styleMask: style, backing: NSBackingStoreType::Buffered, defer: false]
    };
    w.setTitle(&NSString::from_str("Disk"));
    w.setTitleVisibility(NSWindowTitleVisibility::Hidden);
    w.setTitlebarAppearsTransparent(true);
    // Our own band carries close and maximize, as on Windows.
    for b in [
        NSWindowButton::CloseButton,
        NSWindowButton::MiniaturizeButton,
        NSWindowButton::ZoomButton,
    ] {
        if let Some(btn) = w.standardWindowButton(b) {
            btn.setHidden(true);
        }
    }
    w.setOpaque(false);
    w.setBackgroundColor(Some(&NSColor::clearColor()));
    w.setHasShadow(true);
    unsafe { w.setReleasedWhenClosed(false) };
    w.setContentMinSize(NSSize::new(dp::W as f64, MIN_ROOM as f64));
    w.setAcceptsMouseMovedEvents(true);
    let view = DiskView::new(mtm, dp::W, dp::H);
    w.setContentView(Some(&view));
    // A resize from outside (an edge, zoom): the lists take the new size.
    let center = NSNotificationCenter::defaultCenter();
    let resized = block2::RcBlock::new(|_n: std::ptr::NonNull<NSNotification>| {
        with_app(on_resized);
    });
    let obs = unsafe {
        center.addObserverForName_object_queue_usingBlock(
            Some(&NSString::from_str("NSWindowDidResizeNotification")),
            Some(&w),
            Some(&NSOperationQueue::mainQueue()),
            &resized,
        )
    };
    app.disk.win.observers.push(obs);
    app.disk.win.window = Some(w);
    app.disk.win.view = Some(view);
}

/// The room on the orb's screen: `H` when it fits, less on a short one.
fn screen_room(app: &App) -> f32 {
    let (vf, _) = app.screen();
    ((vf.size.height as f32) - 32.0).clamp(MIN_ROOM, dp::H)
}

/// Shows the window centred on the orb's screen, or brings it to the front if it is open.
pub fn show(app: &mut App) {
    let Some(mtm) = MainThreadMarker::new() else {
        return;
    };
    app.close_dial();
    app.close_panel();
    if app.disk.win.window.is_none() {
        make(app, mtm);
    }
    app.theme = crate::settings::theme_now(&app.settings.prefs);
    if !app.disk.win.open {
        app.disk.win.open = true;
        app.disk.win.hover = None;
        app.disk.win.mouse = None;
        app.disk.win.w = dp::W;
        app.disk.win.room = screen_room(app);
        app.disk.win.max = false;
        disk::prepare(app);
        let (lw, lh) = disk::size(app);
        let lh = lh.max(app.disk.win.room);
        let (vf, _) = app.screen();
        let x = vf.origin.x + (vf.size.width - lw as f64) / 2.0;
        let y = vf.origin.y + (vf.size.height - lh as f64) / 2.0;
        if let Some(w) = &app.disk.win.window {
            app.disk.win.sizing = true;
            w.setFrame_display(
                NSRect::new(NSPoint::new(x, y), NSSize::new(lw as f64, lh as f64)),
                false,
            );
            app.disk.win.sizing = false;
        }
        let room = app.disk.win.room;
        app.log(&format!("disk: window open, room {room:.0}"));
    }
    render(app);
    if let Some(w) = &app.disk.win.window {
        NSApplication::sharedApplication(mtm).activate();
        w.makeKeyAndOrderFront(None);
        if let Some(v) = &app.disk.win.view {
            w.makeFirstResponder(Some(v));
        }
    }
}

pub fn close(app: &mut App) {
    if !app.disk.win.open {
        return;
    }
    app.disk.win.open = false;
    app.disk.win.hover = None;
    app.disk.win.mouse = None;
    app.disk.win.ticking = false;
    app.disk.win.dirty = false;
    if let Some(w) = &app.disk.win.window {
        w.orderOut(None);
    }
    // The bitmap is about 4 MB at 2x; let it go while hidden.
    if let Some(layer) = app.disk.win.view.as_ref().and_then(|v| v.layer()) {
        unsafe { layer.setContents(None) };
    }
    disk::on_closed(app);
    app.log("disk: window closed");
}

pub fn toggle_zoom(app: &mut App) {
    if let Some(w) = &app.disk.win.window {
        w.zoom(None);
    }
}

fn on_resized(app: &mut App) {
    if !app.disk.win.open || app.disk.win.sizing {
        return;
    }
    let Some(v) = &app.disk.win.view else { return };
    let b = v.bounds();
    app.disk.win.w = (b.size.width as f32).max(dp::W);
    app.disk.win.room = b.size.height as f32;
    app.disk.win.max = app.disk.win.window.as_ref().is_some_and(|w| w.isZoomed());
    refresh(app);
}

fn follow_hover(app: &mut App) {
    let h = app.disk.win.mouse.and_then(|(x, y)| disk::hit(app, x, y));
    if h != app.disk.win.hover {
        app.disk.win.hover = h;
        disk::on_hover(app, h);
    }
}

/// The data moved under the pointer (a scroll): the frame comes with the next turn of the run
/// loop, so a burst of scroll events costs one frame.
pub fn refresh_soon(app: &mut App) {
    if !app.disk.win.open {
        return;
    }
    disk::prepare(app);
    follow_hover(app);
    app.disk.win.dirty = true;
}

/// Redraw after the data behind the window changed.
pub fn refresh(app: &mut App) {
    if !app.disk.win.open {
        return;
    }
    disk::prepare(app);
    follow_hover(app);
    render(app);
}

/// Draws the window now: its size follows what the screen needs, unless zoomed.
pub fn render(app: &mut App) {
    if !app.disk.win.open {
        return;
    }
    let t0 = Instant::now();
    disk::prepare(app);
    let v = disk::view(app);
    let (lw, lh) = dp::size(&app.fonts, &v);
    let (Some(w), Some(view)) = (app.disk.win.window.clone(), app.disk.win.view.clone()) else {
        return;
    };
    let b = view.bounds();
    let (cw, ch) = (b.size.width as f32, b.size.height as f32);
    if !app.disk.win.max && !w.inLiveResize() && ((lh - ch).abs() > 1.0 || (lw - cw).abs() > 1.0) {
        // Taller for a confirm sheet or more cards: keep the top where it is.
        let f = w.frame();
        let top = f.origin.y + f.size.height;
        let (nw, nh) = (lw.max(cw) as f64, lh as f64);
        app.disk.win.sizing = true;
        w.setFrame_display(
            NSRect::new(NSPoint::new(f.origin.x, top - nh), NSSize::new(nw, nh)),
            false,
        );
        app.disk.win.sizing = false;
    }
    let b = view.bounds();
    let (cw, ch) = (b.size.width as f32, b.size.height as f32);
    let scale = w.backingScaleFactor() as f32;
    let space = w.colorSpace().and_then(|s| s.CGColorSpace());
    let bm = match &space {
        Some(sp) => Bitmap::with_space(cw, ch, scale, sp),
        None => Bitmap::new(cw, ch, scale),
    };
    let mut v = v;
    v.w = cw.max(dp::W);
    v.room = ch;
    dp::draw(
        &mut bm.canvas(),
        &app.theme,
        &app.fonts,
        &v,
        app.disk.win.hover,
    );
    if let (Some(img), Some(layer)) = (bm.image(), view.layer()) {
        CATransaction::begin();
        CATransaction::setDisableActions(true);
        // A CGImage is a CFType, toll-free bridged to an object.
        let obj: &AnyObject =
            unsafe { &*(&*img as *const objc2_core_graphics::CGImage as *const AnyObject) };
        unsafe { layer.setContents(Some(obj)) };
        layer.setContentsScale(scale as f64);
        CATransaction::commit();
    }
    app.disk.win.frames += 1;
    app.disk.win.dirty = false;
    app.disk.win.ticking = disk::animating(app);
    if std::env::var_os("ORB_DISK_FRAMES").is_some_and(|v| v != "0") {
        let n = app.disk.win.frames;
        app.log(&format!(
            "disk: frame {n} {:.2} ms",
            t0.elapsed().as_secs_f64() * 1000.0
        ));
    }
}

fn on_move(app: &mut App, p: (f32, f32)) {
    if !app.disk.win.open {
        return;
    }
    app.disk.win.mouse = Some(p);
    let h = disk::hit(app, p.0, p.1);
    if h != app.disk.win.hover {
        app.disk.win.hover = h;
        disk::on_hover(app, h);
        app.disk.win.dirty = true;
    } else if matches!(h, Some(Hit::Tile(_))) {
        // The label card follows the pointer across a tile.
        app.disk.win.dirty = true;
    }
}

fn on_leave(app: &mut App) {
    app.disk.win.mouse = None;
    if app.disk.win.hover.is_some() {
        app.disk.win.hover = None;
        disk::on_hover(app, None);
        app.disk.win.dirty = true;
    }
}

/// Trackpad deltas come in pixels and add up to whole rows; a mouse wheel's notch is a row.
fn on_scroll(app: &mut App, x: f32, y: f32, dy: f32, precise: bool) {
    let rows = if precise {
        app.disk.win.wheel += dy;
        let n = (app.disk.win.wheel / ROW_PX).trunc();
        app.disk.win.wheel -= n * ROW_PX;
        n as i32
    } else if dy.abs() > 0.01 {
        dy.signum() as i32 * (dy.abs().ceil() as i32).min(3)
    } else {
        0
    };
    // Content moving down (scrolling up) shows earlier rows: Windows' wheel up, +120.
    for _ in 0..rows.unsigned_abs() {
        disk::on_wheel(app, x, y, if rows > 0 { 120 } else { -120 });
    }
}

/// macOS key codes to the Windows virtual keys disk.rs reads.
fn on_key(app: &mut App, code: u16, cmd: bool) {
    const ESC: u16 = 53;
    let vk = match code {
        126 => disk::VK_UP,
        125 => disk::VK_DOWN,
        123 => disk::VK_LEFT,
        124 => disk::VK_RIGHT,
        36 | 76 => disk::VK_RETURN,
        49 => disk::VK_SPACE,
        48 => disk::VK_TAB,
        // Cmd+Delete is the Finder's Move to Trash; Delete alone goes up a level.
        51 if cmd => disk::VK_DELETE,
        51 => disk::VK_BACK,
        117 => disk::VK_DELETE,
        ESC => {
            disk::on_esc(app);
            return;
        }
        _ => return,
    };
    disk::on_keydown(app, vk);
}

/// Scan a folder: the open panel, run modal (the caller does not hold the app). None when
/// cancelled.
pub fn pick_folder() -> Option<std::path::PathBuf> {
    let mtm = MainThreadMarker::new()?;
    let p = objc2_app_kit::NSOpenPanel::openPanel(mtm);
    p.setCanChooseDirectories(true);
    p.setCanChooseFiles(false);
    p.setAllowsMultipleSelection(false);
    p.setPrompt(Some(&NSString::from_str("Scan")));
    p.setMessage(Some(&NSString::from_str("Pick a folder to scan")));
    NSApplication::sharedApplication(mtm).activate();
    if p.runModal() != objc2_app_kit::NSModalResponseOK {
        return None;
    }
    p.URL()
        .and_then(|u| u.path())
        .map(|s| std::path::PathBuf::from(s.to_string()))
}

/// Test: the window's current view drawn to a PNG at 2x (what the window shows; no screen
/// recording needed).
pub fn shot(app: &mut App, path: &str) {
    disk::prepare(app);
    let mut v = disk::view(app);
    v.w = app.disk.win.w;
    v.room = app.disk.win.room;
    let (w, h) = dp::size(&app.fonts, &v);
    let h = h.max(v.room);
    let b = Bitmap::new(w, h, 2.0);
    dp::draw(
        &mut b.canvas(),
        &app.theme,
        &app.fonts,
        &v,
        app.disk.win.hover,
    );
    let p = std::path::Path::new(path);
    if let (Some(dir), Some(name)) = (p.parent(), p.file_stem()) {
        crate::snapshot::write(
            dir,
            &name.to_string_lossy(),
            b.pw as u32,
            b.ph as u32,
            b.rgba(),
        );
        app.log(&format!("disk: shot {path}"));
    }
}

/// Test: `n` frames of scrolling the list down and back up, timed (view, layout and drawing,
/// as the window does per frame).
pub fn scroll_bench(app: &mut App, n: usize) {
    let t0 = Instant::now();
    let cpu0 = cpu_secs();
    let (x, y) = (dp::W - 200.0, 400.0);
    for i in 0..n {
        let delta = if (i / 40) % 2 == 0 { -120 } else { 120 };
        disk::on_wheel(app, x, y, delta);
        render(app);
    }
    let wall = t0.elapsed().as_secs_f64() * 1000.0;
    let cpu = (cpu_secs() - cpu0) * 1000.0;
    let rss = super::disk::resident_bytes() as f64 / 1_048_576.0;
    app.log(&format!(
        "disk: scroll bench {n} frames, {:.2} ms a frame wall, {:.2} ms CPU, resident {rss:.1} MB",
        wall / n.max(1) as f64,
        cpu / n.max(1) as f64
    ));
}

fn cpu_secs() -> f64 {
    // SAFETY: getrusage fills a plain struct.
    let mut ru: libc::rusage = unsafe { std::mem::zeroed() };
    unsafe { libc::getrusage(libc::RUSAGE_SELF, &mut ru) };
    let t = |v: libc::timeval| v.tv_sec as f64 + v.tv_usec as f64 / 1e6;
    t(ru.ru_utime) + t(ru.ru_stime)
}
