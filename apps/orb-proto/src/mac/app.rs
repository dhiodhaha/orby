//! The macOS app: one state on the main thread, four borderless non-activating panels (the
//! orb, the dial, the pill, the panel window) and a menu bar item.
//!
//! The shared units run on it as on Windows: notices.rs (the queue every pill goes through),
//! habits.rs (focus and drink), history.rs, dial_menu.rs (what the dial shows and does), over
//! `crate::sys` (mac/sys.rs), the shims in mac/shim.rs and the panel host mac/panels.rs. Their
//! timers (`sys::Timer`) are deadlines in `timers`, fired by `tick` and waited for by `schedule`.
//!
//! Like Windows it renders only while something moves. Each wake is one NSTimer: 60 Hz while a
//! spring or a rolling number moves, otherwise a one-shot timer at the next deadline (the tuck,
//! a pill's time, the next prayer reminder, the panel's next second). Between them the process
//! sleeps in the run loop. The pointer is followed with global mouse monitors, which need no
//! Accessibility permission (only key monitors do); the shortcut is a Carbon hot key.

use std::cell::{Cell, RefCell};
use std::collections::HashMap;
use std::io::Write as _;
use std::path::PathBuf;
use std::ptr::NonNull;
use std::time::{Duration, Instant};

use app_core::prayer::{self as pr, Prayer, Times};
use block2::RcBlock;
use objc2::rc::Retained;
use objc2::runtime::{AnyObject, NSObject};
use objc2::{
    define_class, msg_send, sel, AllocAnyThread, DefinedClass, MainThreadMarker, MainThreadOnly,
};
use objc2_app_kit::{
    NSApplication, NSApplicationActivationPolicy, NSBackingStoreType, NSColor, NSEvent,
    NSEventMask, NSImage, NSMenu, NSMenuItem, NSPanel, NSScreen, NSStatusBar, NSStatusItem,
    NSTrackingArea, NSTrackingAreaOptions, NSView, NSWindowCollectionBehavior, NSWindowStyleMask,
    NSWorkspace,
};
use objc2_core_foundation::{CGPoint, CGRect, CGSize};
use objc2_foundation::{
    NSDistributedNotificationCenter, NSNotification, NSOperationQueue, NSPoint, NSRect, NSSize,
    NSString, NSTimer, NSUserDefaults,
};
use objc2_quartz_core::{CAMetalLayer, CATransaction};

use super::cg::Bitmap;
use super::clock;
use super::metal::{self, Gpu, ORB, SS, WIN};
use crate::canvas::Canvas;
use crate::dial_menu::{self as menu, has_branch, MORE, PRAYER};
use crate::notices::{self, Source};
use crate::notices_model::STICKY_MS;
use crate::panel;
use crate::settings_model::Prefs;
use crate::sys::Timer;
use crate::ui::{self, Fonts, Icon, Pill, PillLayout, PillPart, Theme};
use crate::{habits, history, panels};

const INSET: f32 = (WIN - ORB) / 2.0;
const EDGE_MARGIN: f32 = 8.0;
const NEAR: f32 = 20.0;
const TUCK_AFTER: Duration = Duration::from_secs(3);
const FRAME: f64 = 1.0 / 60.0;
/// Window levels: the dial and panel float like the status menus, the drop above them, the pill
/// (which grows out of the drop) above that.
const LEVEL_PANEL: isize = 25; // NSStatusWindowLevel
const LEVEL_ORB: isize = 26;
const LEVEL_PILL: isize = 27;

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
enum Kind {
    Orb,
    Dial,
    Pill,
    Panel,
}

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
enum Side {
    Left,
    Right,
}

#[derive(Clone, Copy)]
struct Spring {
    pos: f32,
    vel: f32,
    target: f32,
    k: f32,
    c: f32,
}

impl Spring {
    fn new(v: f32, k: f32, c: f32) -> Spring {
        Spring {
            pos: v,
            vel: 0.0,
            target: v,
            k,
            c,
        }
    }
    fn step(&mut self, dt: f32) {
        let h = dt / 4.0;
        for _ in 0..4 {
            let a = -self.k * (self.pos - self.target) - self.c * self.vel;
            self.vel += a * h;
            self.pos += self.vel * h;
        }
    }
    fn settled(&self, eps: f32) -> bool {
        (self.pos - self.target).abs() < eps && self.vel.abs() < eps * 10.0
    }
    fn snap(&mut self, v: f32) {
        self.pos = v;
        self.target = v;
        self.vel = 0.0;
    }
}

// ---- views and panels ----

pub struct ViewIvars {
    kind: Cell<u8>,
}

define_class!(
    #[unsafe(super(NSView))]
    #[thread_kind = MainThreadOnly]
    #[name = "OrbyView"]
    #[ivars = ViewIvars]
    struct OrbyView;

    impl OrbyView {
        #[unsafe(method(isFlipped))]
        fn is_flipped(&self) -> bool {
            true
        }

        #[unsafe(method(acceptsFirstMouse:))]
        fn accepts_first_mouse(&self, _e: Option<&NSEvent>) -> bool {
            true
        }

        #[unsafe(method(acceptsFirstResponder))]
        fn accepts_first_responder(&self) -> bool {
            true
        }

        #[unsafe(method(mouseDown:))]
        fn mouse_down(&self, e: &NSEvent) {
            self.send(Ev::Down, e);
        }

        #[unsafe(method(mouseDragged:))]
        fn mouse_dragged(&self, e: &NSEvent) {
            self.send(Ev::Drag, e);
        }

        #[unsafe(method(mouseUp:))]
        fn mouse_up(&self, e: &NSEvent) {
            self.send(Ev::Up, e);
        }

        #[unsafe(method(rightMouseUp:))]
        fn right_mouse_up(&self, e: &NSEvent) {
            self.send(Ev::RightUp, e);
        }

        #[unsafe(method(scrollWheel:))]
        fn scroll_wheel(&self, e: &NSEvent) {
            if self.kind() == Kind::Panel {
                let dy = e.scrollingDeltaY();
                with_app(|a| crate::mac::panels::on_wheel(a, dy));
            }
        }

        #[unsafe(method(mouseMoved:))]
        fn mouse_moved(&self, e: &NSEvent) {
            self.send(Ev::Move, e);
        }

        #[unsafe(method(mouseEntered:))]
        fn mouse_entered(&self, e: &NSEvent) {
            self.send(Ev::Move, e);
        }

        #[unsafe(method(mouseExited:))]
        fn mouse_exited(&self, e: &NSEvent) {
            self.send(Ev::Exit, e);
        }

        #[unsafe(method(keyDown:))]
        fn key_down(&self, e: &NSEvent) {
            let chars = e
                .charactersIgnoringModifiers()
                .map(|s| s.to_string())
                .unwrap_or_default();
            let code = e.keyCode();
            with_app(|a| a.on_key(code, &chars));
        }
    }
);

impl OrbyView {
    fn new(mtm: MainThreadMarker, kind: Kind, w: f32, h: f32) -> Retained<OrbyView> {
        let this = Self::alloc(mtm).set_ivars(ViewIvars {
            kind: Cell::new(kind as u8),
        });
        let frame = NSRect::new(NSPoint::new(0.0, 0.0), NSSize::new(w as f64, h as f64));
        let view: Retained<OrbyView> = unsafe { msg_send![super(this), initWithFrame: frame] };
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
        view
    }

    fn kind(&self) -> Kind {
        match self.ivars().kind.get() {
            0 => Kind::Orb,
            1 => Kind::Dial,
            2 => Kind::Pill,
            _ => Kind::Panel,
        }
    }

    fn send(&self, ev: Ev, e: &NSEvent) {
        let p = self.convertPoint_fromView(e.locationInWindow(), None);
        let kind = self.kind();
        let global = NSEvent::mouseLocation();
        with_app(|a| a.on_mouse(kind, ev, p.x as f32, p.y as f32, global));
    }
}

define_class!(
    #[unsafe(super(NSPanel))]
    #[thread_kind = MainThreadOnly]
    #[name = "OrbyPanel"]
    struct OrbyPanel;

    impl OrbyPanel {
        #[unsafe(method(canBecomeKeyWindow))]
        fn can_become_key_window(&self) -> bool {
            true
        }

        #[unsafe(method(canBecomeMainWindow))]
        fn can_become_main_window(&self) -> bool {
            false
        }
    }
);

pub(crate) struct Win {
    panel: Retained<OrbyPanel>,
    view: Retained<OrbyView>,
    pub(crate) w: f32,
    pub(crate) h: f32,
    pub(crate) shown: bool,
}

impl Win {
    fn new(mtm: MainThreadMarker, kind: Kind, w: f32, h: f32, level: isize, shadow: bool) -> Win {
        let rect = NSRect::new(NSPoint::new(0.0, 0.0), NSSize::new(w as f64, h as f64));
        let style = NSWindowStyleMask::Borderless | NSWindowStyleMask::NonactivatingPanel;
        let panel: Retained<OrbyPanel> = unsafe {
            msg_send![OrbyPanel::alloc(mtm), initWithContentRect: rect, styleMask: style, backing: NSBackingStoreType::Buffered, defer: false]
        };
        panel.setOpaque(false);
        panel.setBackgroundColor(Some(&NSColor::clearColor()));
        panel.setHasShadow(shadow);
        // Floating first: it sets the floating level, which the level below then replaces.
        panel.setFloatingPanel(true);
        panel.setLevel(level);
        panel.setCollectionBehavior(
            NSWindowCollectionBehavior::CanJoinAllSpaces
                | NSWindowCollectionBehavior::Stationary
                | NSWindowCollectionBehavior::FullScreenAuxiliary
                | NSWindowCollectionBehavior::IgnoresCycle,
        );
        panel.setHidesOnDeactivate(false);
        unsafe { panel.setReleasedWhenClosed(false) };
        panel.setIgnoresMouseEvents(false);
        panel.setAcceptsMouseMovedEvents(true);
        let view = OrbyView::new(mtm, kind, w, h);
        // The orb hosts its own Metal layer (set before wantsLayer); the others are layer-backed
        // and get a bitmap as contents.
        if kind != Kind::Orb {
            view.setWantsLayer(true);
        }
        panel.setContentView(Some(&view));
        Win {
            panel,
            view,
            w,
            h,
            shown: false,
        }
    }

    pub(crate) fn resize(&mut self, w: f32, h: f32) {
        if (w - self.w).abs() < 0.01 && (h - self.h).abs() < 0.01 {
            return;
        }
        self.w = w;
        self.h = h;
        let f = self.panel.frame();
        let top = f.origin.y + f.size.height;
        self.panel.setFrame_display(
            NSRect::new(
                NSPoint::new(f.origin.x, top - h as f64),
                NSSize::new(w as f64, h as f64),
            ),
            false,
        );
    }

    /// Places the window by its top-left corner in screen coordinates (y up).
    pub(crate) fn place(&self, x: f32, top: f32) {
        self.panel
            .setFrameOrigin(NSPoint::new(x as f64, (top - self.h) as f64));
    }

    /// Makes the window key (it takes the keys; a non-activating panel leaves the app in front
    /// as it is). Returns whether it is key now.
    pub(crate) fn make_key(&self) -> bool {
        self.panel.makeKeyAndOrderFront(None);
        self.panel.isKeyWindow()
    }

    pub(crate) fn show(&mut self) {
        if !self.shown {
            self.panel.orderFrontRegardless();
            self.shown = true;
        }
    }

    pub(crate) fn hide(&mut self) {
        if self.shown {
            self.panel.orderOut(None);
            self.shown = false;
        }
    }

    /// Draws into a subview's layer (`w` x `h`), in this window's colour space.
    pub(crate) fn paint_sub(
        &self,
        sub: &NSView,
        w: f32,
        h: f32,
        scale: f32,
        draw: impl FnOnce(&mut dyn Canvas),
    ) {
        let space = self.panel.colorSpace().and_then(|s| s.CGColorSpace());
        let b = match &space {
            Some(sp) => Bitmap::with_space(w, h, scale, sp),
            None => Bitmap::new(w, h, scale),
        };
        draw(&mut b.canvas());
        set_contents(sub, &b, scale);
    }

    /// Draws with the shared Canvas into a bitmap that becomes the view's layer contents.
    pub(crate) fn paint(&self, scale: f32, draw: impl FnOnce(&mut dyn Canvas)) {
        let space = self.panel.colorSpace().and_then(|s| s.CGColorSpace());
        let b = match &space {
            Some(sp) => Bitmap::with_space(self.w, self.h, scale, sp),
            None => Bitmap::new(self.w, self.h, scale),
        };
        draw(&mut b.canvas());
        set_contents(&self.view, &b, scale);
    }
}

/// Makes a bitmap a view's layer contents, without an implicit animation.
pub(crate) fn set_contents(view: &NSView, b: &Bitmap, scale: f32) {
    let Some(img) = b.image() else { return };
    let Some(layer) = view.layer() else {
        return;
    };
    CATransaction::begin();
    CATransaction::setDisableActions(true);
    // A CGImage is a CFType, toll-free bridged to an object.
    let obj: &AnyObject =
        unsafe { &*(&*img as *const objc2_core_graphics::CGImage as *const AnyObject) };
    unsafe { layer.setContents(Some(obj)) };
    layer.setContentsScale(scale as f64);
    CATransaction::commit();
}

// ---- the menu bar item's target ----

define_class!(
    #[unsafe(super(NSObject))]
    #[thread_kind = MainThreadOnly]
    #[name = "OrbyMenuTarget"]
    struct MenuTarget;

    impl MenuTarget {
        #[unsafe(method(toggleOrb:))]
        fn toggle_orb(&self, _sender: Option<&AnyObject>) {
            with_app(|a| {
                let v = !a.visible;
                a.set_visible(v);
                notices::on_user_visibility(a);
            });
        }

        #[unsafe(method(openPrayer:))]
        fn open_prayer(&self, _sender: Option<&AnyObject>) {
            with_app(|a| {
                a.set_visible(true);
                notices::on_user_visibility(a);
                crate::prayer::open_panel(a);
            });
        }

        #[unsafe(method(openDisk:))]
        fn open_disk(&self, _sender: Option<&AnyObject>) {
            with_app(crate::disk::open);
        }

        #[unsafe(method(toggleLogin:))]
        fn toggle_login(&self, _sender: Option<&AnyObject>) {
            with_app(|a| a.toggle_login());
        }

        #[unsafe(method(openSettings:))]
        fn open_settings(&self, _sender: Option<&AnyObject>) {
            with_app(|a| crate::settings::open(a, crate::settings_model::Section::General));
        }

        /// A Disk worker has news (performSelectorOnMainThread from its thread).
        #[unsafe(method(diskWake:))]
        fn disk_wake(&self, _sender: Option<&AnyObject>) {
            let mut ran = false;
            with_app(|a| {
                ran = true;
                crate::disk::on_msg(a);
            });
            if !ran {
                // The app was busy (AppKit called back from inside a handler): the news waits
                // for the next turn of the run loop instead of being lost.
                let _: () = unsafe {
                    msg_send![self, performSelector: sel!(diskWake:), withObject: None::<&AnyObject>, afterDelay: 0.02f64]
                };
            }
        }

        /// Scan a folder: the open panel runs here, with the app not borrowed.
        #[unsafe(method(pickFolder:))]
        fn pick_folder(&self, _sender: Option<&AnyObject>) {
            let folder = super::disk_win::pick_folder();
            with_app(|a| crate::disk::on_folder_picked(a, folder));
        }

        #[unsafe(method(quit:))]
        fn quit(&self, _sender: Option<&AnyObject>) {
            with_app(|a| a.log("quit from the menu bar"));
            std::process::exit(0);
        }
    }
);

impl MenuTarget {
    fn new(mtm: MainThreadMarker) -> Retained<MenuTarget> {
        unsafe { msg_send![MenuTarget::alloc(mtm), init] }
    }
}

// ---- the state ----

/// A pointer event in one of the app's windows.
#[derive(Clone, Copy, PartialEq, Debug)]
pub(crate) enum Ev {
    Down,
    Drag,
    Up,
    RightUp,
    Move,
    Exit,
}

struct Drag {
    from: NSPoint,
    origin: (f32, f32),
    moved: bool,
}

pub(crate) struct App {
    gpu: Option<Gpu>,
    orb_layer: Option<Retained<CAMetalLayer>>,
    orb: Win,
    dial: Win,
    pill_win: Win,
    pub(crate) panel_win: Win,
    _status: Option<Retained<NSStatusItem>>,
    _menu_target: Retained<MenuTarget>,
    toggle_item: Option<Retained<NSMenuItem>>,
    login_item: Option<Retained<NSMenuItem>>,
    pub(crate) fonts: Fonts,
    pub(crate) theme: Theme,
    pub(crate) scale: f32,
    log_path: PathBuf,
    started: Instant,
    // The drop: its window's left edge (springs) and top, in screen coordinates (y up).
    side: Side,
    x: Spring,
    top: f32,
    opacity: Spring,
    tucked: bool,
    pub(crate) tuck_at: Option<Instant>,
    pub(crate) visible: bool,
    drag: Option<Drag>,
    near: bool,
    orb_dirty: bool,
    // The dial.
    pub(crate) dial_open: bool,
    dial_reveal: Spring,
    dial_aim: Option<usize>,
    dial_keyboard: bool,
    /// The More branch: open, its aimed child, its reveal, and when a steady aim opens it.
    dial_branch: Option<usize>,
    dial_child: Option<usize>,
    branch_reveal: Spring,
    branch_at: Option<Instant>,
    // The pill.
    pub(crate) pill: Option<Pill>,
    pill_layout: PillLayout,
    pill_reveal: Spring,
    pill_until: Option<Instant>,
    pill_hover: PillPart,
    pub(crate) pill_closing: bool,
    // The panel window (mac/panels.rs hosts it): which panel, its hover, Esc's way back.
    pub(crate) panel_open: bool,
    pub(crate) panel_kind: panels::Kind,
    pub(crate) panel_hover: Option<panels::Hit>,
    pub(crate) panel_back: Option<panels::Kind>,
    /// The Focus knob is being dragged.
    pub(crate) knob_drag: bool,
    /// The Prayer countdown's own layer: a second turning redraws only it, not the whole panel.
    pub(crate) countdown_view: Retained<NSView>,
    pub(crate) countdown_shown: String,
    pub(crate) countdown_rolling: bool,
    /// The panel needs a full redraw at the next tick (and the minute it last drew).
    pub(crate) panel_dirty: bool,
    pub(crate) panel_min: i64,
    /// A roll or ripple moved in the last frame (one more frame at rest after it).
    pub(crate) panel_moving: bool,
    // Settings (settings.rs, shared with Windows): `settings.prefs` is what settings.json keeps.
    pub(crate) settings: crate::settings::Settings,
    // Prayer.
    pub(crate) times: Times,
    pub(crate) tomorrow_subuh: u32,
    day: (i32, u32, u32),
    sun_angle: f32,
    brightness: f32,
    sun_at: Instant,
    next_reminder: Option<(Instant, Prayer, i32)>,
    // The shared units' state (the same fields as Windows' App).
    pub(crate) notices: notices::Notices,
    pub(crate) habits: habits::Habits,
    pub(crate) history: history::History,
    /// `sys::Timer` deadlines and their repeat interval (Win32 SetTimer's meaning).
    pub(crate) timers: HashMap<Timer, (Instant, Duration)>,
    /// While away from the computer: when to look for fresh input again (mac/sys.rs).
    pub(crate) away_check_at: Option<Instant>,
    // Disk: its state (shared with Windows), the 10 min low-space check, letting a scan go.
    pub(crate) disk: crate::disk::Disk,
    pub(crate) disk_check_at: Option<Instant>,
    pub(crate) disk_check_every: Duration,
    pub(crate) disk_release_at: Option<Instant>,
    // The run loop.
    timer: Option<Retained<NSTimer>>,
    last_frame: Instant,
}

thread_local! {
    static APP: RefCell<Option<App>> = const { RefCell::new(None) };
}

/// Runs `f` on the app. An event that arrives while the app is busy (AppKit calling back from
/// inside a handler) is dropped rather than re-entered.
pub(crate) fn with_app(f: impl FnOnce(&mut App)) {
    APP.with(|cell| {
        if let Ok(mut b) = cell.try_borrow_mut() {
            if let Some(a) = b.as_mut() {
                f(a);
                a.schedule();
            }
        }
    });
}

pub(crate) fn dark_mode() -> bool {
    let d = NSUserDefaults::standardUserDefaults();
    d.stringForKey(&NSString::from_str("AppleInterfaceStyle"))
        .is_some_and(|s| s.to_string().eq_ignore_ascii_case("dark"))
}

/// The screen under a point, its visible frame (without the menu bar and Dock) and its scale.
fn screen_at(mtm: MainThreadMarker, x: f64, y: f64) -> (NSRect, f32) {
    let screens = NSScreen::screens(mtm);
    let mut best: Option<(NSRect, f32, f64)> = None;
    for s in screens.iter() {
        let f = s.frame();
        let (cx, cy) = (
            x.clamp(f.origin.x, f.origin.x + f.size.width),
            y.clamp(f.origin.y, f.origin.y + f.size.height),
        );
        let d = (cx - x).powi(2) + (cy - y).powi(2);
        if best.as_ref().is_none_or(|b| d < b.2) {
            best = Some((s.visibleFrame(), s.backingScaleFactor() as f32, d));
        }
    }
    best.map(|b| (b.0, b.1)).unwrap_or((
        NSRect::new(NSPoint::new(0.0, 0.0), NSSize::new(1440.0, 900.0)),
        2.0,
    ))
}

fn reminder_events(prefs: &Prefs, t: &Times, tomorrow_subuh: u32) -> Vec<(f64, Prayer, i32)> {
    let offsets: [(usize, i32); 3] = [(0, -30), (1, -10), (2, 0)];
    let mut v = Vec::new();
    for p in Prayer::ALL {
        for (k, off) in offsets {
            if prefs.prayer_reminders[k] {
                v.push((t.of(p) as f64 + off as f64, p, off));
            }
        }
    }
    for (k, off) in offsets {
        if prefs.prayer_reminders[k] {
            v.push((
                1440.0 + tomorrow_subuh as f64 + off as f64,
                Prayer::Subuh,
                off,
            ));
        }
    }
    v.sort_by(|a, b| a.0.total_cmp(&b.0));
    v
}

/// Highlight angle from the time of day, as orb.rs: Subuh 28°, Dzuhur 90°, Ashar 128°,
/// Maghrib 160°; after Isya the metal dims.
fn sun_from_clock(t: &Times) -> (f32, f32) {
    let h = clock::now_minutes() as f32 / 60.0;
    let hr = |m: u32| m as f32 / 60.0;
    let keys = [
        (hr(t.subuh), 28.0),
        (hr(t.dzuhur), 90.0),
        (hr(t.ashar), 128.0),
        (hr(t.maghrib), 160.0),
    ];
    let mut angle = if h <= keys[0].0 { keys[0].1 } else { keys[3].1 };
    for w in keys.windows(2) {
        if h >= w[0].0 && h <= w[1].0 {
            let f = (h - w[0].0) / (w[1].0 - w[0].0);
            angle = w[0].1 + (w[1].1 - w[0].1) * f;
        }
    }
    if h < hr(t.subuh) || h >= hr(t.isya) {
        (90.0, 0.7)
    } else {
        (angle, 1.0)
    }
}

fn tomorrow((y, m, d): (i32, u32, u32)) -> (i32, u32, u32) {
    let leap = (y % 4 == 0 && y % 100 != 0) || y % 400 == 0;
    let days = [
        31,
        if leap { 29 } else { 28 },
        31,
        30,
        31,
        30,
        31,
        31,
        30,
        31,
        30,
        31,
    ][(m - 1) as usize];
    if d < days {
        (y, m, d + 1)
    } else if m < 12 {
        (y, m + 1, 1)
    } else {
        (y + 1, 1, 1)
    }
}

impl App {
    pub(crate) fn log(&self, line: &str) {
        let l = clock::local();
        if let Ok(mut f) = std::fs::OpenOptions::new()
            .create(true)
            .append(true)
            .open(&self.log_path)
        {
            let _ = writeln!(
                f,
                "{:04}-{:02}-{:02} {:02}:{:02}:{:02} {line}",
                l.y, l.m, l.d, l.hour, l.min, l.sec
            );
        }
    }

    /// The menu target, which lives as long as the app: workers wake the main thread through it.
    pub(crate) fn menu_target_ptr(&self) -> usize {
        Retained::as_ptr(&self._menu_target) as usize
    }

    /// Start at login (System Settings, General, Login Items): off until you turn it on.
    fn toggle_login(&mut self) {
        let on = super::login::enabled();
        let r = super::login::set(!on);
        let now = super::login::enabled();
        self.log(&format!(
            "start at login: {} -> {} ({})",
            on,
            now,
            r.err().unwrap_or_else(|| "ok".into())
        ));
        self.sync_login_item();
        crate::settings::refresh_startup(self);
        if super::login::needs_approval() {
            self.reply(Pill {
                icon: Icon::Info,
                dot: false,
                title: "Start at login".into(),
                value: "allow Orby in Login Items".into(),
                action: None,
                closable: true,
                ttl_ms: 6000,
            });
        }
    }

    /// The menu bar's Start at login checkmark shows the login item's state.
    pub(crate) fn sync_login_item(&self) {
        if let Some(item) = &self.login_item {
            item.setState(if super::login::enabled() { 1 } else { 0 });
        }
    }

    /// A theme change: the drop, the dial and the pill draw again (the panel and the windows
    /// redraw on their own, see `settings_sys::redraw_others`).
    pub(crate) fn redraw_for_theme(&mut self) {
        self.render_orb();
        if self.dial.shown {
            self.render_dial();
        }
        if self.pill.is_some() {
            self.render_pill();
        }
    }

    // ---- geometry ----

    pub(crate) fn screen(&self) -> (NSRect, f32) {
        let mtm = MainThreadMarker::new().expect("main thread");
        screen_at(
            mtm,
            (self.x.pos + WIN / 2.0) as f64,
            (self.top - WIN / 2.0) as f64,
        )
    }

    fn dock_x(&self, side: Side, tucked: bool) -> f32 {
        let (vf, _) = self.screen();
        let (l, r) = (vf.origin.x as f32, (vf.origin.x + vf.size.width) as f32);
        match (side, tucked) {
            (Side::Right, false) => r - (WIN - INSET) - EDGE_MARGIN,
            (Side::Right, true) => r - WIN / 2.0,
            (Side::Left, false) => l - INSET + EDGE_MARGIN,
            (Side::Left, true) => l - WIN / 2.0,
        }
    }

    fn clamp_top(&self, top: f32) -> f32 {
        let (vf, _) = self.screen();
        let lo = vf.origin.y as f32 + WIN - INSET + EDGE_MARGIN;
        let hi = (vf.origin.y + vf.size.height) as f32 + INSET - EDGE_MARGIN;
        top.clamp(lo, hi.max(lo))
    }

    /// The drop's centre in screen coordinates (y up).
    fn center(&self) -> (f32, f32) {
        (self.x.pos + WIN / 2.0, self.top - WIN / 2.0)
    }

    fn mirrored(&self) -> bool {
        self.side == Side::Left
    }

    // ---- the drop ----

    /// The drop. The focus ring (`habits::ring`) and the notices badge
    /// (`notices::badge_count`) are drawn here once the UI stage brings them to the Mac.
    pub(crate) fn render_orb(&mut self) {
        let (Some(gpu), Some(layer)) = (&self.gpu, &self.orb_layer) else {
            return;
        };
        let k = self.scale * SS;
        let p = metal::rest_params(
            k,
            self.sun_angle,
            self.brightness,
            self.opacity.pos.clamp(0.0, 1.0),
            1.0,
        );
        gpu.draw(layer, &p);
        self.orb_dirty = false;
    }

    fn place_orb(&mut self) {
        self.orb.place(self.x.pos.round(), self.top.round());
        if self.dial.shown {
            self.place_dial();
        }
        if self.pill_win.shown {
            self.place_pill();
        }
    }

    pub(crate) fn set_tucked(&mut self, tucked: bool) {
        if self.tucked == tucked {
            return;
        }
        self.tucked = tucked;
        self.x.target = self.dock_x(self.side, tucked);
        (self.x.k, self.x.c) = if tucked { (190.0, 26.0) } else { (480.0, 34.0) };
        self.opacity.target = if tucked { 0.4 } else { 1.0 };
        self.tuck_at = None;
    }

    /// Tucks 3 s from now, unless something is open or the pointer is close.
    pub(crate) fn arm_tuck(&mut self) {
        let busy = self.dial_open || self.panel_open || self.pill.is_some() || self.drag.is_some();
        self.tuck_at = if busy || self.near || self.tucked {
            None
        } else {
            Some(Instant::now() + TUCK_AFTER)
        };
    }

    pub(crate) fn set_visible(&mut self, v: bool) {
        if self.visible == v {
            return;
        }
        self.visible = v;
        if v {
            self.orb.show();
            self.render_orb();
            self.arm_tuck();
        } else {
            self.close_dial();
            self.close_panel();
            self.orb.hide();
        }
        if let Some(item) = &self.toggle_item {
            item.setTitle(&NSString::from_str(if v {
                "Hide Orby"
            } else {
                "Show Orby"
            }));
        }
        self.log(if v { "orb shown" } else { "orb hidden" });
    }

    /// The pointer moved anywhere (`p` in screen coordinates, y up).
    fn on_pointer(&mut self, p: NSPoint) {
        let (cx, cy) = self.center();
        let (dx, dy) = (p.x as f32 - cx, p.y as f32 - cy);
        let half = ORB / 2.0 + NEAR;
        let near = self.visible && dx.abs() <= half && dy.abs() <= half;
        if near != self.near {
            self.near = near;
            if near {
                self.set_tucked(false);
                self.tuck_at = None;
            } else {
                self.arm_tuck();
            }
            // While a scan runs, coming close names it with a counting pill; while focus
            // runs, "Focus 18 min left".
            if near {
                crate::disk::on_orb_near(self, true);
                notices::on_orb_near(self, true);
            } else {
                crate::disk::on_orb_near(self, false);
                notices::on_orb_near(self, false);
                crate::disk::on_orb_gone(self);
                notices::on_orb_gone(self);
            }
        }
        // Away from the computer: the pointer moving means you are back.
        if self.habits.away {
            habits::on_input(self);
        }
        if self.dial_open {
            self.aim(dx, -dy);
        }
    }

    // ---- the dial ----

    fn place_dial(&mut self) {
        let (cx, cy) = self.center();
        let (dx, dy) = ui::dial_center(self.mirrored());
        self.dial.place(cx - dx, cy + dy);
    }

    /// The open branch's children and its default child (dial_menu.rs, shared with Windows).
    fn branch_children(&self) -> Vec<ui::DialChild> {
        self.dial_branch
            .map(|p| menu::children(self, p).0)
            .unwrap_or_default()
    }

    pub(crate) fn render_dial(&mut self) {
        let items = menu::items(self);
        let kids = self.branch_children();
        let branch = self.dial_branch.map(|parent| ui::DialBranch {
            parent,
            children: &kids,
            aimed: self.dial_child,
            reveal: self.branch_reveal.pos,
        });
        let ring = habits::ring(self);
        let (t, f) = (self.theme, &self.fonts);
        let (aim, rv, mir, kb) = (
            self.dial_aim,
            self.dial_reveal.pos,
            self.mirrored(),
            self.dial_keyboard,
        );
        // Value tags step aside while a branch is out.
        let tags = if self.dial_branch.is_some() { 0.0 } else { 1.0 };
        self.dial.paint(self.scale, |cv| {
            ui::draw_dial(
                cv,
                &t,
                f,
                &items,
                aim,
                rv,
                mir,
                kb,
                branch.as_ref(),
                ring,
                tags,
            );
        });
    }

    /// Fans out `parent`'s children (Focus, Drink or More); `aim_default` aims its default.
    fn open_branch(&mut self, parent: usize, aim_default: bool) {
        self.branch_at = None;
        self.dial_aim = Some(parent);
        if self.dial_branch != Some(parent) {
            self.dial_branch = Some(parent);
            self.branch_reveal = Spring::new(0.0, 190.0, 26.0);
            self.branch_reveal.target = 1.0;
        }
        self.dial_child = aim_default.then(|| menu::children(self, parent).1);
        self.render_dial();
    }

    fn close_branch(&mut self) {
        self.dial_branch = None;
        self.dial_child = None;
        self.branch_at = None;
        self.render_dial();
    }

    /// A branch child does its action at once (dial_menu.rs): start a session, log a glass, or
    /// open More's child.
    fn pick_child(&mut self, parent: usize, j: usize) {
        self.close_dial();
        let name = menu::branch_log_name(parent);
        self.log(&format!("dial pick {name} child {j}"));
        menu::pick_child(self, parent, j);
    }
    pub(crate) fn open_dial(&mut self, keyboard: bool) {
        if self.dial_open {
            return;
        }
        if self.pill.is_some() {
            self.dismiss_pill();
        }
        self.close_panel();
        self.set_tucked(false);
        self.tuck_at = None;
        self.theme = crate::settings::theme_now(&self.settings.prefs);
        self.dial_open = true;
        self.dial_keyboard = keyboard;
        self.dial_aim = if keyboard { Some(0) } else { None };
        self.dial_branch = None;
        self.dial_child = None;
        self.branch_at = None;
        self.dial_reveal = Spring::new(0.0, 190.0, 26.0);
        self.dial_reveal.target = 1.0;
        self.place_dial();
        self.render_dial();
        self.dial.show();
        if keyboard {
            self.dial.panel.makeKeyAndOrderFront(None);
        }
        // Keep the drop above the dial.
        self.orb.panel.orderFrontRegardless();
    }

    pub(crate) fn close_dial(&mut self) {
        if !self.dial_open {
            return;
        }
        self.dial_open = false;
        self.dial_branch = None;
        self.dial_child = None;
        self.branch_at = None;
        self.dial_reveal.k = 480.0;
        self.dial_reveal.c = 40.0;
        self.dial_reveal.target = 0.0;
        self.arm_tuck();
    }

    /// The pointer aims by direction from the drop (`dy` up); past the main band it aims the
    /// open branch's children. A steady aim on Focus, Drink or More opens its branch (120 ms).
    fn aim(&mut self, dx: f32, dy: f32) {
        let far = (dx * dx + dy * dy).sqrt() >= ui::BRANCH_AIM;
        let (aim, child) = match self.dial_branch {
            Some(parent) if far => {
                let n = menu::children(self, parent).0.len();
                (
                    Some(parent),
                    ui::aim_child(dx, dy, self.mirrored(), parent, n),
                )
            }
            _ => (ui::aim(dx, dy, self.mirrored(), menu::N), None),
        };
        if aim == self.dial_aim && child == self.dial_child && !self.dial_keyboard {
            return;
        }
        if aim != self.dial_aim {
            if self.dial_branch.is_some_and(|b| Some(b) != aim) {
                self.dial_branch = None;
                self.dial_child = None;
            }
            self.branch_at = (aim.is_some_and(has_branch) && self.dial_branch.is_none())
                .then(|| Instant::now() + Duration::from_millis(120));
        }
        self.dial_aim = aim;
        self.dial_child = child;
        self.dial_keyboard = false;
        self.render_dial();
    }

    /// A main item: More fans out its children; the others close the dial and open their panel
    /// (dial_menu.rs).
    fn pick(&mut self, i: usize) {
        if i == MORE {
            self.log("dial pick more");
            let kb = self.dial_keyboard;
            self.open_branch(MORE, kb);
            return;
        }
        self.close_dial();
        let name = menu::LOG_NAMES.get(i).copied().unwrap_or("?");
        self.log(&format!("dial pick {name}"));
        menu::pick(self, i);
    }

    fn on_shortcut(&mut self) {
        // The first run's "try it" takes the press; the orb stays put.
        if crate::settings::takes_shortcut(self) {
            crate::settings::on_shortcut_pressed(self);
            return;
        }
        if !self.visible {
            self.set_visible(true);
            notices::on_user_visibility(self);
            self.open_dial(true);
        } else if self.dial_open {
            self.close_dial();
            self.set_visible(false);
            notices::on_user_visibility(self);
        } else {
            self.open_dial(true);
        }
    }

    /// Keys, as on Windows: in the dial the arrows go round the ring (or the open branch), the
    /// arrow toward the screen centre enters a branch and the other steps back out, Return picks
    /// or opens a branch, 1 to 6 jump; Esc steps back. With a panel open, its unit has the keys.
    fn on_key(&mut self, code: u16, chars: &str) {
        const ESC: u16 = 53;
        const RETURN: u16 = 36;
        const ENTER: u16 = 76;
        const LEFT: u16 = 123;
        const RIGHT: u16 = 124;
        const UP: u16 = 126;
        const DOWN: u16 = 125;
        if !self.dial_open {
            if self.panel_open {
                if let Some(k) = crate::sys::vk_from_mac(code) {
                    panels::on_key(self, k);
                }
            }
            return;
        }
        let (inward, outward) = if self.mirrored() {
            (RIGHT, LEFT)
        } else {
            (LEFT, RIGHT)
        };
        let aim = self.dial_aim.unwrap_or(0);
        let digit = chars
            .chars()
            .next()
            .and_then(|c| c.to_digit(10))
            .filter(|n| (1..=6).contains(n))
            .map(|n| n as usize - 1);
        match (code, self.dial_branch, self.dial_child) {
            (ESC, Some(_), _) => {
                self.close_branch();
                self.dial_keyboard = true;
            }
            (ESC, None, _) => self.close_dial(),
            (UP | DOWN, Some(parent), Some(j)) => {
                let n = menu::children(self, parent).0.len().max(1);
                self.dial_keyboard = true;
                self.dial_child = Some(if code == UP {
                    (j + n - 1) % n
                } else {
                    (j + 1) % n
                });
                self.render_dial();
            }
            (UP | DOWN, _, _) => {
                self.dial_branch = None;
                self.dial_child = None;
                self.branch_at = None;
                self.dial_keyboard = true;
                let n = menu::N;
                self.dial_aim = Some(if code == UP {
                    (aim + n - 1) % n
                } else {
                    (aim + 1) % n
                });
                self.render_dial();
            }
            (k, _, None) if k == inward && has_branch(aim) => {
                self.dial_keyboard = true;
                self.open_branch(aim, true);
            }
            (k, Some(_), Some(_)) if k == outward => {
                self.dial_keyboard = true;
                self.dial_child = None;
                self.render_dial();
            }
            (RETURN | ENTER, Some(parent), Some(j)) => self.pick_child(parent, j),
            // With the branch open and the parent aimed, Return opens the parent's panel.
            (RETURN | ENTER, Some(parent), None) if parent != MORE => self.pick(parent),
            (RETURN | ENTER, Some(_), None) => {}
            (RETURN | ENTER, None, _) if has_branch(aim) => {
                self.dial_keyboard = true;
                self.open_branch(aim, true);
            }
            (RETURN | ENTER, None, _) => self.pick(aim),
            _ => match (digit, self.dial_branch) {
                (Some(j), Some(parent)) => {
                    if j < menu::children(self, parent).0.len() {
                        self.pick_child(parent, j);
                    }
                }
                (Some(i), None) if has_branch(i) => {
                    self.dial_keyboard = true;
                    self.open_branch(i, true);
                }
                (Some(i), None) => self.pick(i),
                _ => {}
            },
        }
    }
    // ---- pills and the notice queue ----

    fn place_pill(&mut self) {
        let (ox, top) = (self.x.pos, self.top);
        let drop_l = ox + INSET;
        let x = if self.side == Side::Right {
            drop_l + ORB + ui::PILL_PAD - self.pill_win.w
        } else {
            drop_l - ui::PILL_PAD
        };
        self.pill_win.place(x, top - INSET + ui::PILL_PAD);
    }

    pub(crate) fn render_pill(&mut self) {
        let Some(p) = self.pill.clone() else { return };
        let (t, f, l) = (self.theme, &self.fonts, self.pill_layout);
        let (rv, grow_left, hover) = (
            self.pill_reveal.pos,
            self.side == Side::Right,
            self.pill_hover,
        );
        self.pill_win.paint(self.scale, |cv| {
            ui::draw_pill(
                cv,
                &t,
                f,
                &p,
                &l,
                rv,
                grow_left,
                hover,
                ui::PILL_PAD,
                ui::PILL_PAD,
            );
        });
    }

    /// Shows a pill from the drop, or new words in the pill on screen (the reveal carries on).
    /// Only notices.rs (through `crate::pill`) calls this: every pill goes through the queue.
    pub(crate) fn show_pill(&mut self, p: Pill) {
        self.theme = crate::settings::theme_now(&self.settings.prefs);
        self.pill_layout = ui::layout_pill(&self.fonts, &p);
        self.pill_win.resize(
            self.pill_layout.w + 2.0 * ui::PILL_PAD,
            ui::PILL_H + 2.0 * ui::PILL_PAD,
        );
        let fresh = self.pill.is_none() || self.pill_closing;
        self.pill_until =
            (p.ttl_ms < STICKY_MS).then(|| Instant::now() + Duration::from_millis(p.ttl_ms as u64));
        self.log(&format!("pill \"{}\" {}", p.title, p.value));
        self.pill = Some(p);
        self.pill_closing = false;
        self.pill_hover = PillPart::None;
        if fresh {
            self.pill_reveal = Spring::new(0.0, 320.0, 17.0);
        }
        self.pill_reveal.target = 1.0;
        if self.visible {
            self.set_tucked(false);
            self.tuck_at = None;
        }
        self.place_pill();
        self.render_pill();
        self.pill_win.show();
    }

    /// An answer to something you just did: shows at once, never waits in the queue.
    pub(crate) fn reply(&mut self, p: Pill) {
        notices::reply(self, p);
    }

    /// The pill starts to leave; the notice queue hears of it (`notices::on_dismiss`), and once
    /// it has gone (`tick`) the next one may come (`notices::on_pill_gone`).
    pub(crate) fn dismiss_pill(&mut self) {
        if self.pill.is_none() || self.pill_closing {
            return;
        }
        self.pill_closing = true;
        self.pill_until = None;
        self.pill_reveal.k = 480.0;
        self.pill_reveal.c = 40.0;
        self.pill_reveal.target = 0.0;
        notices::on_dismiss(self);
    }

    /// A pill's action button, as Windows' pill.rs (the actions whose unit is on the Mac).
    fn pill_action(&mut self, a: ui::PillAction) {
        use ui::PillAction as A;
        match a {
            A::AddDrink(ml) => {
                let p = habits::log_drink(self, ml);
                notices::reply(self, p);
            }
            A::UndoDrink => {
                habits::undo_drink(self);
                self.dismiss_pill();
            }
            A::UndoFocus => habits::undo_focus(self),
            A::ResumeFocus => {
                habits::resume(self);
                self.dismiss_pill();
            }
            A::StartFocus => {
                let m = self.habits.prefs.length_min;
                habits::start_focus(self, m);
                self.dismiss_pill();
            }
            A::ShowWaited => {
                self.dismiss_pill();
                notices::open_panel(self);
            }
            A::FindBigFolders(d) => {
                self.dismiss_pill();
                crate::disk::find_big_folders(self, d);
            }
            A::OpenDisk => {
                self.dismiss_pill();
                crate::disk::open(self);
            }
            A::UndoRecycle => {
                self.dismiss_pill();
                crate::disk::undo(self);
            }
            A::RetryScan => {
                self.dismiss_pill();
                crate::disk::retry_scan(self);
            }
            A::OpenSettings => {
                self.dismiss_pill();
                crate::settings::open(self, crate::settings_model::Section::General);
            }
            A::UndoShortcut => crate::settings::undo_shortcut(self),
            other => {
                self.dismiss_pill();
                self.log(&format!("pill action {other:?} is not on the Mac yet"));
            }
        }
    }
    // ---- prayer ----

    pub(crate) fn refresh_day(&mut self) {
        let d = clock::today();
        if d != self.day {
            self.day = d;
            let prefs = &self.settings.prefs;
            (self.times, self.tomorrow_subuh) =
                crate::prayer::compute_times(prefs, prefs.place.coords(), d);
            self.log(&format!(
                "times {}: Subuh {} Dzuhur {} Ashar {} Maghrib {} Isya {}",
                self.settings.prefs.place.short(),
                pr::hhmm(self.times.subuh),
                pr::hhmm(self.times.dzuhur),
                pr::hhmm(self.times.ashar),
                pr::hhmm(self.times.maghrib),
                pr::hhmm(self.times.isya)
            ));
        }
        (self.sun_angle, self.brightness) = sun_from_clock(&self.times);
        self.sun_at = Instant::now() + Duration::from_secs(300);
        self.orb_dirty = true;
    }

    /// The place, method or adjustments changed: today's times again, and the drop's sun now.
    pub(crate) fn recompute_times(&mut self) {
        self.day = (0, 0, 0);
        self.refresh_day();
        self.render_orb();
    }

    pub(crate) fn schedule_reminders(&mut self) {
        let now = clock::now_minutes();
        self.next_reminder =
            reminder_events(&self.settings.prefs, &self.times, self.tomorrow_subuh)
                .into_iter()
                .find(|e| e.0 > now + 1.0 / 60.0)
                .map(|(at, p, off)| {
                    let secs = ((at - now) * 60.0).max(0.5);
                    self.log(&format!(
                        "next reminder: {} {off:+} min, in {:.1} min",
                        p.name(),
                        at - now
                    ));
                    (Instant::now() + Duration::from_secs_f64(secs), p, off)
                });
    }

    fn prayer_moment(&self, p: Prayer) -> app_habits::Moment {
        let now_min = clock::now_minutes();
        let at = self.times.of(p);
        let day = if p == Prayer::Subuh && (at as f64) + 1.0 < now_min {
            tomorrow(self.day)
        } else {
            self.day
        };
        let at = if day != self.day {
            self.tomorrow_subuh
        } else {
            at
        };
        app_habits::Date::new(day.0, day.1, day.2)
            .map(|d| d.at(at / 60, at % 60))
            .unwrap_or_else(clock::now)
    }

    /// As Windows' prayer.rs: −30 and −10 are normal notices (−10 stays, counting down), the
    /// time itself is urgent and pauses a running focus session for the prayer.
    fn fire_reminder(&mut self, p: Prayer, off: i32) {
        let at = pr::hhmm(self.times.of(p));
        if off < 0 {
            let sticky = off >= -10;
            let pill = Pill {
                icon: Icon::Bell,
                dot: false,
                title: format!("{} menit lagi {}", -off, p.name()),
                value: at,
                action: None,
                closable: true,
                ttl_ms: if sticky { STICKY_MS } else { 6000 },
            };
            if !notices::hidden_by_you(self) {
                let due = self.prayer_moment(p);
                notices::post(self, notices::prayer_before(p.name(), due, pill, sticky));
            }
        } else {
            notices::drop_prayer(self, p.name());
            let pill = Pill {
                icon: Icon::Bell,
                dot: true,
                title: format!("Memasuki waktu {}", p.name()),
                value: String::new(),
                action: None,
                closable: true,
                ttl_ms: 10_000,
            };
            notices::post(self, notices::urgent(Source::Prayer, pill));
            // Settings' At the time: Bunyi is the system's short sound (an adzan chosen on
            // Windows plays as Bunyi here); never in full screen or while presenting.
            let sound = crate::settings_model::os::shown_sound(self.settings.prefs.at_time_sound);
            if sound != crate::settings_model::AtTimeSound::Silent && !notices::silent(self) {
                crate::sys::play_notify_sound();
            }
            self.set_tucked(false);
            habits::on_prayer_time(self, p.name());
        }
        self.log(&format!("reminder {} {off:+}", p.name()));
    }

    /// The panel window beside the drop, centred on it vertically, for its current height.
    pub(crate) fn place_panel(&mut self) {
        let (cx, cy) = self.center();
        let (vf, _) = self.screen();
        let gap = 12.0;
        let (w, h) = (self.panel_win.w, self.panel_win.h);
        let x = if self.side == Side::Right {
            cx - ORB / 2.0 - gap - w
        } else {
            cx + ORB / 2.0 + gap
        };
        let lo = vf.origin.y as f32 + h + 8.0;
        let hi = (vf.origin.y + vf.size.height) as f32 - 8.0;
        let top = (cy + h / 2.0).clamp(lo, hi.max(lo));
        self.panel_win.place(x, top);
    }

    pub(crate) fn close_panel(&mut self) {
        panels::close_panel(self);
    }
    // ---- input ----

    fn on_mouse(&mut self, kind: Kind, ev: Ev, x: f32, y: f32, global: NSPoint) {
        match kind {
            Kind::Orb => self.on_orb_mouse(ev, global),
            Kind::Dial => match ev {
                Ev::Move | Ev::Drag => self.on_pointer(global),
                Ev::Up => match (self.dial_branch, self.dial_child, self.dial_aim) {
                    (Some(parent), Some(j), _) => self.pick_child(parent, j),
                    (_, _, Some(i)) => self.pick(i),
                    _ => self.close_dial(),
                },
                _ => {}
            },
            Kind::Pill => {
                let menu = false;
                let part = ui::hit_pill(&self.pill_layout, menu, x, y);
                match ev {
                    Ev::Move => {
                        if part != self.pill_hover {
                            self.pill_hover = part;
                            self.render_pill();
                        }
                    }
                    Ev::Exit => {
                        self.pill_hover = PillPart::None;
                        self.render_pill();
                    }
                    // A closing pill still shows for a moment; its action was already taken.
                    Ev::Up if self.pill_closing => {}
                    Ev::Up => match part {
                        PillPart::Close => {
                            // Your X: a pill that stays until closed is done with.
                            notices::closed_by_you(self);
                            self.dismiss_pill();
                        }
                        PillPart::Action => {
                            if let Some((_, a)) = self.pill.as_ref().and_then(|p| p.action.clone())
                            {
                                notices::acted(self);
                                self.pill_action(a);
                            }
                        }
                        PillPart::Body => {
                            let prayer = self.pill.as_ref().is_some_and(|p| p.icon == Icon::Bell);
                            notices::closed_by_you(self);
                            self.dismiss_pill();
                            if prayer {
                                crate::prayer::open_panel(self);
                            }
                        }
                        _ => {}
                    },
                    _ => {}
                }
            }
            Kind::Panel => panels::on_mouse(self, ev, x, y),
        }
    }

    fn on_orb_mouse(&mut self, ev: Ev, g: NSPoint) {
        match ev {
            Ev::Down => {
                self.drag = Some(Drag {
                    from: g,
                    origin: (self.x.pos, self.top),
                    moved: false,
                });
                self.tuck_at = None;
            }
            Ev::Drag => {
                if let Some(d) = &mut self.drag {
                    let (dx, dy) = ((g.x - d.from.x) as f32, (g.y - d.from.y) as f32);
                    if dx.abs() + dy.abs() > 3.0 {
                        d.moved = true;
                    }
                    if d.moved {
                        let (x, top) = (d.origin.0 + dx, d.origin.1 + dy);
                        self.x.snap(x);
                        self.top = top;
                        self.close_dial();
                        self.close_panel();
                        self.place_orb();
                    }
                }
            }
            Ev::Up => {
                let Some(d) = self.drag.take() else { return };
                if d.moved {
                    // Snap to the nearest side of the screen it was dropped on.
                    let (vf, scale) = self.screen();
                    self.scale = scale;
                    let mid = (vf.origin.x + vf.size.width / 2.0) as f32;
                    self.side = if self.x.pos + WIN / 2.0 < mid {
                        Side::Left
                    } else {
                        Side::Right
                    };
                    self.tucked = false;
                    self.x.target = self.dock_x(self.side, false);
                    (self.x.k, self.x.c) = (480.0, 34.0);
                    self.top = self.clamp_top(self.top);
                    self.place_orb();
                    self.save_position();
                    self.arm_tuck();
                } else if self.dial_open {
                    self.close_dial();
                } else {
                    self.open_dial(false);
                }
            }
            Ev::Move => self.on_pointer(g),
            Ev::Exit | Ev::RightUp => {}
        }
    }

    fn on_global_down(&mut self) {
        if self.dial_open {
            self.close_dial();
        }
        if self.panel_open {
            self.close_panel();
        }
    }

    fn save_position(&self) {
        let path = clock::data_dir().join("mac-orb.json");
        let v = serde_json::json!({
            "side": if self.side == Side::Left { "left" } else { "right" },
            "top": self.top,
        });
        let _ = std::fs::write(path, v.to_string());
    }

    /// Test commands from `dev.dhio.orby.test` (see `run`).
    fn test_command(&mut self, cmd: &str) {
        self.log(&format!("test: {cmd}"));
        match cmd {
            "dial" => self.open_dial(true),
            "dial-mouse" => {
                self.open_dial(false);
                self.dial_aim = Some(PRAYER);
                self.render_dial();
            }
            "close" => {
                self.close_dial();
                self.close_panel();
                self.dismiss_pill();
            }
            "panel" => crate::prayer::open_panel(self),
            "focus" => panels::open_panel(self, panels::Kind::Focus),
            "drink" => panels::open_panel(self, panels::Kind::Drink),
            "history" => history::open(self, None),
            "notices" => notices::open_panel(self),
            "drink-undo" => habits::undo_drink(self),
            "focus-stop" => habits::stop(self),
            "state" => {
                let line = self.state_line();
                self.log(&format!("state: {line}"));
            }
            "shortcut" => self.on_shortcut(),
            "tuck" => self.set_tucked(true),
            "untuck" => self.set_tucked(false),
            "hide" => self.set_visible(false),
            "show" => self.set_visible(true),
            "pill" => self.pick(0),
            "more" => {
                self.open_dial(true);
                self.pick(MORE);
            }
            "disk" => crate::disk::open(self),
            "more-pick" => {
                if let Some(parent) = self.dial_branch {
                    let j = self
                        .dial_child
                        .unwrap_or_else(|| menu::children(self, parent).1);
                    self.pick_child(parent, j);
                }
            }
            "disk-close" => super::disk_win::close(self),
            "disk-check" => crate::disk::check_space(self),
            "disk-undo" => crate::disk::undo(self),
            "disk-state" => {
                let line = crate::disk::state_line(self);
                self.log(&format!("disk state: {line}"));
            }
            "login" => self.log(&format!(
                "start at login: {}",
                if super::login::enabled() { "on" } else { "off" }
            )),
            "settings" => crate::settings::open(self, crate::settings_model::Section::General),
            "settings-close" => crate::settings::close(self),
            "settings-state" => {
                let line = crate::settings::state_line(self);
                let login = if super::login::enabled() {
                    "on"
                } else if super::login::needs_approval() {
                    "waiting for approval"
                } else {
                    "off"
                };
                self.log(&format!("settings state: {line}, login item {login}"));
            }
            "first-run" => crate::settings::open_first_run(self),
            "first-run-skip" => crate::settings::skip_first_run(self),
            _ if cmd.starts_with("settings:") => {
                let name = &cmd["settings:".len()..];
                match crate::settings_model::Section::ALL
                    .iter()
                    .find(|s| s.name().eq_ignore_ascii_case(name))
                {
                    Some(s) => crate::settings::go_to(self, *s),
                    None => self.log(&format!("settings: no section {name}")),
                }
            }
            _ if cmd.starts_with("settings-click:") => {
                let xy: Vec<f32> = cmd["settings-click:".len()..]
                    .split(',')
                    .filter_map(|v| v.trim().parse().ok())
                    .collect();
                if let [x, y] = xy[..] {
                    crate::settings::click_at(self, x, y);
                }
            }
            _ if cmd.starts_with("settings-type:") => {
                crate::settings::on_text(self, &cmd["settings-type:".len()..]);
            }
            _ if cmd.starts_with("settings-key:") => {
                // A Windows virtual key (0x.. or decimal), pressed and let go; ",down" or ",up"
                // for one half (a modifier held while the shortcut is chosen).
                let arg = &cmd["settings-key:".len()..];
                let (num, half) = arg.split_once(',').unwrap_or((arg, ""));
                let k = num
                    .strip_prefix("0x")
                    .map_or_else(|| num.parse().ok(), |h| u16::from_str_radix(h, 16).ok());
                if let Some(k) = k {
                    if half != "up" {
                        crate::settings::on_key(self, k, true, false);
                    }
                    if half != "down" {
                        crate::settings::on_key(self, k, false, false);
                    }
                }
            }
            _ if cmd.starts_with("settings-shot:") => {
                super::settings_win::shot(self, &cmd["settings-shot:".len()..]);
            }
            "windows" => {
                for (name, w) in [
                    ("orb", &self.orb),
                    ("dial", &self.dial),
                    ("pill", &self.pill_win),
                    ("panel", &self.panel_win),
                ] {
                    let f = w.panel.frame();
                    self.log(&format!(
                        "window {name} #{} visible {} level {} frame {:.0},{:.0} {:.0}x{:.0}",
                        w.panel.windowNumber(),
                        w.panel.isVisible(),
                        w.panel.level(),
                        f.origin.x,
                        f.origin.y,
                        f.size.width,
                        f.size.height
                    ));
                }
            }
            _ if cmd.starts_with("dial-shot:") => {
                let path = cmd["dial-shot:".len()..].to_string();
                let items = menu::items(self);
                let kids = self.branch_children();
                let branch = self.dial_branch.map(|parent| ui::DialBranch {
                    parent,
                    children: &kids,
                    aimed: self.dial_child,
                    reveal: 1.0,
                });
                let b = Bitmap::new(ui::DIAL_W, ui::DIAL_H, 2.0);
                let tags = if branch.is_some() { 0.0 } else { 1.0 };
                ui::draw_dial(
                    &mut b.canvas(),
                    &self.theme,
                    &self.fonts,
                    &items,
                    self.dial_aim,
                    1.0,
                    self.mirrored(),
                    self.dial_keyboard,
                    branch.as_ref(),
                    habits::ring(self),
                    tags,
                );
                let p = std::path::Path::new(&path);
                if let (Some(dir), Some(name)) = (p.parent(), p.file_stem()) {
                    crate::snapshot::write(
                        dir,
                        &name.to_string_lossy(),
                        b.pw as u32,
                        b.ph as u32,
                        b.rgba(),
                    );
                }
            }
            _ if cmd.starts_with("panel-shot:") => {
                panels::shot(self, &cmd["panel-shot:".len()..]);
            }
            _ if cmd.starts_with("drink-log:") => {
                let ml = cmd["drink-log:".len()..].parse().unwrap_or(250);
                let p = habits::log_drink(self, ml);
                notices::reply(self, p);
            }
            _ if cmd.starts_with("focus-start:") => {
                let m = cmd["focus-start:".len()..].parse().unwrap_or(25);
                habits::start_focus(self, m);
            }
            _ if cmd.starts_with("disk-shot:") => {
                super::disk_win::shot(self, &cmd["disk-shot:".len()..]);
            }
            _ if cmd.starts_with("disk-bench:") => {
                let n = cmd["disk-bench:".len()..].parse().unwrap_or(200);
                super::disk_win::scroll_bench(self, n);
            }
            "disk-resources" => {
                let rss = super::disk::resident_bytes() as f64 / 1_048_576.0;
                self.log(&format!("resources: resident {rss:.1} MB"));
            }
            _ if cmd.starts_with("disk-") => crate::disk::test_command(self, cmd),
            _ => {
                if let Some(off) = cmd
                    .strip_prefix("remind")
                    .and_then(|o| o.parse::<i32>().ok())
                {
                    let (np, _, _) =
                        pr::next(&self.times, clock::now_minutes(), self.tomorrow_subuh);
                    self.fire_reminder(np, off);
                }
            }
        }
    }

    /// One line for the test channel's `state`: the panel, drink, focus and the queue.
    fn state_line(&self) -> String {
        let kind = if self.panel_open {
            format!("{:?}", self.panel_kind)
        } else {
            "none".into()
        };
        let (w, h) = (self.panel_win.w, self.panel_win.h);
        let phase = match self.habits.focus.phase() {
            app_habits::focus::Phase::Idle => "idle".to_string(),
            p => format!("{p:?} ({})", habits::dial_focus_value(self)),
        };
        let timers: Vec<String> = self.timers.keys().map(|t| format!("{t:?}")).collect();
        format!(
            "panel {kind} {w:.0}x{h:.0}, drink {} ml, focus {phase}, notices {} waiting (badge {}), pill {}, timers [{}], away {}",
            habits::drink_total(self),
            self.notices.q.waiting(),
            notices::badge_count(self),
            self.pill.as_ref().map_or("none".into(), |p| format!("\"{}\"", p.title)),
            timers.join(" "),
            self.habits.away,
        )
    }

    /// A shared unit's timer fired (`sys::Timer`, mac/sys.rs).
    fn fire_timer(&mut self, t: Timer) {
        match t {
            Timer::Focus => habits::on_focus_timer(self),
            Timer::Drink => habits::on_drink_timer(self),
            Timer::Notices => notices::on_timer(self),
            Timer::Panel => panels::on_timer(self),
            Timer::Tasks | Timer::CalCue | Timer::CalRead => {
                self.timers.remove(&t);
                self.log(&format!("timer {t:?}: not on the Mac yet"));
            }
        }
    }

    // ---- the frame loop ----

    fn animating(&self) -> bool {
        !self.x.settled(0.3)
            || !self.opacity.settled(0.002)
            || (self.dial.shown && !self.dial_reveal.settled(0.002))
            || (self.pill_win.shown && !self.pill_reveal.settled(0.002))
            || panels::animating(self)
            || (self.dial_branch.is_some() && !self.branch_reveal.settled(0.002))
            || (self.disk.win.open && (self.disk.win.ticking || self.disk.win.dirty))
    }

    fn tick(&mut self) {
        let now = Instant::now();
        let dt = now
            .duration_since(self.last_frame)
            .as_secs_f32()
            .clamp(0.0, 0.05);
        self.last_frame = now;

        // Deadlines.
        if self.tuck_at.is_some_and(|t| now >= t) {
            self.tuck_at = None;
            if !self.near && !self.dial_open && !self.panel_open && self.pill.is_none() {
                self.set_tucked(true);
            }
        }
        if self.pill_until.is_some_and(|t| now >= t) {
            if self.pill_hover != PillPart::None {
                self.pill_until = Some(now + Duration::from_secs(2));
            } else {
                self.dismiss_pill();
            }
        }
        if let Some((at, p, off)) = self.next_reminder {
            if now >= at {
                self.refresh_day();
                self.fire_reminder(p, off);
                self.schedule_reminders();
            }
        }
        // The shared units' timers, oldest first; each repeats (SetTimer) unless its handler
        // disarms or re-arms it. A handler may change the others, so each is checked again.
        let mut due: Vec<(Instant, Timer)> = self
            .timers
            .iter()
            .filter(|(_, (at, _))| now >= *at)
            .map(|(t, (at, _))| (*at, *t))
            .collect();
        due.sort_by_key(|d| d.0);
        for (_, t) in due {
            match self.timers.get(&t).copied() {
                Some((at, every)) if now >= at => {
                    self.timers.insert(t, (now + every, every));
                    self.fire_timer(t);
                }
                _ => {}
            }
        }
        if self.away_check_at.is_some_and(|t| now >= t) {
            super::sys::on_away_check(self);
        }
        if now >= self.sun_at {
            self.refresh_day();
        }
        if self.branch_at.is_some_and(|t| now >= t) {
            self.branch_at = None;
            if let Some(i) = self
                .dial_aim
                .filter(|i| self.dial_open && has_branch(*i) && self.dial_branch.is_none())
            {
                self.open_branch(i, false);
            }
        }
        if self.disk_check_at.is_some_and(|t| now >= t) {
            self.disk_check_at = Some(now + self.disk_check_every);
            crate::disk::on_timer(self);
        }
        if self.disk_release_at.is_some_and(|t| now >= t) {
            self.disk_release_at = None;
            crate::disk::on_release_timer(self);
        }

        // Springs.
        let moving_x = !self.x.settled(0.3);
        let fading = !self.opacity.settled(0.002);
        self.x.step(dt);
        self.opacity.step(dt);
        if moving_x {
            self.place_orb();
        }
        if fading || self.orb_dirty {
            self.render_orb();
        }
        if self.dial.shown {
            self.dial_reveal.step(dt);
            if !self.dial_open && self.dial_reveal.pos < 0.02 {
                self.dial.hide();
            } else {
                self.render_dial();
            }
        }
        if self.pill_win.shown {
            self.pill_reveal.step(dt);
            if self.pill_closing && self.pill_reveal.pos < 0.05 {
                self.pill_win.hide();
                self.pill = None;
                self.pill_closing = false;
                self.arm_tuck();
                notices::on_pill_gone(self);
            } else if !self.pill_reveal.settled(0.002) {
                self.render_pill();
            }
        }
        panels::tick(self);
        if self.dial_branch.is_some() && !self.branch_reveal.settled(0.002) {
            self.branch_reveal.step(dt);
            self.render_dial();
        }
        if self.disk.win.open && (self.disk.win.ticking || self.disk.win.dirty) {
            super::disk_win::render(self);
        }
    }

    /// Sets the one timer to the next frame or the next deadline.
    fn schedule(&mut self) {
        let wait = if self.animating() {
            FRAME
        } else {
            let now = Instant::now();
            [
                self.tuck_at,
                self.pill_until,
                self.next_reminder.map(|r| r.0),
                Some(self.sun_at),
                self.branch_at,
                self.disk_check_at,
                self.disk_release_at,
                self.away_check_at,
            ]
            .into_iter()
            .flatten()
            .chain(self.timers.values().map(|(at, _)| *at))
            .min()
            .map(|t| t.saturating_duration_since(now).as_secs_f64().max(0.001))
            .unwrap_or(3600.0)
        };
        if let Some(t) = self.timer.take() {
            t.invalidate();
        }
        if !self.animating() {
            self.last_frame = Instant::now();
        }
        let block = RcBlock::new(|_t: NonNull<NSTimer>| {
            APP.with(|cell| {
                if let Ok(mut b) = cell.try_borrow_mut() {
                    if let Some(a) = b.as_mut() {
                        a.tick();
                        a.schedule();
                    }
                }
            });
        });
        self.timer = Some(unsafe {
            NSTimer::scheduledTimerWithTimeInterval_repeats_block(wait, false, &block)
        });
    }
}

// ---- start ----

fn status_icon() -> Retained<NSImage> {
    // A drop with its highlight, drawn with the shared Canvas: a template image, so the menu bar
    // tints it for light, dark and selected.
    let b = Bitmap::new(18.0, 18.0, 2.0);
    {
        let mut cv = b.canvas();
        let ink = crate::canvas::rgba(0x000000, 1.0);
        cv.stroke_ellipse(
            &crate::canvas::ellipse(9.0, 9.0, 6.6),
            ink,
            1.6,
            crate::canvas::Stroke::Plain,
        );
        cv.fill_ellipse(&crate::canvas::ellipse(7.2, 7.2, 2.2), ink);
    }
    let img = b.image().expect("the status icon");
    let ns = NSImage::initWithCGImage_size(NSImage::alloc(), &img, NSSize::new(18.0, 18.0));
    ns.setTemplate(true);
    ns
}

fn menu_item(
    mtm: MainThreadMarker,
    title: &str,
    action: objc2::runtime::Sel,
    target: &MenuTarget,
) -> Retained<NSMenuItem> {
    let item = unsafe {
        NSMenuItem::initWithTitle_action_keyEquivalent(
            NSMenuItem::alloc(mtm),
            &NSString::from_str(title),
            Some(action),
            &NSString::from_str(""),
        )
    };
    unsafe { item.setTarget(Some(target)) };
    item
}

pub fn run() {
    let mtm = MainThreadMarker::new().expect("Orby starts on the main thread");
    let app = NSApplication::sharedApplication(mtm);
    app.setActivationPolicy(NSApplicationActivationPolicy::Accessory);

    let dir = clock::data_dir();
    let settings = crate::settings::Settings::load(&dir, Default::default());
    let day = clock::today();
    let (times, tomorrow_subuh) =
        crate::prayer::compute_times(&settings.prefs, settings.prefs.place.coords(), day);
    let theme = crate::settings::theme_now(&settings.prefs);

    // The drop's place: kept from the last run, else the right edge, a third down.
    let saved: serde_json::Value = std::fs::read_to_string(dir.join("mac-orb.json"))
        .ok()
        .and_then(|s| serde_json::from_str(&s).ok())
        .unwrap_or_default();
    let main_screen = screen_at(mtm, 0.0, 0.0);
    let (vf, scale) = main_screen;
    let side = if saved["side"] == "left" {
        Side::Left
    } else {
        Side::Right
    };
    let top = saved["top"]
        .as_f64()
        .map(|t| t as f32)
        .unwrap_or((vf.origin.y + vf.size.height * 2.0 / 3.0) as f32);

    let gpu = Gpu::new();
    let orb = Win::new(mtm, Kind::Orb, WIN, WIN, LEVEL_ORB, false);
    let orb_layer = gpu.as_ref().ok().map(|g| {
        let layer = g.layer(WIN * scale * SS);
        layer.setFrame(CGRect::new(
            CGPoint::new(0.0, 0.0),
            CGSize::new(WIN as f64, WIN as f64),
        ));
        layer.setContentsScale(scale as f64);
        orb.view.setLayer(Some(&layer));
        orb.view.setWantsLayer(true);
        layer
    });
    let dial = Win::new(mtm, Kind::Dial, ui::DIAL_W, ui::DIAL_H, LEVEL_PANEL, false);
    let pill_win = Win::new(
        mtm,
        Kind::Pill,
        200.0,
        ui::PILL_H + 2.0 * ui::PILL_PAD,
        LEVEL_PILL,
        false,
    );
    let panel_win = Win::new(mtm, Kind::Panel, panel::W, panel::H, LEVEL_PANEL, true);
    let countdown_view = {
        let (l, t, r, b) = panel::countdown_rect(&Fonts::new());
        let frame = NSRect::new(
            NSPoint::new(l as f64, t as f64),
            NSSize::new((r - l) as f64, (b - t) as f64),
        );
        let v = NSView::initWithFrame(NSView::alloc(mtm), frame);
        v.setWantsLayer(true);
        panel_win.view.addSubview(&v);
        v
    };

    // The menu bar item.
    let target = MenuTarget::new(mtm);
    let status = NSStatusBar::systemStatusBar().statusItemWithLength(-1.0);
    if let Some(button) = status.button(mtm) {
        button.setImage(Some(&status_icon()));
        button.setToolTip(Some(&NSString::from_str("Orby")));
    }
    let menu = NSMenu::new(mtm);
    let toggle = menu_item(mtm, "Hide Orby", sel!(toggleOrb:), &target);
    menu.addItem(&toggle);
    menu.addItem(&menu_item(mtm, "Prayer", sel!(openPrayer:), &target));
    menu.addItem(&menu_item(mtm, "Disk…", sel!(openDisk:), &target));
    let settings_item = menu_item(mtm, "Settings…", sel!(openSettings:), &target);
    settings_item.setKeyEquivalent(&NSString::from_str(","));
    menu.addItem(&settings_item);
    menu.addItem(&NSMenuItem::separatorItem(mtm));
    let login = menu_item(mtm, "Start at login", sel!(toggleLogin:), &target);
    menu.addItem(&login);
    menu.addItem(&NSMenuItem::separatorItem(mtm));
    menu.addItem(&menu_item(mtm, "Quit Orby", sel!(quit:), &target));
    status.setMenu(Some(&menu));

    let gpu_note = match &gpu {
        Ok(_) => "Metal".to_string(),
        Err(e) => format!("no Metal ({e})"),
    };
    let mut a = App {
        gpu: gpu.ok(),
        orb_layer,
        orb,
        dial,
        pill_win,
        panel_win,
        _status: Some(status),
        _menu_target: target,
        toggle_item: Some(toggle),
        login_item: Some(login),
        fonts: Fonts::new(),
        theme,
        scale,
        log_path: dir.join("orb-proto.log"),
        started: Instant::now(),
        side,
        x: Spring::new(0.0, 480.0, 34.0),
        top,
        opacity: Spring::new(1.0, 300.0, 30.0),
        tucked: false,
        tuck_at: None,
        visible: true,
        drag: None,
        near: false,
        orb_dirty: true,
        dial_open: false,
        dial_reveal: Spring::new(0.0, 190.0, 26.0),
        dial_aim: None,
        dial_keyboard: false,
        dial_branch: None,
        dial_child: None,
        branch_reveal: Spring::new(0.0, 190.0, 26.0),
        branch_at: None,
        pill: None,
        pill_layout: PillLayout::default(),
        pill_reveal: Spring::new(0.0, 320.0, 17.0),
        pill_until: None,
        pill_hover: PillPart::None,
        pill_closing: false,
        panel_open: false,
        panel_kind: panels::Kind::Prayer,
        panel_hover: None,
        panel_back: None,
        knob_drag: false,
        panel_moving: false,
        countdown_view,
        countdown_shown: String::new(),
        countdown_rolling: false,
        panel_dirty: true,
        panel_min: -1,
        settings,
        times,
        tomorrow_subuh,
        day,
        sun_angle: 90.0,
        brightness: 1.0,
        sun_at: Instant::now(),
        next_reminder: None,
        notices: notices::Notices::load(&dir),
        habits: habits::Habits::load(dir.join("habits.json")),
        history: history::History::new(),
        timers: HashMap::new(),
        away_check_at: None,
        disk: crate::disk::Disk::load(),
        disk_check_at: None,
        disk_check_every: Duration::from_secs(600),
        disk_release_at: None,
        timer: None,
        last_frame: Instant::now(),
    };
    a.top = a.clamp_top(a.top);
    a.x.snap(a.dock_x(a.side, false));
    a.log(&format!(
        "start: Orby for macOS, {gpu_note}, scale {scale}, {} theme, place {}",
        if a.theme.dark { "dark" } else { "light" },
        a.settings.prefs.place.short()
    ));
    a.day = (0, 0, 0); // computes and logs today's times
    a.refresh_day();
    // Notices before any source can post; then prayer, drink and focus (as Windows starts).
    notices::start(&mut a);
    a.schedule_reminders();
    habits::schedule_drink(&mut a);
    habits::arm_focus(&mut a);
    a.place_orb();
    a.orb.show();
    a.render_orb();
    a.arm_tuck();
    // The shortcut (Carbon hot key) is registered by settings::start, which also shows the first
    // run on a fresh data folder or starts the orb hidden, before any source can post.
    if !super::hotkey::install(|| with_app(|a| a.on_shortcut())) {
        a.log("shortcut: Carbon refused the handler");
    }
    crate::settings::start(&mut a);
    a.sync_login_item();
    a.log(&format!(
        "start at login: {}",
        if super::login::enabled() { "on" } else { "off" }
    ));
    // The low-space check: now and every 10 min (a pill under 10% free).
    crate::disk::start(&mut a);
    APP.with(|c| *c.borrow_mut() = Some(a));

    // The pointer anywhere: near the drop (slide out), aiming the dial, clicks elsewhere.
    let moved = RcBlock::new(|_e: NonNull<NSEvent>| {
        let p = NSEvent::mouseLocation();
        with_app(|a| a.on_pointer(p));
    });
    let down = RcBlock::new(|_e: NonNull<NSEvent>| {
        with_app(|a| a.on_global_down());
    });
    let _m1 = NSEvent::addGlobalMonitorForEventsMatchingMask_handler(
        NSEventMask::MouseMoved | NSEventMask::LeftMouseDragged,
        &moved,
    );
    let _m2 = NSEvent::addGlobalMonitorForEventsMatchingMask_handler(
        NSEventMask::LeftMouseDown | NSEventMask::RightMouseDown,
        &down,
    );
    // Test commands, like the tray's test items on Windows. From a shell:
    // osascript -l JavaScript -e "ObjC.import('Foundation');
    //   $.NSDistributedNotificationCenter.defaultCenter
    //   .postNotificationNameObjectUserInfoDeliverImmediately('dev.dhio.orby.test', 'dial', null, true)"
    let test = RcBlock::new(|n: NonNull<NSNotification>| {
        let n = unsafe { n.as_ref() };
        let cmd = n
            .object()
            .and_then(|o| o.downcast::<NSString>().ok())
            .map(|s| s.to_string())
            .unwrap_or_default();
        with_app(|a| a.test_command(&cmd));
    });
    let _obs = unsafe {
        NSDistributedNotificationCenter::defaultCenter().addObserverForName_object_queue_usingBlock(
            Some(&NSString::from_str("dev.dhio.orby.test")),
            None,
            Some(&NSOperationQueue::mainQueue()),
            &test,
        )
    };
    // After sleep the monotonic clock has stood still: recompute today and the next reminder
    // from the wall clock.
    let woke = RcBlock::new(|_n: NonNull<NSNotification>| {
        with_app(|a| {
            a.log("woke from sleep");
            a.refresh_day();
            a.schedule_reminders();
            habits::schedule_drink(a);
            habits::arm_focus(a);
            notices::pump(a);
        });
    });
    // Light and dark switched in System Settings: a theme that follows the system follows it.
    let appearance = RcBlock::new(|_n: NonNull<NSNotification>| {
        with_app(crate::settings::on_system_theme);
    });
    let _appearance = unsafe {
        NSDistributedNotificationCenter::defaultCenter().addObserverForName_object_queue_usingBlock(
            Some(&NSString::from_str(
                "AppleInterfaceThemeChangedNotification",
            )),
            None,
            Some(&NSOperationQueue::mainQueue()),
            &appearance,
        )
    };
    let _wake = unsafe {
        NSWorkspace::sharedWorkspace()
            .notificationCenter()
            .addObserverForName_object_queue_usingBlock(
                Some(&NSString::from_str("NSWorkspaceDidWakeNotification")),
                None,
                Some(&NSOperationQueue::mainQueue()),
                &woke,
            )
    };
    with_app(|_| {});
    app.run();
}
