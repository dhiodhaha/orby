//! Frame pipeline measurement on macOS (`ORB_FRAMES=1`, or the test command `frames-on`).
//!
//! Every window that draws through the shared Canvas reports each frame here with the time
//! of its stages (state, view, Core Graphics, the handoff to the layer, the commit) and the
//! input events it absorbed. Times are `mach_absolute_time` seconds, the clock `NSEvent`'s
//! timestamps use, so an event's latency runs from when the system made it to the commit.
//! `frames-report` logs, per window, the commit-to-commit interval (p50/p95/max, over 17.5
//! and 25 ms; the vsync slots the commits skipped while input kept coming, and commits that
//! shared one), the main-thread work per frame, the input-to-commit latency and, for windows
//! on a display link, the spacing of the refreshes the frames were drawn for (the cadence on
//! the glass, give or take the window server). Off, every call returns at its first branch.
//! The test drivers (`drive:...`) make synthetic scroll and mouse events on a thread, at
//! trackpad rates, and hand each to its window on the main queue (`sendEvent:`, as AppKit
//! dispatches one): the main thread sees them only when it gets to them.

use std::cell::{Cell, RefCell};
use std::collections::BTreeMap;
use std::sync::atomic::{AtomicBool, Ordering};

static ON: AtomicBool = AtomicBool::new(false);

/// The refresh period the slots are counted in (60 Hz panels; the link reports the real one).
const DEFAULT_PERIOD: f64 = 1.0 / 60.0;

pub fn init() {
    if std::env::var_os("ORB_FRAMES")
        .or_else(|| std::env::var_os("ORB_DISK_FRAMES"))
        .is_some_and(|v| v != "0")
    {
        ON.store(true, Ordering::Relaxed);
    }
}

#[inline]
pub fn on() -> bool {
    ON.load(Ordering::Relaxed)
}

pub fn set(on: bool) {
    ON.store(on, Ordering::Relaxed);
}

#[repr(C)]
struct Timebase {
    numer: u32,
    denom: u32,
}

extern "C" {
    fn mach_absolute_time() -> u64;
    fn mach_timebase_info(info: *mut Timebase) -> i32;
}

/// Seconds on the system's uptime clock (`mach_absolute_time`), as `NSEvent.timestamp`.
pub fn now() -> f64 {
    thread_local! {
        static SCALE: Cell<f64> = const { Cell::new(0.0) };
    }
    let s = SCALE.with(|c| {
        if c.get() == 0.0 {
            let mut tb = Timebase { numer: 0, denom: 0 };
            // SAFETY: fills a plain struct.
            unsafe { mach_timebase_info(&mut tb) };
            c.set(tb.numer as f64 / tb.denom.max(1) as f64 / 1e9);
        }
        c.get()
    });
    // SAFETY: reads the clock.
    (unsafe { mach_absolute_time() }) as f64 * s
}

/// One drawn frame of one window.
#[derive(Clone, Default)]
pub struct Frame {
    /// When the frame started (after the run loop woke for it).
    pub start: f64,
    /// When the layer's new contents were committed.
    pub end: f64,
    /// Milliseconds per stage, in the order the window names them.
    pub stages: Vec<(&'static str, f64)>,
    /// Input events folded into this frame.
    pub events: u32,
    /// The oldest of them: its system timestamp.
    pub first_event: Option<f64>,
    /// Milliseconds the input handlers spent on those events (state updates, hit tests).
    pub input_ms: f64,
    /// Only part of the window was drawn (a list scroll over the last frame).
    pub part: bool,
}

#[derive(Default)]
struct Pending {
    events: u32,
    first: Option<f64>,
    input_ms: f64,
}

/// A window's display link: its callbacks and the refreshes they drew for.
#[derive(Default)]
struct Ticks {
    ticks: u32,
    /// Callbacks that drew a frame.
    drew: u32,
    /// The vsync of each callback that drew: the refresh each frame was made for, so their
    /// spacing is the cadence on the glass, give or take the window server. (While a list
    /// scrolls the link pauses after each frame and the next event starts it again, so the
    /// runs are told apart by time, not by pauses.)
    drawn: Vec<f64>,
}

#[derive(Default)]
struct Log {
    frames: BTreeMap<&'static str, Vec<Frame>>,
    pending: BTreeMap<&'static str, Pending>,
    ticks: BTreeMap<&'static str, Ticks>,
    /// A vsync timestamp from the display link and its period: the slot grid.
    vsync: Option<(f64, f64)>,
    started: f64,
    cpu0: f64,
}

thread_local! {
    static LOG: RefCell<Log> = RefCell::new(Log::default());
}

/// An input event reached `win`'s handler: `stamp` is the event's own timestamp, `ms` what the
/// handler cost.
pub fn input(win: &'static str, stamp: f64, ms: f64) {
    if !on() {
        return;
    }
    LOG.with(|l| {
        let mut l = l.borrow_mut();
        let p = l.pending.entry(win).or_default();
        p.events += 1;
        p.input_ms += ms;
        if p.first.is_none() {
            p.first = Some(stamp);
        }
    });
}

/// A frame of `win` was committed.
pub fn frame(win: &'static str, mut f: Frame) {
    if !on() {
        return;
    }
    LOG.with(|l| {
        let mut l = l.borrow_mut();
        if let Some(p) = l.pending.remove(win) {
            f.events = p.events;
            f.first_event = p.first;
            f.input_ms = p.input_ms;
        }
        l.frames.entry(win).or_default().push(f);
    });
}

/// A display link callback for `win` at vsync `t` (period `period`), which drew a frame or not.
/// Input that came before a callback that drew nothing changed nothing on screen: its wait
/// is not counted as latency.
pub fn tick(win: &'static str, t: f64, period: f64, drew: bool) {
    if !on() {
        return;
    }
    vsync(t, period);
    LOG.with(|l| {
        let mut l = l.borrow_mut();
        let k = l.ticks.entry(win).or_default();
        k.ticks += 1;
        if drew {
            k.drew += 1;
            k.drawn.push(t);
        }
        if !drew {
            l.pending.remove(win);
        }
    });
}

/// Frames `win` has drawn since `reset`.
pub fn count(win: &'static str) -> usize {
    LOG.with(|l| l.borrow().frames.get(win).map_or(0, |f| f.len()))
}

/// The display's vsync: a timestamp and the refresh period, both seconds.
pub fn vsync(t: f64, period: f64) {
    LOG.with(|l| {
        let mut l = l.borrow_mut();
        if l.vsync.is_none() && period > 0.001 {
            l.vsync = Some((t, period));
        }
    });
}

pub fn has_vsync() -> bool {
    LOG.with(|l| l.borrow().vsync.is_some())
}

pub fn reset() {
    LOG.with(|l| {
        let mut l = l.borrow_mut();
        // The vsync grid is read again (from the next display link callback or probe): over
        // minutes, a period rounded to the microsecond drifts by more than a frame.
        *l = Log::default();
        l.started = now();
        l.cpu0 = cpu_secs();
    });
}

fn pct(v: &[f64], p: f64) -> f64 {
    if v.is_empty() {
        return 0.0;
    }
    let mut s = v.to_vec();
    s.sort_by(|a, b| a.total_cmp(b));
    let i = ((s.len() as f64 - 1.0) * p).round() as usize;
    s[i.min(s.len() - 1)]
}

fn max(v: &[f64]) -> f64 {
    v.iter().copied().fold(0.0, f64::max)
}

/// The summary lines, one per window that drew since `reset`.
pub fn report() -> Vec<String> {
    LOG.with(|l| {
        let l = l.borrow();
        let wall = now() - l.started;
        let cpu = cpu_secs() - l.cpu0;
        let mut out = vec![format!(
            "frames: {:.2} s, CPU {:.1}% of one core",
            wall,
            if wall > 0.0 { cpu / wall * 100.0 } else { 0.0 }
        )];
        let (t0, period) = l.vsync.unwrap_or((0.0, DEFAULT_PERIOD));
        for (win, fs) in &l.frames {
            if fs.is_empty() {
                continue;
            }
            let iv: Vec<f64> = fs
                .windows(2)
                .map(|w| (w[1].end - w[0].end) * 1000.0)
                .collect();
            // Intervals after the first frame (the first frame of a burst follows idle time).
            let busy: Vec<f64> = fs
                .windows(2)
                .filter(|w| w[1].first_event.is_some() || w[1].events == 0)
                .map(|w| (w[1].end - w[0].end) * 1000.0)
                .filter(|ms| *ms < 250.0)
                .collect();
            let work: Vec<f64> = fs.iter().map(|f| (f.end - f.start) * 1000.0).collect();
            let main: Vec<f64> = fs
                .iter()
                .map(|f| (f.end - f.start) * 1000.0 + f.input_ms)
                .collect();
            let lat: Vec<f64> = fs
                .iter()
                .filter_map(|f| f.first_event.map(|e| (f.end - e) * 1000.0))
                .collect();
            let events: u32 = fs.iter().map(|f| f.events).sum();
            let parts = fs.iter().filter(|f| f.part).count();
            // Slots: the vsync after each commit. A slot with two commits wasted one; a gap of
            // k slots between commits less than 250 ms apart (input kept coming) dropped k-1.
            let slot = |t: f64| ((t - t0) / period).ceil() as i64;
            let (mut dropped, mut doubled) = (0i64, 0i64);
            for w in fs.windows(2) {
                let (a, b) = (slot(w[0].end), slot(w[1].end));
                let gap = w[1].end - w[0].end;
                if b == a {
                    doubled += 1;
                } else if gap < 0.25 {
                    dropped += b - a - 1;
                }
            }
            let over = |ms: f64| busy.iter().filter(|v| **v > ms).count();
            out.push(format!(
                "frames {win}: {} frames ({parts} only the list), {events} events; interval p50 {:.2} p95 {:.2} max {:.2} ms (all: max {:.2}), >17.5 ms {}, >25 ms {}; dropped slots {dropped}, commits sharing a slot {doubled}; work p50 {:.2} p95 {:.2} max {:.2} ms; main thread per frame p95 {:.2} max {:.2} ms; input to commit p50 {:.2} p95 {:.2} ms",
                fs.len(),
                pct(&busy, 0.5),
                pct(&busy, 0.95),
                max(&busy),
                max(&iv),
                over(17.5),
                over(25.0),
                pct(&work, 0.5),
                pct(&work, 0.95),
                max(&work),
                pct(&main, 0.95),
                max(&main),
                pct(&lat, 0.5),
                pct(&lat, 0.95),
            ));
            // The stages, by name, p50 / p95 / max.
            let mut names: Vec<&'static str> = Vec::new();
            for f in fs {
                for (n, _) in &f.stages {
                    if !names.contains(n) {
                        names.push(n);
                    }
                }
            }
            let mut parts = Vec::new();
            for n in names {
                let v: Vec<f64> = fs
                    .iter()
                    .filter_map(|f| f.stages.iter().find(|s| s.0 == n).map(|s| s.1))
                    .collect();
                parts.push(format!(
                    "{n} {:.2}/{:.2}/{:.2}",
                    pct(&v, 0.5),
                    pct(&v, 0.95),
                    max(&v)
                ));
            }
            if let Some(k) = l.ticks.get(win) {
                // Between frames made for consecutive refreshes 16.7 ms; a refresh with no new
                // frame between two that came less than 100 ms apart adds 16.7 (a frame or
                // other main-thread work ran past a refresh, or nothing had changed).
                let gaps: Vec<f64> = k
                    .drawn
                    .windows(2)
                    .map(|w| (w[1] - w[0]) * 1000.0)
                    .filter(|g| *g < 100.0)
                    .collect();
                let p = if period > 0.0 { period * 1000.0 } else { 16.667 };
                let skipped: i64 = gaps.iter().map(|g| (g / p).round() as i64 - 1).sum();
                out.push(format!(
                    "frames {win} display link: {} callbacks ({} drew); frames by their vsync: p50 {:.2} p95 {:.2} max {:.2} ms apart, {skipped} refreshes without a new frame in between",
                    k.ticks,
                    k.drew,
                    pct(&gaps, 0.5),
                    pct(&gaps, 0.95),
                    max(&gaps)
                ));
            }
            let inp: Vec<f64> = fs
                .iter()
                .filter(|f| f.events > 0)
                .map(|f| f.input_ms / f.events as f64)
                .collect();
            out.push(format!(
                "frames {win} stages (ms p50/p95/max): {}; input handler per event {:.3}/{:.3}",
                parts.join(", "),
                pct(&inp, 0.5),
                pct(&inp, 0.95)
            ));
        }
        out
    })
}

fn cpu_secs() -> f64 {
    // SAFETY: getrusage fills a plain struct.
    let mut ru: libc::rusage = unsafe { std::mem::zeroed() };
    unsafe { libc::getrusage(libc::RUSAGE_SELF, &mut ru) };
    let t = |v: libc::timeval| v.tv_sec as f64 + v.tv_usec as f64 / 1e6;
    t(ru.ru_utime) + t(ru.ru_stime)
}

/// A stopwatch for a frame's stages.
pub struct Stopwatch {
    pub frame: Frame,
    at: f64,
}

impl Stopwatch {
    pub fn start() -> Stopwatch {
        let t = if on() { now() } else { 0.0 };
        Stopwatch {
            frame: Frame {
                start: t,
                ..Frame::default()
            },
            at: t,
        }
    }

    /// Ends the stage `name` now.
    pub fn lap(&mut self, name: &'static str) {
        if !on() {
            return;
        }
        let t = now();
        self.frame.stages.push((name, (t - self.at) * 1000.0));
        self.at = t;
    }

    /// The frame is committed: reports it for `win`.
    pub fn done(mut self, win: &'static str) {
        if !on() {
            return;
        }
        self.frame.end = now();
        frame(win, self.frame);
    }
}

// ---- the test drivers: synthetic input at trackpad rates ----

pub mod drive {
    use std::ffi::c_void;
    use std::time::{Duration, Instant};

    use objc2::rc::autoreleasepool;
    use objc2::runtime::AnyObject;
    use objc2::{class, msg_send};
    use objc2_core_foundation::CGPoint;
    use objc2_foundation::NSPoint;

    #[link(name = "CoreGraphics", kind = "framework")]
    extern "C" {
        fn CGEventCreateScrollWheelEvent2(
            source: *const c_void,
            units: u32,
            count: u32,
            w1: i32,
            w2: i32,
            w3: i32,
        ) -> *mut c_void;
        fn CGEventSetIntegerValueField(e: *mut c_void, field: u32, v: i64);
        fn CGEventSetDoubleValueField(e: *mut c_void, field: u32, v: f64);
        fn CGEventSetLocation(e: *mut c_void, p: CGPoint);
        fn CGEventSetTimestamp(e: *mut c_void, ns: u64);
    }
    #[link(name = "CoreFoundation", kind = "framework")]
    extern "C" {
        fn CFRelease(p: *const c_void);
    }

    const WINDOW: u32 = 51;
    const UNIT_PIXEL: u32 = 0;
    const UNIT_LINE: u32 = 1;
    const IS_CONTINUOUS: u32 = 88;
    const WINDOW_UNDER: u32 = 91;
    const WINDOW_UNDER_HANDLES: u32 = 92;
    const POINT_DELTA_1: u32 = 96;
    const FIXED_DELTA_1: u32 = 93;
    const SCROLL_PHASE: u32 = 99;

    /// Where to aim: the window's number, its frame in Cocoa screen coordinates (y up) and the
    /// primary screen's height (for Quartz's y-down global space).
    #[derive(Clone, Copy)]
    pub struct Target {
        pub window: isize,
        pub frame: (f64, f64, f64, f64),
        pub screen_h: f64,
    }

    #[derive(Clone, Copy)]
    pub enum Kind {
        /// Trackpad: `hz` events a second of small pixel deltas, the direction turning every
        /// 1.5 s, so a long list never hits its end.
        Trackpad { hz: f64, px: f64 },
        /// A mouse wheel: line notches at `hz`.
        Wheel { hz: f64 },
        /// The pointer moving up and down over the list at `hz`.
        Hover { hz: f64 },
        /// The window's bottom right edge dragged in and out by 120 px over 2 s, a step at
        /// `hz` (`setFrame:display:` on the main thread, as a live resize does per step).
        Resize { hz: f64 },
    }

    /// Posts `kind` for `secs` at window point (`x`, `y`) (logical, y down), from a thread.
    pub fn start(t: Target, kind: Kind, secs: f64, x: f64, y: f64) {
        std::thread::spawn(move || {
            let hz = match kind {
                Kind::Trackpad { hz, .. }
                | Kind::Wheel { hz }
                | Kind::Hover { hz }
                | Kind::Resize { hz } => hz,
            };
            let step = Duration::from_secs_f64(1.0 / hz);
            let t0 = Instant::now();
            let mut next = t0;
            let mut i: u64 = 0;
            while t0.elapsed().as_secs_f64() < secs {
                let el = t0.elapsed().as_secs_f64();
                let dir = if (el / 1.5) as i64 % 2 == 0 {
                    -1.0
                } else {
                    1.0
                };
                autoreleasepool(|_| match kind {
                    Kind::Trackpad { px, .. } => scroll(t, x, y, dir * px, true, i == 0),
                    Kind::Wheel { .. } => scroll(t, x, y, dir, false, false),
                    Kind::Hover { .. } => {
                        // 300 px up and down over 2 s.
                        let ph = (el % 2.0) / 2.0;
                        let dy = if ph < 0.5 { ph * 2.0 } else { 2.0 - ph * 2.0 } * 300.0;
                        mouse_move(t, x, y - 150.0 + dy);
                    }
                    Kind::Resize { .. } => {
                        let ph = (el % 2.0) / 2.0;
                        let d = if ph < 0.5 { ph * 2.0 } else { 2.0 - ph * 2.0 } * 120.0;
                        resize(t, d.round());
                    }
                });
                i += 1;
                next += step;
                let now = Instant::now();
                if next > now {
                    std::thread::sleep(next - now);
                }
            }
        });
    }

    extern "C" {
        static _dispatch_main_q: c_void;
        fn dispatch_async_f(q: *const c_void, ctx: *mut c_void, f: extern "C" fn(*mut c_void));
        fn objc_retain(o: *mut AnyObject) -> *mut AnyObject;
        fn objc_release(o: *mut AnyObject);
    }

    struct Post {
        window: isize,
        ev: *mut AnyObject,
    }

    extern "C" fn deliver(ctx: *mut c_void) {
        // SAFETY: made by `post` below, on the main queue now.
        let p = unsafe { Box::from_raw(ctx as *mut Post) };
        unsafe {
            let app: *mut AnyObject = msg_send![class!(NSApplication), sharedApplication];
            let w: *mut AnyObject = msg_send![app, windowWithWindowNumber: p.window];
            if !w.is_null() {
                let _: () = msg_send![w, sendEvent: p.ev];
            }
            objc_release(p.ev);
        }
    }

    /// Hands the event to its window on the main thread, as the event queue would; through the
    /// window so it arrives whether or not Orby is the active app (a test drive from ssh can
    /// not activate it, and `postEvent` drops a scroll aimed at an inactive app's window).
    fn post(window: isize, ev: *mut AnyObject) {
        if ev.is_null() {
            return;
        }
        let p = Box::new(Post {
            window,
            ev: unsafe { objc_retain(ev) },
        });
        unsafe {
            dispatch_async_f(
                &_dispatch_main_q as *const c_void,
                Box::into_raw(p) as *mut c_void,
                deliver,
            )
        };
    }

    struct Size {
        window: isize,
        frame: (f64, f64, f64, f64),
    }

    extern "C" fn set_size(ctx: *mut c_void) {
        // SAFETY: made by `resize`, on the main queue now.
        let p = unsafe { Box::from_raw(ctx as *mut Size) };
        unsafe {
            let app: *mut AnyObject = msg_send![class!(NSApplication), sharedApplication];
            let w: *mut AnyObject = msg_send![app, windowWithWindowNumber: p.window];
            if !w.is_null() {
                let (x, y, fw, fh) = p.frame;
                let r = objc2_foundation::NSRect::new(
                    NSPoint::new(x, y),
                    objc2_foundation::NSSize::new(fw, fh),
                );
                let _: () = msg_send![w, setFrame: r, display: true];
            }
        }
    }

    /// The window `d` px wider and taller than it started, its top left kept.
    fn resize(t: Target, d: f64) {
        let (x, y, w, h) = t.frame;
        let p = Box::new(Size {
            window: t.window,
            frame: (x, y - d, w + d, h + d),
        });
        unsafe {
            dispatch_async_f(
                &_dispatch_main_q as *const c_void,
                Box::into_raw(p) as *mut c_void,
                set_size,
            )
        };
    }

    fn scroll(t: Target, x: f64, y: f64, d: f64, precise: bool, first: bool) {
        unsafe {
            let (units, w1) = if precise {
                (UNIT_PIXEL, d.round() as i32)
            } else {
                (UNIT_LINE, d.signum() as i32)
            };
            let e = CGEventCreateScrollWheelEvent2(std::ptr::null(), units, 1, w1, 0, 0);
            if e.is_null() {
                return;
            }
            let gx = t.frame.0 + x;
            let gy = t.screen_h - (t.frame.1 + t.frame.3) + y;
            CGEventSetLocation(e, CGPoint { x: gx, y: gy });
            // Field 51 is the event's window (undocumented; 91 and 92 are not read for scrolls).
            CGEventSetIntegerValueField(e, WINDOW, t.window as i64);
            CGEventSetIntegerValueField(e, WINDOW_UNDER, t.window as i64);
            CGEventSetIntegerValueField(e, WINDOW_UNDER_HANDLES, t.window as i64);
            CGEventSetTimestamp(e, (super::now() * 1e9) as u64);
            // With a window set, AppKit reads the point in the window (top-left origin) from
            // the event, not the screen point: CoreGraphics' setter for it is not public.
            let set_wl = libc::dlsym(libc::RTLD_DEFAULT, c"CGEventSetWindowLocation".as_ptr());
            if !set_wl.is_null() {
                let f: extern "C" fn(*mut c_void, CGPoint) = std::mem::transmute(set_wl);
                f(e, CGPoint { x, y });
            }
            if precise {
                CGEventSetIntegerValueField(e, IS_CONTINUOUS, 1);
                CGEventSetIntegerValueField(e, POINT_DELTA_1, d.round() as i64);
                CGEventSetDoubleValueField(e, FIXED_DELTA_1, d);
                // Began, then Changed.
                CGEventSetIntegerValueField(e, SCROLL_PHASE, if first { 1 } else { 2 });
            }
            let ev: *mut AnyObject = msg_send![class!(NSEvent), eventWithCGEvent: e];
            post(t.window, ev);
            CFRelease(e as *const c_void);
        }
    }

    fn mouse_move(t: Target, x: f64, y: f64) {
        unsafe {
            let p = NSPoint::new(x, t.frame.3 - y);
            let ts: f64 = super::now();
            let ev: *mut AnyObject = msg_send![
                class!(NSEvent),
                mouseEventWithType: 5usize,
                location: p,
                modifierFlags: 0usize,
                timestamp: ts,
                windowNumber: t.window,
                context: std::ptr::null_mut::<AnyObject>(),
                eventNumber: 0isize,
                clickCount: 0isize,
                pressure: 0.0f32
            ];
            post(t.window, ev);
        }
    }
}

// ---- the vsync grid: one display link callback gives a vsync time and the period ----

use objc2::rc::Retained;
use objc2::runtime::{AnyObject, NSObject};
use objc2::{define_class, msg_send, sel, AllocAnyThread, DefinedClass};

pub struct ProbeIvars {
    link: Cell<*mut AnyObject>,
}

define_class!(
    #[unsafe(super(NSObject))]
    #[name = "OrbyVsyncProbe"]
    #[ivars = ProbeIvars]
    struct Probe;

    impl Probe {
        #[unsafe(method(tick:))]
        fn tick(&self, link: &AnyObject) {
            let t: f64 = unsafe { msg_send![link, timestamp] };
            let d: f64 = unsafe { msg_send![link, duration] };
            vsync(t, d);
            let l = self.ivars().link.get();
            if !l.is_null() {
                let _: () = unsafe { msg_send![l, invalidate] };
                self.ivars().link.set(std::ptr::null_mut());
            }
        }
    }
);

thread_local! {
    static PROBE: RefCell<Option<Retained<Probe>>> = const { RefCell::new(None) };
}

/// Reads one vsync from `view`'s display (macOS 14: `NSView.displayLink`), once.
pub fn probe_vsync(view: &objc2_app_kit::NSView) {
    if has_vsync() {
        return;
    }
    let p: Retained<Probe> = {
        let this = Probe::alloc().set_ivars(ProbeIvars {
            link: Cell::new(std::ptr::null_mut()),
        });
        unsafe { msg_send![super(this), init] }
    };
    unsafe {
        let link: *mut AnyObject =
            msg_send![view, displayLinkWithTarget: &*p, selector: sel!(tick:)];
        if link.is_null() {
            return;
        }
        let rl: *mut AnyObject = msg_send![objc2::class!(NSRunLoop), mainRunLoop];
        let mode: &objc2_foundation::NSString = objc2_foundation::NSRunLoopCommonModes;
        let _: () = msg_send![link, addToRunLoop: rl, forMode: mode];
        p.ivars().link.set(link);
    }
    PROBE.with(|c| *c.borrow_mut() = Some(p));
}
