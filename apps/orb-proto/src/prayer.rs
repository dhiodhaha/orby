//! Prayer times, reminders and the Prayer panel window (drawing is in panel.rs).

use std::time::Instant;

use windows::Win32::Foundation::*;
use windows::Win32::Graphics::Direct2D::ID2D1DeviceContext;
use windows::Win32::System::SystemInformation::GetLocalTime;
use windows::Win32::UI::WindowsAndMessaging::*;

use app_core::prayer::{self as pr, Location, Method, Prayer, Times};
use app_core::qibla;

use crate::notices::{self, Source};
use crate::orb::set_tucked;
use crate::panels::{self, Kind};
use crate::pill::info_pill;
use crate::settings_model::Section;
use crate::ui::{Icon, Pill};
use crate::{habits, log_line, orb, panel, settings, start_anim, tasks, App, TIMER_REMIND};

/// The place in Settings: a city, or the last Windows location fix.
fn coords(app: &App) -> (f64, f64) {
    app.settings.prefs.place.coords()
}

/// A place in the PC's own time zone: every time in the app is compared with the local clock.
fn location((lat, lng): (f64, f64)) -> Location {
    let tz = tasks::clock().offset_secs as f64 / 3600.0;
    Location {
        lat,
        lng,
        tz,
        elevation_m: 0.0,
    }
}

pub fn now_minutes() -> f32 {
    let st = unsafe { GetLocalTime() };
    st.wHour as f32 * 60.0 + st.wMinute as f32 + st.wSecond as f32 / 60.0
}

pub fn today() -> (i32, u32, u32) {
    let st = unsafe { GetLocalTime() };
    (st.wYear as i32, st.wMonth as u32, st.wDay as u32)
}

/// Tomorrow's date, for Subuh after Isya.
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

pub fn compute_times(at: (f64, f64), day: (i32, u32, u32)) -> (Times, u32) {
    let loc = location(at);
    let t = pr::times(loc, Method::KEMENAG, day.0, day.1, day.2);
    let n = tomorrow(day);
    let t2 = pr::times(loc, Method::KEMENAG, n.0, n.1, n.2);
    (t, t2.subuh)
}

/// The place changed in Settings: today's times, the reminders, Today's markers and the sun.
pub fn apply_place(app: &mut App) {
    let d = today();
    let (t, ts) = compute_times(coords(app), d);
    app.times = t;
    app.tomorrow_subuh = ts;
    app.times_day = d;
    log_times(app);
    schedule_reminders(app);
    tasks::schedule(app);
    let (a, b) = orb::sun_from_clock(&app.times);
    app.sun_angle = a;
    app.brightness = b;
    orb::render_orb(app);
    panels::redraw(app);
}

/// The next prayer, its time (minutes after midnight) and the minutes until it.
pub fn next_prayer(app: &App) -> (Prayer, u32, f64) {
    pr::next(&app.times, now_minutes() as f64, app.tomorrow_subuh)
}

/// Tray item: the 5 s prayer moment, the drop flows and the urgent pill shows.
pub fn play_prayer(app: &mut App) {
    set_tucked(app, false);
    app.flow = Some(Instant::now());
    start_anim(app);
    let (n, _, _) = next_prayer(app);
    let p = Pill {
        icon: Icon::Bell,
        dot: true,
        title: format!("Memasuki waktu {}", n.name()),
        value: String::new(),
        action: None,
        closable: true,
        ttl_ms: 6000,
    };
    notices::post(app, notices::urgent(Source::Prayer, p));
    log_line(app, "prayer moment");
}

// ---- reminders: one exact timer to the next event, nothing polls ----

/// Upcoming reminder events as (minutes after today's midnight, prayer, offset in minutes).
fn reminder_events(app: &App) -> Vec<(f64, Prayer, i32)> {
    let mut v = Vec::new();
    let offsets: [(usize, i32); 3] = [(0, -30), (1, -10), (2, 0)];
    for p in Prayer::ALL {
        for (k, off) in offsets {
            if app.settings.prefs.prayer_reminders[k] {
                v.push((app.times.of(p) as f64 + off as f64, p, off));
            }
        }
    }
    for (k, off) in offsets {
        if app.settings.prefs.prayer_reminders[k] {
            v.push((
                1440.0 + app.tomorrow_subuh as f64 + off as f64,
                Prayer::Subuh,
                off,
            ));
        }
    }
    v.sort_by(|a, b| a.0.partial_cmp(&b.0).unwrap());
    v
}

pub fn schedule_reminders(app: &mut App) {
    let now = now_minutes() as f64;
    unsafe {
        let _ = KillTimer(Some(app.hwnd), TIMER_REMIND);
    }
    app.next_reminder = None;
    if let Some((at, p, off)) = reminder_events(app)
        .into_iter()
        .find(|e| e.0 > now + 1.0 / 60.0)
    {
        let ms = ((at - now) * 60_000.0).max(500.0).min(u32::MAX as f64) as u32;
        app.next_reminder = Some((p, off));
        unsafe {
            SetTimer(Some(app.hwnd), TIMER_REMIND, ms, None);
        }
        log_line(
            app,
            &format!(
                "next reminder: {} {:+} min, in {:.1} min",
                p.name(),
                off,
                at - now
            ),
        );
    }
}

pub fn fire_reminder(app: &mut App, p: Prayer, off: i32) {
    let at = pr::hhmm(app.times.of(p));
    if off < 0 {
        // Normal: a bell, the time, no action, gone after 6 s. It waits for a focus break.
        let pill = Pill {
            icon: Icon::Bell,
            dot: false,
            title: format!("{} menit lagi {}", -off, p.name()),
            value: at,
            action: None,
            closable: true,
            ttl_ms: 6000,
        };
        if !notices::hidden_by_you(app) {
            let due = prayer_moment(app, p);
            notices::post(app, notices::prayer_before(p.name(), due, pill));
        }
    } else {
        // Urgent: shows even when the orb is hidden (the queue shows it, then hides it again),
        // and the drop flows once.
        let pill = Pill {
            icon: Icon::Bell,
            dot: true,
            title: format!("Memasuki waktu {}", p.name()),
            value: String::new(),
            action: None,
            closable: true,
            ttl_ms: 10_000,
        };
        notices::drop_prayer(app, p.name());
        notices::post(app, notices::urgent(Source::Prayer, pill));
        set_tucked(app, false);
        app.flow = Some(Instant::now());
        start_anim(app);
        habits::on_prayer_time(app, p.name());
    }
    log_line(app, &format!("reminder {} {:+}", p.name(), off));
}

/// When `p` next comes: today's time, or tomorrow's once today's has passed (Subuh after Isya).
fn prayer_moment(app: &App, p: Prayer) -> app_habits::Moment {
    let now = habits::now();
    let today = now.date().at_minute(app.times.of(p));
    if today >= now {
        return today;
    }
    let minute = if p == Prayer::Subuh {
        app.tomorrow_subuh
    } else {
        app.times.of(p)
    };
    now.date().next().at_minute(minute)
}

/// Tray test items: fire the next prayer's reminder at `off` minutes now.
pub fn test_reminder(app: &mut App, off: i32) {
    let (p, _, _) = next_prayer(app);
    fire_reminder(app, p, off);
}

pub fn on_remind_timer(app: &mut App, hwnd: HWND) {
    unsafe {
        let _ = KillTimer(Some(hwnd), TIMER_REMIND);
    }
    if let Some((p, off)) = app.next_reminder.take() {
        fire_reminder(app, p, off);
    }
    schedule_reminders(app);
}

/// Recomputes the times after midnight.
pub fn refresh_day(app: &mut App) {
    let d = today();
    if d != app.times_day {
        let (t, ts) = compute_times(coords(app), d);
        app.times = t;
        app.tomorrow_subuh = ts;
        app.times_day = d;
        schedule_reminders(app);
    }
}

pub fn log_times(app: &mut App) {
    let (day, t) = (app.times_day, app.times);
    log_line(app, &format!("times {}-{:02}-{:02}: imsak {} subuh {} terbit {} dzuhur {} ashar {} maghrib {} isya {}", day.0, day.1, day.2, pr::hhmm(t.imsak), pr::hhmm(t.subuh), pr::hhmm(t.terbit), pr::hhmm(t.dzuhur), pr::hhmm(t.ashar), pr::hhmm(t.maghrib), pr::hhmm(t.isya)));
}

// ---- the prayer panel (the window is panels.rs, the drawing panel.rs) ----

pub fn draw_panel(app: &App, dc: &ID2D1DeviceContext, hover: Option<panel::Hit>) {
    let (np, _, mins) = next_prayer(app);
    let (lat, lng) = coords(app);
    let b = qibla::bearing(lat, lng);
    let place = app.settings.prefs.place.short();
    let view = panel::View {
        times: &app.times,
        now_min: now_minutes() as f64,
        next: np,
        mins_left: mins,
        place: &place,
        qibla: format!("{:.0}° {}", b, qibla::compass(b)),
        reminders: app.settings.prefs.prayer_reminders,
        hover,
    };
    panel::draw(&app.g, dc, &app.theme, &app.fonts, &view);
}

pub fn open_panel(app: &mut App) {
    panels::open_panel(app, Kind::Prayer);
}

pub fn panel_click(app: &mut App, hit: panel::Hit) {
    match hit {
        panel::Hit::Chip(i) => {
            settings::toggle_reminder(app, i);
        }
        panel::Hit::Button(2) => {
            let (lat, lng) = coords(app);
            let b = qibla::bearing(lat, lng);
            let d = qibla::distance_km(lat, lng);
            panels::close_panel(app);
            let p = info_pill(
                Icon::Prayer,
                &format!("Qibla {:.0}° {}", b, qibla::compass(b)),
                &format!("{:.0} km to the Kaaba", d),
            );
            notices::reply(app, p);
        }
        panel::Hit::Button(_) => {
            panels::close_panel(app);
            settings::open(app, Section::Prayer);
        }
    }
}
#[cfg(test)]
mod tests {
    use super::tomorrow;

    #[test]
    fn tomorrow_rolls_over_month_year_and_leap_day() {
        assert_eq!(tomorrow((2026, 11, 6)), (2026, 11, 7));
        assert_eq!(tomorrow((2026, 11, 30)), (2026, 12, 1));
        assert_eq!(tomorrow((2026, 12, 31)), (2027, 1, 1));
        assert_eq!(tomorrow((2028, 2, 28)), (2028, 2, 29));
        assert_eq!(tomorrow((2027, 2, 28)), (2027, 3, 1));
        assert_eq!(tomorrow((2100, 2, 28)), (2100, 3, 1));
    }
}
