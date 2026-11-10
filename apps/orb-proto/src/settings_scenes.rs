//! Fixed Settings and first-run screens for the snapshot mode: each section (General with the
//! shortcut being changed, taken and saved; Prayer near the top and scrolled, with Windows
//! location off; Notices with a muted source; Account; About), the city search (typed, a
//! selection, the whole list scrolled, no results) and every first-run step. Hand-made data, no
//! clock and no registry, so Windows and macOS draw the same views.

use app_core::prayer::{hhmm, Prayer};

use crate::notices_model::Source;
use crate::settings_model::{
    self as m, AtTimeSound, Capture, Locate, PrayerMethod, Section, Shortcut, Startup, Step,
    ThemeMode,
};
use crate::settings_panel::{self as sp, CityPick, Hit, Screen, SourceRow, View};
use crate::snapshot::Scene;
use crate::textfield::TextField;

pub struct Pick {
    pub field: TextField,
    pub rows: Vec<usize>,
    pub sel: usize,
    pub scroll: usize,
    pub current: Option<&'static str>,
}

/// One screen's data, owned, so a scene can build its `View` whenever it draws.
pub struct Case {
    pub name: &'static str,
    pub screen: Screen,
    pub prefs: m::Prefs,
    pub startup: Startup,
    pub fr_start: bool,
    pub capture: Option<Capture>,
    pub taken: Option<Shortcut>,
    pub hotkey_ok: bool,
    pub suggestion: Option<Shortcut>,
    pub tried: bool,
    pub want_windows: bool,
    pub locate: Locate,
    pub city: Option<String>,
    pub pick: Option<Pick>,
    pub muted: Option<(Source, &'static str)>,
    pub scroll: f32,
    pub hover: Option<Hit>,
    pub focus: Option<Hit>,
}

const SOURCES: [(Source, &str); 6] = [
    (
        Source::Calendar,
        "Event pills 10 min before and at the start",
    ),
    (Source::Tasks, "A pill when a task is due"),
    (Source::Drink, "\"Minum dulu\" through the day"),
    (Source::Focus, "Breaks and the end of a session"),
    (
        Source::Downloads,
        "Copied links, saved and failed downloads",
    ),
    (Source::Disk, "Low space and finished scans"),
];

const CTRL_ALT_K: Shortcut = Shortcut::ctrl_alt(0x4B);

impl Case {
    fn new(name: &'static str, screen: Screen) -> Case {
        Case {
            name,
            screen,
            prefs: m::Prefs {
                first_run_done: true,
                ..m::Prefs::default()
            },
            startup: Startup::On,
            fr_start: false,
            capture: None,
            taken: None,
            hotkey_ok: true,
            suggestion: None,
            tried: false,
            want_windows: false,
            locate: Locate::Idle,
            city: Some("Yogyakarta".into()),
            pick: None,
            muted: None,
            scroll: 0.0,
            hover: None,
            focus: None,
        }
    }

    pub fn view(&self) -> View<'_> {
        let (t, _) = crate::snapshot::times();
        let times = Prayer::ALL.map(|p| (p.name(), hhmm(t.of(p))));
        let sources = SOURCES
            .iter()
            .map(|(src, what)| {
                let muted = self.muted.filter(|(s, _)| s == src).map(|(_, w)| w);
                SourceRow {
                    source: *src,
                    on: muted.is_none(),
                    words: muted
                        .map(|w| format!("Muted {w}"))
                        .unwrap_or_else(|| (*what).to_string()),
                }
            })
            .collect();
        View {
            screen: self.screen,
            prefs: &self.prefs,
            windows_dark: false,
            startup: self.startup,
            fr_start: self.fr_start,
            capture: self.capture,
            taken: self.taken,
            hotkey_ok: self.hotkey_ok,
            suggestion: self.suggestion,
            tried: self.tried,
            want_windows: self.want_windows,
            locate: self.locate,
            city: self.city.clone(),
            picking: self.pick.as_ref().map(|p| CityPick {
                field: &p.field,
                rows: &p.rows,
                sel: p.sel,
                scroll: p.scroll,
                current: p.current,
            }),
            day: m::day_words(2, 10, 11),
            times,
            qibla: format!("295° WNW · {} km", m::thousands(8_210)),
            sources,
            data_dir: "C:\\Users\\you\\AppData\\Roaming\\Orby".into(),
            scroll: self.scroll,
            hover: self.hover,
            focus: self.focus,
        }
    }

    /// The window's edge radius, as settings.rs rounds it.
    pub fn edge_radius(&self) -> f32 {
        if matches!(self.screen, Screen::FirstRun(_)) {
            20.0
        } else {
            16.0
        }
    }
}

fn pick(text: &str, caret: Option<usize>, select_all: bool) -> Pick {
    let mut field = TextField::default();
    field.insert(text);
    if let Some(c) = caret {
        field.place(c, false);
    }
    if select_all {
        field.select_all();
    }
    Pick {
        rows: m::filter_cities(text),
        field,
        sel: 0,
        scroll: 0,
        current: Some("Yogyakarta"),
    }
}

pub fn cases() -> Vec<Case> {
    use Screen::{FirstRun, Settings};
    let general = Settings(Section::General);
    let prayer = Settings(Section::Prayer);
    let mut out = Vec::new();

    // ---- General ----
    out.push(Case::new("settings-general", general));
    let mut c = Case::new("settings-general-hover", general);
    c.hover = Some(Hit::Section(Section::Notices));
    c.startup = Startup::Off;
    c.prefs.show_orb_at_start = false;
    c.prefs.theme = ThemeMode::Dark;
    out.push(c);
    let mut c = Case::new("settings-general-hover-theme", general);
    c.hover = Some(Hit::Theme(ThemeMode::Light));
    out.push(c);
    let mut c = Case::new("settings-general-hover-change", general);
    c.hover = Some(Hit::ChangeShortcut);
    c.focus = Some(Hit::StartWithWindows);
    out.push(c);
    let mut c = Case::new("settings-general-listening", general);
    c.capture = Some(Capture {
        ctrl: true,
        alt: true,
        ..Capture::default()
    });
    out.push(c);
    let mut c = Case::new("settings-general-needs-modifier", general);
    c.capture = Some(Capture {
        needs_modifier: true,
        ..Capture::default()
    });
    c.hover = Some(Hit::CaptureCancel);
    out.push(c);
    let mut c = Case::new("settings-general-got", general);
    c.capture = Some(Capture {
        ctrl: true,
        alt: true,
        got: Some(CTRL_ALT_K),
        ..Capture::default()
    });
    c.hover = Some(Hit::CaptureSave);
    out.push(c);
    let mut c = Case::new("settings-general-taken", general);
    c.taken = Some(CTRL_ALT_K);
    c.startup = Startup::Blocked;
    c.hover = Some(Hit::PickAnother);
    out.push(c);
    let mut c = Case::new("settings-general-taken-scrolled", general);
    c.taken = Some(CTRL_ALT_K);
    c.startup = Startup::Blocked;
    c.scroll = 120.0;
    out.push(c);
    let mut c = Case::new("settings-general-held", general);
    c.hotkey_ok = false;
    out.push(c);
    let mut c = Case::new("settings-general-saved", general);
    c.prefs.shortcut = CTRL_ALT_K;
    out.push(c);

    // ---- Prayer ----
    let mut c = Case::new("settings-prayer", prayer);
    c.want_windows = true;
    c.locate = Locate::Found {
        lat: -7.7956,
        lng: 110.3695,
    };
    c.hover = Some(Hit::Method(PrayerMethod::Muhammadiyah));
    out.push(c);
    let mut c = Case::new("settings-prayer-location-off", prayer);
    c.want_windows = true;
    c.locate = Locate::Off;
    c.hover = Some(Hit::PlaceCity);
    out.push(c);
    let mut c = Case::new("settings-prayer-city", prayer);
    c.city = Some("Makassar".into());
    c.prefs.prayer_method = PrayerMethod::Muhammadiyah;
    c.hover = Some(Hit::PlaceWindows);
    out.push(c);
    let mut c = Case::new("settings-prayer-scrolled", prayer);
    c.prefs.prayer_adjust = [-m::ADJUST_MAX, 2, 0, -1, m::ADJUST_MAX];
    c.prefs.prayer_reminders = [true, false, true];
    c.hover = Some(Hit::Adjust(1, 1));
    c.scroll = 260.0;
    out.push(c);
    let mut c = Case::new("settings-prayer-adzan", prayer);
    c.prefs.at_time_sound = AtTimeSound::Adzan;
    c.prefs.adzan_file = Some("D:\\Audio\\adzan-makkah-full-recitation-2024.mp3".into());
    c.prefs.adzan_subuh_file = Some("D:\\Audio\\adzan-subuh.wav".into());
    c.hover = Some(Hit::AtTime(AtTimeSound::Silent));
    c.scroll = 640.0;
    out.push(c);

    // ---- the city search ----
    let mut c = Case::new("settings-city-typed", prayer);
    c.pick = Some(pick("ban", None, false));
    c.hover = Some(Hit::CityRow(c.pick.as_ref().unwrap().rows[1]));
    out.push(c);
    let mut c = Case::new("settings-city-caret", prayer);
    c.pick = Some(pick("Yogya", Some(2), false));
    out.push(c);
    let mut c = Case::new("settings-city-selection", prayer);
    c.pick = Some(pick("Bandung", None, true));
    out.push(c);
    let mut c = Case::new("settings-city-all", prayer);
    let mut p = pick("", None, false);
    p.sel = 5;
    p.scroll = 3;
    c.pick = Some(p);
    c.hover = Some(Hit::CityBack);
    out.push(c);
    let mut c = Case::new("settings-city-none", prayer);
    c.pick = Some(pick("Zzyzx", None, false));
    out.push(c);

    // ---- Notices, Account, About ----
    let mut c = Case::new("settings-notices", Settings(Section::Notices));
    c.muted = Some((Source::Drink, "until tomorrow"));
    c.hover = Some(Hit::Source(Source::Tasks));
    out.push(c);
    out.push(Case::new("settings-account", Settings(Section::Account)));
    let mut c = Case::new("settings-about", Settings(Section::About));
    c.hover = Some(Hit::OpenData);
    out.push(c);
    let mut c = Case::new("settings-about-focus", Settings(Section::About));
    c.focus = Some(Hit::Close);
    c.hover = Some(Hit::Close);
    out.push(c);

    // ---- the first run ----
    let fr = |name, step| {
        let mut c = Case::new(name, FirstRun(step));
        c.prefs.first_run_done = false;
        c
    };
    out.push(fr("firstrun-welcome", Step::Welcome));
    let mut c = fr("firstrun-welcome-hover", Step::Welcome);
    c.hover = Some(Hit::Next);
    out.push(c);
    let mut c = fr("firstrun-place-finding", Step::Place);
    c.want_windows = true;
    c.locate = Locate::Finding;
    out.push(c);
    let mut c = fr("firstrun-place-city", Step::Place);
    c.city = Some("Bandung".into());
    c.hover = Some(Hit::PlaceWindows);
    out.push(c);
    let mut c = fr("firstrun-place-failed", Step::Place);
    c.want_windows = true;
    c.locate = Locate::Failed;
    c.hover = Some(Hit::Back);
    out.push(c);
    let mut c = fr("firstrun-place-picking", Step::Place);
    c.pick = Some(pick("ma", None, false));
    out.push(c);
    let mut c = fr("firstrun-place-picking-none", Step::Place);
    c.pick = Some(pick("qqq", None, false));
    out.push(c);
    out.push(fr("firstrun-shortcut", Step::Shortcut));
    let mut c = fr("firstrun-shortcut-tried", Step::Shortcut);
    c.tried = true;
    c.fr_start = true;
    out.push(c);
    let mut c = fr("firstrun-shortcut-listening", Step::Shortcut);
    c.capture = Some(Capture {
        ctrl: true,
        ..Capture::default()
    });
    out.push(c);
    let mut c = fr("firstrun-shortcut-got", Step::Shortcut);
    c.capture = Some(Capture {
        got: Some(CTRL_ALT_K),
        ..Capture::default()
    });
    out.push(c);
    let mut c = fr("firstrun-shortcut-taken", Step::Shortcut);
    c.hotkey_ok = false;
    c.suggestion = Some(CTRL_ALT_K);
    c.hover = Some(Hit::UseSuggestion);
    out.push(c);
    let mut c = fr("firstrun-shortcut-taken-plain", Step::Shortcut);
    c.taken = Some(CTRL_ALT_K);
    out.push(c);
    out.push(fr("firstrun-sync", Step::Sync));
    let mut c = fr("firstrun-done", Step::Done);
    c.fr_start = true;
    out.push(c);
    let mut c = fr("firstrun-done-windows", Step::Done);
    c.want_windows = true;
    c.prefs.place = m::Place::Windows {
        lat: -6.2,
        lng: 106.8166,
    };
    c.prefs.shortcut = CTRL_ALT_K;
    out.push(c);
    out
}

pub fn scenes(_f: &crate::ui::Fonts) -> Vec<Scene> {
    cases()
        .into_iter()
        .map(|c| {
            let (w, h) = sp::size(c.screen);
            Scene::new(c.name, w, h, move |cv, t, f| {
                sp::draw(cv, t, f, &c.view());
                // The window's edge lip, as settings.rs draws it after the panel.
                crate::ui::lip(cv, t, 0.0, 0.0, w, h, c.edge_radius(), 1.0);
            })
        })
        .collect()
}
