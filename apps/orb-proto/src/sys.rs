//! What the shared units (notices.rs, habits.rs, history.rs, store.rs, and next tasks.rs,
//! calendar.rs, settings.rs) ask of the system they run on. They call only these (plus the
//! window modules `crate::panels`, `crate::pill`, `crate::orb`, `crate::dial`, `crate::prayer`,
//! which macOS provides as shims over its `App`), so the same logic runs on Windows (here, over
//! Win32 and the existing window modules) and on macOS (`mac::sys`).
//!
//! Each function is small and says what it does on both systems. The rules a unit can count on:
//! - Timers behave like Win32 `SetTimer`: `arm` (re)starts a repeating timer, it fires every
//!   `ms` until `disarm`; arming again replaces the interval. Each `Timer` has one handler
//!   (Windows: `orb_proc`/`panel_proc`'s WM_TIMER; macOS: `App::fire_timer`).
//! - Keys are Windows virtual-key numbers on both systems (`vk::*`).
//! - Times: `now()` is the local wall clock to the second, `tick_ms()` a monotonic count.

use std::path::PathBuf;

use app_habits::Moment;

#[cfg(target_os = "macos")]
pub(crate) use crate::mac::sys::*;

#[cfg(windows)]
pub(crate) use imp::*;

/// The named timers of the shared units. Windows: one SetTimer id each, on the orb window
/// (the panel's on the panel window). macOS: a deadline each on the `App`.
#[derive(Clone, Copy, PartialEq, Eq, Hash, Debug)]
#[allow(dead_code)] // Tasks, CalCue and CalRead are armed once tasks.rs and calendar.rs use sys.
pub enum Timer {
    /// tasks.rs: the next due pill or overdue change.
    Tasks,
    /// habits.rs: the focus ring's next degree, or the phase's end.
    Focus,
    /// habits.rs: the next drink reminder.
    Drink,
    /// calendar.rs: the next event pill.
    CalCue,
    /// calendar.rs: the 10 min calendar read.
    CalRead,
    /// notices.rs: the queue's next change.
    Notices,
    /// The open panel's tick (the countdowns, Today's minute, History's midnight).
    Panel,
}

/// Windows virtual-key numbers, the keys the shared units read (macOS maps its key codes to
/// these, see `mac::sys::vk_from_mac`).
#[allow(dead_code)] // the full set the units and the next stages read
pub mod vk {
    pub const VK_BACK: u16 = 0x08;
    pub const VK_TAB: u16 = 0x09;
    pub const VK_RETURN: u16 = 0x0D;
    pub const VK_SHIFT: u16 = 0x10;
    pub const VK_CONTROL: u16 = 0x11;
    pub const VK_ESCAPE: u16 = 0x1B;
    pub const VK_SPACE: u16 = 0x20;
    pub const VK_PRIOR: u16 = 0x21;
    pub const VK_NEXT: u16 = 0x22;
    pub const VK_END: u16 = 0x23;
    pub const VK_HOME: u16 = 0x24;
    pub const VK_LEFT: u16 = 0x25;
    pub const VK_UP: u16 = 0x26;
    pub const VK_RIGHT: u16 = 0x27;
    pub const VK_DOWN: u16 = 0x28;
    pub const VK_DELETE: u16 = 0x2E;
    pub const VK_APPS: u16 = 0x5D;
    pub const VK_F2: u16 = 0x71;
    pub const VK_F10: u16 = 0x79;
}

/// Local wall-clock seconds as a `Moment` from a date and a time of day (shared by both `now`s).
pub(crate) fn moment(y: i32, mo: u32, d: u32, h: u32, mi: u32, s: u32) -> Moment {
    app_habits::Date::new(y, mo, d)
        .map(|d| d.at(h, mi) + std::time::Duration::from_secs(s as u64))
        .unwrap_or(Moment::from_unix(0, 0))
}

/// Unix milliseconds now (the store's stamps).
#[allow(dead_code)] // tasks.rs reads it once shared
pub fn unix_ms() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map_or(0, |d| d.as_millis() as u64)
}

/// Where the JSON files live, created on first use (`ORB_PROTO_DATA` overrides on both).
pub fn data_dir() -> PathBuf {
    imp_data_dir()
}

#[cfg(windows)]
mod imp {
    use std::path::PathBuf;
    use std::sync::atomic::{AtomicBool, Ordering};

    use app_habits::{Date, Moment};
    use windows::core::{w, HSTRING, PCWSTR};
    use windows::Win32::Foundation::*;
    use windows::Win32::Graphics::Gdi::{
        GetMonitorInfoW, MonitorFromWindow, MONITORINFO, MONITOR_DEFAULTTONEAREST,
    };
    use windows::Win32::Media::Audio::{PlaySoundW, SND_ALIAS, SND_ASYNC, SND_NODEFAULT};
    use windows::Win32::System::SystemInformation::{GetLocalTime, GetTickCount, GetTickCount64};
    use windows::Win32::System::Time::{
        SystemTimeToTzSpecificLocalTime, TzSpecificLocalTimeToSystemTime,
    };
    use windows::Win32::UI::Accessibility::{SetWinEventHook, HWINEVENTHOOK};
    use windows::Win32::UI::Input::KeyboardAndMouse::{
        GetKeyState, GetLastInputInfo, LASTINPUTINFO, VK_CONTROL, VK_SHIFT,
    };
    use windows::Win32::UI::Input::{
        RegisterRawInputDevices, RAWINPUTDEVICE, RIDEV_INPUTSINK, RIDEV_REMOVE,
    };
    use windows::Win32::UI::Shell::{
        SHQueryUserNotificationState, ShellExecuteW, QUNS_PRESENTATION_MODE,
    };
    use windows::Win32::UI::WindowsAndMessaging::*;

    use super::Timer;
    use crate::App;

    // ---- timers ----

    fn timer_target(app: &App, t: Timer) -> (HWND, usize) {
        match t {
            Timer::Tasks => (app.hwnd, crate::TIMER_TASKS),
            Timer::Focus => (app.hwnd, crate::TIMER_FOCUS),
            Timer::Drink => (app.hwnd, crate::TIMER_DRINK),
            Timer::CalCue => (app.hwnd, crate::TIMER_CAL_CUE),
            Timer::CalRead => (app.hwnd, crate::TIMER_CAL_READ),
            Timer::Notices => (app.hwnd, crate::TIMER_NOTICES),
            Timer::Panel => (app.panel_hwnd, crate::TIMER_PANEL),
        }
    }

    /// SetTimer on the timer's window: fires every `ms` until disarmed.
    pub fn arm(app: &mut App, t: Timer, ms: u32) {
        let (hwnd, id) = timer_target(app, t);
        unsafe {
            SetTimer(Some(hwnd), id, ms, None);
        }
    }

    /// KillTimer.
    pub fn disarm(app: &mut App, t: Timer) {
        let (hwnd, id) = timer_target(app, t);
        unsafe {
            let _ = KillTimer(Some(hwnd), id);
        }
    }

    // ---- clocks ----

    /// GetLocalTime, to the second.
    pub fn now() -> Moment {
        let st = unsafe { GetLocalTime() };
        super::moment(
            st.wYear as i32,
            st.wMonth as u32,
            st.wDay as u32,
            st.wHour as u32,
            st.wMinute as u32,
            st.wSecond as u32,
        )
    }

    /// Milliseconds into the current wall-clock second.
    pub fn ms_into_second() -> u64 {
        unsafe { GetLocalTime() }.wMilliseconds as u64
    }

    /// GetTickCount64: milliseconds since boot.
    pub fn tick_ms() -> u64 {
        unsafe { GetTickCount64() }
    }

    fn epoch_secs(m: Moment) -> i64 {
        let e = Moment::from_unix(0, 0);
        if m >= e {
            m.since(e).as_secs() as i64
        } else {
            -(e.since(m).as_secs() as i64)
        }
    }

    fn to_systemtime(m: Moment) -> SYSTEMTIME {
        let (d, s) = (m.date(), epoch_secs(m).rem_euclid(60));
        let t = m.minute_of_day();
        SYSTEMTIME {
            wYear: d.year() as u16,
            wMonth: d.month() as u16,
            wDayOfWeek: 0,
            wDay: d.day() as u16,
            wHour: (t / 60) as u16,
            wMinute: (t % 60) as u16,
            wSecond: s as u16,
            wMilliseconds: 0,
        }
    }

    fn from_systemtime(st: &SYSTEMTIME) -> Option<Moment> {
        let d = Date::new(st.wYear as i32, st.wMonth as u32, st.wDay as u32)?;
        Some(
            d.at(st.wHour as u32, st.wMinute as u32)
                + std::time::Duration::from_secs(st.wSecond as u64),
        )
    }

    /// A local wall-clock moment as Unix seconds, by the zone's rule for that date (DST too).
    pub fn unix_of_local(m: Moment) -> i64 {
        let mut utc = SYSTEMTIME::default();
        match unsafe { TzSpecificLocalTimeToSystemTime(None, &to_systemtime(m), &mut utc) }
            .ok()
            .and_then(|_| from_systemtime(&utc))
        {
            Some(u) => epoch_secs(u),
            None => epoch_secs(m),
        }
    }

    /// Unix seconds as a local wall-clock moment.
    pub fn local_of_unix(unix_secs: i64) -> Moment {
        let u = Moment::from_unix(unix_secs, 0);
        let mut lt = SYSTEMTIME::default();
        unsafe { SystemTimeToTzSpecificLocalTime(None, &to_systemtime(u), &mut lt) }
            .ok()
            .and_then(|_| from_systemtime(&lt))
            .unwrap_or(u)
    }

    // ---- the user ----

    /// Milliseconds since the last key or mouse input (GetLastInputInfo).
    pub fn idle_ms() -> u32 {
        let mut lii = LASTINPUTINFO {
            cbSize: std::mem::size_of::<LASTINPUTINFO>() as u32,
            dwTime: 0,
        };
        unsafe {
            if !GetLastInputInfo(&mut lii).as_bool() {
                return 0;
            }
            GetTickCount().wrapping_sub(lii.dwTime)
        }
    }

    /// While away: raw input from the mouse and keyboard reaches the orb window (WM_INPUT, then
    /// `habits::on_input`); `false` stops it.
    pub fn watch_input(app: &mut App, on: bool) {
        let devices = [(1u16, 2u16), (1, 6)].map(|(page, usage)| RAWINPUTDEVICE {
            usUsagePage: page,
            usUsage: usage,
            dwFlags: if on { RIDEV_INPUTSINK } else { RIDEV_REMOVE },
            hwndTarget: if on { app.hwnd } else { HWND::default() },
        });
        unsafe {
            let _ = RegisterRawInputDevices(&devices, std::mem::size_of::<RAWINPUTDEVICE>() as u32);
        }
    }

    /// Shift is held (GetKeyState).
    pub fn shift_down() -> bool {
        unsafe { GetKeyState(VK_SHIFT.0 as i32) < 0 }
    }

    /// Ctrl is held (GetKeyState).
    #[allow(dead_code)] // tasks.rs and settings.rs read it once shared
    pub fn ctrl_down() -> bool {
        unsafe { GetKeyState(VK_CONTROL.0 as i32) < 0 }
    }

    // ---- the system ----

    /// The clipboard's text (CF_UNICODETEXT), up to 4096 characters.
    #[allow(dead_code)] // tasks.rs and calendar.rs read it once shared
    pub fn read_clipboard(app: &App) -> Option<String> {
        crate::tasks::read_clipboard(app.hwnd)
    }

    /// Opens a link or file with its default app (ShellExecuteW "open").
    #[allow(dead_code)] // calendar.rs and settings.rs open links once shared
    pub fn open_url(url: &str) {
        unsafe {
            ShellExecuteW(
                None,
                w!("open"),
                &HSTRING::from(url),
                PCWSTR::null(),
                PCWSTR::null(),
                SW_SHOWNORMAL,
            );
        }
    }

    /// The system's short notification sound (Windows' "Notification.Default", never bundled).
    #[allow(dead_code)] // prayer's Bunyi plays it once prayer reminders are shared
    pub fn play_notify_sound() {
        unsafe {
            let _ = PlaySoundW(
                w!("Notification.Default"),
                None,
                SND_ALIAS | SND_ASYNC | SND_NODEFAULT,
            );
        }
    }

    /// Animation effects off (Settings, Accessibility).
    #[allow(dead_code)] // the units ask once they animate on their own
    pub fn reduced_motion() -> bool {
        crate::downloads::reduced_motion()
    }

    /// One line in the log.
    #[allow(dead_code)] // the units call crate::log_line; this is for new code
    pub fn log(app: &mut App, line: &str) {
        crate::log_line(app, line);
    }

    /// Frames keep coming until nothing moves.
    #[allow(dead_code)] // the units start motion once they animate on their own
    pub fn start_anim(app: &mut App) {
        crate::start_anim(app);
    }

    /// `%APPDATA%\orb-proto` (or `ORB_PROTO_DATA`), created on first use.
    pub(super) fn imp_data_dir() -> PathBuf {
        let dir = match std::env::var_os("ORB_PROTO_DATA") {
            Some(d) => PathBuf::from(d),
            None => std::env::var_os("APPDATA")
                .map(PathBuf::from)
                .unwrap_or_else(crate::exe_dir)
                .join("orb-proto"),
        };
        let _ = std::fs::create_dir_all(&dir);
        dir
    }

    // ---- full screen and presenting (notices.rs) ----

    static FG_PENDING: AtomicBool = AtomicBool::new(false);
    static mut FG_HWND: HWND = HWND(std::ptr::null_mut());

    unsafe extern "system" fn win_event(
        _: HWINEVENTHOOK,
        event: u32,
        hwnd: HWND,
        id_object: i32,
        id_child: i32,
        _: u32,
        _: u32,
    ) {
        let relevant = event == EVENT_SYSTEM_FOREGROUND
            || (id_object == OBJID_WINDOW.0 && id_child == 0 && hwnd == GetForegroundWindow());
        // Many location changes in a row become one message.
        if relevant && !FG_PENDING.swap(true, Ordering::AcqRel) {
            let _ = PostMessageW(
                Some(FG_HWND),
                crate::WM_APP_FOREGROUND,
                WPARAM(0),
                LPARAM(0),
            );
        }
    }

    /// Watches foreground changes, and the foreground window's size (a video going full screen
    /// without a change of window). Both arrive as one coalesced WM_APP_FOREGROUND, which calls
    /// `notices::on_foreground`. Returns whether the watch is on.
    pub fn watch_foreground(app: &mut App) -> bool {
        unsafe {
            FG_HWND = app.hwnd;
            let flags = WINEVENT_OUTOFCONTEXT | WINEVENT_SKIPOWNPROCESS;
            let a = SetWinEventHook(
                EVENT_SYSTEM_FOREGROUND,
                EVENT_SYSTEM_FOREGROUND,
                None,
                Some(win_event),
                0,
                0,
                flags,
            );
            let b = SetWinEventHook(
                EVENT_OBJECT_LOCATIONCHANGE,
                EVENT_OBJECT_LOCATIONCHANGE,
                None,
                Some(win_event),
                0,
                0,
                flags,
            );
            !a.is_invalid() && !b.is_invalid()
        }
    }

    /// The foreground message was handled: the next change posts again.
    pub fn foreground_seen() {
        FG_PENDING.store(false, Ordering::Release);
    }

    fn class_of(h: HWND) -> String {
        let mut buf = [0u16; 64];
        let n = unsafe { GetClassNameW(h, &mut buf) };
        String::from_utf16_lossy(&buf[..n.max(0) as usize])
    }

    /// Windows presentation mode (PowerPoint's slide show, "presentation settings").
    pub fn presenting() -> bool {
        unsafe { matches!(SHQueryUserNotificationState(), Ok(s) if s == QUNS_PRESENTATION_MODE) }
    }

    /// Presentation mode, or a full-screen foreground window on the orb's own monitor. A
    /// maximized window with a title bar is not full screen.
    pub fn full_screen(app: &App) -> bool {
        unsafe {
            if presenting() {
                return true;
            }
            let fg = GetForegroundWindow();
            if fg.is_invalid()
                || [app.hwnd, app.pill_hwnd, app.dial_hwnd, app.panel_hwnd].contains(&fg)
            {
                return false;
            }
            if matches!(
                class_of(fg).as_str(),
                "Progman" | "WorkerW" | "Shell_TrayWnd" | "Shell_SecondaryTrayWnd"
            ) {
                return false;
            }
            let mon = MonitorFromWindow(fg, MONITOR_DEFAULTTONEAREST);
            if mon != MonitorFromWindow(app.hwnd, MONITOR_DEFAULTTONEAREST) {
                return false;
            }
            let mut mi = MONITORINFO {
                cbSize: std::mem::size_of::<MONITORINFO>() as u32,
                ..Default::default()
            };
            let _ = GetMonitorInfoW(mon, &mut mi);
            let mut r = RECT::default();
            let _ = GetWindowRect(fg, &mut r);
            let m = mi.rcMonitor;
            let covers =
                r.left <= m.left && r.top <= m.top && r.right >= m.right && r.bottom >= m.bottom;
            let style = GetWindowLongW(fg, GWL_STYLE) as u32;
            let titled = IsZoomed(fg).as_bool() && style & WS_CAPTION.0 == WS_CAPTION.0;
            covers && !titled
        }
    }
}
