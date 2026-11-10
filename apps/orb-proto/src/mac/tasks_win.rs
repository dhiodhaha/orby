//! The Tasks window on macOS: an ordinary titled, resizable NSWindow like the Disk window. It
//! draws its own 44 px band through the shared `tasks_window_panel::draw` (the traffic lights
//! are hidden); the band's empty part drags it, a double click on it zooms, its edges resize down
//! to Windows' minimum (`twp::W` x `twp::MIN_H`). Typing goes through an `InputView` (mac/input.rs)
//! that is first responder while the window is open, so keys, accents and input methods reach
//! the add field and the inline edit. Esc (as on Windows), Cmd+W and the close button close it;
//! Cmd+Q quits Orby.
//!
//! tasks_window.rs owns the state and what everything does; this file is the window, its input
//! and the Undo bar's deadline (`App::tick` fires it, `App::schedule` waits for it).

use std::cell::Cell;
use std::time::{Duration, Instant};

use super::app::with_app;
use super::cg::Bitmap;
use super::input::{Input, InputView};
use super::pacer::{Pacer, Who};
use crate::sys::{self, vk};
use crate::tasks_window::{self as tw};
use crate::tasks_window_panel::{self as twp};
use crate::App;
use objc2::rc::Retained;
use objc2::runtime::{NSObjectProtocol, ProtocolObject};
use objc2::{
    define_class, msg_send, AllocAnyThread, DefinedClass, MainThreadMarker, MainThreadOnly,
};
use objc2_app_kit::{
    NSApplication, NSBackingStoreType, NSColor, NSEvent, NSTrackingArea, NSTrackingAreaOptions,
    NSView, NSWindow, NSWindowButton, NSWindowStyleMask, NSWindowTitleVisibility,
};
use objc2_foundation::{
    NSNotification, NSNotificationCenter, NSOperationQueue, NSPoint, NSRect, NSSize, NSString,
};

/// The window on the Mac: the NSWindow, its view and input view, the Undo bar's deadline.
#[derive(Default)]
pub struct MacWin {
    window: Option<Retained<NSWindow>>,
    view: Option<Retained<TasksView>>,
    input: Option<Retained<InputView>>,
    observers: Vec<Retained<ProtocolObject<dyn NSObjectProtocol>>>,
    /// When the Undo bar goes (Windows' TIMER_UNDO).
    undo_at: Option<Instant>,
    /// The display link's frames, running while there is something to draw.
    pacer: Option<Pacer>,
    /// A redraw is due at the next display refresh.
    dirty: bool,
    /// The pointer moved since the last frame: what it is over is found at the frame.
    hover_due: bool,
}

pub struct TasksViewIvars {
    /// The press was on the band's empty part (a drag or a zoom), not a click in the window.
    band: Cell<bool>,
}

define_class!(
    #[unsafe(super(NSView))]
    #[thread_kind = MainThreadOnly]
    #[name = "OrbyTasksView"]
    #[ivars = TasksViewIvars]
    struct TasksView;

    impl TasksView {
        #[unsafe(method(isFlipped))]
        fn is_flipped(&self) -> bool {
            true
        }

        /// The input view keeps the keyboard: a click here must not take it.
        #[unsafe(method(acceptsFirstResponder))]
        fn accepts_first_responder(&self) -> bool {
            false
        }

        #[unsafe(method(acceptsFirstMouse:))]
        fn accepts_first_mouse(&self, _e: Option<&NSEvent>) -> bool {
            true
        }

        #[unsafe(method(mouseDown:))]
        fn mouse_down(&self, e: &NSEvent) {
            let (x, y) = self.point(e);
            // The band's empty part drags the window (a double click zooms it), as on Windows.
            let mut empty = false;
            with_app(|a| empty = y < twp::TOP && tw::hit(a, x, y).is_none());
            self.ivars().band.set(empty);
            if empty {
                if e.clickCount() == 2 {
                    with_app(toggle_max);
                } else if let Some(w) = self.window() {
                    w.performWindowDragWithEvent(e);
                }
                return;
            }
            with_app(|a| tw::press(a, x, y));
        }

        #[unsafe(method(mouseUp:))]
        fn mouse_up(&self, e: &NSEvent) {
            if self.ivars().band.replace(false) {
                return;
            }
            let (x, y) = self.point(e);
            with_app(|a| {
                let h = tw::hit(a, x, y);
                tw::click(a, h);
            });
        }

        #[unsafe(method(mouseMoved:))]
        fn mouse_moved(&self, e: &NSEvent) {
            let t = super::frames::now();
            let p = self.point(e);
            with_app(|a| on_move(a, p));
            super::frames::input("tasks", e.timestamp(), (super::frames::now() - t) * 1000.0);
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
            let dy = e.scrollingDeltaY();
            let precise = e.hasPreciseScrollingDeltas();
            if dy != 0.0 {
                let t = super::frames::now();
                with_app(|a| tw::on_wheel(a, super::panels::wheel_delta(dy, precise)));
                super::frames::input("tasks", e.timestamp(), (super::frames::now() - t) * 1000.0);
            }
        }
    }
);

impl TasksView {
    fn new(mtm: MainThreadMarker, w: f32, h: f32) -> Retained<TasksView> {
        let this = Self::alloc(mtm).set_ivars(TasksViewIvars {
            band: Cell::new(false),
        });
        let frame = NSRect::new(NSPoint::new(0.0, 0.0), NSSize::new(w as f64, h as f64));
        let view: Retained<TasksView> = unsafe { msg_send![super(this), initWithFrame: frame] };
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
        NSSize::new(twp::W as f64, twp::H as f64),
    );
    let style = NSWindowStyleMask::Titled
        | NSWindowStyleMask::Closable
        | NSWindowStyleMask::Miniaturizable
        | NSWindowStyleMask::Resizable
        | NSWindowStyleMask::FullSizeContentView;
    let w: Retained<NSWindow> = unsafe {
        msg_send![NSWindow::alloc(mtm), initWithContentRect: rect, styleMask: style, backing: NSBackingStoreType::Buffered, defer: false]
    };
    w.setTitle(&NSString::from_str("Orby tasks"));
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
    w.setContentMinSize(NSSize::new(twp::W as f64, twp::MIN_H as f64));
    w.setAcceptsMouseMovedEvents(true);
    let view = TasksView::new(mtm, twp::W, twp::H);
    w.setContentView(Some(&view));
    app.tasks.win.mac.pacer = Some(Pacer::new(&view, Who::Tasks));
    let input = InputView::new(mtm);
    view.addSubview(&input);
    let center = NSNotificationCenter::defaultCenter();
    let watch = |name: &str, f: fn(&mut App)| {
        let block = block2::RcBlock::new(move |_n: std::ptr::NonNull<NSNotification>| {
            with_app(f);
        });
        unsafe {
            center.addObserverForName_object_queue_usingBlock(
                Some(&NSString::from_str(name)),
                Some(&w),
                Some(&NSOperationQueue::mainQueue()),
                &block,
            )
        }
    };
    // A resize from outside (an edge, zoom): the list takes the new size. Becoming key or not:
    // the caret shows only while the window has the keyboard (Windows' WM_ACTIVATE).
    let obs = [
        watch("NSWindowDidResizeNotification", on_resized),
        watch("NSWindowDidBecomeKeyNotification", on_activation),
        watch("NSWindowDidResignKeyNotification", on_activation),
    ];
    app.tasks.win.mac.observers.extend(obs);
    app.tasks.win.mac.window = Some(w);
    app.tasks.win.mac.view = Some(view);
    app.tasks.win.mac.input = Some(input);
}

/// `twp::H` tall when the screen has room, less on a short one.
fn room(app: &App) -> f32 {
    let (vf, _) = app.screen();
    ((vf.size.height as f32) - 32.0).clamp(twp::MIN_H, twp::H)
}

/// The window's size from its view (`w`, `h`, zoomed).
fn sync_size(app: &mut App) {
    let (Some(w), Some(v)) = (&app.tasks.win.mac.window, &app.tasks.win.mac.view) else {
        return;
    };
    let b = v.bounds();
    let max = w.isZoomed();
    let tw = &mut app.tasks.win;
    tw.w = (b.size.width as f32).max(twp::W);
    tw.h = b.size.height as f32;
    tw.max = max;
}

/// The window and its view while it is open (the test drivers aim at them).
pub fn refs(app: &App) -> Option<(Retained<NSWindow>, Retained<NSView>)> {
    if !app.tasks.win.open {
        return None;
    }
    let m = &app.tasks.win.mac;
    Some((m.window.clone()?, Retained::into_super(m.view.clone()?)))
}

/// Shows the window (`tasks_window::show`): `again` brings an open one to the front; otherwise
/// it opens zoomed if it was closed so, else at its own size, centred on the orb's screen. Orby
/// activates (it stays an accessory app) and the window takes the keyboard.
pub fn show_window(app: &mut App, again: bool) {
    let Some(mtm) = MainThreadMarker::new() else {
        return;
    };
    if app.tasks.win.mac.window.is_none() {
        make(app, mtm);
    }
    app.theme = crate::ui::theme(super::app::dark_mode());
    if !again {
        let zoomed = app
            .tasks
            .win
            .mac
            .window
            .as_ref()
            .is_some_and(|w| w.isZoomed());
        if zoomed {
            sync_size(app);
        } else {
            app.tasks.win.w = twp::W;
            app.tasks.win.h = room(app);
            app.tasks.win.max = false;
            let (vf, _) = app.screen();
            let (lw, lh) = (app.tasks.win.w as f64, app.tasks.win.h as f64);
            let x = vf.origin.x + (vf.size.width - lw) / 2.0;
            let y = vf.origin.y + (vf.size.height - lh) / 2.0;
            if let Some(w) = &app.tasks.win.mac.window {
                w.setFrame_display(NSRect::new(NSPoint::new(x, y), NSSize::new(lw, lh)), false);
            }
            sync_size(app);
        }
        // Drawn before it comes on screen, so it never shows empty.
        draw_now(app);
    }
    if let Some(w) = app.tasks.win.mac.window.clone() {
        if w.isMiniaturized() {
            w.deminiaturize(None);
        }
        NSApplication::sharedApplication(mtm).activate();
        w.makeKeyAndOrderFront(None);
        if let Some(i) = &app.tasks.win.mac.input {
            w.makeFirstResponder(Some(i));
        }
        // The key notification arrives while the App is borrowed, so read the result.
        app.tasks.win.active = w.isKeyWindow();
        place_caret(app);
    }
    render(app);
}

pub fn close_window(app: &mut App) {
    let m = &mut app.tasks.win.mac;
    m.undo_at = None;
    m.dirty = false;
    m.hover_due = false;
    if let Some(p) = &mut m.pacer {
        p.pause();
    }
    if let Some(i) = &m.input {
        i.discard_marked();
    }
    if let Some(w) = &m.window {
        w.orderOut(None);
    }
    // The surfaces are a few MB each at 2x; let them go while hidden.
    if let Some(v) = &m.view {
        super::surface::release(v);
    }
}

pub fn toggle_max(app: &mut App) {
    if let Some(w) = &app.tasks.win.mac.window {
        w.zoom(None);
    }
}

pub fn arm_undo(app: &mut App, ms: u32) {
    app.tasks.win.mac.undo_at = Some(Instant::now() + Duration::from_millis(ms as u64));
}

pub fn disarm_undo(app: &mut App) {
    app.tasks.win.mac.undo_at = None;
}

/// The next deadline the run loop waits for (the Undo bar going).
pub(crate) fn deadline(app: &App) -> Option<Instant> {
    app.tasks.win.mac.undo_at.filter(|_| app.tasks.win.open)
}

/// One run-loop wake: the Undo bar's time (frames come from the display link).
pub(crate) fn tick(app: &mut App) {
    if !app.tasks.win.open {
        return;
    }
    if app
        .tasks
        .win
        .mac
        .undo_at
        .is_some_and(|t| Instant::now() >= t)
    {
        app.tasks.win.mac.undo_at = None;
        tw::on_undo_timer(app);
    }
}

/// One display refresh: the pointer since the last one, then at most one frame.
pub fn on_vsync(app: &mut App) {
    if !app.tasks.win.open {
        if let Some(p) = &mut app.tasks.win.mac.pacer {
            p.pause();
        }
        return;
    }
    if std::mem::take(&mut app.tasks.win.mac.hover_due) {
        let h = app.tasks.win.mouse.and_then(|(x, y)| tw::hit(app, x, y));
        if h != app.tasks.win.hover {
            app.tasks.win.hover = h;
            app.tasks.win.mac.dirty = true;
        }
    }
    if app.tasks.win.mac.dirty || app.tasks.win.ticking {
        draw_now(app);
    }
    let w = &mut app.tasks.win;
    if !w.mac.dirty && !w.ticking && !w.mac.hover_due {
        if let Some(p) = &mut w.mac.pacer {
            p.pause();
        }
    }
}

fn wake(app: &mut App) {
    if let Some(p) = &mut app.tasks.win.mac.pacer {
        p.run();
    }
}

fn on_resized(app: &mut App) {
    if !app.tasks.win.open {
        return;
    }
    sync_size(app);
    tw::after_resize(app);
    // Drawn now, in the same transaction as the new size (a live resize stays in step).
    draw_now(app);
}

fn on_activation(app: &mut App) {
    if !app.tasks.win.open {
        return;
    }
    let on = app
        .tasks
        .win
        .mac
        .window
        .as_ref()
        .is_some_and(|w| w.isKeyWindow());
    if on != app.tasks.win.active {
        app.tasks.win.active = on;
        render(app);
    }
}

/// The window is out of date (tasks_window.rs calls this for every change): drawn at the next
/// display refresh, so changes that come together cost one frame.
pub fn render(app: &mut App) {
    if !app.tasks.win.open {
        return;
    }
    app.tasks.win.mac.dirty = true;
    wake(app);
}

/// Draws the window now, at its view's size.
fn draw_now(app: &mut App) {
    if !app.tasks.win.open {
        return;
    }
    let (Some(w), Some(view)) = (
        app.tasks.win.mac.window.clone(),
        app.tasks.win.mac.view.clone(),
    ) else {
        return;
    };
    let mut sw = super::frames::Stopwatch::start();
    let b = view.bounds();
    let (cw, ch) = (b.size.width as f32, b.size.height as f32);
    let scale = w.backingScaleFactor() as f32;
    let space = w.colorSpace().and_then(|s| s.CGColorSpace());
    let v = tw::view(app);
    sw.lap("view");
    let hover = app.tasks.win.hover;
    super::surface::paint(
        &view,
        cw,
        ch,
        scale,
        space.as_deref(),
        Some(&mut sw),
        |cv| twp::draw(cv, &app.theme, &app.fonts, &v, hover),
    );
    sw.done("tasks");
    app.tasks.win.mac.dirty = false;
    // Frames only while a ripple or a rolling number moves.
    app.tasks.win.ticking = crate::motion::active();
}

fn on_move(app: &mut App, p: (f32, f32)) {
    if !app.tasks.win.open {
        return;
    }
    app.tasks.win.mouse = Some(p);
    app.tasks.win.mac.hover_due = true;
    wake(app);
}

fn on_leave(app: &mut App) {
    app.tasks.win.mouse = None;
    if app.tasks.win.hover.take().is_some() {
        render(app);
    }
}

/// Where an input method's candidate window opens: under the add field (its line in
/// tasks_window_panel.rs: the 720 px column's text, 100 px down from the window's top).
fn place_caret(app: &App) {
    let Some(w) = &app.tasks.win.mac.window else {
        return;
    };
    let f = w.frame();
    let col = (app.tasks.win.w as f64).min(720.0);
    let x = f.origin.x + (f.size.width - col) / 2.0 + 58.0;
    let top = f.origin.y + f.size.height;
    super::input::set_caret_rect(NSRect::new(
        NSPoint::new(x, top - 96.0),
        NSSize::new(1.0, 18.0),
    ));
}

// ---- the keyboard ----

/// The window has the keyboard: it is key, or it is open and no typing panel is.
pub(crate) fn has_keyboard(app: &App) -> bool {
    if !app.tasks.win.open {
        return false;
    }
    let key = app
        .tasks
        .win
        .mac
        .window
        .as_ref()
        .is_some_and(|w| w.isKeyWindow());
    key || !(app.panel_open && app.panel_kind == crate::panels::Kind::Tasks)
}

/// What the window's input view delivers, as Windows' WM_KEYDOWN, WM_SYSKEYDOWN and WM_CHAR.
pub(crate) fn on_input(app: &mut App, i: Input) {
    if !app.tasks.win.open {
        return;
    }
    match i {
        Input::Text(s) => super::panels::type_text(app, &s, tw::on_keydown, tw::on_char),
        Input::Marked(s) => {
            tw::focus_field_for_typing(app);
            if let Some(f) = tw::field_mut(app) {
                f.compose(&s);
            }
            place_caret(app);
            render(app);
        }
        Input::Key { vk: k, mods } if mods.cmd => match k {
            // Copy and cut: the Mac's own (Windows' fields have neither).
            0x43 | 0x58 => {
                let sel = tw::field_mut(app)
                    .map(|f| f.selected_text().to_owned())
                    .unwrap_or_default();
                if !sel.is_empty() {
                    sys::write_clipboard(&sel);
                    if k == 0x58 {
                        if let Some(f) = tw::field_mut(app) {
                            f.insert("");
                        }
                        render(app);
                    }
                }
            }
            0x57 => tw::close(app),
            0x51 => {
                app.log("quit (Cmd+Q in the Tasks window)");
                std::process::exit(0);
            }
            _ => {
                tw::on_keydown(app, k);
            }
        },
        // Option+Up and Option+Down move the focused task (Windows' Alt+Up and Alt+Down).
        Input::Key { vk: k, mods } if mods.alt && (k == vk::VK_UP || k == vk::VK_DOWN) => {
            app.tasks.win.kb = true;
            tw::move_task(app, k == vk::VK_UP);
        }
        Input::Key { vk: k, .. } => {
            tw::on_keydown(app, k);
        }
    }
}

/// Test channel `tasks-shot:<png>`: the window's view drawn to a PNG at 2x.
pub(crate) fn shot(app: &mut App, path: &str) {
    if !app.tasks.win.open {
        app.log("tasks-shot: the Tasks window is not open");
        return;
    }
    let v = tw::view(app);
    let (w, h) = (app.tasks.win.w, app.tasks.win.h);
    let b = Bitmap::new(w, h, 2.0);
    twp::draw(
        &mut b.canvas(),
        &app.theme,
        &app.fonts,
        &v,
        app.tasks.win.hover,
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
    }
    app.log(&format!("tasks-shot: {w:.0}x{h:.0} -> {path}"));
}
