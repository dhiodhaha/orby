//! The Windows window modules the shared units call (`crate::pill`, `crate::orb`, `crate::dial`,
//! `crate::prayer`, `crate::settings`, `crate::log_line`, `crate::start_anim`), with the same
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
    use app_core::prayer::{self as pr, Prayer};
    use app_core::qibla;

    use crate::canvas::Canvas;
    use crate::mac::clock;
    use crate::panels::{self, Kind};
    use crate::ui::{info_pill, Icon};
    use crate::{notices, panel, App};

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
        let (lat, lng) = app.prefs.place.coords();
        let b = qibla::bearing(lat, lng);
        Data {
            next,
            mins_left,
            place: app.prefs.place.short(),
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
            method: app.prefs.prayer_method.short(),
            qibla: d.qibla.clone(),
            reminders: app.prefs.prayer_reminders,
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

    pub fn panel_click(app: &mut App, hit: panel::Hit) {
        match hit {
            panel::Hit::Chip(i) => {
                app.prefs.prayer_reminders[i] = !app.prefs.prayer_reminders[i];
                let on = app.prefs.prayer_reminders;
                app.save_setting(
                    "prayer_reminders",
                    serde_json::to_value(on).unwrap_or_default(),
                );
                app.schedule_reminders();
                panels::refresh(app);
            }
            panel::Hit::Button(i) => {
                let what = ["Reminder settings", "Place", "Qibla"][i.min(2)];
                notices::reply(app, info_pill(Icon::Info, "Coming to Mac", what));
            }
        }
    }
}

/// Settings (Windows: settings.rs). Only what notices.rs calls until settings.rs is shared; the
/// settings stage deletes this module and its line in main.rs.
pub mod settings {
    use crate::App;

    /// A mute changed: an open Settings window would show it (there is none on the Mac yet).
    pub fn on_sources_changed(_app: &mut App) {}
}
