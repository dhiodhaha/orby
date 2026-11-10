//! The Windows window modules the shared units call (`crate::pill`, `crate::orb`, `crate::dial`,
//! `crate::prayer`, `crate::log_line`, `crate::start_anim`), with the same
//! paths and signatures, over the Mac `App`. Only what the shared units call is here; main.rs
//! puts these modules at the crate root on macOS. The panel host is `mac::panels`.

use crate::App;

/// One line in the log (Windows: `log_line` in app_windows.rs).
pub fn log_line(app: &mut App, msg: &str) {
    app.log(msg);
}

/// Frames keep coming while something moves: on the Mac the run loop's one timer already
/// follows `App::animating`, so there is nothing to start.
pub fn start_anim(_app: &mut App) {}

/// The notification pill (Windows: pill.rs). The Mac shows one pill, no stack behind it yet.
pub mod pill {
    use crate::ui::Pill;
    use crate::App;

    pub use crate::ui::info_pill;

    /// A pill grows from the drop (or takes the words of the one on screen).
    pub fn show_pill(app: &mut App, p: Pill) {
        app.show_pill(p);
    }

    /// New words for the pill on screen; the reveal carries on where it is.
    pub fn update_pill(app: &mut App, p: Pill) {
        app.show_pill(p);
    }

    /// The pill leaves (and the notice queue hears of it).
    pub fn dismiss_pill(app: &mut App) {
        app.dismiss_pill();
    }

    /// The urgent pill's attention shake: not on the Mac yet (the UI stage).
    pub fn shake(app: &mut App) {
        app.log("notices: urgent (no shake on the Mac yet)");
    }

    /// The count behind the front pill changed: the Mac has no stack peeks yet.
    pub fn restack(_app: &mut App) {}

    /// The pill window floats above the panel's level already.
    pub fn raise(_app: &App) {}
}

/// The drop (Windows: orb.rs).
pub mod orb {
    use crate::App;

    /// Redraws the drop. The focus ring and the notices badge come with the UI stage: read
    /// `habits::ring(app)` and `notices::badge_count(app)` in `App::render_orb`.
    pub fn render_orb(app: &mut App) {
        app.render_orb();
    }

    pub fn set_visible(app: &mut App, v: bool) {
        app.set_visible(v);
    }

    pub fn set_tucked(app: &mut App, tucked: bool) {
        app.set_tucked(tucked);
    }

    /// A pick's pulse through the metal: not on the Mac yet.
    pub fn kick(_app: &mut App, _strength: f32) {}
}

/// The dial (Windows: dial.rs). What it shows and does is shared in dial_menu.rs.
pub mod dial {
    use crate::App;

    pub fn close_dial(app: &mut App) {
        app.close_dial();
    }

    pub fn open_dial(app: &mut App, keyboard: bool) {
        app.open_dial(keyboard);
    }
}

/// Prayer times and the Prayer panel's data (Windows: prayer.rs; its reminders and sounds stay
/// in mac/app.rs until prayer.rs is shared).
pub mod prayer {
    use app_core::prayer::{self as pr, Location, Prayer, Times};
    use app_core::qibla;

    use crate::canvas::Canvas;
    use crate::mac::clock;
    use crate::panels::{self, Kind};
    use crate::settings_model::{Prefs, Section};
    use crate::ui::{info_pill, Icon};
    use crate::{notices, panel, settings, App};

    /// Today's date on the local clock.
    pub fn today() -> (i32, u32, u32) {
        clock::today()
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

    /// Today's times and tomorrow's Subuh at a place, by the method in Settings with your minute
    /// adjustments, on this Mac's own UTC offset.
    pub fn compute_times(prefs: &Prefs, at: (f64, f64), day: (i32, u32, u32)) -> (Times, u32) {
        let loc = Location {
            lat: at.0,
            lng: at.1,
            tz: clock::local().offset as f64 / 3600.0,
            elevation_m: 0.0,
        };
        let m = prefs.prayer_method.method();
        let adj = prefs.prayer_adjust;
        let t = pr::times(loc, m, day.0, day.1, day.2).adjusted(adj);
        let n = tomorrow(day);
        (t, pr::times(loc, m, n.0, n.1, n.2).adjusted(adj).subuh)
    }

    /// The place, the method or an adjustment changed in Settings: today's times, the
    /// reminders, the drop's sun and the open panel.
    pub fn apply_place(app: &mut App) {
        app.recompute_times();
        app.schedule_reminders();
        panels::redraw(app);
    }

    /// The reminders changed: the next one is found again.
    pub fn schedule_reminders(app: &mut App) {
        app.schedule_reminders();
    }

    /// No adzan plays on the Mac yet.
    pub fn stop_adzan(_app: &mut App) {}

    /// The next prayer, its time (minutes after midnight) and the minutes until it.
    pub fn next_prayer(app: &App) -> (Prayer, u32, f64) {
        pr::next(&app.times, clock::now_minutes(), app.tomorrow_subuh)
    }

    /// Today's times, when the day turned over (and the next reminder with them).
    pub fn refresh_day(app: &mut App) {
        let before = app.times_day;
        app.refresh_day();
        if app.times_day != before {
            app.schedule_reminders();
        }
    }

    /// Closing the adzan's pill stops the adzan: the Mac plays no adzan yet.
    pub fn on_pill_closed_by_you(_app: &mut App) {}

    pub fn open_panel(app: &mut App) {
        panels::open_panel(app, Kind::Prayer);
    }

    /// What the panel shows besides the times: the next prayer, minutes left, place, qibla.
    struct Data {
        next: Prayer,
        mins_left: f64,
        place: String,
        qibla: String,
    }

    fn data(app: &App) -> Data {
        let (next, _, mins_left) = next_prayer(app);
        let (lat, lng) = app.settings.prefs.place.coords();
        let b = qibla::bearing(lat, lng);
        Data {
            next,
            mins_left,
            place: app.settings.prefs.place.short(),
            qibla: format!("{:.0}° {}", b, qibla::compass(b)),
        }
    }

    fn view<'a>(
        app: &'a App,
        d: &'a Data,
        hover: Option<panel::Hit>,
        apart: bool,
    ) -> panel::View<'a> {
        panel::View {
            times: &app.times,
            now_min: clock::now_minutes(),
            next: d.next,
            mins_left: d.mins_left,
            place: &d.place,
            method: app.settings.prefs.prayer_method.short(),
            qibla: d.qibla.clone(),
            reminders: app.settings.prefs.prayer_reminders,
            hover,
            countdown_apart: apart,
        }
    }

    /// The whole panel; `apart`: without the countdown, which has its own layer on screen.
    pub fn draw_panel(app: &App, cv: &mut dyn Canvas, hover: Option<panel::Hit>, apart: bool) {
        let d = data(app);
        panel::draw(cv, &app.theme, &app.fonts, &view(app, &d, hover, apart));
    }

    /// The countdown's digits as shown ("1:23:45"): its layer redraws only when they change.
    pub fn countdown_text(app: &App) -> String {
        let d = data(app);
        panel::countdown(&view(app, &d, None, true))
    }

    /// The countdown's own layer.
    pub fn draw_countdown(app: &App, cv: &mut dyn Canvas) {
        let d = data(app);
        panel::draw_countdown(cv, &app.theme, &app.fonts, &view(app, &d, None, true));
    }

    /// As Windows' prayer.rs: a chip flips the reminder (shared with Settings), Qibla says the
    /// bearing in a pill, Reminders and Place open Settings on Prayer.
    pub fn panel_click(app: &mut App, hit: panel::Hit) {
        match hit {
            panel::Hit::Chip(i) => settings::toggle_reminder(app, i),
            panel::Hit::Button(2) => {
                let (lat, lng) = app.settings.prefs.place.coords();
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
}
