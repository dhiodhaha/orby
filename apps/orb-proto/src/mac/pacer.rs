//! Frames on the display's own clock: one `CADisplayLink` per window view (`NSView.displayLink`,
//! macOS 14), running only while the window has something to draw.
//!
//! Input never draws. A scroll, a pointer move or a data change marks the window and wakes its
//! link; the link calls back once per refresh of the screen the window is on, and the window
//! draws at most once there, whatever arrived since (a trackpad sends 120 events a second, a
//! 60 Hz panel shows 60 frames). With nothing left to draw the link pauses, so an open window
//! that is still costs nothing. On macOS 13 (no `NSView.displayLink`) a repeating 60 Hz timer
//! stands in, started and stopped the same way. Both run in the common modes, so frames keep
//! coming during a live resize or while a menu is open.

use std::cell::Cell;

use objc2::rc::Retained;
use objc2::runtime::{AnyObject, NSObject};
use objc2::{define_class, msg_send, sel, AllocAnyThread, DefinedClass};
use objc2_app_kit::NSView;
use objc2_foundation::{NSRunLoop, NSRunLoopCommonModes, NSTimer};

use super::app::with_app;

/// Which window a link belongs to.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Who {
    Disk,
    Tasks,
    Settings,
}

impl Who {
    /// The window's name in the frame log.
    fn name(self) -> &'static str {
        match self {
            Who::Disk => "disk",
            Who::Tasks => "tasks",
            Who::Settings => "settings",
        }
    }
}

pub struct TargetIvars {
    who: Cell<Who>,
}

define_class!(
    #[unsafe(super(NSObject))]
    #[name = "OrbyPacerTarget"]
    #[ivars = TargetIvars]
    struct Target;

    impl Target {
        #[unsafe(method(step:))]
        fn step(&self, sender: &AnyObject) {
            let who = self.ivars().who.get();
            let name = who.name();
            let before = super::frames::count(name);
            with_app(|a| match who {
                Who::Disk => super::disk_win::on_vsync(a),
                Who::Tasks => super::tasks_win::on_vsync(a),
                Who::Settings => super::settings_win::on_vsync(a),
            });
            if super::frames::on() {
                // A CADisplayLink (or, on macOS 13, the timer: no vsync to read).
                let link: bool = unsafe { msg_send![sender, respondsToSelector: sel!(targetTimestamp)] };
                let (t, d) = if link {
                    unsafe { (msg_send![sender, timestamp], msg_send![sender, duration]) }
                } else {
                    (super::frames::now(), 0.0)
                };
                super::frames::tick(name, t, d, super::frames::count(name) > before);
            }
        }
    }
);

pub struct Pacer {
    target: Retained<Target>,
    view: Retained<NSView>,
    /// The CADisplayLink (retained), or null until the first `run` (and on macOS 13).
    link: *mut AnyObject,
    /// macOS 13: the stand-in timer while running.
    timer: Option<Retained<NSTimer>>,
    running: bool,
}

impl Pacer {
    /// A paused pacer for `view`'s window.
    pub fn new(view: &NSView, who: Who) -> Pacer {
        let target: Retained<Target> = {
            let this = Target::alloc().set_ivars(TargetIvars {
                who: Cell::new(who),
            });
            unsafe { msg_send![super(this), init] }
        };
        Pacer {
            target,
            view: objc2::rc::Retained::from(view),
            link: std::ptr::null_mut(),
            timer: None,
            running: false,
        }
    }

    /// The view's display link, made at the first `run` (the window is on screen by then), in
    /// the common modes (the constant: CFRunLoop tells them by the pointer).
    fn link(&mut self) -> *mut AnyObject {
        if self.link.is_null() {
            let has: bool = unsafe {
                msg_send![&*self.view, respondsToSelector: sel!(displayLinkWithTarget:selector:)]
            };
            if has {
                unsafe {
                    let l = self
                        .view
                        .displayLinkWithTarget_selector(&self.target, sel!(step:));
                    l.addToRunLoop_forMode(&NSRunLoop::mainRunLoop(), NSRunLoopCommonModes);
                    self.link = Retained::into_raw(Retained::into_super(Retained::into_super(l)));
                }
            }
        }
        self.link
    }

    /// Frames come from the next refresh on.
    pub fn run(&mut self) {
        if self.running {
            return;
        }
        self.running = true;
        let link = self.link();
        if !link.is_null() {
            unsafe {
                let _: () = msg_send![link, setPaused: false];
            }
            return;
        }
        let t = unsafe {
            NSTimer::timerWithTimeInterval_target_selector_userInfo_repeats(
                1.0 / 60.0,
                &self.target,
                sel!(step:),
                None,
                true,
            )
        };
        unsafe { NSRunLoop::mainRunLoop().addTimer_forMode(&t, NSRunLoopCommonModes) };
        self.timer = Some(t);
    }

    /// No more frames until `run`.
    pub fn pause(&mut self) {
        if !self.running {
            return;
        }
        self.running = false;
        if !self.link.is_null() {
            unsafe {
                let _: () = msg_send![self.link, setPaused: true];
            }
        }
        if let Some(t) = self.timer.take() {
            t.invalidate();
        }
    }

    pub fn running(&self) -> bool {
        self.running
    }

    /// Whether frames come from the display (false: the macOS 13 timer).
    pub fn on_display(&self) -> bool {
        !self.link.is_null()
    }
}

impl Drop for Pacer {
    fn drop(&mut self) {
        self.pause();
        if !self.link.is_null() {
            unsafe {
                let _: () = msg_send![self.link, invalidate];
                let _: () = msg_send![self.link, release];
            }
        }
    }
}
