//! The macOS half of `crate::sys`: what the shared units ask of the system, on AppKit and libc.
//! Timers are deadlines on the `App` (`App::timers`), fired by `App::tick` and waited for by
//! `App::schedule`, with Win32 SetTimer's meaning: they repeat every `ms` until disarmed.

use std::path::PathBuf;
use std::sync::OnceLock;
use std::time::{Duration, Instant};

use app_habits::Moment;
use objc2::msg_send;
use objc2::ClassType;
use objc2_app_kit::{NSEvent, NSEventModifierFlags, NSWorkspace};
use objc2_foundation::{NSString, NSURL};

use super::{clock, input};
use crate::sys::{vk, Timer};
use crate::App;

/// How often the away check looks for fresh input (macOS has no raw input to wait on).
const AWAY_POLL: Duration = Duration::from_secs(20);

// ---- timers ----

/// (Re)starts `t`: it fires `ms` from now, then every `ms`, until disarmed.
pub fn arm(app: &mut App, t: Timer, ms: u32) {
    let every = Duration::from_millis(ms.max(1) as u64);
    app.timers.insert(t, (Instant::now() + every, every));
}

pub fn disarm(app: &mut App, t: Timer) {
    app.timers.remove(&t);
}

// ---- clocks ----

/// The local wall clock, to the second.
pub fn now() -> Moment {
    clock::now()
}

/// Milliseconds into the current wall-clock second.
pub fn ms_into_second() -> u64 {
    clock::local().ms as u64
}

/// Milliseconds on a monotonic clock (since Orby started; only differences matter).
pub fn tick_ms() -> u64 {
    static START: OnceLock<Instant> = OnceLock::new();
    START.get_or_init(Instant::now).elapsed().as_millis() as u64
}

/// A local wall-clock moment as Unix seconds, by the zone's rule for that date (mktime: DST
/// included; a time skipped by a DST change moves forward, as on Windows).
pub fn unix_of_local(m: Moment) -> i64 {
    let d = m.date();
    let mins = m.minute_of_day();
    let secs = m.since(Moment::from_unix(0, 0)).as_secs().rem_euclid(60) as i32;
    // SAFETY: mktime reads and normalizes a plain struct.
    let mut tm: libc::tm = unsafe { std::mem::zeroed() };
    tm.tm_year = d.year() - 1900;
    tm.tm_mon = d.month() as i32 - 1;
    tm.tm_mday = d.day() as i32;
    tm.tm_hour = (mins / 60) as i32;
    tm.tm_min = (mins % 60) as i32;
    tm.tm_sec = secs;
    tm.tm_isdst = -1;
    let t = unsafe { libc::mktime(&mut tm) };
    if t == -1 {
        m.since(Moment::from_unix(0, 0)).as_secs() as i64
    } else {
        t as i64
    }
}

/// Unix seconds as a local wall-clock moment.
pub fn local_of_unix(unix_secs: i64) -> Moment {
    let t = unix_secs as libc::time_t;
    // SAFETY: localtime_r fills a plain struct from a plain number.
    let mut tm: libc::tm = unsafe { std::mem::zeroed() };
    if unsafe { libc::localtime_r(&t, &mut tm) }.is_null() {
        return Moment::from_unix(unix_secs, 0);
    }
    crate::sys::moment(
        tm.tm_year + 1900,
        (tm.tm_mon + 1) as u32,
        tm.tm_mday as u32,
        tm.tm_hour as u32,
        tm.tm_min as u32,
        tm.tm_sec as u32,
    )
}

// ---- the user ----

#[link(name = "CoreGraphics", kind = "framework")]
extern "C" {
    fn CGEventSourceSecondsSinceLastEventType(state: i32, event_type: u32) -> f64;
}

/// Milliseconds since the last key, click or mouse move anywhere (no permission needed).
pub fn idle_ms() -> u32 {
    const COMBINED_SESSION: i32 = 0; // kCGEventSourceStateCombinedSessionState
    const ANY_INPUT: u32 = !0; // kCGAnyInputEventType
                               // SAFETY: a plain query of the window server's input timestamps.
    let s = unsafe { CGEventSourceSecondsSinceLastEventType(COMBINED_SESSION, ANY_INPUT) };
    (s.max(0.0) * 1000.0).min(u32::MAX as f64) as u32
}

/// While away: a check every 20 s for fresh input (then `habits::on_input`); `false` stops it.
pub fn watch_input(app: &mut App, on: bool) {
    app.away_check_at = on.then(|| Instant::now() + AWAY_POLL);
}

/// The away check (`App::tick`): back once there was input since the last look.
pub fn on_away_check(app: &mut App) {
    app.away_check_at = None;
    if !app.habits.away {
        return;
    }
    if (idle_ms() as u64) < AWAY_POLL.as_millis() as u64 {
        crate::habits::on_input(app);
    } else {
        app.away_check_at = Some(Instant::now() + AWAY_POLL);
    }
}

fn flags() -> NSEventModifierFlags {
    // SAFETY: +[NSEvent modifierFlags] is a class method returning the flags now.
    unsafe { msg_send![NSEvent::class(), modifierFlags] }
}

/// Shift is held (the key event's, or the keyboard's now).
pub fn shift_down() -> bool {
    input::mods().shift || flags().contains(NSEventModifierFlags::Shift)
}

/// Ctrl as the Windows units mean it: Command on the Mac (Option for word moves comes through
/// `input::mods` while an InputView delivers).
pub fn ctrl_down() -> bool {
    input::mods().ctrl || flags().contains(NSEventModifierFlags::Command)
}

/// The longest gap between the two clicks of a double click (the system's setting).
pub fn double_click_ms() -> u128 {
    (NSEvent::doubleClickInterval() * 1000.0).max(0.0) as u128
}

/// The panel window has the keyboard (it is the key window).
pub fn panel_is_key(app: &App) -> bool {
    app.panel_win.is_key()
}

/// macOS key codes as the Windows virtual keys the shared units read.
pub fn vk_from_mac(code: u16) -> Option<u16> {
    Some(match code {
        53 => vk::VK_ESCAPE,
        36 | 76 => vk::VK_RETURN,
        48 => vk::VK_TAB,
        49 => vk::VK_SPACE,
        51 => vk::VK_BACK,
        117 => vk::VK_DELETE,
        123 => vk::VK_LEFT,
        124 => vk::VK_RIGHT,
        125 => vk::VK_DOWN,
        126 => vk::VK_UP,
        115 => vk::VK_HOME,
        119 => vk::VK_END,
        116 => vk::VK_PRIOR,
        121 => vk::VK_NEXT,
        120 => vk::VK_F2,
        109 => vk::VK_F10,
        110 => vk::VK_APPS, // the context-menu key on PC keyboards
        _ => return None,
    })
}

// ---- the system ----

/// The pasteboard's text.
pub fn read_clipboard(_app: &App) -> Option<String> {
    input::paste_text()
}

/// Puts text on the pasteboard.
pub fn write_clipboard(s: &str) {
    input::copy_text(s);
}

/// Wakes the main thread from a worker when a calendar read is done: `calWake:` on the menu
/// target (performSelectorOnMainThread), which calls `calendar::on_read_done`.
#[derive(Clone, Copy)]
pub struct CalWaker(usize);

// SAFETY: the pointer is only used for performSelectorOnMainThread, which may be sent from any
// thread, and the object it names lives as long as the app.
unsafe impl Send for CalWaker {}

impl CalWaker {
    pub fn wake(&self) {
        let obj = self.0 as *const objc2::runtime::AnyObject;
        // SAFETY: see above; the selector is defined on the target's class (mac/app.rs).
        unsafe {
            let obj: &objc2::runtime::AnyObject = &*obj;
            let _: () = msg_send![obj, performSelectorOnMainThread: objc2::sel!(calWake:), withObject: None::<&objc2::runtime::AnyObject>, waitUntilDone: false];
        }
    }
}

pub fn cal_waker(app: &App) -> CalWaker {
    CalWaker(app.menu_target_ptr())
}

/// Opens a link with its default app (NSWorkspace).
pub fn open_url(url: &str) {
    if let Some(u) = NSURL::URLWithString(&NSString::from_str(url)) {
        NSWorkspace::sharedWorkspace().openURL(&u);
    }
}

/// The system's short notification sound ("Glass").
pub fn play_notify_sound() {
    input::play_sound("Glass");
}

/// System Settings, Accessibility, Display, Reduce motion.
pub fn reduced_motion() -> bool {
    super::reduced_motion()
}

pub fn log(app: &mut App, line: &str) {
    app.log(line);
}

/// The run loop's one timer already follows `App::animating`.
pub fn start_anim(_app: &mut App) {}

/// `~/Library/Application Support/orb-proto` (or `ORB_PROTO_DATA`).
pub(crate) fn imp_data_dir() -> PathBuf {
    clock::data_dir()
}

// ---- full screen and presenting: not on the Mac yet ----

/// No foreground watch on the Mac yet (full screen is never seen; the orb stays).
pub fn watch_foreground(_app: &mut App) -> bool {
    false
}

pub fn foreground_seen() {}

/// No presentation mode on the Mac.
pub fn presenting() -> bool {
    false
}

/// Not detected on the Mac yet.
pub fn full_screen(_app: &App) -> bool {
    false
}
