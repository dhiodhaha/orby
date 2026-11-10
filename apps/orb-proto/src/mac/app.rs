//! The macOS app: one state on the main thread, four borderless non-activating panels (the
//! orb, the dial, the pill, the Prayer panel) and a menu bar item.
//!
//! Like Windows it renders only while something moves. Each wake is one NSTimer: 60 Hz while a
//! spring or a rolling number moves, otherwise a one-shot timer at the next deadline (the tuck,
//! a pill's time, the next prayer reminder, the panel's next second). Between them the process
//! sleeps in the run loop. The pointer is followed with global mouse monitors, which need no
//! Accessibility permission (only key monitors do); the shortcut is a Carbon hot key.

use std::cell::{Cell, RefCell};
use std::io::Write as _;
use std::path::PathBuf;
use std::ptr::NonNull;
use std::time::{Duration, Instant};

use app_core::prayer::{self as pr, Location, Prayer, Times};
use app_core::qibla;
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
use crate::notices_model::{Next, Notice, Now, Offered, Priority, Queue, Quiet, Source, STICKY_MS};
use crate::panel;
use crate::settings_model::Prefs;
use crate::ui::{self, Fonts, Icon, Pill, PillLayout, PillPart, Theme};

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

struct Win {
    panel: Retained<OrbyPanel>,
    view: Retained<OrbyView>,
    w: f32,
    h: f32,
    shown: bool,
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

    fn resize(&mut self, w: f32, h: f32) {
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
    fn place(&self, x: f32, top: f32) {
        self.panel
            .setFrameOrigin(NSPoint::new(x as f64, (top - self.h) as f64));
    }

    fn show(&mut self) {
        if !self.shown {
            self.panel.orderFrontRegardless();
            self.shown = true;
        }
    }

    fn hide(&mut self) {
        if self.shown {
            self.panel.orderOut(None);
            self.shown = false;
        }
    }

    /// Draws with the shared Canvas into a bitmap that becomes the view's layer contents.
    fn paint(&self, scale: f32, draw: impl FnOnce(&mut dyn Canvas)) {
        let space = self.panel.colorSpace().and_then(|s| s.CGColorSpace());
        let b = match &space {
            Some(sp) => Bitmap::with_space(self.w, self.h, scale, sp),
            None => Bitmap::new(self.w, self.h, scale),
        };
        draw(&mut b.canvas());
        let Some(img) = b.image() else { return };
        let Some(layer) = self.view.layer() else {
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
            });
        }

        #[unsafe(method(openPrayer:))]
        fn open_prayer(&self, _sender: Option<&AnyObject>) {
            with_app(|a| {
                a.set_visible(true);
                a.open_panel();
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

#[derive(Clone, Copy, PartialEq, Debug)]
enum Ev {
    Down,
    Drag,
    Up,
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
    panel_win: Win,
    _status: Option<Retained<NSStatusItem>>,
    _menu_target: Retained<MenuTarget>,
    toggle_item: Option<Retained<NSMenuItem>>,
    login_item: Option<Retained<NSMenuItem>>,
    pub(crate) fonts: Fonts,
    pub(crate) theme: Theme,
    scale: f32,
    log_path: PathBuf,
    started: Instant,
    // The drop: its window's left edge (springs) and top, in screen coordinates (y up).
    side: Side,
    x: Spring,
    top: f32,
    opacity: Spring,
    tucked: bool,
    tuck_at: Option<Instant>,
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
    /// The pill on screen came from the queue (and goes back to it when it closes).
    pub(crate) pill_queued: bool,
    // The Prayer panel.
    panel_open: bool,
    panel_hover: Option<panel::Hit>,
    // Prayer.
    prefs: Prefs,
    times: Times,
    tomorrow_subuh: u32,
    day: (i32, u32, u32),
    sun_angle: f32,
    brightness: f32,
    sun_at: Instant,
    next_reminder: Option<(Instant, Prayer, i32)>,
    pub(crate) queue: Queue,
    queue_wait: Option<Instant>,
    countdown_at: Option<Instant>,
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

fn compute_times(prefs: &Prefs, day: (i32, u32, u32)) -> (Times, u32) {
    let (lat, lng) = prefs.place.coords();
    let loc = Location {
        lat,
        lng,
        tz: clock::local().offset as f64 / 3600.0,
        elevation_m: 0.0,
    };
    let m = prefs.prayer_method.method();
    let adj = prefs.prayer_adjust;
    let t = pr::times(loc, m, day.0, day.1, day.2).adjusted(adj);
    let n = tomorrow(day);
    (t, pr::times(loc, m, n.0, n.1, n.2).adjusted(adj).subuh)
}

fn read_settings(dir: &std::path::Path) -> serde_json::Map<String, serde_json::Value> {
    std::fs::read_to_string(dir.join("settings.json"))
        .ok()
        .and_then(|s| serde_json::from_str::<serde_json::Value>(&s).ok())
        .and_then(|v| v.as_object().cloned())
        .unwrap_or_default()
}

const DIAL_NAMES: [&str; 6] = ["Tasks", "Today", "Prayer", "Focus", "Drink", "More"];
const DIAL_ICONS: [Icon; 6] = [
    Icon::Tasks,
    Icon::Today,
    Icon::Prayer,
    Icon::Focus,
    Icon::Drink,
    Icon::More,
];
const PRAYER: usize = 2;
const MORE: usize = 5;
/// More's branch, in the Windows order.
const MORE_KIDS: [(Icon, &str); 6] = [
    (Icon::Download, "Downloads"),
    (Icon::HardDrive, "Disk"),
    (Icon::Grid, "History"),
    (Icon::Bell, "Notices"),
    (Icon::Gear, "Settings"),
    (Icon::List, "All tasks"),
];
const MORE_DISK: usize = 1;

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

    fn sync_login_item(&self) {
        if let Some(item) = &self.login_item {
            item.setState(if super::login::enabled() { 1 } else { 0 });
        }
    }

    fn now(&self) -> Now {
        Now {
            at: clock::now(),
            ms: self.started.elapsed().as_millis() as u64,
        }
    }

    pub(crate) fn quiet(&self) -> Quiet {
        Quiet {
            hidden: !self.visible,
            full_screen: false,
            focus: false,
            paused: false,
            presenting: false,
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

    fn render_orb(&mut self) {
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

    fn set_tucked(&mut self, tucked: bool) {
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
    fn arm_tuck(&mut self) {
        let busy = self.dial_open || self.panel_open || self.pill.is_some() || self.drag.is_some();
        self.tuck_at = if busy || self.near || self.tucked {
            None
        } else {
            Some(Instant::now() + TUCK_AFTER)
        };
    }

    fn set_visible(&mut self, v: bool) {
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
            // While a scan runs, coming close names it with a counting pill.
            if near {
                crate::disk::on_orb_near(self, true);
            } else {
                crate::disk::on_orb_near(self, false);
                crate::disk::on_orb_gone(self);
            }
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

    fn dial_items(&self) -> Vec<ui::DialItem> {
        let (np, nat, _) = pr::next(&self.times, clock::now_minutes(), self.tomorrow_subuh);
        (0..6)
            .map(|i| ui::DialItem {
                icon: DIAL_ICONS[i],
                name: DIAL_NAMES[i],
                value: if i == PRAYER {
                    format!("{} {}", np.name(), pr::hhmm(nat))
                } else if i == MORE {
                    crate::disk::dial_value(self).unwrap_or_default()
                } else {
                    String::new()
                },
            })
            .collect()
    }

    /// More's children, as on Windows: Downloads, Disk, History, Notices, Settings, All tasks.
    /// Disk works on the Mac; the others say they are coming.
    fn more_children(&self) -> Vec<ui::DialChild> {
        MORE_KIDS
            .iter()
            .enumerate()
            .map(|(j, (icon, name))| {
                if j == MORE_DISK {
                    crate::disk::dial_child(self)
                } else {
                    ui::DialChild {
                        text: String::new(),
                        icon: Some(*icon),
                        name: (*name).into(),
                        value: String::new(),
                    }
                }
            })
            .collect()
    }

    fn render_dial(&mut self) {
        let items = self.dial_items();
        let kids = if self.dial_branch.is_some() {
            self.more_children()
        } else {
            Vec::new()
        };
        let branch = self.dial_branch.map(|parent| ui::DialBranch {
            parent,
            children: &kids,
            aimed: self.dial_child,
            reveal: self.branch_reveal.pos,
        });
        let (t, f) = (self.theme, &self.fonts);
        let (aim, rv, mir, kb) = (
            self.dial_aim,
            self.dial_reveal.pos,
            self.mirrored(),
            self.dial_keyboard,
        );
        // Value tags step aside while the branch is out.
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
                None,
                tags,
            );
        });
    }

    fn open_branch(&mut self, aim_default: bool) {
        self.branch_at = None;
        self.dial_aim = Some(MORE);
        if self.dial_branch != Some(MORE) {
            self.dial_branch = Some(MORE);
            self.branch_reveal = Spring::new(0.0, 190.0, 26.0);
            self.branch_reveal.target = 1.0;
        }
        self.dial_child = aim_default.then_some(MORE_DISK);
        self.render_dial();
    }

    fn close_branch(&mut self) {
        self.dial_branch = None;
        self.dial_child = None;
        self.branch_at = None;
        self.render_dial();
    }

    fn pick_child(&mut self, j: usize) {
        self.close_dial();
        let name = MORE_KIDS.get(j).map_or("?", |k| k.1);
        self.log(&format!("dial pick more child {j} ({name})"));
        if j == MORE_DISK {
            crate::disk::open(self);
        } else {
            self.reply(Pill {
                icon: Icon::Info,
                dot: false,
                title: "Coming to Mac".into(),
                value: name.into(),
                action: None,
                closable: true,
                ttl_ms: 4000,
            });
        }
    }

    fn open_dial(&mut self, keyboard: bool) {
        if self.dial_open {
            return;
        }
        if self.pill.is_some() {
            self.dismiss_pill(false);
        }
        self.close_panel();
        self.set_tucked(false);
        self.tuck_at = None;
        self.theme = ui::theme(dark_mode());
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

    fn aim(&mut self, dx: f32, dy: f32) {
        let a = ui::aim(dx, dy, self.mirrored(), 6);
        if self.dial_branch.is_some() {
            // Past the main band the pointer aims a child by direction.
            let c = ui::aim_child(dx, dy, self.mirrored(), MORE, MORE_KIDS.len());
            if c.is_some() {
                if c != self.dial_child || self.dial_keyboard {
                    self.dial_keyboard = false;
                    self.dial_child = c;
                    self.render_dial();
                }
                return;
            }
            if a.is_some() && a != Some(MORE) {
                self.dial_branch = None;
                self.dial_child = None;
            } else {
                if self.dial_child.is_some() {
                    self.dial_child = None;
                    self.render_dial();
                }
                return;
            }
        }
        if a != self.dial_aim || self.dial_keyboard {
            self.dial_keyboard = false;
            self.dial_aim = a;
            // A steady aim on More opens its branch, as on Windows (120 ms).
            self.branch_at = (a == Some(MORE)).then(|| Instant::now() + Duration::from_millis(120));
            self.render_dial();
        }
    }

    fn pick(&mut self, i: usize) {
        if i == MORE {
            // More has no panel: picking it fans out its children.
            self.log("dial pick more");
            let kb = self.dial_keyboard;
            self.open_branch(kb);
            return;
        }
        self.close_dial();
        self.log(&format!("dial pick {}", DIAL_NAMES[i].to_lowercase()));
        if i == PRAYER {
            self.open_panel();
        } else {
            self.reply(Pill {
                icon: Icon::Info,
                dot: false,
                title: "Coming to Mac".into(),
                value: DIAL_NAMES[i].into(),
                action: None,
                closable: true,
                ttl_ms: 4000,
            });
        }
    }

    fn on_shortcut(&mut self) {
        if !self.visible {
            self.set_visible(true);
            self.open_dial(true);
        } else if self.dial_open {
            self.close_dial();
            self.set_visible(false);
        } else {
            self.open_dial(true);
        }
    }

    fn on_key(&mut self, code: u16, chars: &str) {
        const ESC: u16 = 53;
        const RETURN: u16 = 36;
        const ENTER: u16 = 76;
        const UP: u16 = 126;
        const DOWN: u16 = 125;
        if self.dial_open && self.dial_branch.is_some() {
            let n = MORE_KIDS.len();
            let c = self.dial_child.unwrap_or(MORE_DISK);
            match code {
                ESC => self.close_branch(),
                RETURN | ENTER => self.pick_child(c),
                UP => {
                    self.dial_keyboard = true;
                    self.dial_child = Some((c + n - 1) % n);
                    self.render_dial();
                }
                DOWN => {
                    self.dial_keyboard = true;
                    self.dial_child = Some((c + 1) % n);
                    self.render_dial();
                }
                _ => {}
            }
        } else if self.dial_open {
            let aim = self.dial_aim.unwrap_or(0);
            match code {
                ESC => self.close_dial(),
                RETURN | ENTER => self.pick(aim),
                UP => {
                    self.dial_keyboard = true;
                    self.dial_aim = Some((aim + 5) % 6);
                    self.render_dial();
                }
                DOWN => {
                    self.dial_keyboard = true;
                    self.dial_aim = Some((aim + 1) % 6);
                    self.render_dial();
                }
                _ => {
                    if let Some(n) = chars.chars().next().and_then(|c| c.to_digit(10)) {
                        if (1..=6).contains(&n) {
                            self.pick(n as usize - 1);
                        }
                    }
                }
            }
        } else if self.panel_open && code == ESC {
            self.close_panel();
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

    fn render_pill(&mut self) {
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

    pub(crate) fn show_pill(&mut self, p: Pill, queued: bool) {
        self.theme = ui::theme(dark_mode());
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
        self.pill_queued = queued;
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
        self.queue.reply(self.now());
        self.show_pill(p, false);
    }

    pub(crate) fn dismiss_pill(&mut self, by_you: bool) {
        if self.pill.is_none() || self.pill_closing {
            return;
        }
        self.pill_closing = true;
        self.pill_until = None;
        self.pill_reveal.k = 480.0;
        self.pill_reveal.c = 40.0;
        self.pill_reveal.target = 0.0;
        if self.pill_queued {
            if by_you {
                self.queue.close_front_by_you();
            } else {
                self.queue.close_front(false);
            }
            self.pill_queued = false;
        }
    }

    /// A pill's action button.
    fn pill_action(&mut self, a: ui::PillAction) {
        use ui::PillAction as A;
        match a {
            A::FindBigFolders(d) => crate::disk::find_big_folders(self, d),
            A::OpenDisk => crate::disk::open(self),
            A::UndoRecycle => crate::disk::undo(self),
            A::RetryScan => crate::disk::retry_scan(self),
            _ => self.log("pill action not on the Mac yet"),
        }
    }

    pub(crate) fn post(&mut self, n: Notice) {
        let busy = self.pill.is_some() && !self.pill_closing;
        let title = n.pill.title.clone();
        let out = self
            .queue
            .offer(n, self.now(), clock::unix_now(), self.quiet(), busy);
        match &out {
            Offered::Show(p) => self.show_pill(p.clone(), true),
            Offered::Update(p) => self.show_pill(p.clone(), true),
            _ => {}
        }
        self.log(&format!("notice \"{title}\" {out:?}"));
        self.countdown_at = self.countdown_deadline();
    }

    /// The slot is free: the next waiting pill, or when to ask again.
    fn after_pill(&mut self) {
        self.queue_wait = None;
        match self.queue.next(self.now(), clock::unix_now(), self.quiet()) {
            Next::Show(p, _) => self.show_pill(p, true),
            Next::Wait(ms) => self.queue_wait = Some(Instant::now() + Duration::from_millis(ms)),
            Next::Idle => {}
        }
        self.countdown_at = self.countdown_deadline();
    }

    fn countdown_deadline(&self) -> Option<Instant> {
        self.queue
            .countdown_wait(clock::now())
            .map(|d| Instant::now() + d)
    }

    // ---- prayer ----

    fn refresh_day(&mut self) {
        let d = clock::today();
        if d != self.day {
            self.day = d;
            (self.times, self.tomorrow_subuh) = compute_times(&self.prefs, d);
            self.log(&format!(
                "times {}: Subuh {} Dzuhur {} Ashar {} Maghrib {} Isya {}",
                self.prefs.place.short(),
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

    fn schedule_reminders(&mut self) {
        let now = clock::now_minutes();
        self.next_reminder = reminder_events(&self.prefs, &self.times, self.tomorrow_subuh)
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
            let n = Notice::new(Source::Prayer, Priority::Normal, pill).before_prayer(
                p.name(),
                self.prayer_moment(p),
                clock::now(),
            );
            self.post(if sticky {
                n.sticky().counting_down()
            } else {
                n
            });
        } else {
            self.queue.drop_prayer(p.name());
            if self.pill_queued
                && self
                    .pill
                    .as_ref()
                    .is_some_and(|x| x.title.contains(p.name()))
            {
                self.pill_queued = false;
                self.dismiss_pill(false);
            }
            let pill = Pill {
                icon: Icon::Bell,
                dot: true,
                title: format!("Memasuki waktu {}", p.name()),
                value: String::new(),
                action: None,
                closable: true,
                ttl_ms: 10_000,
            };
            self.post(Notice::new(Source::Prayer, Priority::Urgent, pill));
        }
        self.log(&format!("reminder {} {off:+}", p.name()));
    }

    fn panel_view_data(&self) -> (Prayer, f64, String, String) {
        let (np, _, mins) = pr::next(&self.times, clock::now_minutes(), self.tomorrow_subuh);
        let (lat, lng) = self.prefs.place.coords();
        let b = qibla::bearing(lat, lng);
        (
            np,
            mins,
            self.prefs.place.short(),
            format!("{:.0}° {}", b, qibla::compass(b)),
        )
    }

    fn render_panel(&mut self) {
        let (next, mins_left, place, qibla) = self.panel_view_data();
        let view = panel::View {
            times: &self.times,
            now_min: clock::now_minutes(),
            next,
            mins_left,
            place: &place,
            method: self.prefs.prayer_method.short(),
            qibla,
            reminders: self.prefs.prayer_reminders,
            hover: self.panel_hover,
        };
        let (t, f) = (self.theme, &self.fonts);
        self.panel_win
            .paint(self.scale, |cv| panel::draw(cv, &t, f, &view));
    }

    fn place_panel(&mut self) {
        let (cx, cy) = self.center();
        let (vf, _) = self.screen();
        let gap = 12.0;
        let x = if self.side == Side::Right {
            cx - ORB / 2.0 - gap - panel::W
        } else {
            cx + ORB / 2.0 + gap
        };
        let lo = vf.origin.y as f32 + panel::H + 8.0;
        let hi = (vf.origin.y + vf.size.height) as f32 - 8.0;
        let top = (cy + panel::H / 2.0).clamp(lo, hi.max(lo));
        self.panel_win.place(x, top);
    }

    fn open_panel(&mut self) {
        self.close_dial();
        self.set_tucked(false);
        self.tuck_at = None;
        self.theme = ui::theme(dark_mode());
        self.refresh_day();
        self.panel_open = true;
        self.panel_hover = None;
        self.place_panel();
        self.render_panel();
        self.panel_win.show();
        self.panel_win.panel.makeKeyAndOrderFront(None);
        self.log("panel prayer");
    }

    pub(crate) fn close_panel(&mut self) {
        if !self.panel_open {
            return;
        }
        self.panel_open = false;
        self.panel_win.hide();
        self.arm_tuck();
    }

    fn panel_click(&mut self, hit: panel::Hit) {
        match hit {
            panel::Hit::Chip(i) => {
                self.prefs.prayer_reminders[i] = !self.prefs.prayer_reminders[i];
                let dir = clock::data_dir();
                let mut map = read_settings(&dir);
                map.insert(
                    "prayer_reminders".into(),
                    serde_json::to_value(self.prefs.prayer_reminders).unwrap_or_default(),
                );
                if let Ok(bytes) = serde_json::to_vec_pretty(&serde_json::Value::Object(map)) {
                    let path = dir.join("settings.json");
                    let tmp = path.with_extension("json.tmp");
                    if std::fs::write(&tmp, bytes).is_ok() {
                        let _ = std::fs::rename(&tmp, &path);
                    }
                }
                self.schedule_reminders();
                self.render_panel();
            }
            panel::Hit::Button(i) => {
                let what = ["Reminder settings", "Place", "Qibla"][i.min(2)];
                self.reply(Pill {
                    icon: Icon::Info,
                    dot: false,
                    title: "Coming to Mac".into(),
                    value: what.into(),
                    action: None,
                    closable: true,
                    ttl_ms: 4000,
                });
            }
        }
    }

    // ---- input ----

    fn on_mouse(&mut self, kind: Kind, ev: Ev, x: f32, y: f32, global: NSPoint) {
        match kind {
            Kind::Orb => self.on_orb_mouse(ev, global),
            Kind::Dial => match ev {
                Ev::Move | Ev::Drag => self.on_pointer(global),
                Ev::Up => match (self.dial_child, self.dial_aim) {
                    (Some(j), _) if self.dial_branch.is_some() => self.pick_child(j),
                    (_, Some(i)) => self.pick(i),
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
                    Ev::Up => match part {
                        PillPart::Close => self.dismiss_pill(true),
                        PillPart::Action => {
                            let action = self.pill.as_ref().and_then(|p| p.action.clone());
                            self.dismiss_pill(true);
                            if let Some((_, a)) = action {
                                self.pill_action(a);
                            }
                        }
                        PillPart::Body => {
                            let prayer = self.pill.as_ref().is_some_and(|p| p.icon == Icon::Bell);
                            self.dismiss_pill(true);
                            if prayer {
                                self.open_panel();
                            }
                        }
                        _ => {}
                    },
                    _ => {}
                }
            }
            Kind::Panel => {
                let hit = panel::hit(&self.fonts, x, y);
                match ev {
                    Ev::Move | Ev::Exit => {
                        let hit = if ev == Ev::Exit { None } else { hit };
                        if hit != self.panel_hover {
                            self.panel_hover = hit;
                            self.render_panel();
                        }
                    }
                    Ev::Up => {
                        if let Some(h) = hit {
                            self.panel_click(h);
                        }
                    }
                    _ => {}
                }
            }
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
            Ev::Exit => {}
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
                self.dismiss_pill(false);
            }
            "panel" => self.open_panel(),
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
                if self.dial_branch.is_some() {
                    let j = self.dial_child.unwrap_or(MORE_DISK);
                    self.pick_child(j);
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
                let items = self.dial_items();
                let kids = if self.dial_branch.is_some() {
                    self.more_children()
                } else {
                    Vec::new()
                };
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
                    None,
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

    // ---- the frame loop ----

    fn animating(&self) -> bool {
        !self.x.settled(0.3)
            || !self.opacity.settled(0.002)
            || (self.dial.shown && !self.dial_reveal.settled(0.002))
            || (self.pill_win.shown && !self.pill_reveal.settled(0.002))
            || (self.panel_open && crate::motion::active())
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
                self.dismiss_pill(false);
            }
        }
        if let Some((at, p, off)) = self.next_reminder {
            if now >= at {
                self.refresh_day();
                self.fire_reminder(p, off);
                self.schedule_reminders();
            }
        }
        if self.countdown_at.is_some_and(|t| now >= t) {
            if let Some(p) = self.queue.countdown_tick(clock::now()) {
                if self.pill_queued {
                    self.show_pill(p, true);
                }
            }
            self.countdown_at = self.countdown_deadline();
        }
        if self.queue_wait.is_some_and(|t| now >= t) && self.pill.is_none() {
            self.after_pill();
        }
        if now >= self.sun_at {
            self.refresh_day();
        }
        if self.branch_at.is_some_and(|t| now >= t) {
            self.branch_at = None;
            if self.dial_open && self.dial_aim == Some(MORE) && self.dial_branch.is_none() {
                self.open_branch(false);
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
                self.after_pill();
            } else if !self.pill_reveal.settled(0.002) {
                self.render_pill();
            }
        }
        if self.panel_open {
            self.render_panel();
        }
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
            let mut next: Vec<Instant> = [
                self.tuck_at,
                self.pill_until,
                self.next_reminder.map(|r| r.0),
                self.countdown_at,
                self.queue_wait,
                Some(self.sun_at),
                self.branch_at,
                self.disk_check_at,
                self.disk_release_at,
            ]
            .into_iter()
            .flatten()
            .collect();
            if self.panel_open {
                next.push(now + Duration::from_millis(clock::ms_to_next_second()));
            }
            next.into_iter()
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
    let prefs = Prefs::from_map(&read_settings(&dir));
    let day = clock::today();
    let (times, tomorrow_subuh) = compute_times(&prefs, day);

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
        theme: ui::theme(dark_mode()),
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
        pill_queued: false,
        panel_open: false,
        panel_hover: None,
        prefs,
        times,
        tomorrow_subuh,
        day,
        sun_angle: 90.0,
        brightness: 1.0,
        sun_at: Instant::now(),
        next_reminder: None,
        queue: Queue::new(),
        queue_wait: None,
        countdown_at: None,
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
        a.prefs.place.short()
    ));
    a.day = (0, 0, 0); // computes and logs today's times
    a.refresh_day();
    a.schedule_reminders();
    a.place_orb();
    a.orb.show();
    a.render_orb();
    a.arm_tuck();
    let hotkey = super::hotkey::register(|| with_app(|a| a.on_shortcut()));
    a.log(if hotkey {
        "shortcut Ctrl+Option+Space"
    } else {
        "shortcut Ctrl+Option+Space is taken"
    });
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
        });
    });
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
