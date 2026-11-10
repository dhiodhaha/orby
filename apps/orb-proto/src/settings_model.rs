//! Settings and the first run as plain values: what is kept in `settings.json`, the shortcut and
//! its capture, the place for prayer times (Windows location or a city), the first run's steps
//! and the words the Settings window shows. No OS calls; settings.rs does those.

use serde::de::DeserializeOwned;
use serde::{Deserialize, Serialize};
use serde_json::{Map, Value};

// ---- theme ----

#[derive(Clone, Copy, PartialEq, Eq, Debug, Default, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ThemeMode {
    #[default]
    Windows,
    Light,
    Dark,
}

impl ThemeMode {
    pub const ALL: [ThemeMode; 3] = [ThemeMode::Windows, ThemeMode::Light, ThemeMode::Dark];

    pub fn name(self) -> &'static str {
        match self {
            ThemeMode::Windows => "Windows",
            ThemeMode::Light => "Light",
            ThemeMode::Dark => "Dark",
        }
    }

    /// Whether the app draws dark, given what Windows uses for apps.
    pub fn is_dark(self, windows_dark: bool) -> bool {
        match self {
            ThemeMode::Windows => windows_dark,
            ThemeMode::Light => false,
            ThemeMode::Dark => true,
        }
    }

    /// The Theme row's second line.
    pub fn words(self, windows_dark: bool) -> String {
        match self {
            ThemeMode::Windows => format!(
                "Follows Windows. {} right now.",
                if windows_dark { "Dark" } else { "Light" }
            ),
            ThemeMode::Light => "Always light".into(),
            ThemeMode::Dark => "Always dark".into(),
        }
    }
}

// ---- the shortcut ----

pub const VK_SPACE: u32 = 0x20;
const VK_ESCAPE: u32 = 0x1B;

/// One global shortcut: modifiers plus one key (a Windows virtual-key code).
#[derive(Clone, Copy, PartialEq, Eq, Debug, Serialize, Deserialize)]
#[serde(into = "String", try_from = "String")]
pub struct Shortcut {
    pub ctrl: bool,
    pub alt: bool,
    pub shift: bool,
    pub win: bool,
    pub vk: u32,
}

impl Default for Shortcut {
    fn default() -> Self {
        Shortcut::DEFAULT
    }
}

impl Shortcut {
    pub const DEFAULT: Shortcut = Shortcut {
        ctrl: true,
        alt: true,
        shift: false,
        win: false,
        vk: VK_SPACE,
    };

    pub const fn ctrl_alt(vk: u32) -> Shortcut {
        Shortcut {
            ctrl: true,
            alt: true,
            shift: false,
            win: false,
            vk,
        }
    }

    /// The keycaps in order: "Ctrl", "Alt", "Space".
    pub fn parts(&self) -> Vec<String> {
        let mut v: Vec<String> = Vec::new();
        for (on, name) in [
            (self.ctrl, "Ctrl"),
            (self.alt, "Alt"),
            (self.shift, "Shift"),
            (self.win, "Win"),
        ] {
            if on {
                v.push(name.into());
            }
        }
        v.push(key_name(self.vk).unwrap_or_else(|| format!("Key {}", self.vk)));
        v
    }

    pub fn label(&self) -> String {
        self.parts().join("+")
    }

    pub fn parse(s: &str) -> Option<Shortcut> {
        let mut k = Shortcut {
            ctrl: false,
            alt: false,
            shift: false,
            win: false,
            vk: 0,
        };
        let parts: Vec<&str> = s.split('+').map(str::trim).collect();
        let (key, mods) = parts.split_last()?;
        for m in mods {
            match m.to_ascii_lowercase().as_str() {
                "ctrl" | "control" => k.ctrl = true,
                "alt" => k.alt = true,
                "shift" => k.shift = true,
                "win" => k.win = true,
                _ => return None,
            }
        }
        k.vk = vk_of(key)?;
        k.valid().then_some(k)
    }

    /// RegisterHotKey's modifier bits (MOD_ALT 1, MOD_CONTROL 2, MOD_SHIFT 4, MOD_WIN 8).
    pub fn mods(&self) -> u32 {
        (self.alt as u32)
            | (self.ctrl as u32) << 1
            | (self.shift as u32) << 2
            | (self.win as u32) << 3
    }

    /// Ctrl, Alt or Win with one key we can name. Shift alone would steal a capital letter.
    pub fn valid(&self) -> bool {
        (self.ctrl || self.alt || self.win) && key_name(self.vk).is_some()
    }
}

impl From<Shortcut> for String {
    fn from(s: Shortcut) -> String {
        s.label()
    }
}

impl TryFrom<String> for Shortcut {
    type Error = String;
    fn try_from(s: String) -> Result<Shortcut, String> {
        Shortcut::parse(&s).ok_or_else(|| format!("not a shortcut: {s}"))
    }
}

const NAMED: [(u32, &str); 22] = [
    (0x20, "Space"),
    (0x21, "Page Up"),
    (0x22, "Page Down"),
    (0x23, "End"),
    (0x24, "Home"),
    (0x25, "Left"),
    (0x26, "Up"),
    (0x27, "Right"),
    (0x28, "Down"),
    (0x2D, "Insert"),
    (0x2E, "Delete"),
    (0xBA, ";"),
    (0xBB, "="),
    (0xBC, ","),
    (0xBD, "-"),
    (0xBE, "."),
    (0xBF, "/"),
    (0xC0, "`"),
    (0xDB, "["),
    (0xDC, "\\"),
    (0xDD, "]"),
    (0xDE, "'"),
];

/// The keycap for a key a shortcut may end in: letters, digits, F1 to F24, Space, the arrows and
/// the punctuation keys. Enter, Tab, Esc and Backspace are not offered: they belong to the window.
pub fn key_name(vk: u32) -> Option<String> {
    match vk {
        0x30..=0x39 | 0x41..=0x5A => char::from_u32(vk).map(|c| c.to_string()),
        0x70..=0x87 => Some(format!("F{}", vk - 0x6F)),
        _ => NAMED
            .iter()
            .find(|(k, _)| *k == vk)
            .map(|(_, n)| n.to_string()),
    }
}

fn vk_of(name: &str) -> Option<u32> {
    let up = name.to_ascii_uppercase();
    if up.len() == 1 {
        let c = up.as_bytes()[0] as u32;
        if (0x30..=0x39).contains(&c) || (0x41..=0x5A).contains(&c) {
            return Some(c);
        }
    }
    if let Some(n) = up
        .strip_prefix('F')
        .and_then(|d| d.parse::<u32>().ok())
        .filter(|n| (1..=24).contains(n))
    {
        return Some(0x6F + n);
    }
    NAMED
        .iter()
        .find(|(_, n)| n.eq_ignore_ascii_case(name))
        .map(|(k, _)| *k)
}

/// What a key did while the shortcut is being changed.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum CaptureKey {
    /// A modifier went down or up; the keycaps show what is held.
    Held,
    /// A whole shortcut: Save can take it.
    Got,
    /// A key without Ctrl, Alt or Win.
    NeedsModifier,
    Cancel,
    Ignored,
}

/// Changing the shortcut: the modifiers held so far and the last whole shortcut pressed. It
/// tracks the modifiers from the key messages themselves, so it works the same for real keys and
/// for posted ones.
#[derive(Clone, Copy, PartialEq, Eq, Debug, Default)]
pub struct Capture {
    pub ctrl: bool,
    pub alt: bool,
    pub shift: bool,
    pub win: bool,
    pub got: Option<Shortcut>,
    pub needs_modifier: bool,
}

impl Capture {
    pub fn key(&mut self, vk: u32, down: bool) -> CaptureKey {
        match vk {
            0x11 | 0xA2 | 0xA3 => self.ctrl = down,
            0x12 | 0xA4 | 0xA5 => self.alt = down,
            0x10 | 0xA0 | 0xA1 => self.shift = down,
            0x5B | 0x5C => self.win = down,
            VK_ESCAPE if down => return CaptureKey::Cancel,
            _ if !down || key_name(vk).is_none() => return CaptureKey::Ignored,
            _ => {
                let s = Shortcut {
                    ctrl: self.ctrl,
                    alt: self.alt,
                    shift: self.shift,
                    win: self.win,
                    vk,
                };
                if s.valid() {
                    self.got = Some(s);
                    self.needs_modifier = false;
                    return CaptureKey::Got;
                }
                self.needs_modifier = true;
                return CaptureKey::NeedsModifier;
            }
        }
        CaptureKey::Held
    }

    /// The keycaps to show: the whole shortcut once pressed, else the modifiers held now.
    pub fn caps(&self) -> (Vec<String>, bool) {
        if let Some(s) = self.got {
            return (s.parts(), true);
        }
        let mut v = Vec::new();
        for (on, name) in [
            (self.ctrl, "Ctrl"),
            (self.alt, "Alt"),
            (self.shift, "Shift"),
            (self.win, "Win"),
        ] {
            if on {
                v.push(name.to_string());
            }
        }
        (v, false)
    }

    pub fn hint(&self) -> &'static str {
        if self.needs_modifier {
            "Add Ctrl, Alt or Win to that key."
        } else {
            "Hold Ctrl, Alt or Win, then one more key."
        }
    }
}

/// Shortcuts the first run offers when Ctrl+Alt+Space is taken, in order.
pub const SUGGESTIONS: [Shortcut; 4] = [
    Shortcut::ctrl_alt(0x4B),
    Shortcut::ctrl_alt(0x4A),
    Shortcut::ctrl_alt(0x4F),
    Shortcut {
        ctrl: true,
        alt: false,
        shift: true,
        win: false,
        vk: VK_SPACE,
    },
];

// ---- the place ----

pub struct City {
    pub name: &'static str,
    pub region: &'static str,
    pub lat: f64,
    pub lng: f64,
}

const fn c(name: &'static str, region: &'static str, lat: f64, lng: f64) -> City {
    City {
        name,
        region,
        lat,
        lng,
    }
}

/// Provincial capitals and the big kota and kabupaten seats, west to east. Yogyakarta keeps the
/// exact point the Kemenag calibration was done on.
pub const CITIES: [City; 54] = [
    c("Banda Aceh", "Aceh", 5.55, 95.32),
    c("Medan", "Sumatera Utara", 3.59, 98.67),
    c("Padang", "Sumatera Barat", -0.95, 100.35),
    c("Pekanbaru", "Riau", 0.51, 101.45),
    c("Batam", "Kepulauan Riau", 1.13, 104.05),
    c("Tanjung Pinang", "Kepulauan Riau", 0.92, 104.45),
    c("Jambi", "Jambi", -1.61, 103.61),
    c("Palembang", "Sumatera Selatan", -2.98, 104.76),
    c("Bengkulu", "Bengkulu", -3.80, 102.27),
    c("Pangkal Pinang", "Bangka Belitung", -2.13, 106.11),
    c("Bandar Lampung", "Lampung", -5.43, 105.26),
    c("Serang", "Banten", -6.12, 106.15),
    c("Tangerang", "Banten", -6.18, 106.63),
    c("Jakarta", "DKI Jakarta", -6.21, 106.85),
    c("Bogor", "Jawa Barat", -6.60, 106.80),
    c("Depok", "Jawa Barat", -6.40, 106.82),
    c("Bekasi", "Jawa Barat", -6.24, 106.99),
    c("Bandung", "Jawa Barat", -6.91, 107.61),
    c("Cirebon", "Jawa Barat", -6.71, 108.56),
    c("Tasikmalaya", "Jawa Barat", -7.33, 108.22),
    c("Purwokerto", "Jawa Tengah", -7.42, 109.23),
    c("Semarang", "Jawa Tengah", -6.99, 110.42),
    c("Magelang", "Jawa Tengah", -7.47, 110.22),
    c("Surakarta", "Jawa Tengah", -7.57, 110.82),
    c("Yogyakarta", "DIY", -7.7956, 110.3695),
    c("Sleman", "DIY", -7.72, 110.36),
    c("Bantul", "DIY", -7.89, 110.33),
    c("Surabaya", "Jawa Timur", -7.25, 112.75),
    c("Malang", "Jawa Timur", -7.98, 112.63),
    c("Kediri", "Jawa Timur", -7.82, 112.01),
    c("Jember", "Jawa Timur", -8.17, 113.70),
    c("Denpasar", "Bali", -8.65, 115.22),
    c("Mataram", "NTB", -8.58, 116.12),
    c("Kupang", "NTT", -10.16, 123.61),
    c("Pontianak", "Kalimantan Barat", -0.03, 109.33),
    c("Palangka Raya", "Kalimantan Tengah", -2.21, 113.92),
    c("Banjarmasin", "Kalimantan Selatan", -3.32, 114.59),
    c("Samarinda", "Kalimantan Timur", -0.50, 117.15),
    c("Balikpapan", "Kalimantan Timur", -1.27, 116.83),
    c("Tanjung Selor", "Kalimantan Utara", 2.84, 117.37),
    c("Manado", "Sulawesi Utara", 1.47, 124.84),
    c("Gorontalo", "Gorontalo", 0.54, 123.06),
    c("Palu", "Sulawesi Tengah", -0.90, 119.87),
    c("Mamuju", "Sulawesi Barat", -2.68, 118.89),
    c("Makassar", "Sulawesi Selatan", -5.15, 119.43),
    c("Kendari", "Sulawesi Tenggara", -3.97, 122.51),
    c("Ambon", "Maluku", -3.70, 128.18),
    c("Ternate", "Maluku Utara", 0.79, 127.38),
    c("Sofifi", "Maluku Utara", 0.74, 127.56),
    c("Manokwari", "Papua Barat", -0.86, 134.06),
    c("Sorong", "Papua Barat Daya", -0.88, 131.26),
    c("Jayapura", "Papua", -2.53, 140.72),
    c("Merauke", "Papua Selatan", -8.49, 140.40),
    c("Nabire", "Papua Tengah", -3.37, 135.50),
];

pub fn find_city(name: &str) -> Option<&'static City> {
    CITIES.iter().find(|c| c.name.eq_ignore_ascii_case(name))
}

/// Cities whose name starts with the query first, then those that only contain it (in the name
/// or the province), each group west to east.
pub fn filter_cities(query: &str) -> Vec<usize> {
    let q = query.trim().to_lowercase();
    if q.is_empty() {
        return (0..CITIES.len()).collect();
    }
    let mut starts = Vec::new();
    let mut contains = Vec::new();
    for (i, c) in CITIES.iter().enumerate() {
        let (n, r) = (c.name.to_lowercase(), c.region.to_lowercase());
        if n.starts_with(&q) {
            starts.push(i);
        } else if n.contains(&q) || r.contains(&q) {
            contains.push(i);
        }
    }
    starts.extend(contains);
    starts
}

/// Great-circle distance in km.
pub fn distance_km(a: (f64, f64), b: (f64, f64)) -> f64 {
    let (la1, la2) = (a.0.to_radians(), b.0.to_radians());
    let (dla, dlo) = ((b.0 - a.0).to_radians(), (b.1 - a.1).to_radians());
    let h = (dla / 2.0).sin().powi(2) + la1.cos() * la2.cos() * (dlo / 2.0).sin().powi(2);
    2.0 * 6371.0 * h.sqrt().asin()
}

pub fn nearest_city(lat: f64, lng: f64) -> (&'static City, f64) {
    CITIES
        .iter()
        .map(|c| (c, distance_km((lat, lng), (c.lat, c.lng))))
        .min_by(|a, b| a.1.total_cmp(&b.1))
        .expect("cities")
}

/// "Yogyakarta, DIY" for a fix within 40 km of a listed city, else the coordinates.
pub fn near_words(lat: f64, lng: f64) -> String {
    let (c, km) = nearest_city(lat, lng);
    if km <= 40.0 {
        format!("{}, {}", c.name, c.region)
    } else {
        format!("{:.2}°, {:.2}°", lat, lng)
    }
}

/// Where prayer times are worked out for.
#[derive(Clone, PartialEq, Debug, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum Place {
    City {
        name: String,
    },
    /// The last fix from Windows location; it is read again at each launch.
    Windows {
        lat: f64,
        lng: f64,
    },
}

impl Default for Place {
    fn default() -> Self {
        Place::City {
            name: "Yogyakarta".into(),
        }
    }
}

impl Place {
    pub fn coords(&self) -> (f64, f64) {
        match self {
            Place::City { name } => find_city(name)
                .or_else(|| find_city("Yogyakarta"))
                .map(|c| (c.lat, c.lng))
                .unwrap_or((-7.7956, 110.3695)),
            Place::Windows { lat, lng } => (*lat, *lng),
        }
    }

    /// "Yogyakarta, DIY".
    pub fn words(&self) -> String {
        match self {
            Place::City { name } => find_city(name)
                .map(|c| format!("{}, {}", c.name, c.region))
                .unwrap_or_else(|| name.clone()),
            Place::Windows { lat, lng } => near_words(*lat, *lng),
        }
    }

    /// "Yogyakarta" (the Prayer panel's header).
    pub fn short(&self) -> String {
        self.words()
            .split(',')
            .next()
            .unwrap_or_default()
            .to_string()
    }

    pub fn is_windows(&self) -> bool {
        matches!(self, Place::Windows { .. })
    }
}

/// Windows location, as the Prayer section and the first run show it.
#[derive(Clone, Copy, PartialEq, Debug)]
pub enum Locate {
    Idle,
    Finding,
    Found {
        lat: f64,
        lng: f64,
    },
    /// Location services are off, or desktop apps may not use them.
    Off,
    /// No fix in time (no Wi-Fi nearby, a timeout).
    Failed,
}

impl Locate {
    /// The Windows location row's second line.
    pub fn words(self) -> String {
        match self {
            Locate::Idle => "Uses this PC's location".into(),
            Locate::Finding => "Finding your location…".into(),
            Locate::Found { lat, lng } => format!("{} · follows you", near_words(lat, lng)),
            Locate::Off => "Location is off in Windows".into(),
            Locate::Failed => "Couldn't find it just now".into(),
        }
    }
}

// ---- the prayer method and the sound at the time ----

/// Whose schedule the times follow.
#[derive(Clone, Copy, PartialEq, Eq, Debug, Default, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum PrayerMethod {
    #[default]
    Kemenag,
    Muhammadiyah,
}

impl PrayerMethod {
    pub const ALL: [PrayerMethod; 2] = [PrayerMethod::Kemenag, PrayerMethod::Muhammadiyah];

    pub fn name(self) -> &'static str {
        match self {
            PrayerMethod::Kemenag => "Kemenag RI",
            PrayerMethod::Muhammadiyah => "Muhammadiyah",
        }
    }

    /// The Prayer panel's header: "Yogyakarta · Kemenag".
    pub fn short(self) -> &'static str {
        match self {
            PrayerMethod::Kemenag => "Kemenag",
            PrayerMethod::Muhammadiyah => "Muhammadiyah",
        }
    }

    pub fn words(self) -> &'static str {
        match self {
            PrayerMethod::Kemenag => "Subuh 20°, Isya 18°, +2 min ihtiyat",
            PrayerMethod::Muhammadiyah => "Majelis Tarjih: Subuh 18°, Isya 18°",
        }
    }

    pub fn method(self) -> app_core::prayer::Method {
        match self {
            PrayerMethod::Kemenag => app_core::prayer::Method::KEMENAG,
            PrayerMethod::Muhammadiyah => app_core::prayer::Method::MUHAMMADIYAH,
        }
    }
}

/// What "Memasuki waktu" sounds like. Never a sound while full screen or presenting.
#[derive(Clone, Copy, PartialEq, Eq, Debug, Default, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum AtTimeSound {
    /// Diam: the pill only.
    Silent,
    /// Bunyi: Windows' short notification sound.
    #[default]
    Chime,
    /// Adzan: an audio file you choose (nothing is bundled or downloaded).
    Adzan,
}

impl AtTimeSound {
    pub const ALL: [AtTimeSound; 3] = [AtTimeSound::Silent, AtTimeSound::Chime, AtTimeSound::Adzan];

    pub fn name(self) -> &'static str {
        match self {
            AtTimeSound::Silent => "Diam",
            AtTimeSound::Chime => "Bunyi",
            AtTimeSound::Adzan => "Adzan",
        }
    }
}

/// The most a manual adjustment moves one prayer, either way.
pub const ADJUST_MAX: i32 = app_core::prayer::MAX_ADJUST_MIN;

/// "+2", "0", "−3".
pub fn adjust_words(m: i32) -> String {
    match m {
        0 => "0".into(),
        m if m > 0 => format!("+{m}"),
        m => format!("\u{2212}{}", -m),
    }
}

/// The file name a path ends in ("Adzan Makkah.mp3"). Splits on both separators, so a path
/// saved on Windows reads the same on macOS.
pub fn file_name(path: &str) -> String {
    path.trim_end_matches(['/', '\\'])
        .rsplit(['/', '\\'])
        .next()
        .filter(|n| !n.is_empty())
        .unwrap_or(path)
        .to_string()
}

/// Audio the adzan can play (winmm MCI opens these by extension).
pub fn is_audio(path: &str) -> bool {
    let ext = std::path::Path::new(path)
        .extension()
        .map(|e| e.to_string_lossy().to_ascii_lowercase());
    matches!(ext.as_deref(), Some("mp3" | "wav"))
}

// ---- what is kept ----

#[derive(Clone, PartialEq, Debug)]
pub struct Prefs {
    pub first_run_done: bool,
    pub show_orb_at_start: bool,
    pub theme: ThemeMode,
    pub shortcut: Shortcut,
    pub place: Place,
    /// −30 min, −10 min, at the time.
    pub prayer_reminders: [bool; 3],
    pub prayer_method: PrayerMethod,
    /// Minutes added to [subuh, dzuhur, ashar, maghrib, isya], −10 to +10 (a local mosque).
    pub prayer_adjust: [i32; 5],
    pub at_time_sound: AtTimeSound,
    /// The adzan file, and an optional one for Subuh (its adzan differs).
    pub adzan_file: Option<String>,
    pub adzan_subuh_file: Option<String>,
}

impl Default for Prefs {
    fn default() -> Self {
        Prefs {
            first_run_done: false,
            show_orb_at_start: true,
            theme: ThemeMode::Windows,
            shortcut: Shortcut::DEFAULT,
            place: Place::default(),
            prayer_reminders: [true, true, true],
            prayer_method: PrayerMethod::Kemenag,
            prayer_adjust: [0; 5],
            at_time_sound: AtTimeSound::Chime,
            adzan_file: None,
            adzan_subuh_file: None,
        }
    }
}

impl Prefs {
    /// The adzan for this prayer: Subuh's own file if there is one, else the main one.
    pub fn adzan_for(&self, subuh: bool) -> Option<&str> {
        let own = if subuh {
            self.adzan_subuh_file.as_deref()
        } else {
            None
        };
        own.or(self.adzan_file.as_deref())
    }
}

const KEYS: [&str; 11] = [
    "first_run_done",
    "show_orb_at_start",
    "theme",
    "shortcut",
    "place",
    "prayer_reminders",
    "prayer_method",
    "prayer_adjust",
    "at_time_sound",
    "adzan_file",
    "adzan_subuh_file",
];

fn get<T: DeserializeOwned>(m: &Map<String, Value>, key: &str) -> Option<T> {
    m.get(key)
        .and_then(|v| serde_json::from_value(v.clone()).ok())
}

impl Prefs {
    /// Each key on its own, so one bad value falls back to its default without resetting the
    /// rest (a reset `first_run_done` would show the first run again).
    pub fn from_map(m: &Map<String, Value>) -> Prefs {
        let d = Prefs::default();
        Prefs {
            first_run_done: get(m, "first_run_done").unwrap_or(d.first_run_done),
            show_orb_at_start: get(m, "show_orb_at_start").unwrap_or(d.show_orb_at_start),
            theme: get(m, "theme").unwrap_or(d.theme),
            shortcut: get(m, "shortcut").unwrap_or(d.shortcut),
            place: get(m, "place").unwrap_or(d.place),
            prayer_reminders: get(m, "prayer_reminders").unwrap_or(d.prayer_reminders),
            prayer_method: get(m, "prayer_method").unwrap_or(d.prayer_method),
            prayer_adjust: get::<[i32; 5]>(m, "prayer_adjust")
                .map(|a| a.map(|v| v.clamp(-ADJUST_MAX, ADJUST_MAX)))
                .unwrap_or(d.prayer_adjust),
            at_time_sound: get(m, "at_time_sound").unwrap_or(d.at_time_sound),
            adzan_file: get(m, "adzan_file").unwrap_or(d.adzan_file),
            adzan_subuh_file: get(m, "adzan_subuh_file").unwrap_or(d.adzan_subuh_file),
        }
    }

    /// Writes these keys into the file's map; other keys (the calendar link) stay as they are.
    pub fn write_into(&self, m: &mut Map<String, Value>) {
        let vals = [
            Value::Bool(self.first_run_done),
            Value::Bool(self.show_orb_at_start),
            serde_json::to_value(self.theme).unwrap_or(Value::Null),
            serde_json::to_value(self.shortcut).unwrap_or(Value::Null),
            serde_json::to_value(&self.place).unwrap_or(Value::Null),
            serde_json::to_value(self.prayer_reminders).unwrap_or(Value::Null),
            serde_json::to_value(self.prayer_method).unwrap_or(Value::Null),
            serde_json::to_value(self.prayer_adjust).unwrap_or(Value::Null),
            serde_json::to_value(self.at_time_sound).unwrap_or(Value::Null),
            serde_json::to_value(&self.adzan_file).unwrap_or(Value::Null),
            serde_json::to_value(&self.adzan_subuh_file).unwrap_or(Value::Null),
        ];
        for (k, v) in KEYS.iter().zip(vals) {
            m.insert((*k).to_string(), v);
        }
    }
}

// ---- the first run ----

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Step {
    Welcome,
    Place,
    Shortcut,
    Sync,
    Done,
}

impl Step {
    pub const ALL: [Step; 5] = [
        Step::Welcome,
        Step::Place,
        Step::Shortcut,
        Step::Sync,
        Step::Done,
    ];

    pub fn index(self) -> usize {
        Step::ALL.iter().position(|s| *s == self).unwrap_or(0)
    }

    pub fn next(self) -> Step {
        Step::ALL[(self.index() + 1).min(4)]
    }

    pub fn back(self) -> Step {
        Step::ALL[self.index().saturating_sub(1)]
    }
}

/// Continue on the place step waits for a place: a Windows fix, or the city picked.
pub fn place_ready(want_windows: bool, locate: Locate) -> bool {
    !want_windows || matches!(locate, Locate::Found { .. })
}

// ---- the Settings window ----

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Section {
    General,
    Prayer,
    Notices,
    Account,
    About,
}

impl Section {
    pub const ALL: [Section; 5] = [
        Section::General,
        Section::Prayer,
        Section::Notices,
        Section::Account,
        Section::About,
    ];

    pub fn name(self) -> &'static str {
        match self {
            Section::General => "General",
            Section::Prayer => "Prayer",
            Section::Notices => "Notices",
            Section::Account => "Account",
            Section::About => "About",
        }
    }
}

/// HKCU Run, as the Startup row reads it.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Startup {
    On,
    Off,
    /// Our Run entry is there but switched off in Task Manager or Startup apps.
    Blocked,
}

pub const REMINDERS: [&str; 3] = ["30 min before", "10 min before", "At the time"];

/// "Today, Sat 7 Nov" (weekday 0 = Sunday).
pub fn day_words(weekday: u32, day: u32, month: u32) -> String {
    const DAYS: [&str; 7] = ["Sun", "Mon", "Tue", "Wed", "Thu", "Fri", "Sat"];
    const MONTHS: [&str; 12] = [
        "Jan", "Feb", "Mar", "Apr", "May", "Jun", "Jul", "Aug", "Sep", "Oct", "Nov", "Dec",
    ];
    format!(
        "Today, {} {} {}",
        DAYS[(weekday % 7) as usize],
        day,
        MONTHS[(month.clamp(1, 12) - 1) as usize]
    )
}

/// "8,344".
pub fn thousands(n: u64) -> String {
    let d = n.to_string();
    let mut out = String::new();
    for (i, c) in d.chars().enumerate() {
        if i > 0 && (d.len() - i).is_multiple_of(3) {
            out.push(',');
        }
        out.push(c);
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn distances_read_with_thousands() {
        assert_eq!(thousands(8344), "8,344");
        assert_eq!(thousands(950), "950");
        assert_eq!(thousands(1_234_567), "1,234,567");
    }

    #[test]
    fn a_new_install_runs_the_first_run_and_an_old_file_keeps_its_other_keys() {
        let mut m: Map<String, Value> =
            serde_json::from_str(r#"{"calendar_link":"https://example.com/a.ics"}"#).unwrap();
        let p = Prefs::from_map(&m);
        assert!(!p.first_run_done);
        assert_eq!(p, Prefs::default());
        let done = Prefs {
            first_run_done: true,
            theme: ThemeMode::Dark,
            shortcut: Shortcut::ctrl_alt(0x4B),
            place: Place::City {
                name: "Makassar".into(),
            },
            prayer_reminders: [false, true, true],
            prayer_method: PrayerMethod::Muhammadiyah,
            prayer_adjust: [2, 0, 0, 1, -3],
            at_time_sound: AtTimeSound::Adzan,
            adzan_file: Some(r"C:\Users\me\Music\adzan.mp3".into()),
            ..p
        };
        done.write_into(&mut m);
        assert_eq!(m["calendar_link"], "https://example.com/a.ics");
        assert_eq!(m["shortcut"], "Ctrl+Alt+K");
        let text = serde_json::to_string(&m).unwrap();
        let back = Prefs::from_map(&serde_json::from_str(&text).unwrap());
        assert_eq!(back, done);
    }

    #[test]
    fn prayer_choices_default_to_kemenag_and_bunyi_and_adjustments_are_clamped() {
        let p = Prefs::default();
        assert_eq!(p.prayer_method, PrayerMethod::Kemenag);
        assert_eq!(p.at_time_sound, AtTimeSound::Chime);
        assert_eq!(p.at_time_sound.name(), "Bunyi");
        let m: Map<String, Value> = serde_json::from_str(
            r#"{"prayer_method":"muhammadiyah","prayer_adjust":[30,-2,0,0,-99],"at_time_sound":"adzan","adzan_file":"a.mp3","adzan_subuh_file":"s.wav"}"#,
        )
        .unwrap();
        let p = Prefs::from_map(&m);
        assert_eq!(p.prayer_method, PrayerMethod::Muhammadiyah);
        assert_eq!(p.prayer_adjust, [10, -2, 0, 0, -10]);
        assert_eq!(p.adzan_for(true), Some("s.wav"));
        assert_eq!(p.adzan_for(false), Some("a.mp3"));
        let only_main = Prefs {
            adzan_subuh_file: None,
            ..p
        };
        assert_eq!(
            only_main.adzan_for(true),
            Some("a.mp3"),
            "Subuh falls back to the main file"
        );
        assert_eq!(adjust_words(2), "+2");
        assert_eq!(adjust_words(-3), "\u{2212}3");
        assert_eq!(file_name(r"C:\Music\Adzan Makkah.mp3"), "Adzan Makkah.mp3");
        assert!(is_audio("x.MP3") && is_audio("y.wav") && !is_audio("z.ogg"));
    }

    #[test]
    fn one_bad_value_does_not_show_the_first_run_again() {
        let m: Map<String, Value> = serde_json::from_str(
            r#"{"first_run_done":true,"shortcut":"Ctrl+Banana","theme":"purple"}"#,
        )
        .unwrap();
        let p = Prefs::from_map(&m);
        assert!(p.first_run_done);
        assert_eq!(p.shortcut, Shortcut::DEFAULT);
        assert_eq!(p.theme, ThemeMode::Windows);
    }

    #[test]
    fn shortcuts_read_and_write_as_their_keycaps() {
        assert_eq!(Shortcut::DEFAULT.label(), "Ctrl+Alt+Space");
        assert_eq!(Shortcut::DEFAULT.mods(), 3);
        for s in [
            "Ctrl+Alt+K",
            "Ctrl+Shift+Space",
            "Win+Alt+F9",
            "Alt+;",
            "Ctrl+Alt+Page Down",
            "Ctrl+7",
        ] {
            let k = Shortcut::parse(s).unwrap_or_else(|| panic!("{s}"));
            assert_eq!(Shortcut::parse(&k.label()), Some(k), "{s}");
        }
        assert_eq!(Shortcut::parse("ctrl+alt+k").unwrap().label(), "Ctrl+Alt+K");
        assert_eq!(
            Shortcut::parse("Shift+K"),
            None,
            "Shift alone would take a capital letter"
        );
        assert_eq!(Shortcut::parse("Ctrl+Enter"), None);
        assert_eq!(Shortcut::parse("K"), None);
    }

    #[test]
    fn changing_the_shortcut_needs_a_modifier_and_esc_cancels() {
        let mut c = Capture::default();
        assert_eq!(c.key(0x4B, true), CaptureKey::NeedsModifier);
        assert_eq!(c.hint(), "Add Ctrl, Alt or Win to that key.");
        assert_eq!(c.key(0xA2, true), CaptureKey::Held);
        assert_eq!(c.key(0x12, true), CaptureKey::Held);
        assert_eq!(
            c.caps(),
            (vec!["Ctrl".to_string(), "Alt".to_string()], false)
        );
        assert_eq!(
            c.key(0x0D, true),
            CaptureKey::Ignored,
            "Enter is not a shortcut key"
        );
        assert_eq!(c.key(0x4A, true), CaptureKey::Got);
        assert_eq!(c.got.unwrap().label(), "Ctrl+Alt+J");
        // Letting go keeps what was pressed, so Save can take it.
        c.key(0x12, false);
        c.key(0xA2, false);
        assert_eq!(c.caps().0.join("+"), "Ctrl+Alt+J");
        assert_eq!(c.key(0x1B, true), CaptureKey::Cancel);
    }

    #[test]
    fn cities_filter_by_name_first_then_by_province() {
        let names = |q: &str| {
            filter_cities(q)
                .into_iter()
                .map(|i| CITIES[i].name)
                .collect::<Vec<_>>()
        };
        assert_eq!(names("yog"), vec!["Yogyakarta"]);
        let diy = names("diy");
        assert_eq!(diy, vec!["Yogyakarta", "Sleman", "Bantul"]);
        assert_eq!(names("ma")[..3], ["Magelang", "Malang", "Mataram"]);
        assert!(names("ma").contains(&"Palembang") || names("ma").contains(&"Semarang"));
        assert_eq!(names("  ").len(), CITIES.len());
        assert!(names("xyz").is_empty());
    }

    #[test]
    fn a_windows_fix_is_named_after_the_nearest_city() {
        assert_eq!(near_words(-7.80, 110.37), "Yogyakarta, DIY");
        assert_eq!(nearest_city(-6.2, 106.85).0.name, "Jakarta");
        assert_eq!(near_words(-20.0, 110.0), "-20.00°, 110.00°");
        assert!((distance_km((-7.7956, 110.3695), (-6.21, 106.85)) - 427.0).abs() < 10.0);
        let p = Place::Windows {
            lat: -7.79,
            lng: 110.36,
        };
        assert_eq!(p.short(), "Yogyakarta");
        assert_eq!(Place::default().coords(), (-7.7956, 110.3695));
        assert_eq!(
            Place::City {
                name: "Nowhere".into()
            }
            .coords(),
            (-7.7956, 110.3695)
        );
    }

    #[test]
    fn the_first_run_steps_go_forward_and_back_and_wait_for_a_place() {
        assert_eq!(Step::Welcome.next(), Step::Place);
        assert_eq!(Step::Done.next(), Step::Done);
        assert_eq!(Step::Welcome.back(), Step::Welcome);
        assert_eq!(Step::Sync.back(), Step::Shortcut);
        assert_eq!(Step::Done.index(), 4);
        assert!(!place_ready(true, Locate::Finding));
        assert!(!place_ready(true, Locate::Off));
        assert!(place_ready(true, Locate::Found { lat: 0.0, lng: 0.0 }));
        assert!(place_ready(false, Locate::Off));
    }

    #[test]
    fn words_stay_plain() {
        assert_eq!(
            ThemeMode::Windows.words(true),
            "Follows Windows. Dark right now."
        );
        assert_eq!(day_words(6, 7, 11), "Today, Sat 7 Nov");
        assert_eq!(Locate::Off.words(), "Location is off in Windows");
        for w in [
            Locate::Finding.words(),
            Locate::Found {
                lat: -7.8,
                lng: 110.37,
            }
            .words(),
            Capture::default().hint().to_string(),
        ] {
            assert!(!w.contains('\u{2014}'), "no em dash: {w}");
        }
    }
}
